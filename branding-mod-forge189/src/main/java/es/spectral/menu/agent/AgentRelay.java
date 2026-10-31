package es.spectral.menu.agent;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.Charset;
import java.util.concurrent.CompletableFuture;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import net.minecraft.client.Minecraft;
import net.minecraftforge.fml.common.eventhandler.SubscribeEvent;
import net.minecraftforge.fml.common.gameevent.TickEvent;

/**
 * In-game agent relay: one daemon thread long-polls
 * {@code POST /agent/poll}, dispatches each command on the client thread and
 * posts the result back; a second thread reports state on change and at least
 * every 10 s.
 *
 * <p>Java 8 port of {@code es.spectral.menu.agent.AgentRelay}: no
 * {@code java.net.http}, no records — {@link HttpURLConnection} and manual JSON
 * building. Same wire contract, same headers, same error codes.
 */
public final class AgentRelay {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    private static final int CONNECT_TIMEOUT_MS = 5_000;
    /** Slightly above the engine's poll budget so a parked poll survives. */
    private static final int POLL_READ_TIMEOUT_MS = 35_000;
    private static final long BACKOFF_START_MS = 1_000L;
    private static final long BACKOFF_MAX_MS = 30_000L;
    private static final long STATE_INTERVAL_MS = 10_000L;
    private static final long STATE_MIN_GAP_MS = 250L;

    private static AgentRelay instance;
    private static volatile boolean stopping;

    private final String base;
    private final String token;

    private volatile AgentState lastState;
    private volatile long lastStateAt;

    private AgentRelay(String base, String token) {
        this.base = base;
        this.token = token;
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    public static synchronized void start() {
        if (instance != null) return;
        if (!AgentRuntime.enabled()) return;
        String base = AgentRuntime.bridgeUrl();
        String token = AgentRuntime.bridgeToken();
        if (base == null || token == null) {
            LOGGER.warn("agent: relay disabled (missing -Despectral.bridge.url/token)");
            return;
        }
        AgentRelay relay = new AgentRelay(base, token);
        instance = relay;
        stopping = false;
        startThread(relay::pollLoop, "espectral-agent-relay");
        startThread(relay::stateLoop, "espectral-agent-state");
        LOGGER.info("agent: relay started ({})", base);
    }

    private static void startThread(Runnable body, String name) {
        Thread thread = new Thread(body, name);
        thread.setDaemon(true);
        thread.start();
    }

    /** Samples the client state once per tick (500 ms is plenty). */
    public static final class TickHook {

        private int ticks;

        @SubscribeEvent
        public void onClientTick(TickEvent.ClientTickEvent event) {
            if (event.phase != TickEvent.Phase.END) return;
            if (instance == null) return;
            if ((ticks++ % 10) != 0) return;
            AgentStatePublisher.sample(Minecraft.getMinecraft());
        }
    }

    // ------------------------------------------------------------------
    // Poll loop
    // ------------------------------------------------------------------

    private void pollLoop() {
        long backoff = BACKOFF_START_MS;
        while (!stopping) {
            try {
                if (pollOnce()) {
                    backoff = BACKOFF_START_MS;
                } else {
                    sleep(backoff);
                    backoff = Math.min(BACKOFF_MAX_MS, backoff * 2);
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return;
            } catch (Throwable t) {
                LOGGER.debug("agent: poll failed: {}", t.toString());
                try {
                    sleep(backoff);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    return;
                }
                backoff = Math.min(BACKOFF_MAX_MS, backoff * 2);
            }
        }
    }

    private void stateLoop() {
        while (!stopping) {
            try {
                reportStateIfDue();
            } catch (Throwable t) {
                LOGGER.debug("agent: state report failed: {}", t.toString());
            }
            try {
                Thread.sleep(500L);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return;
            }
        }
    }

    private static void sleep(long ms) throws InterruptedException {
        Thread.sleep(ms);
    }

    /** One poll round-trip; false when the engine was unreachable. */
    private boolean pollOnce() throws IOException {
        JsonObject reply = post("/agent/poll", new JsonObject(), POLL_READ_TIMEOUT_MS);
        if (reply == null) return false;
        JsonElement commands = reply.get("commands");
        if (commands != null && commands.isJsonArray()) {
            for (JsonElement element : commands.getAsJsonArray()) {
                if (element.isJsonObject()) handleCommand(element.getAsJsonObject());
            }
        }
        return true;
    }

    private void handleCommand(JsonObject command) {
        String id = string(command, "id");
        String tool = string(command, "tool");
        if (id == null || tool == null) {
            LOGGER.debug("agent: dropping malformed command {}", command);
            return;
        }
        JsonObject args = new JsonObject();
        JsonElement rawArgs = command.get("args");
        if (rawArgs != null && rawArgs.isJsonObject()) args = rawArgs.getAsJsonObject();
        long timeoutMs = 30_000L;
        JsonElement rawTimeout = command.get("timeout_ms");
        if (rawTimeout != null && rawTimeout.isJsonPrimitive()
                && rawTimeout.getAsJsonPrimitive().isNumber()) {
            timeoutMs = rawTimeout.getAsLong();
        }
        CompletableFuture<JsonObject> future = AgentTools.dispatch(
                Minecraft.getMinecraft(), tool, args, timeoutMs);
        try {
            JsonObject data = AgentTools.await(future, timeoutMs);
            postResult(id, true, data, null, null);
        } catch (AgentException e) {
            postResult(id, false, null, e.code(), e.getMessage());
        } catch (Throwable t) {
            postResult(id, false, null, "FAILED", String.valueOf(t));
        }
    }

    private void postResult(String id, boolean ok, JsonObject data,
            String errorCode, String errorMessage) {
        JsonObject body = new JsonObject();
        body.addProperty("id", id);
        body.addProperty("ok", ok);
        if (ok && data != null) body.add("data", data);
        if (!ok) {
            JsonObject error = new JsonObject();
            error.addProperty("code", errorCode != null ? errorCode : "FAILED");
            error.addProperty("message", errorMessage != null ? errorMessage : "the tool failed");
            body.add("error", error);
        }
        try {
            post("/agent/result", body, 5_000);
        } catch (Exception e) {
            // engine closed the socket (timed out, restarted): caller already got its error
            LOGGER.debug("agent: could not post result {}: {}", id, e.toString());
        }
    }

    // ------------------------------------------------------------------
    // State reporting
    // ------------------------------------------------------------------

    void reportStateIfDue() {
        AgentState state = AgentStatePublisher.current();
        if (state == null) return;
        long now = System.currentTimeMillis();
        boolean changed = !state.sameAs(lastState);
        if (!changed && now - lastStateAt < STATE_INTERVAL_MS) return;
        if (changed && now - lastStateAt < STATE_MIN_GAP_MS) return;
        postState(state);
    }

    private void postState(AgentState state) {
        lastState = state;
        lastStateAt = System.currentTimeMillis();
        try {
            post("/agent/state", state.toJson(), 5_000);
        } catch (Exception e) {
            LOGGER.debug("agent: could not post state: {}", e.toString());
        }
    }

    // ------------------------------------------------------------------
    // HTTP (Java 8: HttpURLConnection)
    // ------------------------------------------------------------------

    /** POSTs JSON and returns the parsed reply, or null on a non-200 answer. */
    private JsonObject post(String path, JsonObject body, int readTimeoutMs) throws IOException {
        HttpURLConnection connection = null;
        try {
            URL url = new URL(base + path);
            connection = (HttpURLConnection) url.openConnection();
            connection.setConnectTimeout(CONNECT_TIMEOUT_MS);
            connection.setReadTimeout(readTimeoutMs);
            connection.setRequestMethod("POST");
            connection.setDoOutput(true);
            connection.setRequestProperty("Content-Type", "application/json; charset=utf-8");
            connection.setRequestProperty("Accept", "application/json");
            // Contract headers, byte-for-byte the modern mod's.
            connection.setRequestProperty("x-espectral-client", "1");
            connection.setRequestProperty("x-espectral-bridge", token);
            byte[] payload = body.toString().getBytes(Charset.forName("UTF-8"));
            connection.setFixedLengthStreamingMode(payload.length);
            OutputStream out = connection.getOutputStream();
            try {
                out.write(payload);
            } finally {
                out.close();
            }
            int status = connection.getResponseCode();
            if (status != 200) {
                LOGGER.debug("agent: {} answered {}", path, status);
                drain(connection.getErrorStream());
                return null;
            }
            String text = read(connection.getInputStream());
            if (text == null || text.isEmpty()) return null;
            JsonElement parsed = new JsonParser().parse(text);
            return parsed != null && parsed.isJsonObject() ? parsed.getAsJsonObject() : null;
        } finally {
            if (connection != null) connection.disconnect();
        }
    }

    private static String read(InputStream in) throws IOException {
        if (in == null) return null;
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buffer = new byte[4096];
        try {
            int read;
            while ((read = in.read(buffer)) != -1) {
                out.write(buffer, 0, read);
            }
        } finally {
            in.close();
        }
        return new String(out.toByteArray(), Charset.forName("UTF-8"));
    }

    private static void drain(InputStream in) {
        try {
            read(in);
        } catch (IOException ignored) {
            // error body is only useful for logs
        }
    }

    private static String string(JsonObject json, String name) {
        JsonElement el = json.get(name);
        return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isString()
                ? el.getAsString() : null;
    }
}
