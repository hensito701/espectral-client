package es.spectral.menu.agent;

import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

import com.mojang.blaze3d.platform.InputConstants;

import net.minecraft.client.Minecraft;

import es.spectral.menu.Keys;

/**
 * Agent input: mouse and keyboard events routed through the vanilla handlers
 * (so hover logic, double-click detection, focus and screen routing are the
 * real ones), plus the held-key lease bookkeeping the contract requires.
 *
 * <p>Rules enforced here:
 * <ul>
 *   <li>{@code key action=down} holds a key for at most 2 s unless the agent
 *       sends another {@code down} (each renews the lease).</li>
 *   <li>Opening a screen or a world change releases every held key.</li>
 *   <li>Injected events never toggle the agent pause: {@link AgentPause} reads
 *       the pause binding from {@code KeyMapping} state, which only real input
 *       sets (injected keys go through the handler, not through the binding,
 *       and the pause check runs before dispatch).</li>
 * </ul>
 */
public final class AgentInput {

    private static final org.slf4j.Logger LOGGER =
            org.slf4j.LoggerFactory.getLogger("espectral-client");

    /** Held-key lease (contract: 2 s, renewed by another `down`). */
    public static final long LEASE_MS = 2_000L;

    private static final class Held {
        long expiresAt;

        Held(long expiresAt) {
            this.expiresAt = expiresAt;
        }
    }

    /** key value -> lease; insertion-ordered so the sweep is cheap. */
    private static final Map<Integer, Held> HELD = new LinkedHashMap<>();

    private AgentInput() {}

    /** Releases keys whose lease expired (called from the client tick). */
    public static void tick(Minecraft minecraft, long now) {
        if (HELD.isEmpty() || minecraft == null) return;
        List<Integer> expired = null;
        Iterator<Map.Entry<Integer, Held>> it = HELD.entrySet().iterator();
        while (it.hasNext()) {
            Map.Entry<Integer, Held> entry = it.next();
            if (entry.getValue().expiresAt <= now) {
                if (expired == null) expired = new ArrayList<>(2);
                expired.add(entry.getKey());
                it.remove();
            }
        }
        if (expired == null) return;
        for (int key : expired) {
            AgentInputCompat.key(minecraft, key, InputConstants.RELEASE, 0);
            LOGGER.info("agent: key {} auto-released (lease expired)", key);
        }
    }

    /** Releases every held key (screen change / world change / disconnect). */
    public static void releaseAll(Minecraft minecraft) {
        if (HELD.isEmpty()) return;
        List<Integer> keys = new ArrayList<>(HELD.keySet());
        HELD.clear();
        if (minecraft == null) return;
        for (int key : keys) {
            try {
                AgentInputCompat.key(minecraft, key, InputConstants.RELEASE, 0);
                LOGGER.info("agent: key {} released (screen or world change)", key);
            } catch (Throwable t) {
                // Input is best-effort: a failing release must never crash.
            }
        }
    }

    // ------------------------------------------------------------------
    // Mouse
    // ------------------------------------------------------------------

    /**
     * Moves the pointer to window-pixel coordinates, then presses and releases
     * the given button. {@code doubleClick} emits two real click pairs so
     * vanilla's own 250 ms double-click detection sees them.
     */
    public static void click(Minecraft minecraft, double windowX, double windowY,
            int button, boolean doubleClick) {
        AgentInputCompat.moveMouse(minecraft, windowX, windowY);
        int pairs = doubleClick ? 2 : 1;
        for (int i = 0; i < pairs; i++) {
            AgentInputCompat.mouseButton(minecraft, button, InputConstants.PRESS);
            AgentInputCompat.mouseButton(minecraft, button, InputConstants.RELEASE);
        }
    }

    // ------------------------------------------------------------------
    // Keyboard
    // ------------------------------------------------------------------

    /** Press = down + up. */
    public static void pressKey(Minecraft minecraft, int key, int modifiers) {
        AgentInputCompat.key(minecraft, key, InputConstants.PRESS, modifiers);
        AgentInputCompat.key(minecraft, key, InputConstants.RELEASE, modifiers);
    }

    /** Down, holding a lease until {@code now + LEASE_MS}. */
    public static void keyDown(Minecraft minecraft, int key, int modifiers, long now) {
        AgentInputCompat.key(minecraft, key, InputConstants.PRESS, modifiers);
        HELD.put(key, new Held(now + LEASE_MS));
    }

    /** Up, dropping the lease if it was held. */
    public static void keyUp(Minecraft minecraft, int key, int modifiers) {
        HELD.remove(key);
        AgentInputCompat.key(minecraft, key, InputConstants.RELEASE, modifiers);
    }

    /** Types {@code text} one character event per code point. */
    public static void typeText(Minecraft minecraft, String text) {
        text.codePoints().forEach(cp -> AgentInputCompat.charTyped(minecraft, cp));
    }

    /**
     * Resolves a key argument: a vanilla {@code key.keyboard.*} name, a bare
     * short form ({@code e}, {@code escape}, {@code f3}, {@code space},
     * {@code left.shift}), or a single character.
     *
     * @return the key value, or -1 when unknown
     */
    public static int resolveKey(String raw) {
        if (raw == null) return -1;
        String name = raw.trim().toLowerCase(Locale.ROOT);
        if (name.isEmpty()) return -1;
        if (name.startsWith("key.")) {
            return valueOrMinusOne(name);
        }
        int direct = valueOrMinusOne("key.keyboard." + name);
        if (direct != -1) return direct;
        // A layout-specific symbol may have no name entry: fall back to the
        // platform label lookup (the same one vanilla uses for labels).
        int count = name.codePointCount(0, name.length());
        if (count == 1) {
            int cp = name.codePointAt(0);
            int code = Keys.forCharacter(Character.toUpperCase(cp));
            if (code != -1) return code;
            code = Keys.forCharacter(cp);
            if (code != -1) return code;
        }
        return -1;
    }

    private static int valueOrMinusOne(String name) {
        try {
            return InputConstants.getKey(name).getValue();
        } catch (Exception e) {
            return -1;
        }
    }
}
