package es.spectral.menu;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import net.fabricmc.loader.api.FabricLoader;

import es.spectral.menu.cosmetics.Loadout;

/**
 * Versioned config store for &lt;instanceDir&gt;/config/espectral-client.json (schema 2).
 * Reads, preserves unknown fields, and writes atomically.
 *
 * <p>Schema 2 adds the {@code suite.enabled} master switch (default true).
 * {@link #isFeatureEnabled(String)} is the ONE choke point every owned feature
 * reads through: suite gate AND stored flag. The stored flags are never
 * rewritten by the gate, so flipping the master off and on restores exactly
 * the previous feature states. Loading a v1 file migrates in memory (master
 * defaults to true, stored flags untouched) and the next save writes schema 2.
 *
 * <p>Feature defaults come from {@link FeatureRegistry} — the canonical
 * registry shipped in the jar — never from a second hardcoded list.
 */
public final class ClientConfig {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();
    private static final ClientConfig INSTANCE = new ClientConfig();

    /** Current config schema; written on every save. */
    public static final int SCHEMA_VERSION = 2;

    public static ClientConfig getInstance() {
        return INSTANCE;
    }

    /** Schema this code writes (2). */
    public static int schemaVersion() {
        return SCHEMA_VERSION;
    }

    public static final class FeatureConfig {
        public boolean enabled;
        public final JsonObject rawObject;

        public FeatureConfig(boolean enabled, JsonObject rawObject) {
            this.enabled = enabled;
            this.rawObject = rawObject != null ? rawObject : new JsonObject();
            this.rawObject.addProperty("enabled", enabled);
        }
    }

    public static final class MacroAction {
        public final String type;
        public final String text;

        public MacroAction(String type, String text) {
            this.type = type != null ? type : "chat";
            this.text = text != null ? text : "";
        }
    }

    public static final class MacroConfig {
        public final String id;
        public final String name;
        public final String keybind;
        public final List<MacroAction> actions;
        public final JsonObject rawObject;

        public MacroConfig(String id, String name, String keybind, List<MacroAction> actions, JsonObject rawObject) {
            this.id = id != null ? id : "";
            this.name = name != null ? name : "";
            this.keybind = keybind != null ? keybind : "";
            this.actions = actions != null ? actions : Collections.emptyList();
            this.rawObject = rawObject != null ? rawObject : new JsonObject();
        }
    }

    // --- cosmetics viewer state (Suite → Cosméticos sub-screen) -------------
    // Stored flattened inside the cosmetics feature object
    // (features.cosmetics.own, .others, .slot_espalda, …), which is exactly how
    // the registry's `state` block lands in the file (see FEATURE_DEFAULTS in
    // the engine): every key survives a save() untouched because it lives in
    // the feature's rawObject.

    /** Show the local player's own cosmetics. */
    public static final String COSMETICS_OWN = "own";
    /** Show other players' cosmetics. */
    public static final String COSMETICS_OTHERS = "others";
    /** Local emotes switch, ANDed with espectral-cosmetics.json's `emotes`. */
    public static final String COSMETICS_EMOTES = "emotes";
    /** Cloth/spring simulation; off = the reduced-motion static pose. */
    public static final String COSMETICS_PHYSICS = "physics";

    /** Config key of the per-slot visibility flag for a {@link Loadout} slot. */
    public static String cosmeticsSlotKey(String slot) {
        return "slot_" + slot;
    }

    /**
     * Immutable O(1) snapshot of the cosmetics viewer toggles. Rebuilt only
     * when the config loads or a toggle changes, so the render thread never
     * touches Gson, the file or the lock.
     */
    public static final class CosmeticsState {
        /** Slot bit indices, in {@link Loadout} field order. */
        public static final int BIT_ESPALDA = 1;
        public static final int BIT_CUELLO = 1 << 1;
        public static final int BIT_CABEZA = 1 << 2;
        public static final int BIT_ELITROS = 1 << 4;
        /** Every slot visible. */
        public static final int ALL_SLOTS = BIT_ESPALDA | BIT_CUELLO | BIT_CABEZA
                | BIT_ELITROS;

        public final boolean own;
        public final boolean others;
        public final boolean emotes;
        public final boolean physics;
        /** Bitmask of the own slots that are visible; {@link #ALL_SLOTS} = all. */
        public final int slotMask;

        public CosmeticsState(boolean own, boolean others, boolean emotes,
                boolean physics, int slotMask) {
            this.own = own;
            this.others = others;
            this.emotes = emotes;
            this.physics = physics;
            this.slotMask = slotMask & ALL_SLOTS;
        }

        /** Contract defaults: everything on. */
        public static final CosmeticsState DEFAULTS =
                new CosmeticsState(true, true, true, true, ALL_SLOTS);

        public boolean slotVisible(String slot) {
            int bit = bitOf(slot);
            return bit != 0 && (slotMask & bit) != 0;
        }

        /** Slot bit for a {@link Loadout} slot id, or 0 for an unknown slot. */
        public static int bitOf(String slot) {
            if (slot == null) return 0;
            return switch (slot) {
                case "espalda" -> BIT_ESPALDA;
                case "cuello" -> BIT_CUELLO;
                case "cabeza" -> BIT_CABEZA;
                case "elitros" -> BIT_ELITROS;
                default -> 0;
            };
        }
    }

    /** True when {@code key} is a cosmetics state key this screen can write. */
    public static boolean isCosmeticsStateKey(String key) {
        if (key == null) return false;
        return COSMETICS_OWN.equals(key) || COSMETICS_OTHERS.equals(key)
                || COSMETICS_EMOTES.equals(key) || COSMETICS_PHYSICS.equals(key)
                || (key.startsWith("slot_") && CosmeticsState.bitOf(key.substring(5)) != 0);
    }

    private int schema = SCHEMA_VERSION;
    /** Master switch; gates every owned feature through {@link #isFeatureEnabled}. */
    private boolean suiteEnabled = true;
    /** Cosmetics viewer toggles; volatile so the render thread reads lock-free. */
    private volatile CosmeticsState cosmeticsState = CosmeticsState.DEFAULTS;
    private final Map<String, FeatureConfig> features = new HashMap<>();
    private final List<MacroConfig> macros = new ArrayList<>();
    private JsonObject rawRoot = new JsonObject();
    /** Last observed config-file mtime (millis); drives throttled live reload. */
    private long lastKnownModified = -1L;

    private ClientConfig() {
        initDefaults();
    }

    /**
     * Seeds stored flags from the canonical registry. Unknown ids already in
     * the map are dropped here; {@link #load()} re-applies the file's entries
     * (including unknown ids) over these defaults.
     */
    private void initDefaults() {
        features.clear();
        for (FeatureRegistry.Feature f : FeatureRegistry.all()) {
            features.put(f.id(), new FeatureConfig(f.defaultEnabled(), null));
        }
    }

    public static Path getConfigPath() {
        return FabricLoader.getInstance().getConfigDir().resolve("espectral-client.json");
    }

    public synchronized void load() {
        Path path = getConfigPath();
        if (!Files.exists(path)) {
            LOGGER.info("Config file {} not found; using defaults", path);
            save();
            return;
        }

        try {
            String content = Files.readString(path, StandardCharsets.UTF_8);
            JsonElement rootElem = JsonParser.parseString(content);
            if (rootElem != null && rootElem.isJsonObject()) {
                JsonObject rootObj = rootElem.getAsJsonObject();
                int parsedSchema = schema;
                if (rootObj.has("schema") && rootObj.get("schema").isJsonPrimitive()) {
                    parsedSchema = rootObj.get("schema").getAsInt();
                }

                // Master switch; v1 files predate it and default to enabled
                // without touching any stored feature flag.
                boolean parsedSuite = true;
                if (rootObj.has("suite") && rootObj.get("suite").isJsonObject()) {
                    JsonObject suiteObj = rootObj.getAsJsonObject("suite");
                    if (suiteObj.has("enabled") && suiteObj.get("enabled").isJsonPrimitive()
                            && suiteObj.get("enabled").getAsJsonPrimitive().isBoolean()) {
                        parsedSuite = suiteObj.get("enabled").getAsBoolean();
                    }
                }
                Map<String, FeatureConfig> parsedFeatures = new HashMap<>();
                initDefaults();
                parsedFeatures.putAll(this.features);

                if (rootObj.has("features") && rootObj.get("features").isJsonObject()) {
                    JsonObject featObj = rootObj.getAsJsonObject("features");
                    for (Map.Entry<String, JsonElement> entry : featObj.entrySet()) {
                        String id = entry.getKey();
                        JsonElement val = entry.getValue();
                        if (val.isJsonObject()) {
                            JsonObject obj = val.getAsJsonObject();
                            boolean en = obj.has("enabled") && obj.get("enabled").isJsonPrimitive()
                                    && obj.get("enabled").getAsJsonPrimitive().isBoolean()
                                    && obj.get("enabled").getAsBoolean();
                            parsedFeatures.put(id, new FeatureConfig(en, obj));
                        } else if (val.isJsonPrimitive() && val.getAsJsonPrimitive().isBoolean()) {
                            boolean en = val.getAsBoolean();
                            JsonObject obj = new JsonObject();
                            obj.addProperty("enabled", en);
                            parsedFeatures.put(id, new FeatureConfig(en, obj));
                        }
                    }
                }

                List<MacroConfig> parsedMacros = new ArrayList<>();
                if (rootObj.has("macros") && rootObj.get("macros").isJsonArray()) {
                    JsonArray macArray = rootObj.getAsJsonArray("macros");
                    for (JsonElement elem : macArray) {
                        if (elem.isJsonObject()) {
                            JsonObject mObj = elem.getAsJsonObject();
                            String mId = mObj.has("id") ? mObj.get("id").getAsString() : "";
                            String mName = mObj.has("name") ? mObj.get("name").getAsString() : "";
                            String mKey = mObj.has("keybind") ? mObj.get("keybind").getAsString() : "";
                            List<MacroAction> mActs = new ArrayList<>();
                            if (mObj.has("actions") && mObj.get("actions").isJsonArray()) {
                                for (JsonElement aElem : mObj.getAsJsonArray("actions")) {
                                    if (aElem.isJsonObject()) {
                                        JsonObject aObj = aElem.getAsJsonObject();
                                        String aType = aObj.has("type") ? aObj.get("type").getAsString() : "chat";
                                        String aText = aObj.has("text") ? aObj.get("text").getAsString() : "";
                                        mActs.add(new MacroAction(aType, aText));
                                    }
                                }
                            }
                            parsedMacros.add(new MacroConfig(mId, mName, mKey, mActs, mObj));
                        }
                    }
                }

                this.rawRoot = rootObj;
                // Migrate in memory: anything older than schema 2 becomes 2.
                this.schema = Math.max(parsedSchema, SCHEMA_VERSION);
                this.suiteEnabled = parsedSuite;
                this.features.clear();
                this.features.putAll(parsedFeatures);
                this.macros.clear();
                this.macros.addAll(parsedMacros);
                this.cosmeticsState = readCosmeticsState();
            }
        } catch (Exception e) {
            LOGGER.warn("Failed to parse config {}: {}; preserving memory state", path, e.getMessage());
        }
        refreshLastModified();
    }

    /** Rebuilds the cosmetics snapshot from the current feature raw objects. */
    private CosmeticsState readCosmeticsState() {
        FeatureConfig fc = features.get("cosmetics");
        JsonObject obj = fc != null ? fc.rawObject : null;
        int mask = 0;
        for (int bit : new int[] {CosmeticsState.BIT_ESPALDA, CosmeticsState.BIT_CUELLO,
                CosmeticsState.BIT_CABEZA, CosmeticsState.BIT_ELITROS}) {
            if (stateFlag(obj, "slot_" + slotOf(bit), true)) mask |= bit;
        }
        return new CosmeticsState(
                stateFlag(obj, COSMETICS_OWN, true),
                stateFlag(obj, COSMETICS_OTHERS, true),
                stateFlag(obj, COSMETICS_EMOTES, true),
                stateFlag(obj, COSMETICS_PHYSICS, true),
                mask);
    }

    /** {@link Loadout} slot id for a cosmetics slot bit. */
    public static String slotOf(int bit) {
        return switch (bit) {
            case CosmeticsState.BIT_ESPALDA -> "espalda";
            case CosmeticsState.BIT_CUELLO -> "cuello";
            case CosmeticsState.BIT_CABEZA -> "cabeza";
            case CosmeticsState.BIT_ELITROS -> "elitros";
            default -> "";
        };
    }

    /** Boolean state key inside a feature object, with a contract default. */
    private static boolean stateFlag(JsonObject obj, String key, boolean fallback) {
        if (obj == null || !obj.has(key)) return fallback;
        JsonElement el = obj.get(key);
        if (el == null || !el.isJsonPrimitive() || !el.getAsJsonPrimitive().isBoolean()) {
            return fallback;
        }
        return el.getAsBoolean();
    }

    /**
     * Reloads the config file if it changed on disk since the last load/save
     * (e.g. a launcher PATCH while the game runs). Called throttled from the
     * client tick (~every 20 ticks). A reload right after our own screen-save
     * just re-reads identical content, so it is harmless.
     */
    public synchronized void maybeReload() {
        try {
            Path path = getConfigPath();
            if (!Files.exists(path)) return;
            long current = Files.getLastModifiedTime(path).toMillis();
            if (current != lastKnownModified) {
                LOGGER.info("Config file changed on disk; reloading {}", path);
                load();
            }
        } catch (Exception e) {
            LOGGER.debug("Config maybeReload check failed: {}", e.getMessage());
        }
    }

    private void refreshLastModified() {
        try {
            Path path = getConfigPath();
            if (Files.exists(path)) {
                lastKnownModified = Files.getLastModifiedTime(path).toMillis();
            }
        } catch (Exception e) {
            LOGGER.debug("Config mtime refresh failed: {}", e.getMessage());
        }
    }

    public synchronized void save() {
        Path path = getConfigPath();
        try {
            Files.createDirectories(path.getParent());

            rawRoot.addProperty("schema", SCHEMA_VERSION);
            this.schema = SCHEMA_VERSION;

            JsonObject suiteObj = rawRoot.has("suite") && rawRoot.get("suite").isJsonObject()
                    ? rawRoot.getAsJsonObject("suite")
                    : new JsonObject();
            suiteObj.addProperty("enabled", suiteEnabled);
            rawRoot.add("suite", suiteObj);

            JsonObject featObj = rawRoot.has("features") && rawRoot.get("features").isJsonObject()
                    ? rawRoot.getAsJsonObject("features")
                    : new JsonObject();

            for (Map.Entry<String, FeatureConfig> entry : features.entrySet()) {
                FeatureConfig fc = entry.getValue();
                JsonObject obj = fc.rawObject;
                obj.addProperty("enabled", fc.enabled);
                featObj.add(entry.getKey(), obj);
            }
            rawRoot.add("features", featObj);

            JsonArray macArray = new JsonArray();
            for (MacroConfig mc : macros) {
                JsonObject mObj = mc.rawObject;
                mObj.addProperty("id", mc.id);
                mObj.addProperty("name", mc.name);
                mObj.addProperty("keybind", mc.keybind);
                JsonArray aArray = new JsonArray();
                for (MacroAction ma : mc.actions) {
                    JsonObject aObj = new JsonObject();
                    aObj.addProperty("type", ma.type);
                    aObj.addProperty("text", ma.text);
                    aArray.add(aObj);
                }
                mObj.add("actions", aArray);
                macArray.add(mObj);
            }
            rawRoot.add("macros", macArray);

            String jsonStr = GSON.toJson(rawRoot);
            Path tmpPath = path.resolveSibling(path.getFileName().toString() + ".tmp");
            Files.writeString(tmpPath, jsonStr + "\n", StandardCharsets.UTF_8);

            try {
                Files.move(tmpPath, path, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            } catch (IOException atomicEx) {
                Files.move(tmpPath, path, StandardCopyOption.REPLACE_EXISTING);
            }
            LOGGER.info("Persisted config to {}", path);
            refreshLastModified();
        } catch (Exception e) {
            LOGGER.error("Failed to save config to {}: {}", path, e.getMessage());
        }
    }

    /** Master switch state. */
    public synchronized boolean isSuiteEnabled() {
        return suiteEnabled;
    }

    /** Sets the master switch and persists immediately; stored flags are untouched. */
    public synchronized void setSuiteEnabled(boolean enabled) {
        this.suiteEnabled = enabled;
        save();
    }

    /**
     * The ONE choke point every owned feature reads through: suite gate AND
     * stored flag. With the master off everything evaluates false while the
     * stored flags stay unchanged.
     */
    public synchronized boolean isFeatureEnabled(String id) {
        if (!suiteEnabled) return false;
        return isFeatureEnabledRaw(id);
    }

    /** Stored flag with the master ignored (for UI state that must stay operable). */
    public synchronized boolean isFeatureEnabledRaw(String id) {
        FeatureConfig fc = features.get(id);
        return fc != null && fc.enabled;
    }

    // --- personalization (Workstream 4, free features) ----------------------
    // Lives in the top-level "personalization" object of espectral-client.json;
    // it round-trips untouched through save() via rawRoot, so these are
    // read-only views with the contract defaults when absent or invalid.

    private static final java.util.Set<String> THEME_IDS =
            java.util.Set.of("oro", "carmesi", "amatista", "esmeralda", "glaciar");
    private static final java.util.Set<String> HUD_THEME_IDS =
            java.util.Set.of("cristal", "oro", "minimo", "pixel");

    private JsonObject personalizationObj() {
        return rawRoot != null && rawRoot.has("personalization")
                && rawRoot.get("personalization").isJsonObject()
                ? rawRoot.getAsJsonObject("personalization") : null;
    }

    private String personalizationString(JsonObject p, String key) {
        if (p == null || !p.has(key) || !p.get(key).isJsonPrimitive()
                || !p.get(key).getAsJsonPrimitive().isString()) {
            return null;
        }
        return p.get(key).getAsString();
    }

    /** Client colour theme id (oro|carmesi|amatista|esmeralda|glaciar); default oro. */
    public synchronized String theme() {
        String v = personalizationString(personalizationObj(), "theme");
        return v != null && THEME_IDS.contains(v) ? v : "oro";
    }

    /** HUD theme id (cristal|oro|minimo|pixel); default cristal. */
    public synchronized String hudTheme() {
        String v = personalizationString(personalizationObj(), "hud_theme");
        return v != null && HUD_THEME_IDS.contains(v) ? v : "cristal";
    }

    /** Menu background pack id, or {@code null} for the default background. */
    public synchronized String backgroundPack() {
        String v = personalizationString(personalizationObj(), "background");
        return v != null && !v.isEmpty() ? v : null;
    }

    /** Longest custom title-screen tagline (matches the launcher's limit). */
    public static final int TAGLINE_MAX = 40;

    /** Raw value {@link #taglineClean} was derived from (read every frame). */
    private String taglineRaw;
    private String taglineClean;

    /**
     * Custom title-screen tagline, or {@code null} for the localized default.
     * Hand-edited files are sanitised the same way the launcher validates:
     * trimmed, § codes and control characters dropped, capped in length.
     * Cached per raw value — the title screen asks every frame.
     */
    public synchronized String tagline() {
        String v = personalizationString(personalizationObj(), "tagline");
        if (v == null) {
            return null;
        }
        if (!v.equals(taglineRaw)) {
            String clean = v.replaceAll("[\\p{Cntrl}\u00a7]", "").trim();
            if (clean.codePointCount(0, clean.length()) > TAGLINE_MAX) {
                clean = clean.substring(0, clean.offsetByCodePoints(0, TAGLINE_MAX)).trim();
            }
            taglineRaw = v;
            taglineClean = clean.isEmpty() ? null : clean;
        }
        return taglineClean;
    }

    /** Reduced-motion flag; default false. */
    public synchronized boolean reducedMotion() {
        JsonObject p = personalizationObj();
        return p != null && p.has("reduced_motion") && p.get("reduced_motion").isJsonPrimitive()
                && p.get("reduced_motion").getAsJsonPrimitive().isBoolean()
                && p.get("reduced_motion").getAsBoolean();
    }

    /**
     * Resets every registry feature to its canonical default. The master
     * switch and unknown (non-registry) feature ids are left untouched.
     * Persists immediately so a screen re-init (which reloads from disk)
     * can never revert the reset.
     */
    public synchronized void resetFeaturesToDefaults() {
        for (FeatureRegistry.Feature f : FeatureRegistry.all()) {
            FeatureConfig fc = features.get(f.id());
            if (fc != null) {
                fc.enabled = f.defaultEnabled();
                fc.rawObject.addProperty("enabled", f.defaultEnabled());
            } else {
                features.put(f.id(), new FeatureConfig(f.defaultEnabled(), null));
            }
        }
        this.cosmeticsState = readCosmeticsState();
        save();
    }
    /**
     * Stores one feature flag and persists immediately, exactly like the
     * master switch: toggling from the Suite screen must survive Done/Esc
     * (which reload from disk on the next open), not just live in memory.
     */
    public synchronized void setFeatureEnabled(String id, boolean enabled) {
        FeatureConfig fc = features.get(id);
        if (fc != null) {
            fc.enabled = enabled;
            fc.rawObject.addProperty("enabled", enabled);
        } else {
            JsonObject obj = new JsonObject();
            obj.addProperty("enabled", enabled);
            features.put(id, new FeatureConfig(enabled, obj));
        }
        save();
    }

    /**
     * Stored pixel position for a HUD overlay block, or {@code null} when the
     * feature still uses its default anchor. Positions live inside the
     * feature's raw object ({@code features.<id>.x/y}) so no schema bump is
     * needed and a reset-to-defaults leaves them intact.
     */
    public synchronized int[] getFeaturePos(String id) {
        FeatureConfig fc = features.get(id);
        if (fc == null) return null;
        JsonObject o = fc.rawObject;
        if (!o.has("x") || !o.has("y")) return null;
        try {
            return new int[] { o.get("x").getAsInt(), o.get("y").getAsInt() };
        } catch (Exception e) {
            return null;
        }
    }

    /** Persists a dragged overlay position; saves immediately like toggles. */
    public synchronized void setFeaturePos(String id, int x, int y) {
        FeatureConfig fc = features.get(id);
        if (fc == null) {
            fc = new FeatureConfig(false, null);
            features.put(id, fc);
        }
        fc.rawObject.addProperty("x", x);
        fc.rawObject.addProperty("y", y);
        save();
    }

    /** Clears a dragged position so the overlay returns to its default anchor. */
    public synchronized void clearFeaturePos(String id) {
        FeatureConfig fc = features.get(id);
        if (fc != null) {
            fc.rawObject.remove("x");
            fc.rawObject.remove("y");
            save();
        }
    }


    public synchronized List<MacroConfig> getMacros() {
        return Collections.unmodifiableList(new ArrayList<>(macros));
    }

    // --- cosmetics viewer state -------------------------------------------

    /**
     * Lock-free cosmetics toggles for the render/tick threads. Never touches
     * the file, Gson or this instance's monitor: the snapshot is rebuilt on
     * load and on every {@link #setCosmeticsState}, so a render-thread read is
     * one volatile field load.
     */
    public CosmeticsState cosmeticsState() {
        return cosmeticsState;
    }

    /**
     * Contract default for one cosmetics state key (everything on). Reads the
     * volatile snapshot only — no monitor, no file access — so the render
     * thread can call it.
     */
    public boolean getCosmeticsState(String key) {
        CosmeticsState s = cosmeticsState;
        if (COSMETICS_OWN.equals(key)) return s.own;
        if (COSMETICS_OTHERS.equals(key)) return s.others;
        if (COSMETICS_EMOTES.equals(key)) return s.emotes;
        if (COSMETICS_PHYSICS.equals(key)) return s.physics;
        if (key != null && key.startsWith("slot_")) return s.slotVisible(key.substring(5));
        return true;
    }

    /**
     * Stores one cosmetics toggle inside {@code features.cosmetics.<key>} and
     * persists immediately (the Suite screens reload from disk on open).
     * Unknown keys are ignored so the launcher's schema stays the contract.
     */
    public synchronized void setCosmeticsState(String key, boolean value) {
        if (!isCosmeticsStateKey(key)) return;
        FeatureConfig fc = features.get("cosmetics");
        if (fc == null) {
            fc = new FeatureConfig(defaultFeatureEnabled("cosmetics"), null);
            features.put("cosmetics", fc);
        }
        fc.rawObject.addProperty(key, value);
        this.cosmeticsState = readCosmeticsState();
        save();
    }

    private static boolean defaultFeatureEnabled(String id) {
        FeatureRegistry.Feature feature = FeatureRegistry.findById(id);
        return feature != null && feature.defaultEnabled();
    }

    // --- macro list mutations (Suite → Macros sub-screen) ------------------

    /** Upper bound the launcher accepts ({@code MAX_MACROS} in client.mjs). */
    public static final int MAX_MACROS = 32;
    /** Upper bound for the macro label; launcher limit is 40. */
    public static final int MAX_MACRO_NAME = 40;
    /** Chat limit, and the launcher's per-action text cap. */
    public static final int MAX_MACRO_TEXT = 256;

    /**
     * Inserts or replaces a macro, matched by id. Callers pass a sanitized id
     * ({@code [A-Za-z0-9_-]{1,32}}); the launcher's PATCH validation owns
     * that rule and this only keeps the file parseable. Persists immediately.
     * Returns false when the list is at {@link #MAX_MACROS} and the id is new.
     */
    public synchronized boolean putMacro(String id, String name, String keybind,
            String actionType, String text) {
        if (id == null || id.isEmpty()) return false;
        String cleanText = clamp(text, MAX_MACRO_TEXT);
        // The launcher rejects empty action text (BAD_MACRO): refuse rather
        // than write a file the launcher can no longer PATCH.
        if (cleanText.isEmpty()) return false;
        // The launcher rejects empty names too; derive one from the text.
        String cleanName = clamp(name, MAX_MACRO_NAME);
        if (cleanName.isEmpty()) {
            String base = cleanText.startsWith("/") ? cleanText.substring(1) : cleanText;
            cleanName = clamp(base, MAX_MACRO_NAME);
            if (cleanName.isEmpty()) cleanName = id;
        }
        ClientConfig.MacroAction action = new MacroAction(
                cleanText.startsWith("/") ? "command" : actionType, cleanText);

        for (int i = 0; i < macros.size(); i++) {
            MacroConfig existing = macros.get(i);
            if (!existing.id.equals(id)) continue;
            // The editor owns the FIRST action; any further actions a launcher
            // macro carried are kept verbatim instead of being dropped.
            List<MacroAction> actions = new ArrayList<>();
            actions.add(action);
            for (int a = 1; a < existing.actions.size(); a++) {
                actions.add(existing.actions.get(a));
            }
            MacroConfig updated = new MacroConfig(id, cleanName, keybind,
                    actions, existing.rawObject);
            macros.set(i, updated);
            save();
            return true;
        }
        if (macros.size() >= MAX_MACROS) return false;
        macros.add(new MacroConfig(id, cleanName, keybind, List.of(action), null));
        save();
        return true;
    }

    /** Removes a macro by id; persists immediately. Returns true when found. */
    public synchronized boolean removeMacro(String id) {
        if (id == null) return false;
        boolean removed = macros.removeIf(macro -> macro.id.equals(id));
        if (removed) save();
        return removed;
    }

    /**
     * Unique macro id derived from {@code seed}: lowercased ASCII word chars,
     * numeric suffix when it would collide. Always matches the launcher's
     * {@code ^[A-Za-z0-9_-]{1,32}$} so a config written here passes PATCH.
     */
    public synchronized String newMacroId(String seed) {
        String base = seed == null ? "" : seed.toLowerCase(Locale.ROOT)
                .replaceAll("[^a-z0-9_-]", "-")
                .replaceAll("^-+|-+$", "");
        if (base.isEmpty()) base = "macro";
        if (base.length() > 28) base = base.substring(0, 28);
        String candidate = base;
        int n = 2;
        while (macroIdTaken(candidate)) {
            candidate = base + "-" + n++;
        }
        return candidate;
    }

    private boolean macroIdTaken(String id) {
        for (MacroConfig macro : macros) {
            if (macro.id.equals(id)) return true;
        }
        return false;
    }

    private static String clamp(String value, int max) {
        if (value == null) return "";
        String trimmed = value.trim();
        return trimmed.length() <= max ? trimmed : trimmed.substring(0, max);
    }
}
