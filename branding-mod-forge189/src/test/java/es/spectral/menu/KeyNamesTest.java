package es.spectral.menu;

import static org.junit.Assert.assertEquals;

import org.junit.Test;
import org.lwjgl.input.Keyboard;

/** Keybinds stay in the launcher/modern format while 1.8.9 runs on LWJGL 2 codes. */
public class KeyNamesTest {

    @Test
    public void modernNamesMapToLwjglCodesAndBack() {
        String[] names = {"key.keyboard.z", "key.keyboard.f6", "key.keyboard.left.shift", "key.keyboard.keypad.7",
                "key.keyboard.semicolon", "key.keyboard.page.down", "key.keyboard.grave.accent", "key.keyboard.9"};
        for (String name : names) {
            assertEquals(name, KeyNames.keybindFor(KeyNames.keyCode(name)));
        }
        assertEquals(Keyboard.KEY_Z, KeyNames.keyCode("key.keyboard.z"));
        assertEquals(Keyboard.KEY_RSHIFT, KeyNames.keyCode("key.keyboard.right.shift"));
        assertEquals(Keyboard.KEY_NEXT, KeyNames.keyCode("key.keyboard.page.down"));
    }

    @Test
    public void shortFormsAndAliasesResolve() {
        assertEquals(Keyboard.KEY_ESCAPE, KeyNames.keyCode("esc"));
        assertEquals(Keyboard.KEY_LCONTROL, KeyNames.keyCode("ctrl"));
        assertEquals(Keyboard.KEY_E, KeyNames.keyCode("E"));
        assertEquals(Keyboard.KEY_F3, KeyNames.keyCode("f3"));
        assertEquals(Keyboard.KEY_NUMPAD0, KeyNames.keyCode("numpad 0"));
    }

    @Test
    public void unknownKeysAreRefused() {
        assertEquals(-1, KeyNames.keyCode("key.keyboard.world.1"));
        assertEquals(-1, KeyNames.keyCode(""));
        assertEquals(-1, KeyNames.keyCode(null));
        assertEquals("", KeyNames.keybindFor(Keyboard.KEY_NONE));
    }

    @Test
    public void displayNamesAreReadable() {
        assertEquals("Z", KeyNames.displayName("key.keyboard.z"));
        assertEquals("Left Shift", KeyNames.displayName("key.keyboard.left.shift"));
        assertEquals("F6", KeyNames.displayName("key.keyboard.f6"));
    }
}
