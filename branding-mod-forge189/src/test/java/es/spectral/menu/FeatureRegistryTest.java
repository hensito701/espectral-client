package es.spectral.menu;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import java.util.ArrayList;
import java.util.List;

import org.junit.Test;

import com.google.gson.JsonParser;

/** The jar carries the launcher's registry; 1.8.9 lists only what it implements. */
public class FeatureRegistryTest {

    @Test
    public void bundledRegistryLoadsWithDefaultsAndKeybinds() {
        assertTrue(FeatureRegistry.isLoaded());
        FeatureRegistry.Feature zoom = FeatureRegistry.findById("zoom");
        assertNotNull(zoom);
        assertEquals("key.keyboard.z", zoom.keybind);
        assertTrue(zoom.defaultEnabled);
        assertFalse(FeatureRegistry.findById("nofog").defaultEnabled);
    }

    @Test
    public void cosmeticsAndBadgesAreNotOfferedOn189() {
        for (FeatureRegistry.Feature f : FeatureRegistry.available(null)) {
            assertFalse(f.id, f.id.equals("cosmetics") || f.id.equals("badges"));
        }
        assertFalse(FeatureRegistry.availableCategories().contains("cosmetics"));
        assertNotNull("still known for config round-trips", FeatureRegistry.findById("cosmetics"));
    }

    @Test
    public void parseKeepsRegistryOrderAndCategories() {
        List<String> categories = new ArrayList<String>();
        List<FeatureRegistry.Feature> features = FeatureRegistry.parse(new JsonParser().parse(
                "{\"categories\":[{\"id\":\"visual\"},{\"id\":\"hud\"}],\"features\":["
                        + "{\"id\":\"b\",\"category\":\"hud\",\"default_enabled\":true},"
                        + "{\"id\":\"a\",\"category\":\"extra\"},{\"category\":\"hud\"}]}"), categories);
        assertEquals(2, features.size());
        assertEquals("b", features.get(0).id);
        assertTrue(features.get(0).defaultEnabled);
        assertEquals("owned", features.get(1).kind);
        assertEquals(3, categories.size());
        assertEquals("extra", categories.get(2));
    }
}
