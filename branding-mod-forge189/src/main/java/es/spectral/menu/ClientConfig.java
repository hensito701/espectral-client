package es.spectral.menu;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Versioned config store for &lt;instanceDir&gt;/config/espectral-client.json (schema 2):
 * the same file, schema and semantics as the modern Fabric mod's
 * {@code ClientConfig}, so the launcher's PATCH /client and both mods read
 * and write each other's files. Reads, preserves unknown fields (including
 * the cosmetics feature's state keys, which 1.8.9 does not use), and writes
 * atomically.
 *
 * <p>{@link #isFeatureEnabled(String)} is the ONE choke point every owned
 * feature reads through: suite gate AND stored flag. The stored flags are
 * never rewritten by the gate, so flipping the master off and on restores the
 * previous feature states. A v1 file migrates in memory (master defaults to
 * true) and the next save writes schema 2.
 *
 * <p>Feature defaults come from {@link FeatureRegistry} (the launcher's
 * canonical registry copied into the jar), never from a second list.
 */
public final class ClientConfig {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().disableHtmlEscaping().create();
    private static final ClientConfig INSTANCE = new ClientConfig();

    /** Current config schema; written on every save. */
    public static final int SCHEMA_VERSION = 2;

    public static ClientConfig getInstance() {
        return INSTANCE;
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
            this.actions = actions != null ? actions : Collections.<MacroAction>emptyList();
            this.rawObject = rawObject != null ? rawObject : new JsonObject();
        }
    }

    /** Directory holding espectral-client.json; set at mod init (FML config dir). */
    private static volatile File configDir = new File("config");

    private int schema = SCHEMA_VERSION;
    /** Master switch; gates every owned feature through {@link #isFeatureEnabled}. */
    private boolean suiteEnabled = true;
    private final Map<String, FeatureConfig> features = new HashMap<String, FeatureConfig>();
    private final List<MacroConfig> macros = new ArrayList<MacroConfig>();
    private JsonObject rawRoot = new JsonObject();
    /** Last observed config-file mtime (millis); drives throttled live reload. */
    private long lastKnownModified = -1L;

    private ClientConfig() {
        initDefaults();
    }

    /** Points the store at {@code dir}/espectral-client.json (FML's config dir). */
    public static void setConfigDir(File dir) {
        configDir = dir;
    }

    public static Path getConfigPath() {
        return new File(configDir, "espectral-client.json").toPath();
    }

    /** Seeds stored flags from the canonical registry. */
    private void initDefaults() {
        features.clear();
        for (FeatureRegistry.Feature f : FeatureRegistry.all()) {
            features.put(f.id, new FeatureConfig(f.defaultEnabled, null));
        }
    }

    public synchronized void load() {
        Path path = getConfigPath();
        if (!Files.exists(path)) {
            LOGGER.info("Config file {} not found; using defaults", path);
            initDefaults();
            rawRoot = new JsonObject();
            suiteEnabled = true;
            macros.clear();
            save();
            return;
        }
        try {
            String content = new String(Files.readAllBytes(path), StandardCharsets.UTF_8);
            JsonElement rootElem = new JsonParser().parse(content);
            if (rootElem != null && rootElem.isJsonObject()) {
                JsonObject rootObj = rootElem.getAsJsonObject();
                int parsedSchema = schema;
                if (rootObj.has("schema") && rootObj.get("schema").isJsonPrimitive()) {
                    parsedSchema = rootObj.get("schema").getAsInt();
                }
                boolean parsedSuite = true;
                if (rootObj.has("suite") && rootObj.get("suite").isJsonObject()) {
                    JsonObject suiteObj = rootObj.getAsJsonObject("suite");
                    if (isBoolean(suiteObj.get("enabled"))) {
                        parsedSuite = suiteObj.get("enabled").getAsBoolean();
                    }
                }
                initDefaults();
                Map<String, FeatureConfig> parsedFeatures = new HashMap<String, FeatureConfig>(this.features);
                if (rootObj.has("features") && rootObj.get("features").isJsonObject()) {
                    for (Map.Entry<String, JsonElement> entry : rootObj.getAsJsonObject("features").entrySet()) {
                        JsonElement val = entry.getValue();
                        if (val.isJsonObject()) {
                            JsonObject obj = val.getAsJsonObject();
                            boolean en = isBoolean(obj.get("enabled")) && obj.get("enabled").getAsBoolean();
                            parsedFeatures.put(entry.getKey(), new FeatureConfig(en, obj));
                        } else if (isBoolean(val)) {
                            boolean en = val.getAsBoolean();
                            JsonObject obj = new JsonObject();
                            obj.addProperty("enabled", en);
                            parsedFeatures.put(entry.getKey(), new FeatureConfig(en, obj));
                        }
                    }
                }
                List<MacroConfig> parsedMacros = new ArrayList<MacroConfig>();
                if (rootObj.has("macros") && rootObj.get("macros").isJsonArray()) {
                    for (JsonElement elem : rootObj.getAsJsonArray("macros")) {
                        if (!elem.isJsonObject()) continue;
                        JsonObject mObj = elem.getAsJsonObject();
                        List<MacroAction> mActs = new ArrayList<MacroAction>();
                        if (mObj.has("actions") && mObj.get("actions").isJsonArray()) {
                            for (JsonElement aElem : mObj.getAsJsonArray("actions")) {
                                if (aElem.isJsonObject()) {
                                    JsonObject aObj = aElem.getAsJsonObject();
                                    mActs.add(new MacroAction(str(aObj, "type", "chat"), str(aObj, "text", "")));
                                }
                            }
                        }
                        parsedMacros.add(new MacroConfig(str(mObj, "id", ""), str(mObj, "name", ""),
                                str(mObj, "keybind", ""), mActs, mObj));
                    }
                }
                this.rawRoot = rootObj;
                this.schema = Math.max(parsedSchema, SCHEMA_VERSION);
                this.suiteEnabled = parsedSuite;
                this.features.clear();
                this.features.putAll(parsedFeatures);
                this.macros.clear();
                this.macros.addAll(parsedMacros);
                LOGGER.info("Loaded config {} (schema {}, {} features, {} macros)", path, parsedSchema,
                        features.size(), macros.size());
            }
        } catch (Exception e) {
            LOGGER.warn("Failed to parse config {}: {}; preserving memory state", path, e.getMessage());
        }
        refreshLastModified();
    }

    private static boolean isBoolean(JsonElement el) {
        return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isBoolean();
    }

    private static String str(JsonObject o, String key, String fallback) {
        JsonElement el = o.get(key);
        return el != null && el.isJsonPrimitive() ? el.getAsString() : fallback;
    }

    /**
     * Reloads the file if it changed on disk since the last load/save (a
     * launcher PATCH while the game runs). Called throttled from the client tick.
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
                    ? rawRoot.getAsJsonObject("suite") : new JsonObject();
            suiteObj.addProperty("enabled", suiteEnabled);
            rawRoot.add("suite", suiteObj);

            JsonObject featObj = rawRoot.has("features") && rawRoot.get("features").isJsonObject()
                    ? rawRoot.getAsJsonObject("features") : new JsonObject();
            for (Map.Entry<String, FeatureConfig> entry : features.entrySet()) {
                FeatureConfig fc = entry.getValue();
                fc.rawObject.addProperty("enabled", fc.enabled);
                featObj.add(entry.getKey(), fc.rawObject);
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

            Path tmpPath = path.resolveSibling(path.getFileName().toString() + ".tmp");
            Files.write(tmpPath, (GSON.toJson(rawRoot) + "\n").getBytes(StandardCharsets.UTF_8));
            try {
                Files.move(tmpPath, path, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            } catch (IOException atomicEx) {
                Files.move(tmpPath, path, StandardCopyOption.REPLACE_EXISTING);
            }
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

    /** Suite gate AND stored flag: the one check every owned feature uses. */
    public synchronized boolean isFeatureEnabled(String id) {
        return suiteEnabled && isFeatureEnabledRaw(id);
    }

    /** Stored flag with the master ignored (for UI state that must stay operable). */
    public synchronized boolean isFeatureEnabledRaw(String id) {
        FeatureConfig fc = features.get(id);
        return fc != null && fc.enabled;
    }

    // --- personalization (read-only views of the launcher's object) --------

    private static final Set<String> THEME_IDS =
            new HashSet<String>(Arrays.asList("oro", "carmesi", "amatista", "esmeralda", "glaciar"));
    private static final Set<String> HUD_THEME_IDS =
            new HashSet<String>(Arrays.asList("cristal", "oro", "minimo", "pixel"));

    private JsonObject personalizationObj() {
        return rawRoot != null && rawRoot.has("personalization") && rawRoot.get("personalization").isJsonObject()
                ? rawRoot.getAsJsonObject("personalization") : null;
    }

    private static String personalizationString(JsonObject p, String key) {
        if (p == null || !p.has(key) || !p.get(key).isJsonPrimitive() || !p.get(key).getAsJsonPrimitive().isString()) {
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

    private String taglineRaw;
    private String taglineClean;

    /**
     * Custom title-screen tagline, or {@code null} for the localized default;
     * sanitised like the launcher validates (trim, no § or control chars, capped).
     */
    public synchronized String tagline() {
        String v = personalizationString(personalizationObj(), "tagline");
        if (v == null) return null;
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
        return p != null && isBoolean(p.get("reduced_motion")) && p.get("reduced_motion").getAsBoolean();
    }

    /** Resets every registry feature to its default; master and unknown ids untouched. */
    public synchronized void resetFeaturesToDefaults() {
        for (FeatureRegistry.Feature f : FeatureRegistry.all()) {
            FeatureConfig fc = features.get(f.id);
            if (fc != null) {
                fc.enabled = f.defaultEnabled;
                fc.rawObject.addProperty("enabled", f.defaultEnabled);
            } else {
                features.put(f.id, new FeatureConfig(f.defaultEnabled, null));
            }
        }
        save();
    }

    /** Stores one feature flag and persists immediately. */
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

    /** Stored HUD position ({@code features.<id>.x/y}), or {@code null} for the default anchor. */
    public synchronized int[] getFeaturePos(String id) {
        FeatureConfig fc = features.get(id);
        if (fc == null) return null;
        JsonObject o = fc.rawObject;
        if (!o.has("x") || !o.has("y")) return null;
        try {
            return new int[] {o.get("x").getAsInt(), o.get("y").getAsInt()};
        } catch (Exception e) {
            return null;
        }
    }

    /** Persists a dragged overlay position. */
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
        return Collections.unmodifiableList(new ArrayList<MacroConfig>(macros));
    }

    // --- macro list mutations (Suite → Macros) -----------------------------

    /** Upper bound the launcher accepts. */
    public static final int MAX_MACROS = 32;
    /** Upper bound for the macro label; launcher limit is 40. */
    public static final int MAX_MACRO_NAME = 40;
    /** Chat limit (1.8.9 chat is 100 chars; the launcher caps actions at 256). */
    public static final int MAX_MACRO_TEXT = 256;

    /**
     * Inserts or replaces a macro (matched by id) and persists. Refuses empty
     * text (the launcher rejects it) and a new id once {@link #MAX_MACROS} exist.
     */
    public synchronized boolean putMacro(String id, String name, String keybind, String actionType, String text) {
        if (id == null || id.isEmpty()) return false;
        String cleanText = clamp(text, MAX_MACRO_TEXT);
        if (cleanText.isEmpty()) return false;
        String cleanName = clamp(name, MAX_MACRO_NAME);
        if (cleanName.isEmpty()) {
            String base = cleanText.startsWith("/") ? cleanText.substring(1) : cleanText;
            cleanName = clamp(base, MAX_MACRO_NAME);
            if (cleanName.isEmpty()) cleanName = id;
        }
        MacroAction action = new MacroAction(cleanText.startsWith("/") ? "command" : actionType, cleanText);
        for (int i = 0; i < macros.size(); i++) {
            MacroConfig existing = macros.get(i);
            if (!existing.id.equals(id)) continue;
            // The editor owns the FIRST action; further launcher actions are kept.
            List<MacroAction> actions = new ArrayList<MacroAction>();
            actions.add(action);
            for (int a = 1; a < existing.actions.size(); a++) {
                actions.add(existing.actions.get(a));
            }
            macros.set(i, new MacroConfig(id, cleanName, keybind, actions, existing.rawObject));
            save();
            return true;
        }
        if (macros.size() >= MAX_MACROS) return false;
        macros.add(new MacroConfig(id, cleanName, keybind, Collections.singletonList(action), null));
        save();
        return true;
    }

    /** Removes a macro by id; persists. Returns true when found. */
    public synchronized boolean removeMacro(String id) {
        if (id == null) return false;
        boolean removed = false;
        for (Iterator<MacroConfig> it = macros.iterator(); it.hasNext();) {
            if (it.next().id.equals(id)) {
                it.remove();
                removed = true;
            }
        }
        if (removed) save();
        return removed;
    }

    /** Unique id from {@code seed} matching the launcher's {@code ^[A-Za-z0-9_-]{1,32}$}. */
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
