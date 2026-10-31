package es.spectral.menu.agent;

import com.mojang.blaze3d.platform.InputConstants;

import net.minecraft.client.Minecraft;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonInfo;


/**
 * 26.3 input plumbing. SDL3 made {@code KeyboardHandler.keyPress}/{@code
 * charTyped} and {@code MouseHandler.onButton}/{@code onMove} public, so the
 * public entry points are called directly (no invoker needed on this lane;
 * the same {@code KeyboardHandlerInvoker} interface is still registered, which
 * is harmless).
 *
 * <p>Key values are SDL scancodes and the left mouse button is 1, both
 * supplied by {@code InputConstants} at runtime — never hardcoded.
 */
public final class AgentInputCompat {

    private AgentInputCompat() {}

    public static void moveMouse(Minecraft minecraft, double windowX, double windowY) {
        if (minecraft == null || minecraft.mouseHandler == null) return;
        // 26.3's onMove takes absolute position plus relative deltas; the
        // real SDL path passes both, so derive the deltas from the previous
        // position to keep camera/hover maths consistent.
        double prevX = minecraft.mouseHandler.xpos();
        double prevY = minecraft.mouseHandler.ypos();
        minecraft.mouseHandler.onMove(windowHandle(minecraft), windowX, windowY,
                windowX - prevX, windowY - prevY);
    }

    public static void mouseButton(Minecraft minecraft, int button, int action) {
        if (minecraft == null || minecraft.mouseHandler == null) return;
        minecraft.mouseHandler.onButton(windowHandle(minecraft),
                new MouseButtonInfo(button, 0), action);
    }

    public static void key(Minecraft minecraft, int key, int action, int modifiers) {
        if (minecraft == null || minecraft.keyboardHandler == null) return;
        // 26.3 KeyEvent(key = scancode, keycode = SDL keycode, modifiers).
        minecraft.keyboardHandler.keyPress(windowHandle(minecraft),
                action, new KeyEvent(key, keycodeOf(key), modifiers));
    }

    public static void charTyped(Minecraft minecraft, int codePoint) {
        if (minecraft == null || minecraft.keyboardHandler == null) return;
        minecraft.keyboardHandler.charTyped(windowHandle(minecraft),
                new CharacterEvent(codePoint));
    }

    /** SDL keycode for a scancode (0 when unknown; screens don't need it). */
    private static int keycodeOf(int scancode) {
        try {
            return org.lwjgl.sdl.SDLKeyboard.SDL_GetKeyFromScancode(scancode, (short) 0, false);
        } catch (Throwable t) {
            return 0;
        }
    }

    private static long windowHandle(Minecraft minecraft) {
        return minecraft.getWindow() != null ? minecraft.getWindow().handle() : 0L;
    }
}
