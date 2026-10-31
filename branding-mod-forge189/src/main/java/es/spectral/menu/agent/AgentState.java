package es.spectral.menu.agent;

import com.google.gson.JsonObject;

/**
 * Immutable state snapshot posted to {@code /agent/state}. Field-for-field the
 * same shape as the modern mod's {@code es.spectral.menu.agent.AgentState}:
 * {@code phase, screen, server, world, context, paused, recording} with nulls
 * omitted (Java 8, so a plain final class instead of a record).
 */
public final class AgentState {

    public static final String LOADING = "loading";
    public static final String TITLE = "title";
    public static final String JOINING = "joining";
    public static final String IN_WORLD = "in_world";
    public static final String DISCONNECTED = "disconnected";

    private final String phase;
    private final String screen;
    private final String server;
    private final String world;
    private final String context;
    private final boolean paused;
    private final boolean recording;

    public AgentState(String phase, String screen, String server, String world,
            String context, boolean paused, boolean recording) {
        this.phase = phase;
        this.screen = screen;
        this.server = server;
        this.world = world;
        this.context = context;
        this.paused = paused;
        this.recording = recording;
    }

    public JsonObject toJson() {
        JsonObject json = new JsonObject();
        json.addProperty("phase", phase);
        if (screen != null) json.addProperty("screen", screen);
        if (server != null) json.addProperty("server", server);
        if (world != null) json.addProperty("world", world);
        json.addProperty("context", context != null ? context : "menu");
        json.addProperty("paused", paused);
        json.addProperty("recording", recording);
        return json;
    }

    public boolean sameAs(AgentState other) {
        if (other == null) return false;
        return eq(phase, other.phase) && eq(screen, other.screen) && eq(server, other.server)
                && eq(world, other.world) && eq(context, other.context)
                && paused == other.paused && recording == other.recording;
    }

    private static boolean eq(String a, String b) {
        return a == null ? b == null : a.equals(b);
    }
}
