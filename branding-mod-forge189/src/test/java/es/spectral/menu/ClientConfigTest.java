package es.spectral.menu;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;

import org.junit.Before;
import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/** The config file contract shared with the launcher and the Fabric mod. */
public class ClientConfigTest {

    @Rule
    public TemporaryFolder tmp = new TemporaryFolder();

    private File dir;

    @Before
    public void setUp() throws Exception {
        dir = tmp.newFolder("config");
        ClientConfig.setConfigDir(dir);
    }

    private void write(String json) throws Exception {
        Files.write(new File(dir, "espectral-client.json").toPath(), json.getBytes(StandardCharsets.UTF_8));
    }

    private JsonObject read() throws Exception {
        String text = new String(Files.readAllBytes(new File(dir, "espectral-client.json").toPath()), StandardCharsets.UTF_8);
        return new JsonParser().parse(text).getAsJsonObject();
    }

    @Test
    public void unknownKeysSurviveASave() throws Exception {
        write("{\"schema\":2,\"suite\":{\"enabled\":true,\"x\":1},\"launcher_only\":{\"a\":[1,2]},"
                + "\"personalization\":{\"theme\":\"glaciar\"},"
                + "\"features\":{\"cosmetics\":{\"enabled\":true,\"slot_cuello\":false},\"zoom\":{\"enabled\":false,\"fov\":30}},"
                + "\"macros\":[{\"id\":\"gg\",\"name\":\"GG\",\"keybind\":\"key.keyboard.g\",\"extra\":7,"
                + "\"actions\":[{\"type\":\"chat\",\"text\":\"gg\"}]}]}");
        ClientConfig config = ClientConfig.getInstance();
        config.load();
        config.setFeatureEnabled("zoom", true);
        JsonObject saved = read();
        assertEquals(2, saved.get("launcher_only").getAsJsonObject().get("a").getAsJsonArray().size());
        assertEquals(1, saved.getAsJsonObject("suite").get("x").getAsInt());
        assertEquals("glaciar", saved.getAsJsonObject("personalization").get("theme").getAsString());
        assertFalse(saved.getAsJsonObject("features").getAsJsonObject("cosmetics").get("slot_cuello").getAsBoolean());
        JsonObject zoom = saved.getAsJsonObject("features").getAsJsonObject("zoom");
        assertTrue(zoom.get("enabled").getAsBoolean());
        assertEquals(30, zoom.get("fov").getAsInt());
        assertEquals(7, saved.getAsJsonArray("macros").get(0).getAsJsonObject().get("extra").getAsInt());
        assertEquals("glaciar", config.theme());
    }

    @Test
    public void suiteMasterGatesWithoutTouchingStoredFlags() throws Exception {
        write("{\"schema\":2,\"suite\":{\"enabled\":true},\"features\":{\"nofog\":{\"enabled\":true}}}");
        ClientConfig config = ClientConfig.getInstance();
        config.load();
        assertTrue(config.isFeatureEnabled("nofog"));
        config.setSuiteEnabled(false);
        assertFalse(config.isFeatureEnabled("nofog"));
        assertTrue(config.isFeatureEnabledRaw("nofog"));
        config.setSuiteEnabled(true);
        assertTrue(config.isFeatureEnabled("nofog"));
    }

    @Test
    public void schemaOneMigratesWithTheMasterOn() throws Exception {
        write("{\"schema\":1,\"features\":{\"coords\":{\"enabled\":true}}}");
        ClientConfig config = ClientConfig.getInstance();
        config.load();
        assertTrue(config.isSuiteEnabled());
        assertTrue(config.isFeatureEnabled("coords"));
        config.save();
        JsonObject saved = read();
        assertEquals(2, saved.get("schema").getAsInt());
        assertTrue(saved.getAsJsonObject("suite").get("enabled").getAsBoolean());
    }

    @Test
    public void missingFileGetsRegistryDefaults() throws Exception {
        ClientConfig config = ClientConfig.getInstance();
        config.load();
        assertTrue("fullbright defaults on", config.isFeatureEnabledRaw("fullbright"));
        assertFalse("nofog defaults off", config.isFeatureEnabledRaw("nofog"));
        assertTrue(new File(dir, "espectral-client.json").isFile());
    }

    @Test
    public void macrosValidateLikeTheLauncher() throws Exception {
        ClientConfig config = ClientConfig.getInstance();
        config.load();
        assertFalse("empty text is refused", config.putMacro("a", "A", "key.keyboard.g", "chat", "   "));
        assertTrue(config.putMacro("hi", "", "key.keyboard.h", "chat", "/spawn now"));
        ClientConfig.MacroConfig macro = config.getMacros().get(0);
        assertEquals("command", macro.actions.get(0).type);
        assertEquals("spawn now", macro.name);
        assertEquals("hi-2", config.newMacroId("HI"));
        assertEquals("hola-a-todos", config.newMacroId("¡Hola a todos!"));
        assertEquals("macro", config.newMacroId("!!!"));
        assertTrue(config.removeMacro("hi"));
        assertTrue(config.getMacros().isEmpty());
        for (int i = 0; i < ClientConfig.MAX_MACROS; i++) {
            assertTrue(config.putMacro("m" + i, "M", "key.keyboard.f" + (i % 12 + 1), "chat", "x"));
        }
        assertFalse("a new id past the cap is refused", config.putMacro("over", "O", "key.keyboard.o", "chat", "x"));
    }

    @Test
    public void taglineIsSanitisedLikeTheLauncher() throws Exception {
        write("{\"schema\":2,\"personalization\":{\"tagline\":\"  \u00a7cHola\\u0007 mundo  \"}}");
        ClientConfig config = ClientConfig.getInstance();
        config.load();
        assertEquals("cHola mundo", config.tagline());
    }

    @Test
    public void anAppliedDefaultStaysAppliedAcrossRestarts() throws Exception {
        write("{\"schema\":2,\"applied_defaults\":[\"other\"]}");
        ClientConfig config = ClientConfig.getInstance();
        config.load();
        assertTrue(config.isDefaultApplied("other"));
        assertFalse(config.isDefaultApplied("vbo"));
        config.markDefaultApplied("vbo");
        config.markDefaultApplied("vbo");
        config.load();
        assertTrue("the mark survives a restart", config.isDefaultApplied("vbo"));
        assertEquals(2, read().getAsJsonArray("applied_defaults").size());
    }
}
