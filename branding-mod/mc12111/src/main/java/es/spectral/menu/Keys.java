package es.spectral.menu;

import java.util.Locale;

import com.mojang.blaze3d.platform.InputConstants;

import net.minecraft.client.Minecraft;
import org.lwjgl.glfw.GLFW;

/**
 * Keyboard platform layer (GLFW: key values are GLFW keysyms). 26.3 moved to
 * SDL scancodes and ships its own copy of this class; shared code goes through
 * here and through the vanilla {@code key.keyboard.*} names, never through
 * raw backend constants.
 */
public final class Keys {

    private Keys() {}

    /** The keyboard {@link InputConstants.Type} (KEYSYM under GLFW). */
    public static InputConstants.Type keyboard() {
        return InputConstants.Type.KEYSYM;
    }

    public static boolean isDown(Minecraft minecraft, int code) {
        return InputConstants.isKeyDown(minecraft.getWindow(), code);
    }

    /**
     * Physical key whose unshifted label in the current layout is that
     * character (Windows/GLFW reports the layout name), or -1.
     */
    public static int forCharacter(int codePoint) {
        String wanted = new String(Character.toChars(codePoint)).toLowerCase(Locale.ROOT);
        for (int code = GLFW.GLFW_KEY_SPACE; code <= GLFW.GLFW_KEY_LAST; code++) {
            String name = GLFW.glfwGetKeyName(code, 0);
            if (name != null && name.toLowerCase(Locale.ROOT).equals(wanted)) return code;
        }
        return -1;
    }

    /** Highest valid key value (numeric {@code key.keyboard.<n>} names are bounded by it). */
    public static int maxCode() {
        return GLFW.GLFW_KEY_LAST;
    }
}
