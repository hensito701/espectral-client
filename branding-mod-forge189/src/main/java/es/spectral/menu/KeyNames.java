package es.spectral.menu;

import java.util.HashMap;
import java.util.Locale;
import java.util.Map;

import org.lwjgl.input.Keyboard;

/**
 * Two-way table between the launcher's keybind format ({@code key.keyboard.<name>},
 * the modern vanilla / GLFW names the Fabric mods and the launcher store) and
 * LWJGL 2 key codes, so a macro bound in either game version means the same
 * physical key. Keys without a modern name cannot be stored.
 */
public final class KeyNames {

    public static final String PREFIX = "key.keyboard.";

    private static final Map<String, Integer> BY_NAME = new HashMap<String, Integer>();
    private static final Map<Integer, String> BY_CODE = new HashMap<Integer, String>();

    static {
        for (char c = 'a'; c <= 'z'; c++) {
            put(String.valueOf(c), Keyboard.getKeyIndex(String.valueOf(Character.toUpperCase(c))));
        }
        int[] digits = {Keyboard.KEY_0, Keyboard.KEY_1, Keyboard.KEY_2, Keyboard.KEY_3, Keyboard.KEY_4,
                Keyboard.KEY_5, Keyboard.KEY_6, Keyboard.KEY_7, Keyboard.KEY_8, Keyboard.KEY_9};
        int[] keypad = {Keyboard.KEY_NUMPAD0, Keyboard.KEY_NUMPAD1, Keyboard.KEY_NUMPAD2, Keyboard.KEY_NUMPAD3,
                Keyboard.KEY_NUMPAD4, Keyboard.KEY_NUMPAD5, Keyboard.KEY_NUMPAD6, Keyboard.KEY_NUMPAD7,
                Keyboard.KEY_NUMPAD8, Keyboard.KEY_NUMPAD9};
        for (int i = 0; i <= 9; i++) {
            put(String.valueOf(i), digits[i]);
            put("keypad." + i, keypad[i]);
        }
        int[] fkeys = {Keyboard.KEY_F1, Keyboard.KEY_F2, Keyboard.KEY_F3, Keyboard.KEY_F4, Keyboard.KEY_F5,
                Keyboard.KEY_F6, Keyboard.KEY_F7, Keyboard.KEY_F8, Keyboard.KEY_F9, Keyboard.KEY_F10, Keyboard.KEY_F11,
                Keyboard.KEY_F12, Keyboard.KEY_F13, Keyboard.KEY_F14, Keyboard.KEY_F15, Keyboard.KEY_F16,
                Keyboard.KEY_F17, Keyboard.KEY_F18, Keyboard.KEY_F19};
        for (int i = 0; i < fkeys.length; i++) {
            put("f" + (i + 1), fkeys[i]);
        }
        put("keypad.decimal", Keyboard.KEY_DECIMAL);
        put("keypad.divide", Keyboard.KEY_DIVIDE);
        put("keypad.multiply", Keyboard.KEY_MULTIPLY);
        put("keypad.subtract", Keyboard.KEY_SUBTRACT);
        put("keypad.add", Keyboard.KEY_ADD);
        put("keypad.enter", Keyboard.KEY_NUMPADENTER);
        put("keypad.equal", Keyboard.KEY_NUMPADEQUALS);
        put("apostrophe", Keyboard.KEY_APOSTROPHE);
        put("backslash", Keyboard.KEY_BACKSLASH);
        put("comma", Keyboard.KEY_COMMA);
        put("equal", Keyboard.KEY_EQUALS);
        put("grave.accent", Keyboard.KEY_GRAVE);
        put("left.bracket", Keyboard.KEY_LBRACKET);
        put("minus", Keyboard.KEY_MINUS);
        put("period", Keyboard.KEY_PERIOD);
        put("right.bracket", Keyboard.KEY_RBRACKET);
        put("semicolon", Keyboard.KEY_SEMICOLON);
        put("slash", Keyboard.KEY_SLASH);
        put("space", Keyboard.KEY_SPACE);
        put("tab", Keyboard.KEY_TAB);
        put("left.alt", Keyboard.KEY_LMENU);
        put("left.control", Keyboard.KEY_LCONTROL);
        put("left.shift", Keyboard.KEY_LSHIFT);
        put("left.win", Keyboard.KEY_LMETA);
        put("right.alt", Keyboard.KEY_RMENU);
        put("right.control", Keyboard.KEY_RCONTROL);
        put("right.shift", Keyboard.KEY_RSHIFT);
        put("right.win", Keyboard.KEY_RMETA);
        put("enter", Keyboard.KEY_RETURN);
        put("escape", Keyboard.KEY_ESCAPE);
        put("backspace", Keyboard.KEY_BACK);
        put("delete", Keyboard.KEY_DELETE);
        put("end", Keyboard.KEY_END);
        put("home", Keyboard.KEY_HOME);
        put("insert", Keyboard.KEY_INSERT);
        put("page.down", Keyboard.KEY_NEXT);
        put("page.up", Keyboard.KEY_PRIOR);
        put("caps.lock", Keyboard.KEY_CAPITAL);
        put("pause", Keyboard.KEY_PAUSE);
        put("scroll.lock", Keyboard.KEY_SCROLL);
        put("menu", Keyboard.KEY_APPS);
        put("print.screen", Keyboard.KEY_SYSRQ);
        put("num.lock", Keyboard.KEY_NUMLOCK);
        put("up", Keyboard.KEY_UP);
        put("down", Keyboard.KEY_DOWN);
        put("left", Keyboard.KEY_LEFT);
        put("right", Keyboard.KEY_RIGHT);
    }

    /** Short spellings hand-written configs use (same aliases as the Fabric mod). */
    private static final Map<String, String> ALIASES = new HashMap<String, String>();

    static {
        ALIASES.put(" ", "space");
        ALIASES.put("esc", "escape");
        ALIASES.put("shift", "left.shift");
        ALIASES.put("control", "left.control");
        ALIASES.put("ctrl", "left.control");
        ALIASES.put("alt", "left.alt");
        ALIASES.put("grave", "grave.accent");
        ALIASES.put("equals", "equal");
        ALIASES.put("lbracket", "left.bracket");
        ALIASES.put("rbracket", "right.bracket");
    }

    private KeyNames() {}

    private static void put(String name, int code) {
        BY_NAME.put(name, code);
        BY_CODE.put(code, name);
    }

    /** LWJGL 2 key code for a stored keybind, or -1 when it names no known key. */
    public static int keyCode(String keybind) {
        if (keybind == null) return -1;
        String lower = keybind.trim().toLowerCase(Locale.ROOT);
        if (lower.isEmpty()) return -1;
        String name = lower.startsWith(PREFIX) ? lower.substring(PREFIX.length()) : lower;
        String aliased = ALIASES.containsKey(name) ? ALIASES.get(name) : name.replace(' ', '.').replace('_', '.');
        if (aliased.startsWith("numpad.") || aliased.startsWith("kp.")) {
            aliased = "keypad." + aliased.substring(aliased.indexOf('.') + 1);
        }
        Integer code = BY_NAME.get(aliased);
        return code == null ? -1 : code;
    }

    /** Stored keybind ({@code key.keyboard.<name>}) for an LWJGL 2 key code, or "" when unnamed. */
    public static String keybindFor(int code) {
        String name = BY_CODE.get(code);
        return name == null ? "" : PREFIX + name;
    }

    /** Readable label for a stored keybind ("Z", "F6", "Left Shift"), raw text when unknown. */
    public static String displayName(String keybind) {
        int code = keyCode(keybind);
        if (code <= 0) return keybind == null ? "" : keybind;
        String name = BY_CODE.get(code);
        if (name.length() == 1) return name.toUpperCase(Locale.ROOT);
        StringBuilder out = new StringBuilder();
        for (String part : name.split("\\.")) {
            if (out.length() > 0) out.append(' ');
            out.append(Character.toUpperCase(part.charAt(0))).append(part.substring(1));
        }
        return out.toString();
    }
}
