package es.spectral.menu.agent;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import org.junit.Test;

/** Argument validation: every bad shape must surface as BAD_ARGS. */
public class AgentArgsTest {

    private static AgentArgs args(String json) {
        return new AgentArgs(new JsonParser().parse(json).getAsJsonObject());
    }

    @Test
    public void boolAcceptsBooleanAndStringifiedBoolean() throws Exception {
        assertTrue(args("{\"hud\":true}").bool("hud", false));
        assertTrue(args("{\"hud\":\"true\"}").bool("hud", false));
        assertFalse(args("{\"hud\":\"false\"}").bool("hud", true));
        assertTrue(args("{}").bool("hud", true));
    }

    @Test
    public void boolRejectsNumbers() {
        try {
            args("{\"hud\":1}").bool("hud", true);
            fail("expected BAD_ARGS");
        } catch (AgentException e) {
            assertEquals("BAD_ARGS", e.code());
        }
    }

    @Test
    public void optIntUsesFallbackAndRejectsStrings() throws Exception {
        assertEquals(0, args("{}").optInt("width", 0));
        assertEquals(1280, args("{\"width\":1280}").optInt("width", 0));
        try {
            args("{\"width\":\"1280\"}").optInt("width", 0);
            fail("expected BAD_ARGS");
        } catch (AgentException e) {
            assertEquals("BAD_ARGS", e.code());
        }
    }

    @Test
    public void requiredStringRejectsMissingAndEmpty() {
        try {
            args("{}").requiredString("text");
            fail("expected BAD_ARGS");
        } catch (AgentException e) {
            assertEquals("BAD_ARGS", e.code());
        }
        try {
            args("{\"text\":\"\"}").requiredString("text");
            fail("expected BAD_ARGS");
        } catch (AgentException e) {
            assertEquals("BAD_ARGS", e.code());
        }
    }

    @Test
    public void containsDistinguishesNullFromAbsent() {
        assertTrue(args("{\"width\":null}").contains("width"));
        assertFalse(args("{}").contains("width"));
        assertFalse(args("{\"width\":null}").has("width"));
    }

    @Test
    public void nullArgsObjectBecomesEmpty() throws Exception {
        AgentArgs empty = new AgentArgs((JsonObject) null);
        assertFalse(empty.contains("x"));
        assertEquals("fallback", empty.string("x", "fallback"));
    }
}
