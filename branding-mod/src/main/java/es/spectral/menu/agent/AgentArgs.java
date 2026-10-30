package es.spectral.menu.agent;

import java.util.LinkedHashMap;
import java.util.Map;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonPrimitive;

/**
 * Small typed accessors over the {@code args} object of a command. Bad input
 * becomes a {@code BAD_ARGS} {@link AgentException} here, so every tool starts
 * from validated values and the relay reports the contract's error codes.
 */
public final class AgentArgs {

    private final JsonObject json;

    public AgentArgs(JsonObject json) {
        this.json = json != null ? json : new JsonObject();
    }

    public boolean has(String name) {
        JsonElement el = json.get(name);
        return el != null && !el.isJsonNull();
    }

    public String string(String name, String fallback) throws AgentException {
        JsonElement el = json.get(name);
        if (el == null || el.isJsonNull()) return fallback;
        if (!el.isJsonPrimitive() || !el.getAsJsonPrimitive().isString()) {
            throw new AgentException("BAD_ARGS", "'" + name + "' must be a string");
        }
        return el.getAsString();
    }

    public String requiredString(String name) throws AgentException {
        String value = string(name, null);
        if (value == null || value.isEmpty()) {
            throw new AgentException("BAD_ARGS", "'" + name + "' is required");
        }
        return value;
    }

    public boolean bool(String name, boolean fallback) throws AgentException {
        JsonElement el = json.get(name);
        if (el == null || el.isJsonNull()) return fallback;
        if (!el.isJsonPrimitive()) {
            throw new AgentException("BAD_ARGS", "'" + name + "' must be a boolean");
        }
        JsonPrimitive primitive = el.getAsJsonPrimitive();
        if (primitive.isBoolean()) return primitive.getAsBoolean();
        if (primitive.isString()) {
            // Accept "true"/"false": MCP clients sometimes stringify booleans.
            String v = primitive.getAsString().trim();
            if (v.equalsIgnoreCase("true")) return true;
            if (v.equalsIgnoreCase("false")) return false;
        }
        throw new AgentException("BAD_ARGS", "'" + name + "' must be a boolean");
    }

    public int optInt(String name, int fallback) throws AgentException {
        JsonElement el = json.get(name);
        if (el == null || el.isJsonNull()) return fallback;
        if (!el.isJsonPrimitive() || !el.getAsJsonPrimitive().isNumber()) {
            throw new AgentException("BAD_ARGS", "'" + name + "' must be a number");
        }
        return el.getAsInt();
    }

    public int requiredInt(String name) throws AgentException {
        if (!has(name)) throw new AgentException("BAD_ARGS", "'" + name + "' is required");
        return optInt(name, 0);
    }

    /** True when the key is present at all (even as JSON null). */
    public boolean contains(String name) {
        return json.has(name);
    }

    /** Remaining unread keys (debug aid for the unknown-argument log line). */
    public Map<String, JsonElement> asMap() {
        Map<String, JsonElement> out = new LinkedHashMap<>();
        for (Map.Entry<String, JsonElement> entry : json.entrySet()) {
            out.put(entry.getKey(), entry.getValue());
        }
        return out;
    }
}
