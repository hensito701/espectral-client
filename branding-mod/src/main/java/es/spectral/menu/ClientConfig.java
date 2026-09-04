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

/**
 * Versioned config store for <instanceDir>/config/espectral-client.json (Contract A).
 * Reads, preserves unknown fields, and writes atomically.
 */
public final class ClientConfig {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();
    private static final ClientConfig INSTANCE = new ClientConfig();

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
            this.actions = actions != null ? actions : Collections.emptyList();
            this.rawObject = rawObject != null ? rawObject : new JsonObject();
        }
    }

    private int schema = 1;
    private final Map<String, FeatureConfig> features = new HashMap<>();
    private final List<MacroConfig> macros = new ArrayList<>();
    private JsonObject rawRoot = new JsonObject();
    /** Last observed config-file mtime (millis); drives throttled live reload. */
    private long lastKnownModified = -1L;

    private ClientConfig() {
        initDefaults();
    }

    private void initDefaults() {
        features.put("fullbright", new FeatureConfig(true, null));
        features.put("nofog", new FeatureConfig(true, null));
        features.put("zoom", new FeatureConfig(true, null));
        features.put("macros", new FeatureConfig(true, null));
        features.put("potionstatus", new FeatureConfig(false, null));
        features.put("coords", new FeatureConfig(false, null));
        features.put("healthstatus", new FeatureConfig(false, null));
        features.put("armorstatus", new FeatureConfig(false, null));
        features.put("fpsping", new FeatureConfig(false, null));
        features.put("lowfire", new FeatureConfig(false, null));
        features.put("clearwater", new FeatureConfig(false, null));
        features.put("chatheads", new FeatureConfig(false, null));
        features.put("skin3d", new FeatureConfig(false, null));
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
                            boolean en = obj.has("enabled") && obj.get("enabled").getAsBoolean();
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
                this.schema = parsedSchema;
                this.features.clear();
                this.features.putAll(parsedFeatures);
                this.macros.clear();
                this.macros.addAll(parsedMacros);
            }
        } catch (Exception e) {
            LOGGER.warn("Failed to parse config {}: {}; preserving memory state", path, e.getMessage());
        }
        refreshLastModified();
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

            rawRoot.addProperty("schema", schema);

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

    public synchronized boolean isFeatureEnabled(String id) {
        FeatureConfig fc = features.get(id);
        return fc != null && fc.enabled;
    }

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
    }

    public synchronized List<MacroConfig> getMacros() {
        return Collections.unmodifiableList(new ArrayList<>(macros));
    }
}
