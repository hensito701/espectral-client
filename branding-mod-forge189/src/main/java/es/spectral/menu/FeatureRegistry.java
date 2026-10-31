package es.spectral.menu;

import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Canonical in-game feature registry: {@code native/engine/data/suite-registry.json}
 * from the launcher repo, copied into the jar by the build as
 * {@code assets/espectralmenu/suite-registry.json}. Never a second hand-kept list.
 * Names/descriptions live in the generated lang files ({@link #nameKey}).
 *
 * <p>A missing or unparseable resource logs one warning and behaves as an
 * empty registry. Features the 1.8.9 port does not implement yet (cosmetics,
 * badges) stay in {@link #all()} so their config round-trips, but are left out
 * of {@link #available()}, which is what the Suite screen lists.
 */
public final class FeatureRegistry {

    /** Jar resource carrying the canonical registry (generated, never hand-edited). */
    public static final String REGISTRY_RESOURCE = "assets/espectralmenu/suite-registry.json";

    /** Registry features not available on 1.8.9 (cosmetics land later). */
    private static final Set<String> NOT_ON_1_8_9 = new HashSet<String>(Arrays.asList("cosmetics", "badges"));

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    public static final class Feature {
        public final String id;
        public final String category;
        public final String kind;
        public final boolean defaultEnabled;
        /** Modern key name ({@code key.keyboard.z}) or null. */
        public final String keybind;

        public Feature(String id, String category, String kind, boolean defaultEnabled, String keybind) {
            this.id = id;
            this.category = category;
            this.kind = kind;
            this.defaultEnabled = defaultEnabled;
            this.keybind = keybind;
        }
    }

    private static final List<Feature> ALL;
    private static final List<String> CATEGORY_IDS;

    static {
        List<Feature> features = Collections.emptyList();
        List<String> categories = Collections.emptyList();
        InputStream in = FeatureRegistry.class.getClassLoader().getResourceAsStream(REGISTRY_RESOURCE);
        if (in == null) {
            LOGGER.warn("Suite registry resource {} missing from jar; behaving as an empty registry", REGISTRY_RESOURCE);
        } else {
            try (Reader reader = new InputStreamReader(in, StandardCharsets.UTF_8)) {
                List<String> cats = new ArrayList<String>();
                features = parse(new JsonParser().parse(reader), cats);
                categories = cats;
            } catch (Exception e) {
                LOGGER.warn("Failed to parse suite registry {}; behaving as an empty registry: {}",
                        REGISTRY_RESOURCE, e.getMessage());
                features = Collections.emptyList();
                categories = Collections.emptyList();
            }
        }
        ALL = Collections.unmodifiableList(features);
        CATEGORY_IDS = Collections.unmodifiableList(categories);
    }

    private FeatureRegistry() {}

    /** Parses a registry document; category ids (registry order) go to {@code categoriesOut}. */
    static List<Feature> parse(JsonElement root, List<String> categoriesOut) {
        List<Feature> features = new ArrayList<Feature>();
        if (root == null || !root.isJsonObject()) return features;
        JsonObject rootObj = root.getAsJsonObject();
        if (rootObj.has("categories") && rootObj.get("categories").isJsonArray()) {
            for (JsonElement elem : rootObj.getAsJsonArray("categories")) {
                if (elem.isJsonObject() && elem.getAsJsonObject().has("id")) {
                    String catId = elem.getAsJsonObject().get("id").getAsString();
                    if (!catId.isEmpty() && !categoriesOut.contains(catId)) categoriesOut.add(catId);
                }
            }
        }
        if (rootObj.has("features") && rootObj.get("features").isJsonArray()) {
            for (JsonElement elem : rootObj.getAsJsonArray("features")) {
                if (!elem.isJsonObject()) continue;
                JsonObject obj = elem.getAsJsonObject();
                if (!obj.has("id") || obj.get("id").isJsonNull()) continue;
                String id = obj.get("id").getAsString();
                if (id.isEmpty()) continue;
                String category = string(obj, "category", "");
                String kind = string(obj, "kind", "owned");
                JsonElement def = obj.get("default_enabled");
                boolean defaultEnabled = def != null && def.isJsonPrimitive() && def.getAsJsonPrimitive().isBoolean()
                        && def.getAsBoolean();
                features.add(new Feature(id, category, kind, defaultEnabled, string(obj, "keybind", null)));
                if (!category.isEmpty() && !categoriesOut.contains(category)) categoriesOut.add(category);
            }
        }
        return features;
    }

    private static String string(JsonObject obj, String key, String fallback) {
        JsonElement el = obj.get(key);
        return el != null && !el.isJsonNull() ? el.getAsString() : fallback;
    }

    /** All features in canonical registry order. */
    public static List<Feature> all() {
        return ALL;
    }

    /** True when the 1.8.9 port implements {@code id}. */
    public static boolean isAvailable(String id) {
        return !NOT_ON_1_8_9.contains(id);
    }

    /** Features of {@code categoryId} the 1.8.9 port implements (canonical order). */
    public static List<Feature> available(String categoryId) {
        List<Feature> out = new ArrayList<Feature>();
        for (Feature f : ALL) {
            if (isAvailable(f.id) && (categoryId == null || f.category.equalsIgnoreCase(categoryId))) out.add(f);
        }
        return out;
    }

    /** Category ids that have at least one available feature, in registry order. */
    public static List<String> availableCategories() {
        List<String> out = new ArrayList<String>();
        for (String c : CATEGORY_IDS) {
            if (!available(c).isEmpty()) out.add(c);
        }
        return out;
    }

    public static Feature findById(String id) {
        if (id == null) return null;
        for (Feature f : ALL) {
            if (f.id.equalsIgnoreCase(id)) return f;
        }
        return null;
    }

    /** True when the jar carried a parseable, non-empty registry. */
    public static boolean isLoaded() {
        return !ALL.isEmpty();
    }

    public static String nameKey(String id) {
        return "espectral.feature." + id + ".name";
    }

    public static String descriptionKey(String id) {
        return "espectral.feature." + id + ".description";
    }

    public static String categoryKey(String categoryId) {
        return "espectral.category." + categoryId;
    }
}
