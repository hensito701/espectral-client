package es.spectral.menu;

import java.util.Locale;

import com.mojang.blaze3d.platform.InputConstants;

import net.minecraft.client.Minecraft;
import org.lwjgl.sdl.SDLKeyboard;
import org.lwjgl.sdl.SDLScancode;

/**
 * Keyboard platform layer for 26.3, which replaced GLFW with SDL3: key values
 * are SDL scancodes (A = 4, not 65) and the keyboard type is KEYBOARD. The
 * vanilla {@code key.keyboard.*} names are unchanged, so stored keybinds keep
 * working across versions.
 */
public final class Keys {

    private Keys() {}

    /** The keyboard {@link InputConstants.Type} (KEYBOARD under SDL). */
    public static InputConstants.Type keyboard() {
        return InputConstants.Type.KEYBOARD;
    }

    public static boolean isDown(Minecraft minecraft, int code) {
        return InputConstants.isKeyDown(code);
    }

    /**
     * Physical key (scancode) whose unshifted label in the current layout is
     * that character, or -1 — the same lookup vanilla uses for key labels.
     */
    public static int forCharacter(int codePoint) {
        String wanted = new String(Character.toChars(codePoint)).toLowerCase(Locale.ROOT);
        for (int scancode = 1; scancode < SDLScancode.SDL_SCANCODE_COUNT; scancode++) {
            int keycode = SDLKeyboard.SDL_GetKeyFromScancode(scancode, (short) 0, false);
            if (keycode == 0) continue;
            String name = SDLKeyboard.SDL_GetKeyName(keycode);
            if (name != null && name.toLowerCase(Locale.ROOT).equals(wanted)) return scancode;
        }
        return -1;
    }

    /** Highest valid key value (numeric {@code key.keyboard.<n>} names are bounded by it). */
    public static int maxCode() {
        return SDLScancode.SDL_SCANCODE_COUNT - 1;
    }
}
