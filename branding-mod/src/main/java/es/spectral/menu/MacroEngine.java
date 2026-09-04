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

        // KeyBinding only (discoverable in Controls): unbinding it in Controls
        // is honored. No raw-GLFW polling fallback.
        if (EspectralClient.CLIENT_KEY != null && EspectralClient.CLIENT_KEY.consumeClick() && !isScreenOpen) {
            Compat.open(minecraft, new EspectralClientScreen(null));
            // Re-evaluate isScreenOpen after opening so macro block below is skipped
            isScreenOpen = true;
        }

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
        if (keybindStr == null || keybindStr.isEmpty()) return -1;
        String lower = keybindStr.toLowerCase(Locale.ROOT);

        if (lower.startsWith("key.keyboard.")) {
            String name = lower.substring("key.keyboard.".length());
            return getGlfwKeyFromName(name);
        }

        if (lower.length() == 1) {
            char c = lower.charAt(0);
            if (c >= 'a' && c <= 'z') return GLFW.GLFW_KEY_A + (c - 'a');
            if (c >= '0' && c <= '9') return GLFW.GLFW_KEY_0 + (c - '0');
        }

        return getGlfwKeyFromName(lower);
    }

    private static int getGlfwKeyFromName(String name) {
        if (name == null || name.isEmpty()) return -1;
        if (name.length() == 1) {
            char c = name.charAt(0);
            if (c == ' ') return GLFW.GLFW_KEY_SPACE;
            if (c >= 'a' && c <= 'z') return GLFW.GLFW_KEY_A + (c - 'a');
            if (c >= '0' && c <= '9') return GLFW.GLFW_KEY_0 + (c - '0');
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
}
