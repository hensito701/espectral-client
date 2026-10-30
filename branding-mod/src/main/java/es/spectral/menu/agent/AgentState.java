package es.spectral.menu.agent;

import com.google.gson.JsonObject;

/**
 * The state report the mod posts to {@code /api/bridge/agent/state} and the
 * engine re-emits as SSE {@code agent-state}. Immutable snapshot so the relay
 * thread and the render thread never share mutable state.
 *
 * <p>{@code phase} is one of {@code loading|title|joining|in_world|disconnected};
 * {@code context} one of {@link AgentPolicy.Context}.
 */
public record AgentState(String phase, String screen, String server, String world,
        AgentPolicy.Context context, boolean paused, boolean recording) {

    public static final String LOADING = "loading";
    public static final String TITLE = "title";
    public static final String JOINING = "joining";
    public static final String IN_WORLD = "in_world";
    public static final String DISCONNECTED = "disconnected";

    /** JSON body for the state route (nulls omitted). */
    public JsonObject toJson() {
        JsonObject json = new JsonObject();
        json.addProperty("phase", phase);
        if (screen != null) json.addProperty("screen", screen);
        if (server != null) json.addProperty("server", server);
        if (world != null) json.addProperty("world", world);
        json.addProperty("context", context != null ? context.wireName() : AgentPolicy.Context.MENU.wireName());
        json.addProperty("paused", paused);
        json.addProperty("recording", recording);
        return json;
    }

    /** Change detection for the "on change + every 10 s" report cadence. */
    public boolean sameAs(AgentState other) {
        if (other == null) return false;
        return eq(phase, other.phase) && eq(screen, other.screen) && eq(server, other.server)
                && eq(world, other.world) && context == other.context
                && paused == other.paused && recording == other.recording;
    }

    private static boolean eq(String a, String b) {
        return a == null ? b == null : a.equals(b);
    }
}
