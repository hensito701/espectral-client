package es.spectral.menu.agent;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import org.junit.Test;

/**
 * Wire-contract tests that need no client: the state JSON shape must stay
 * byte-compatible with the modern mod's {@code AgentState.toJson()}.
 */
public class AgentStateTest {

    @Test
    public void titleStateOmitsNullsAndKeepsBooleans() {
        AgentState state = new AgentState(AgentState.TITLE, "GuiMainMenu", null, null,
                "menu", false, false);
        JsonObject json = state.toJson();
        assertEquals("title", json.get("phase").getAsString());
        assertEquals("GuiMainMenu", json.get("screen").getAsString());
        assertEquals("menu", json.get("context").getAsString());
        assertFalse(json.has("server"));
        assertFalse(json.has("world"));
        assertFalse(json.get("paused").getAsBoolean());
        assertFalse(json.get("recording").getAsBoolean());
        assertEquals(5, json.entrySet().size());
    }

    @Test
    public void inWorldStateCarriesServerAndWorld() {
        AgentState state = new AgentState(AgentState.IN_WORLD, "GuiIngame", "play.example.net",
                "New World", "server", true, false);
        JsonObject json = state.toJson();
        assertEquals("in_world", json.get("phase").getAsString());
        assertEquals("play.example.net", json.get("server").getAsString());
        assertEquals("New World", json.get("world").getAsString());
        assertTrue(json.get("paused").getAsBoolean());
    }

    @Test
    public void changeDetectionIgnoresIdentity() {
        AgentState a = new AgentState(AgentState.TITLE, "GuiMainMenu", null, null, "menu", false, false);
        AgentState b = new AgentState(AgentState.TITLE, "GuiMainMenu", null, null, "menu", false, false);
        AgentState c = new AgentState(AgentState.TITLE, "GuiOptions", null, null, "menu", false, false);
        assertTrue(a.sameAs(b));
        assertFalse(a.sameAs(c));
        assertFalse(a.sameAs(null));
    }

    @Test
    public void roundTripsThroughJson() {
        AgentState state = new AgentState(AgentState.JOINING, "GuiConnecting", "127.0.0.1:25565",
                null, "server", false, false);
        JsonObject parsed = new JsonParser().parse(state.toJson().toString()).getAsJsonObject();
        assertEquals("joining", parsed.get("phase").getAsString());
        assertEquals("127.0.0.1:25565", parsed.get("server").getAsString());
    }
}
