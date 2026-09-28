package es.spectral.menu.cosmetics;

import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The bundled cosmetics catalog ({@code assets/espectral-menu/cosmetics-catalog.json},
 * synced byte-identical from {@code src/engine/cosmetics-catalog.json}).
 * Immutable once loaded; a missing/broken resource yields an empty catalog
 * (everything renders nothing) instead of a crash.
 */
public final class Catalog {

    /** Jar resource carrying the canonical catalog (generated, never hand-edited). */
    private static final String RESOURCE = "assets/espectral-menu/cosmetics-catalog.json";

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    /** One catalog entry: the slot it occupies and the flag it requires. */
    public record Item(String id, String slot, String requires, Map<String, String> names) {
        public String name(String lang) {
            String n = names.get(lang);
            return n != null ? n : names.getOrDefault("en", id);
        }
    }

    private static final Catalog INSTANCE = load();

    private final Map<String, Item> items;
    private final List<String> slots;
    /** Rules: feature in {@code slot} conflicts with every id in the set. */
    private final Map<String, Set<String>> exclusivity;

    private Catalog(Map<String, Item> items, List<String> slots, Map<String, Set<String>> exclusivity) {
        this.items = items;
        this.slots = slots;
        this.exclusivity = exclusivity;
    }

    public static Catalog get() {
        return INSTANCE;
    }

    public Item item(String id) {
        return id == null ? null : items.get(id);
    }

    public boolean isItem(String id) {
        return id != null && items.containsKey(id);
    }

    public List<String> slots() {
        return slots;
    }

    /** True when {@code itemId} may be equipped given {@code flags}. */
    public boolean allowed(String itemId, Set<String> flags) {
        Item item = item(itemId);
        if (item == null) return false;
        return item.requires() == null || item.requires().isEmpty()
                || (flags != null && flags.contains(item.requires()));
    }

    /**
     * Drops every slot entry that is not a catalog item or whose required flag
     * is missing, then applies exclusivity: an equipped {@code cuello} removes
     * conflicting {@code espalda} items (rule orientation: the new neck item
     * wins; the conflicting cloaks fall off).
     */
    public Loadout filter(Loadout loadout, Set<String> flags) {
        if (loadout == null) return Loadout.EMPTY;
        Loadout kept = new Loadout(
                keep("espalda", loadout.espalda(), flags),
                keep("cuello", loadout.cuello(), flags),
                keep("cabeza", loadout.cabeza(), flags),
                keep("brazos", loadout.brazos(), flags),
                keep("elitros", loadout.elitros(), flags));
        return applyExclusivity(kept);
    }

    /** Keeps an id only when it is a catalog item for {@code slot} and allowed. */
    private String keep(String slot, String itemId, Set<String> flags) {
        if (itemId == null || itemId.isEmpty()) return null;
        Item item = item(itemId);
        if (item == null || !slot.equals(item.slot())) return null;
        return allowed(itemId, flags) ? itemId : null;
    }

    /** Parses a raw loadout map ({@code slot -> itemId}) into a {@link Loadout}. */
    public Loadout fromMap(Map<String, String> bySlot) {
        if (bySlot == null) return Loadout.EMPTY;
        String espalda = inSlot("espalda", bySlot.get("espalda"));
        String cuello = inSlot("cuello", bySlot.get("cuello"));
        String cabeza = inSlot("cabeza", bySlot.get("cabeza"));
        String brazos = inSlot("brazos", bySlot.get("brazos"));
        String elitros = inSlot("elitros", bySlot.get("elitros"));
        return new Loadout(espalda, cuello, cabeza, brazos, elitros);
    }

    /** Accepts only ids that are catalog items declared for {@code slot}. */
    private String inSlot(String slot, String itemId) {
        Item item = item(itemId);
        return item != null && slot.equals(item.slot()) ? itemId : null;
    }

    private Loadout applyExclusivity(Loadout loadout) {
        for (Map.Entry<String, Set<String>> rule : exclusivity.entrySet()) {
            String equipped = loadout.forSlot(rule.getKey());
            if (equipped == null) continue;
            Set<String> blocked = rule.getValue();
            if (blocked.contains(loadout.espalda())) {
                loadout = new Loadout(null, loadout.cuello(), loadout.cabeza(),
                        loadout.brazos(), loadout.elitros());
            }
            if (blocked.contains(loadout.cuello())) {
                loadout = new Loadout(loadout.espalda(), null, loadout.cabeza(),
                        loadout.brazos(), loadout.elitros());
            }
            if (blocked.contains(loadout.cabeza())) {
                loadout = new Loadout(loadout.espalda(), loadout.cuello(), null,
                        loadout.brazos(), loadout.elitros());
            }
            if (blocked.contains(loadout.brazos())) {
                loadout = new Loadout(loadout.espalda(), loadout.cuello(), loadout.cabeza(),
                        null, loadout.elitros());
            }
            if (blocked.contains(loadout.elitros())) {
                loadout = new Loadout(loadout.espalda(), loadout.cuello(), loadout.cabeza(),
                        loadout.brazos(), null);
            }
        }
        return loadout;
    }

    private static Catalog load() {
        Map<String, Item> items = new HashMap<>();
        List<String> slots = new ArrayList<>();
        Map<String, Set<String>> exclusivity = new HashMap<>();
        try (InputStream in = Catalog.class.getClassLoader().getResourceAsStream(RESOURCE)) {
            if (in == null) {
                LOGGER.warn("Cosmetics catalog {} missing from jar; cosmetics disabled", RESOURCE);
            } else {
                JsonObject root = JsonParser.parseReader(
                        new InputStreamReader(in, StandardCharsets.UTF_8)).getAsJsonObject();
                JsonArray slotArr = root.getAsJsonArray("slots");
                if (slotArr != null) {
                    for (JsonElement e : slotArr) slots.add(e.getAsString());
                }
                JsonArray exArr = root.getAsJsonArray("exclusive");
                if (exArr != null) {
                    for (JsonElement e : exArr) {
                        JsonArray pair = e.getAsJsonArray();
                        if (pair.size() < 2) continue;
                        String slot = pair.get(0).getAsString();
                        Set<String> blocked = new HashSet<>();
                        for (JsonElement id : pair.get(1).getAsJsonArray()) {
                            blocked.add(id.getAsString());
                        }
                        exclusivity.put(slot, blocked);
                    }
                }
                JsonArray itemArr = root.getAsJsonArray("items");
                if (itemArr != null) {
                    for (JsonElement e : itemArr) {
                        JsonObject o = e.getAsJsonObject();
                        String id = o.get("id").getAsString();
                        String slot = o.get("slot").getAsString();
                        String requires = o.has("requires") ? o.get("requires").getAsString() : null;
                        Map<String, String> names = new HashMap<>();
                        if (o.has("name") && o.get("name").isJsonObject()) {
                            for (Map.Entry<String, JsonElement> n : o.getAsJsonObject("name").entrySet()) {
                                names.put(n.getKey(), n.getValue().getAsString());
                            }
                        }
                        items.put(id, new Item(id, slot, requires, Map.copyOf(names)));
                    }
                }
            }
        } catch (Exception e) {
            LOGGER.warn("Failed to parse cosmetics catalog {}; cosmetics disabled: {}",
                    RESOURCE, e.getMessage());
            items.clear();
            slots.clear();
            exclusivity.clear();
        }
        return new Catalog(Map.copyOf(items), List.copyOf(slots), Map.copyOf(exclusivity));
    }
}
