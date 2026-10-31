package es.spectral.menu.agent;

import java.util.ArrayDeque;
import java.util.Deque;
import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;

import org.lwjgl.input.Keyboard;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.settings.KeyBinding;

import es.spectral.menu.KeyNames;
import es.spectral.menu.mixin.agent.GuiScreenInvoker;

/**
 * Synthetic input for the act tools. Screens get the calls LWJGL input would
 * produce ({@code mouseClicked}/{@code mouseReleased}, {@code keyTyped}). In
 * the world, keys become synthetic keyboard events that
 * {@code Minecraft.runTick} reads through {@code MinecraftKeyboardMixin}
 * exactly like real ones — so bindings, F1/F3/F5, Esc and Forge key events
 * all behave as if typed. {@code press} = down + up, {@code down} holds the
 * key until {@code up} or a {@link #LEASE_MS} lease expires (a lost
 * {@code up} can never leave a key stuck). Synthetic events are only
 * delivered while no screen is open (screens read the real LWJGL event);
 * releases pending when a screen opens are applied to the bindings directly.
 */
public final class AgentInput {

    public static final long LEASE_MS = 10_000L;

    private static final class Event {
        final int code;
        final boolean down;
        final char character;

        Event(int code, boolean down, char character) {
            this.code = code;
            this.down = down;
            this.character = character;
        }
    }

    private static final Deque<Event> QUEUE = new ArrayDeque<Event>();
    private static final Map<Integer, Long> HELD = new HashMap<Integer, Long>();
    private static Event current;

    private AgentInput() {}

    /** Key code from a {@code key.keyboard.*} name or short form (e, escape, f3), or -1. */
    public static int resolveKey(String raw) {
        return KeyNames.keyCode(raw);
    }

    public static void click(GuiScreen screen, int x, int y, int button, boolean doubleClick) {
        GuiScreenInvoker invoker = (GuiScreenInvoker) screen;
        int times = doubleClick ? 2 : 1;
        for (int i = 0; i < times; i++) {
            invoker.espectral$mouseClicked(x, y, button);
            invoker.espectral$mouseReleased(x, y, button);
        }
    }

    /** One key press: typed into the open screen, else a synthetic down + up. */
    public static void pressKey(Minecraft mc, int code) {
        if (mc.currentScreen != null) {
            ((GuiScreenInvoker) mc.currentScreen).espectral$keyTyped(charFor(code), code);
            return;
        }
        QUEUE.add(new Event(code, true, charFor(code)));
        QUEUE.add(new Event(code, false, (char) 0));
    }

    public static void keyDown(Minecraft mc, int code) {
        if (mc.currentScreen != null) {
            ((GuiScreenInvoker) mc.currentScreen).espectral$keyTyped(charFor(code), code);
        } else if (!HELD.containsKey(code)) {
            QUEUE.add(new Event(code, true, charFor(code)));
        }
        HELD.put(code, System.currentTimeMillis() + LEASE_MS);
    }

    public static void keyUp(int code) {
        if (HELD.remove(code) != null) QUEUE.add(new Event(code, false, (char) 0));
    }

    public static void typeText(GuiScreen screen, String text) {
        GuiScreenInvoker invoker = (GuiScreenInvoker) screen;
        for (int i = 0; i < text.length(); i++) {
            invoker.espectral$keyTyped(text.charAt(i), 0);
        }
    }

    /** Client tick: expire leases. */
    public static void onTick() {
        long now = System.currentTimeMillis();
        for (Iterator<Map.Entry<Integer, Long>> it = HELD.entrySet().iterator(); it.hasNext();) {
            Map.Entry<Integer, Long> e = it.next();
            if (e.getValue() < now) {
                QUEUE.add(new Event(e.getKey(), false, (char) 0));
                it.remove();
            }
        }
    }

    /** Releases everything the agent holds (disconnect). */
    public static void releaseAll() {
        for (Integer code : HELD.keySet()) KeyBinding.setKeyBindState(code, false);
        HELD.clear();
        QUEUE.clear();
        current = null;
    }

    // --- MinecraftKeyboardMixin (client thread, inside runTick's keyboard loop) ---

    /** Advances to the next synthetic event; false when none should be delivered now. */
    public static boolean nextSynthetic(Minecraft mc) {
        current = null;
        if (QUEUE.isEmpty()) return false;
        if (mc.currentScreen != null) {
            for (Event e : QUEUE) {
                if (!e.down) KeyBinding.setKeyBindState(e.code, false);
            }
            QUEUE.clear();
            return false;
        }
        current = QUEUE.poll();
        return true;
    }

    public static boolean hasCurrent() {
        return current != null;
    }

    public static int eventKey() {
        return current != null ? current.code : Keyboard.getEventKey();
    }

    public static boolean eventKeyState() {
        return current != null ? current.down : Keyboard.getEventKeyState();
    }

    public static char eventCharacter() {
        return current != null ? current.character : Keyboard.getEventCharacter();
    }

    /** The character a key types on a US layout (letters, digits, space), else 0. */
    private static char charFor(int code) {
        String name = Keyboard.getKeyName(code);
        if (name != null && name.length() == 1) return Character.toLowerCase(name.charAt(0));
        return code == Keyboard.KEY_SPACE ? ' ' : (char) 0;
    }
}
