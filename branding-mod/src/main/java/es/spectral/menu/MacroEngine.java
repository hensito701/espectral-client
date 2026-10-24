package es.spectral.menu;

import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;

import org.lwjgl.glfw.GLFW;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.mojang.blaze3d.platform.InputConstants;

import net.minecraft.client.Minecraft;

/**
 * Macro execution engine (Contract A & \u00a73.4).
 * Chat and command macros triggered by keybinds, with per-key repeat guard.
 */
public final class MacroEngine {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-macros");
    private static final MacroEngine INSTANCE = new MacroEngine();

    private final Set<Integer> pressedKeys = new HashSet<>();
    /** Macro ids already warned about for unparseable keybinds (warn once). */
    private final Set<String> warnedUnparseable = new HashSet<>();

    public static MacroEngine getInstance() {
        return INSTANCE;
    }

    private MacroEngine() {}

    /**
     * Polled every client tick via MinecraftMixin.
     */
    public synchronized void onTick(Minecraft minecraft) {
        if (minecraft == null || minecraft.getWindow() == null) return;

        boolean isScreenOpen = Compat.isScreenOpen(minecraft);

        // The Right Shift shortcut lives in SuiteShortcut now (it opens the
        // Suite screen); this engine only handles chat/command macros below.

        // Always clean up released keys so repeat guard doesn't get stuck if released inside a screen
        pressedKeys.removeIf(code -> !InputConstants.isKeyDown(minecraft.getWindow(), code));

        if (isScreenOpen || !ClientConfig.getInstance().isFeatureEnabled("macros")) {
            return;
        }

        List<ClientConfig.MacroConfig> macros = ClientConfig.getInstance().getMacros();
        Set<Integer> newlyPressed = new HashSet<>();
        Set<String> seenIds = new HashSet<>();
        for (ClientConfig.MacroConfig macro : macros) {
            seenIds.add(macro.id);
            int keyCode = getGlfwKeyCode(macro.keybind);
            if (keyCode <= 0) {
                if (warnedUnparseable.add(macro.id)) {
                    LOGGER.warn("Ignoring macro '{}' (id '{}'): unparseable keybind '{}'",
                            macro.name, macro.id, macro.keybind);
                }
                continue;
            }
            warnedUnparseable.remove(macro.id);

            boolean keyDown = InputConstants.isKeyDown(minecraft.getWindow(), keyCode);
            if (keyDown && !pressedKeys.contains(keyCode) && !newlyPressed.contains(keyCode)) {
                newlyPressed.add(keyCode);
                executeMacro(macro, minecraft);
            }
        }
        warnedUnparseable.retainAll(seenIds);
        pressedKeys.addAll(newlyPressed);
    }

    public static int getGlfwKeyCode(String keybindStr) {
        if (keybindStr == null) return -1;
        String trimmed = keybindStr.trim();
        if (trimmed.isEmpty()) return -1;
        String lower = trimmed.toLowerCase(Locale.ROOT);

        if (lower.startsWith("key.keyboard.")) {
            String name = lower.substring("key.keyboard.".length());
            return getGlfwKeyFromName(name);
        }

        // A single non-ASCII character ("ñ", "ç", "ß"): the literal key label a
        // hand-written config may carry. Resolved against the live keyboard
        // layout so it means the same key the user types.
        if (trimmed.codePointCount(0, trimmed.length()) == 1) {
            int cp = trimmed.codePointAt(0);
            if (cp > 0x7F) {
                int resolved = glfwKeyForCharacter(cp);
                if (resolved > 0) return resolved;
            }
        }

        return getGlfwKeyFromName(lower);
    }

    /**
     * Reverse lookup for a printed character in the current layout: scans the
     * GLFW key range for the physical key whose unshifted name is that
     * character (Windows/GLFW reports the layout name). Returns -1 when the
     * character is not a plain key on this keyboard (accented letters are
     * dead-key compositions, so a macro for "á" cannot exist — that is a
     * keyboard limitation, not a config one).
     */
    private static int glfwKeyForCharacter(int codePoint) {
        String wanted = new String(Character.toChars(codePoint)).toLowerCase(Locale.ROOT);
        for (int code = GLFW.GLFW_KEY_SPACE; code <= GLFW.GLFW_KEY_LAST; code++) {
            String name = GLFW.glfwGetKeyName(code, 0);
            if (name != null && name.toLowerCase(Locale.ROOT).equals(wanted)) {
                return code;
            }
        }
        return -1;
    }

    private static int getGlfwKeyFromName(String name) {
        if (name == null || name.isEmpty()) return -1;
        if (name.length() == 1) {
            char c = name.charAt(0);
            if (c == ' ') return GLFW.GLFW_KEY_SPACE;
            if (c >= 'a' && c <= 'z') return GLFW.GLFW_KEY_A + (c - 'a');
            if (c >= '0' && c <= '9') return GLFW.GLFW_KEY_0 + (c - '0');
        }
        // Numeric keysym ("key.keyboard.341"): the name vanilla fabricates for
        // a key absent from its name table. Storing it keeps exotic/layout keys
        // round-trippable without inventing names.
        if (name.length() > 1 && name.chars().allMatch(ch -> ch >= '0' && ch <= '9')) {
            try {
                int code = Integer.parseInt(name);
                return code >= 0 && code <= GLFW.GLFW_KEY_LAST ? code : -1;
            } catch (NumberFormatException e) {
                return -1;
            }
        }
        // Numpad digits accept the common spellings ("numpad.0", "keypad.0",
        // "kp.0", "numpad 0") since configs vary.
        String normalized = name.replace(' ', '.');
        if (normalized.startsWith("numpad.") || normalized.startsWith("keypad.") || normalized.startsWith("kp.")) {
            String tail = normalized.substring(normalized.indexOf('.') + 1);
            if (tail.length() == 1 && tail.charAt(0) >= '0' && tail.charAt(0) <= '9') {
                return GLFW.GLFW_KEY_KP_0 + (tail.charAt(0) - '0');
            }
            return switch (tail) {
                case "add" -> GLFW.GLFW_KEY_KP_ADD;
                case "subtract" -> GLFW.GLFW_KEY_KP_SUBTRACT;
                case "multiply" -> GLFW.GLFW_KEY_KP_MULTIPLY;
                case "divide" -> GLFW.GLFW_KEY_KP_DIVIDE;
                case "enter" -> GLFW.GLFW_KEY_KP_ENTER;
                case "decimal" -> GLFW.GLFW_KEY_KP_DECIMAL;
                default -> -1;
            };
        }
        return switch (normalized) {
            case "space" -> GLFW.GLFW_KEY_SPACE;
            case "enter" -> GLFW.GLFW_KEY_ENTER;
            case "tab" -> GLFW.GLFW_KEY_TAB;
            case "backspace" -> GLFW.GLFW_KEY_BACKSPACE;
            case "escape", "esc" -> GLFW.GLFW_KEY_ESCAPE;
            case "f1" -> GLFW.GLFW_KEY_F1;
            case "f2" -> GLFW.GLFW_KEY_F2;
            case "f3" -> GLFW.GLFW_KEY_F3;
            case "f4" -> GLFW.GLFW_KEY_F4;
            case "f5" -> GLFW.GLFW_KEY_F5;
            case "f6" -> GLFW.GLFW_KEY_F6;
            case "f7" -> GLFW.GLFW_KEY_F7;
            case "f8" -> GLFW.GLFW_KEY_F8;
            case "f9" -> GLFW.GLFW_KEY_F9;
            case "f10" -> GLFW.GLFW_KEY_F10;
            case "f11" -> GLFW.GLFW_KEY_F11;
            case "f12" -> GLFW.GLFW_KEY_F12;
            case "f13" -> GLFW.GLFW_KEY_F13;
            case "f14" -> GLFW.GLFW_KEY_F14;
            case "f15" -> GLFW.GLFW_KEY_F15;
            case "shift" -> GLFW.GLFW_KEY_LEFT_SHIFT;
            case "right.shift", "right_shift" -> GLFW.GLFW_KEY_RIGHT_SHIFT;
            case "left.shift", "left_shift" -> GLFW.GLFW_KEY_LEFT_SHIFT;
            case "control", "ctrl" -> GLFW.GLFW_KEY_LEFT_CONTROL;
            case "right.control", "right_control" -> GLFW.GLFW_KEY_RIGHT_CONTROL;
            case "left.control", "left_control" -> GLFW.GLFW_KEY_LEFT_CONTROL;
            case "alt" -> GLFW.GLFW_KEY_LEFT_ALT;
            case "right.alt", "right_alt" -> GLFW.GLFW_KEY_RIGHT_ALT;
            case "left.alt", "left_alt" -> GLFW.GLFW_KEY_LEFT_ALT;
            case "grave", "grave.accent" -> GLFW.GLFW_KEY_GRAVE_ACCENT;
            case "minus" -> GLFW.GLFW_KEY_MINUS;
            case "equals", "equal" -> GLFW.GLFW_KEY_EQUAL;
            case "lbracket" -> GLFW.GLFW_KEY_LEFT_BRACKET;
            case "rbracket" -> GLFW.GLFW_KEY_RIGHT_BRACKET;
            case "semicolon" -> GLFW.GLFW_KEY_SEMICOLON;
            case "apostrophe" -> GLFW.GLFW_KEY_APOSTROPHE;
            case "comma" -> GLFW.GLFW_KEY_COMMA;
            case "period" -> GLFW.GLFW_KEY_PERIOD;
            case "slash" -> GLFW.GLFW_KEY_SLASH;
            case "backslash" -> GLFW.GLFW_KEY_BACKSLASH;
            case "up" -> GLFW.GLFW_KEY_UP;
            case "down" -> GLFW.GLFW_KEY_DOWN;
            case "left" -> GLFW.GLFW_KEY_LEFT;
            case "right" -> GLFW.GLFW_KEY_RIGHT;
            case "insert" -> GLFW.GLFW_KEY_INSERT;
            case "delete" -> GLFW.GLFW_KEY_DELETE;
            case "home" -> GLFW.GLFW_KEY_HOME;
            case "end" -> GLFW.GLFW_KEY_END;
            case "page.up", "page_up" -> GLFW.GLFW_KEY_PAGE_UP;
            case "page.down", "page_down" -> GLFW.GLFW_KEY_PAGE_DOWN;
            case "caps.lock", "caps_lock" -> GLFW.GLFW_KEY_CAPS_LOCK;
            case "print.screen", "print_screen" -> GLFW.GLFW_KEY_PRINT_SCREEN;
            case "scroll.lock", "scroll_lock" -> GLFW.GLFW_KEY_SCROLL_LOCK;
            case "pause" -> GLFW.GLFW_KEY_PAUSE;
            default -> -1;
        };
    }

    /**
     * Executes one macro: a leading {@code /} goes through the vanilla
     * command API, everything else through the vanilla chat API, so a macro
     * can never reach a network path the player could not type by hand.
     */
    private void executeMacro(ClientConfig.MacroConfig macro, Minecraft minecraft) {
        LOGGER.info("Executing macro: {} ({})", macro.name, macro.id);
        for (ClientConfig.MacroAction action : macro.actions) {
            String text = action.text;
            if (text == null || text.isEmpty()) continue;
            if ("command".equalsIgnoreCase(action.type) || text.startsWith("/")) {
                Compat.sendChat(minecraft, text.startsWith("/") ? text : "/" + text);
            } else {
                Compat.sendChat(minecraft, text);
            }
        }
    }

    // ---------------------------------------------------------------
    // Keybind format (shared with the Macros editor screen)
    // ---------------------------------------------------------------

    /** Maximum stored keybind length; the launcher rejects longer ones. */
    public static final int MAX_KEYBIND_LENGTH = 32;

    /**
     * Canonical keybind string for a GLFW key code, in the same
     * {@code key.keyboard.<name>} form the launcher and {@code options.txt}
     * write: {@link InputConstants} resolves the name from its own table and,
     * for keys it has no entry for (e.g. {@code Ç} on a Spanish layout,
     * keysym 231), fabricates {@code key.keyboard.<keysym>} — which vanilla's
     * {@link InputConstants#getKey(String)} and {@link #getGlfwKeyCode(String)}
     * both parse back to the same code. Empty string for an unknown code.
     */
    public static String keybindFor(int glfwKey) {
        if (glfwKey <= 0) return "";
        return InputConstants.Type.KEYSYM.getOrCreate(glfwKey).getName();
    }

    /**
     * Layout-aware label for a stored keybind (e.g. {@code ñ}, {@code Ç},
     * {@code Z}, {@code Page Down}). Resolved through the vanilla display name
     * so the label matches the Controls screen; memoised per key by vanilla.
     * Falls back to the raw keybind when it cannot be parsed.
     */
    public static net.minecraft.network.chat.Component keyDisplayName(String keybind) {
        int code = getGlfwKeyCode(keybind);
        if (code <= 0) {
            return net.minecraft.network.chat.Component.literal(
                    keybind == null ? "" : keybind);
        }
        return InputConstants.Type.KEYSYM.getOrCreate(code).getDisplayName();
    }

    /** Reserved key: Esc always closes screens, so it can never be a macro. */
    public static boolean isReserved(int glfwKey) {
        return glfwKey == GLFW.GLFW_KEY_ESCAPE
                || glfwKey == GLFW.GLFW_KEY_RIGHT_SHIFT;
    }

    /**
     * An already-bound key that a new macro would shadow. {@code fromMacro} is
     * true for a clash with another macro (the editor refuses those outright);
     * false for a vanilla/mod/suite binding (warned, then confirmable).
     */
    public record Conflict(boolean fromMacro, net.minecraft.network.chat.Component label) {}

    /**
     * Finds the binding already using {@code keybind}, or {@code null} when
     * the key is free. Checked in this order: another macro
     * ({@code ignoreMacroId} excluded), every entry of
     * {@code Minecraft.options.keyMappings} (vanilla and mod keys — the same
     * list the Controls screen shows, and the one Right Shift / Z / B land in
     * once {@code EspectralClient.ensureClientKeyRegistered} ran), and the
     * Suite registry's own {@code keybind} values.
     */
    public static Conflict findConflict(String keybind, String ignoreMacroId) {
        int code = getGlfwKeyCode(keybind);
        if (code <= 0) return null;
        String canonical = keybindFor(code);

        ClientConfig config = ClientConfig.getInstance();
        for (ClientConfig.MacroConfig other : config.getMacros()) {
            if (other.id.equals(ignoreMacroId)) continue;
            if (getGlfwKeyCode(other.keybind) != code) continue;
            String name = other.name == null || other.name.isBlank() ? other.id : other.name;
            return new Conflict(true, net.minecraft.network.chat.Component.literal(name));
        }

        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft != null && minecraft.options != null) {
            net.minecraft.client.KeyMapping[] mappings = minecraft.options.keyMappings;
            if (mappings != null) {
                for (net.minecraft.client.KeyMapping mapping : mappings) {
                    if (mapping == null || mapping.isUnbound()) continue;
                    if (!canonical.equals(mapping.saveString())) continue;
                    return new Conflict(false,
                            net.minecraft.network.chat.Component.translatable(mapping.getName()));
                }
            }
        }

        for (FeatureRegistry.Feature feature : FeatureRegistry.all()) {
            String suiteKey = feature.keybind();
            if (suiteKey == null || suiteKey.isEmpty()) continue;
            if (getGlfwKeyCode(suiteKey) != code) continue;
            return new Conflict(false,
                    net.minecraft.network.chat.Component.translatable(
                            FeatureRegistry.nameKey(feature.id())));
        }
        return null;
    }
}
