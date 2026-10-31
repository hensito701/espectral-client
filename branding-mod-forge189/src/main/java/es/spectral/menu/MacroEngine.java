package es.spectral.menu;

import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Set;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.lwjgl.input.Keyboard;

import net.minecraft.client.Minecraft;
import net.minecraft.client.settings.KeyBinding;
import net.minecraftforge.client.ClientCommandHandler;

/**
 * Macro execution (same contract as the Fabric mod): chat/command macros
 * from {@code espectral-client.json}, triggered by their keybind with a
 * per-key repeat guard, only in game (no screen open) and only while the
 * {@code macros} feature is on. A leading {@code /} runs as a command —
 * Forge client commands first, then the server — anything else is chat, so a
 * macro can never reach a path the player could not type by hand.
 */
public final class MacroEngine {

    private static final Logger LOGGER = LogManager.getLogger("espectral-macros");

    private static final Set<Integer> pressed = new HashSet<Integer>();
    private static final Set<String> warnedUnparseable = new HashSet<String>();

    private MacroEngine() {}

    /** Client tick. */
    public static void onTick(Minecraft mc) {
        for (Iterator<Integer> it = pressed.iterator(); it.hasNext();) {
            if (!Keyboard.isKeyDown(it.next())) it.remove();
        }
        if (mc.currentScreen != null || mc.thePlayer == null
                || !ClientConfig.getInstance().isFeatureEnabled("macros")) {
            return;
        }
        Set<Integer> newlyPressed = new HashSet<Integer>();
        Set<String> seen = new HashSet<String>();
        for (ClientConfig.MacroConfig macro : ClientConfig.getInstance().getMacros()) {
            seen.add(macro.id);
            int code = KeyNames.keyCode(macro.keybind);
            if (code <= 0) {
                if (warnedUnparseable.add(macro.id)) {
                    LOGGER.warn("Ignoring macro '{}' (id '{}'): unparseable keybind '{}'", macro.name, macro.id, macro.keybind);
                }
                continue;
            }
            warnedUnparseable.remove(macro.id);
            if (Keyboard.isKeyDown(code) && !pressed.contains(code) && newlyPressed.add(code)) {
                execute(mc, macro);
            }
        }
        warnedUnparseable.retainAll(seen);
        pressed.addAll(newlyPressed);
    }

    private static void execute(Minecraft mc, ClientConfig.MacroConfig macro) {
        LOGGER.info("Executing macro: {} ({})", macro.name, macro.id);
        for (ClientConfig.MacroAction action : macro.actions) {
            String text = action.text;
            if (text == null || text.isEmpty()) continue;
            if ("command".equalsIgnoreCase(action.type) || text.startsWith("/")) {
                sendChat(mc, text.startsWith("/") ? text : "/" + text);
            } else {
                sendChat(mc, text);
            }
        }
    }

    /** Sends like the chat screen does: Forge client commands, else to the server. */
    public static void sendChat(Minecraft mc, String text) {
        if (mc.thePlayer == null) return;
        if (text.startsWith("/") && ClientCommandHandler.instance.executeCommand(mc.thePlayer, text) != 0) {
            return;
        }
        mc.thePlayer.sendChatMessage(text);
    }

    /** Esc always closes screens and Right Shift opens the Suite. */
    public static boolean isReserved(int code) {
        return code == Keyboard.KEY_ESCAPE || code == Keyboard.KEY_RSHIFT;
    }

    /**
     * The binding already using {@code keybind}, or null: another macro
     * ({@code fromMacro}, refused by the editor), a vanilla/mod key binding,
     * or a Suite registry keybind (warned, then confirmable).
     */
    public static Conflict findConflict(String keybind, String ignoreMacroId) {
        int code = KeyNames.keyCode(keybind);
        if (code <= 0) return null;
        List<ClientConfig.MacroConfig> macros = ClientConfig.getInstance().getMacros();
        for (ClientConfig.MacroConfig other : macros) {
            if (other.id.equals(ignoreMacroId) || KeyNames.keyCode(other.keybind) != code) continue;
            return new Conflict(true, other.name == null || other.name.trim().isEmpty() ? other.id : other.name);
        }
        Minecraft mc = Minecraft.getMinecraft();
        if (mc != null && mc.gameSettings != null) {
            for (KeyBinding binding : mc.gameSettings.keyBindings) {
                if (binding != null && binding.getKeyCode() == code) {
                    return new Conflict(false, Lang.tr(binding.getKeyDescription()));
                }
            }
        }
        for (FeatureRegistry.Feature feature : FeatureRegistry.all()) {
            if (feature.keybind != null && KeyNames.keyCode(feature.keybind) == code) {
                return new Conflict(false, Lang.tr(FeatureRegistry.nameKey(feature.id)));
            }
        }
        return null;
    }

    public static final class Conflict {
        public final boolean fromMacro;
        public final String label;

        Conflict(boolean fromMacro, String label) {
            this.fromMacro = fromMacro;
            this.label = label;
        }
    }
}
