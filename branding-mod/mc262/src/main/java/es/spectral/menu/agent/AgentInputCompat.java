package es.spectral.menu.agent;

import com.mojang.blaze3d.platform.InputConstants;

import net.minecraft.client.Minecraft;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonInfo;

import es.spectral.menu.mixin.KeyboardHandlerInvoker;
import es.spectral.menu.mixin.MouseHandlerInvoker;

/**
 * 26.2 input plumbing: {@code KeyboardHandler.keyPress}/{@code charTyped} and
 * {@code MouseHandler.onButton}/{@code onMove} are private, reached through the
 * per-lane invoker mixins. The same source compiles for 1.21.11 (identical
 * descriptors). Key values are GLFW keysyms, left button is 0 — both from
 * {@code InputConstants} at runtime.
 */
public final class AgentInputCompat {

    private AgentInputCompat() {}

    public static void moveMouse(Minecraft minecraft, double windowX, double windowY) {
        if (minecraft == null || minecraft.mouseHandler == null) return;
        ((MouseHandlerInvoker) (Object) minecraft.mouseHandler)
                .espectral$onMove(windowHandle(minecraft), windowX, windowY);
    }

    public static void mouseButton(Minecraft minecraft, int button, int action) {
        if (minecraft == null || minecraft.mouseHandler == null) return;
        ((MouseHandlerInvoker) (Object) minecraft.mouseHandler)
                .espectral$onButton(windowHandle(minecraft), new MouseButtonInfo(button, 0), action);
    }

    public static void key(Minecraft minecraft, int key, int action, int modifiers) {
        if (minecraft == null || minecraft.keyboardHandler == null) return;
        // KeyEvent(key = keysym, scancode, modifiers); 0 = unknown scancode,
        // which vanilla resolves through the keysym branch.
        ((KeyboardHandlerInvoker) (Object) minecraft.keyboardHandler)
                .espectral$keyPress(windowHandle(minecraft), action, new KeyEvent(key, 0, modifiers));
    }

    public static void charTyped(Minecraft minecraft, int codePoint) {
        if (minecraft == null || minecraft.keyboardHandler == null) return;
        ((KeyboardHandlerInvoker) (Object) minecraft.keyboardHandler)
                .espectral$charTyped(windowHandle(minecraft), new CharacterEvent(codePoint));
    }

    private static long windowHandle(Minecraft minecraft) {
        return minecraft.getWindow() != null ? minecraft.getWindow().handle() : 0L;
    }
}
