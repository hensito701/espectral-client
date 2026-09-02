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
    private boolean rShiftWasDown = false;

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

        // KeyBinding (discoverable in Controls) — primary. Polling edge-detect is
        // a fallback and is skipped this tick when consumeClick() consumed the
        // press, so duplicate-keybind triggers fire at most once per tick.
        boolean keyBindingPressed = false;
        if (EspectralClient.CLIENT_KEY != null && EspectralClient.CLIENT_KEY.consumeClick()) {
            keyBindingPressed = true;
        }

        boolean rShiftDown = InputConstants.isKeyDown(minecraft.getWindow(), GLFW.GLFW_KEY_RIGHT_SHIFT);
        boolean pollingPressed = rShiftDown && !rShiftWasDown && !isScreenOpen;
        rShiftWasDown = rShiftDown;
        if (keyBindingPressed) {
            pollingPressed = false;
        }

        if ((keyBindingPressed || pollingPressed) && !isScreenOpen) {
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
        for (ClientConfig.MacroConfig macro : macros) {
            int keyCode = getGlfwKeyCode(macro.keybind);
            if (keyCode <= 0) continue;

            boolean keyDown = InputConstants.isKeyDown(minecraft.getWindow(), keyCode);
            if (keyDown && !pressedKeys.contains(keyCode) && !newlyPressed.contains(keyCode)) {
                newlyPressed.add(keyCode);
                executeMacro(macro, minecraft);
            }
        }
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
            if (c >= 'a' && c <= 'z') return GLFW.GLFW_KEY_A + (c - 'a');
            if (c >= '0' && c <= '9') return GLFW.GLFW_KEY_0 + (c - '0');
        }
        return switch (name) {
            case "space" -> GLFW.GLFW_KEY_SPACE;
            case "enter" -> GLFW.GLFW_KEY_ENTER;
            case "tab" -> GLFW.GLFW_KEY_TAB;
            case "backspace" -> GLFW.GLFW_KEY_BACKSPACE;
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
            case "right.shift" -> GLFW.GLFW_KEY_RIGHT_SHIFT;
            case "left.shift" -> GLFW.GLFW_KEY_LEFT_SHIFT;
            case "right.control" -> GLFW.GLFW_KEY_RIGHT_CONTROL;
            case "left.control" -> GLFW.GLFW_KEY_LEFT_CONTROL;
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
