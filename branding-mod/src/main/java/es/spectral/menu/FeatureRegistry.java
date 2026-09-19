package es.spectral.menu;

import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Canonical in-game feature registry.
 *
 * <p>The single source of truth is {@code src/engine/data/suite-registry.json}
 * in the launcher repo; the generator script copies it into the mod jar as
 * {@code assets/espectral-menu/suite-registry.json}. This class loads that jar
 * resource with Gson and never hand-maintains a second list, so mod defaults
 * cannot drift from the launcher's (notably {@code nofog} defaulting to false
 * everywhere). User-visible names/descriptions live in the jar's language
 * files, generated from the same registry — see {@link #nameKey(String)}.
 *
 * <p>If the resource is missing or unparseable, one warning is logged and the
 * registry behaves as empty (no crash); {@link ClientConfig} then simply has
 * no seeded defaults and every stored-flag lookup is false.
 */
public final class FeatureRegistry {

    /** Jar resource carrying the canonical registry (generated, never hand-edited). */
    private static final String REGISTRY_RESOURCE = "assets/espectral-menu/suite-registry.json";

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    public record Feature(String id, String category, String kind, boolean defaultEnabled, String keybind) {
        public boolean isManaged() {
            return "managed".equalsIgnoreCase(kind);
        }

        public boolean isOwned() {
            return "owned".equalsIgnoreCase(kind);
        }
    }

    private static final List<Feature> ALL;
    private static final List<String> CATEGORY_IDS;
    private static final boolean LOADED;

    static {
        List<Feature> features = new ArrayList<>();
        List<String> categories = new ArrayList<>();
        boolean loaded = false;
        try (InputStream in = FeatureRegistry.class.getClassLoader().getResourceAsStream(REGISTRY_RESOURCE)) {
            if (in == null) {
                LOGGER.warn("Suite registry resource {} missing from jar; behaving as an empty registry",
                        REGISTRY_RESOURCE);
            } else {
                JsonElement root = JsonParser.parseReader(
                        new InputStreamReader(in, StandardCharsets.UTF_8));
                if (root != null && root.isJsonObject()) {
                    JsonObject rootObj = root.getAsJsonObject();
                    if (rootObj.has("categories") && rootObj.get("categories").isJsonArray()) {
                        for (JsonElement elem : rootObj.getAsJsonArray("categories")) {
                            if (elem.isJsonObject()) {
                                JsonObject catObj = elem.getAsJsonObject();
                                if (catObj.has("id")) {
                                    String catId = catObj.get("id").getAsString();
                                    if (catId != null && !catId.isEmpty() && !categories.contains(catId)) {
                                        categories.add(catId);
                                    }
                                }
                            }
                        }
                    }
                    if (rootObj.has("features") && rootObj.get("features").isJsonArray()) {
                        JsonArray featArray = rootObj.getAsJsonArray("features");
                        for (JsonElement elem : featArray) {
                            if (!elem.isJsonObject()) continue;
                            JsonObject obj = elem.getAsJsonObject();
                            if (!obj.has("id") || obj.get("id").isJsonNull()) continue;
                            String id = obj.get("id").getAsString();
                            if (id == null || id.isEmpty()) continue;
                            String category = obj.has("category") && !obj.get("category").isJsonNull()
                                    ? obj.get("category").getAsString() : "";
                            String kind = obj.has("kind") && !obj.get("kind").isJsonNull()
                                    ? obj.get("kind").getAsString() : "owned";
                            boolean defaultEnabled = obj.has("default_enabled")
                                    && obj.get("default_enabled").isJsonPrimitive()
                                    && obj.get("default_enabled").getAsJsonPrimitive().isBoolean()
                                    ? obj.get("default_enabled").getAsBoolean()
                                    : false;
                            String keybind = obj.has("keybind") && !obj.get("keybind").isJsonNull()
                                    ? obj.get("keybind").getAsString()
                                    : null;
                            features.add(new Feature(id, category != null ? category : "",
                                    kind != null ? kind : "owned", defaultEnabled, keybind));
                            if (category != null && !category.isEmpty() && !categories.contains(category)) {
                                categories.add(category);
                            }
                        }
                    }
                }
                // An empty parse is not a loaded registry: all(), findById() and
                // the config defaults derived from them must never look live.
                loaded = !features.isEmpty();
            }
        } catch (Exception e) {
            LOGGER.warn("Failed to parse suite registry resource {}; behaving as an empty registry: {}",
                    REGISTRY_RESOURCE, e.getMessage());
            features.clear();
            categories.clear();
        }
        ALL = Collections.unmodifiableList(features);
        CATEGORY_IDS = Collections.unmodifiableList(categories);
        LOADED = loaded;
    }

    private FeatureRegistry() {}

    /** All features in canonical registry order. */
    public static List<Feature> all() {
        return ALL;
    }

    /**
     * Features of one category; null or "all" returns everything (canonical order).
     */
    public static List<Feature> byCategory(String id) {
        if (id == null || id.equalsIgnoreCase("all")) return ALL;
        List<Feature> out = new ArrayList<>();
        for (Feature f : ALL) {
            if (f.category().equalsIgnoreCase(id)) out.add(f);
        }
        return Collections.unmodifiableList(out);
    }

    public static Feature findById(String id) {
        if (id == null) return null;
        for (Feature f : ALL) {
            if (f.id().equalsIgnoreCase(id)) return f;
        }
        return null;
    }

    /** Category ids in registry order (visual, hud, chat, controls). */
    public static List<String> categoryIds() {
        return CATEGORY_IDS;
    }

    /** True when the jar carried a parseable registry. */
    public static boolean isLoaded() {
        return LOADED;
    }

    /** Language key for a feature's display name. */
    public static String nameKey(String id) {
        return "espectral.feature." + id + ".name";
    }

    /** Language key for a feature's description. */
    public static String descriptionKey(String id) {
        return "espectral.feature." + id + ".description";
    }

    /** Language key for a category's display name. */
    public static String categoryKey(String categoryId) {
        return "espectral.category." + categoryId;
    }
}
