package es.spectral.menu;

import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

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
        pressedKeys.removeIf(code -> !Keys.isDown(minecraft, code));

        if (isScreenOpen || !ClientConfig.getInstance().isFeatureEnabled("macros")) {
            return;
        }

        List<ClientConfig.MacroConfig> macros = ClientConfig.getInstance().getMacros();
        Set<Integer> newlyPressed = new HashSet<>();
        Set<String> seenIds = new HashSet<>();
        for (ClientConfig.MacroConfig macro : macros) {
            seenIds.add(macro.id);
            int keyCode = keyCode(macro.keybind);
            if (keyCode <= 0) {
                if (warnedUnparseable.add(macro.id)) {
                    LOGGER.warn("Ignoring macro '{}' (id '{}'): unparseable keybind '{}'",
                            macro.name, macro.id, macro.keybind);
                }
                continue;
            }
            warnedUnparseable.remove(macro.id);

            boolean keyDown = Keys.isDown(minecraft, keyCode);
            if (keyDown && !pressedKeys.contains(keyCode) && !newlyPressed.contains(keyCode)) {
                newlyPressed.add(keyCode);
                executeMacro(macro, minecraft);
            }
        }
        warnedUnparseable.retainAll(seenIds);
        pressedKeys.addAll(newlyPressed);
    }

    /**
     * Key value for a stored keybind ({@code key.keyboard.<name>}, a bare
     * name/alias, or a single layout character), or -1. Values are the running
     * backend's (GLFW keysyms before 26.3, SDL scancodes from 26.3): names are
     * resolved through vanilla's own table, so one stored keybind means the
     * same physical key on every version.
     */
    public static int keyCode(String keybindStr) {
        if (keybindStr == null) return -1;
        String trimmed = keybindStr.trim();
        if (trimmed.isEmpty()) return -1;
        String lower = trimmed.toLowerCase(Locale.ROOT);

        if (lower.startsWith("key.keyboard.")) {
            return keyFromName(lower.substring("key.keyboard.".length()));
        }

        // A single non-ASCII character ("ñ", "ç", "ß"): the literal key label a
        // hand-written config may carry. Resolved against the live keyboard
        // layout so it means the same key the user types. Accented letters are
        // dead-key compositions, so a macro for "á" cannot exist — a keyboard
        // limitation, not a config one.
        if (trimmed.codePointCount(0, trimmed.length()) == 1) {
            int cp = trimmed.codePointAt(0);
            if (cp > 0x7F) {
                int resolved = Keys.forCharacter(cp);
                if (resolved > 0) return resolved;
            }
        }

        return keyFromName(lower);
    }

    /** Short spellings hand-written configs use, mapped to vanilla key names. */
    private static final Map<String, String> ALIASES = Map.ofEntries(
            Map.entry(" ", "space"),
            Map.entry("esc", "escape"),
            Map.entry("shift", "left.shift"),
            Map.entry("control", "left.control"),
            Map.entry("ctrl", "left.control"),
            Map.entry("alt", "left.alt"),
            Map.entry("grave", "grave.accent"),
            Map.entry("equals", "equal"),
            Map.entry("lbracket", "left.bracket"),
            Map.entry("rbracket", "right.bracket"));

    private static int keyFromName(String name) {
        if (name == null || name.isEmpty()) return -1;
        // Numeric keysym ("key.keyboard.341"): the name vanilla fabricates for
        // a key absent from its name table. Storing it keeps exotic/layout keys
        // round-trippable without inventing names.
        if (name.length() > 1 && name.chars().allMatch(ch -> ch >= '0' && ch <= '9')) {
            try {
                int code = Integer.parseInt(name);
                return code >= 0 && code <= Keys.maxCode() ? code : -1;
            } catch (NumberFormatException e) {
                return -1;
            }
        }
        String normalized = ALIASES.getOrDefault(name, name.replace(' ', '.').replace('_', '.'));
        // Numpad keys accept the common spellings ("numpad.0", "keypad.0",
        // "kp.0", "numpad 0") since configs vary.
        if (normalized.startsWith("numpad.") || normalized.startsWith("kp.")) {
            normalized = "keypad." + normalized.substring(normalized.indexOf('.') + 1);
        }
        InputConstants.Key key;
        try {
            key = InputConstants.getKey("key.keyboard." + normalized);
        } catch (RuntimeException e) {
            return -1;
        }
        // getKey fabricates entries only for numeric names (handled above);
        // anything else must be a real vanilla keyboard key.
        return key.getType() == Keys.keyboard() && key.getValue() > 0 ? key.getValue() : -1;
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
     * Canonical keybind string for a key value, in the same
     * {@code key.keyboard.<name>} form the launcher and {@code options.txt}
     * write: {@link InputConstants} resolves the name from its own table and,
     * for keys it has no entry for (e.g. {@code Ç} on a Spanish layout),
     * fabricates {@code key.keyboard.<value>} — which vanilla's
     * {@link InputConstants#getKey(String)} and {@link #keyCode(String)}
     * both parse back to the same code. Empty string for an unknown code.
     */
    public static String keybindFor(int key) {
        if (key <= 0) return "";
        return Keys.keyboard().getOrCreate(key).getName();
    }

    /**
     * Layout-aware label for a stored keybind (e.g. {@code ñ}, {@code Ç},
     * {@code Z}, {@code Page Down}). Resolved through the vanilla display name
     * so the label matches the Controls screen; memoised per key by vanilla.
     * Falls back to the raw keybind when it cannot be parsed.
     */
    public static net.minecraft.network.chat.Component keyDisplayName(String keybind) {
        int code = keyCode(keybind);
        if (code <= 0) {
            return net.minecraft.network.chat.Component.literal(
                    keybind == null ? "" : keybind);
        }
        return Keys.keyboard().getOrCreate(code).getDisplayName();
    }

    /** Reserved keys: Esc always closes screens and Right Shift opens the Suite. */
    public static boolean isReserved(int key) {
        return key == InputConstants.KEY_ESCAPE || key == InputConstants.KEY_RSHIFT;
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
        int code = keyCode(keybind);
        if (code <= 0) return null;
        String canonical = keybindFor(code);

        ClientConfig config = ClientConfig.getInstance();
        for (ClientConfig.MacroConfig other : config.getMacros()) {
            if (other.id.equals(ignoreMacroId)) continue;
            if (keyCode(other.keybind) != code) continue;
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
            if (keyCode(suiteKey) != code) continue;
            return new Conflict(false,
                    net.minecraft.network.chat.Component.translatable(
                            FeatureRegistry.nameKey(feature.id())));
        }
        return null;
    }
}
