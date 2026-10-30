package es.spectral.menu.agent;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;

/**
 * The in-game agent relay: one daemon thread that long-polls
 * {@code POST /api/bridge/agent/poll}, dispatches each command to the render
 * thread ({@code Minecraft.execute}) and posts the result back.
 *
 * <p>Design points the contract cares about:
 * <ul>
 *   <li>Inert unless {@code -Despectral.agent=1}: no thread, no resolver, no
 *       per-frame work.</li>
 *   <li>The render thread is never blocked on the network: the poller owns all
 *       I/O, and results are posted from the poller thread too.</li>
 *   <li>Relay errors never crash the game: log + capped exponential backoff,
 *       retrying forever (the launcher may restart its engine under us).</li>
 *   <li>State is reported on change and at least every 10 s (the engine's
 *       {@code wait_for} relies on it).</li>
 * </ul>
 */
public final class AgentRelay {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    private static final Duration CONNECT_TIMEOUT = Duration.ofSeconds(5);
    /** Slightly above the engine's 25 s poll budget so a parked poll survives. */
    private static final Duration POLL_TIMEOUT = Duration.ofSeconds(35);
    private static final long BACKOFF_START_MS = 1_000L;
    private static final long BACKOFF_MAX_MS = 30_000L;
    private static final long STATE_INTERVAL_MS = 10_000L;
    private static final long STATE_MIN_GAP_MS = 250L;

    private static volatile Thread thread;
    private static volatile boolean stopping;
    private static AgentRelay instance;

    private final String pollUrl;
    private final String resultUrl;
    private final String stateUrl;
    private final String token;
    private final HttpClient http;

    /** Last state posted (change detection) and when. */
    private AgentState lastState;
    private long lastStateAt;

    private AgentRelay(String base, String token) {
        this.pollUrl = base + "/agent/poll";
        this.resultUrl = base + "/agent/result";
        this.stateUrl = base + "/agent/state";
        this.token = token;
        this.http = HttpClient.newBuilder()
                .connectTimeout(CONNECT_TIMEOUT)
                .version(HttpClient.Version.HTTP_1_1)
                .build();
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    /**
     * Starts the relay once (idempotent). Returns immediately; all I/O happens
     * on the daemon thread. No-op without agent mode, a bridge URL or a token.
     */
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
        Thread poller = new Thread(relay::loop, "espectral-agent-relay");
        poller.setDaemon(true);
        Thread stateThread = new Thread(relay::stateLoop, "espectral-agent-state");
        stateThread.setDaemon(true);
        // A JVM shutdown hook releases held keys so a stop mid-hold leaves no
        // phantom key pressed in the OS.
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            stopping = true;
            poller.interrupt();
            stateThread.interrupt();
            AgentInput.releaseAll(Minecraft.getInstance());
        }, "espectral-agent-shutdown"));
        thread = poller;
        poller.start();
        stateThread.start();
        AgentPolicy.startResolver();
        LOGGER.info("agent: relay started ({})", base);
    }

    // ------------------------------------------------------------------
    // Poll loop
    // ------------------------------------------------------------------

    private void loop() {
        long backoff = BACKOFF_START_MS;
        while (!stopping) {
            try {
                boolean ok = pollOnce();
                if (ok) {
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

    /**
     * State reporter. A separate thread because the poll can legitimately park
     * for 25 s: a phase change must reach the engine in ~0.5 s regardless (the
     * engine's {@code wait_for} polls every 250 ms).
     */
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

    /** One poll round-trip; returns false when the engine was unreachable. */
    private boolean pollOnce() throws Exception {
        JsonObject body = new JsonObject();
        HttpRequest request = request(pollUrl)
                .timeout(POLL_TIMEOUT)
                .POST(HttpRequest.BodyPublishers.ofString(body.toString(), StandardCharsets.UTF_8))
                .build();
        HttpResponse<String> response =
                http.send(request, HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
        if (response.statusCode() != 200) {
            LOGGER.debug("agent: poll answered {}: {}", response.statusCode(), response.body());
            // 409 = this launch is not an agent launch; retrying forever would
            // just spam. The launcher relaunches with agent mode when needed.
            return response.statusCode() < 500;
        }
        JsonObject json = JsonParser.parseString(response.body()).getAsJsonObject();
        JsonElement policy = json.get("policy");
        if (policy != null && policy.isJsonObject()) {
            AgentPolicy.apply(policy.getAsJsonObject());
        }
        JsonElement commands = json.get("commands");
        if (commands != null && commands.isJsonArray()) {
            for (JsonElement element : commands.getAsJsonArray()) {
                if (element.isJsonObject()) {
                    handleCommand(element.getAsJsonObject());
                }
            }
        }
        return true;
    }

    /**
     * Dispatches one command. Everything that touches the game happens on the
     * render thread; only the JSON parsing and the result POST stay here.
     */
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
        Minecraft minecraft = Minecraft.getInstance();
        CompletableFuture<JsonObject> future;
        try {
            future = AgentTools.dispatch(minecraft, tool, args, timeoutMs);
        } catch (Throwable t) {
            postResult(id, false, null, "FAILED", String.valueOf(t.getMessage()));
            return;
        }
        // Await the render thread, but never longer than the command's budget:
        // a frozen client must not wedge the poll loop.
        try {
            JsonObject data = future.get(Math.max(1_000L, timeoutMs), TimeUnit.MILLISECONDS);
            postResult(id, true, data, null, null);
        } catch (java.util.concurrent.TimeoutException e) {
            postResult(id, false, null, "BUSY", "the client did not run '" + tool + "' in time");
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (java.util.concurrent.ExecutionException e) {
            Throwable cause = e.getCause();
            if (cause instanceof AgentException agent) {
                postResult(id, false, null, agent.code(), agent.getMessage());
            } else {
                postResult(id, false, null, "FAILED", String.valueOf(cause));
            }
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
            HttpRequest request = request(resultUrl)
                    .timeout(Duration.ofSeconds(5))
                    .POST(HttpRequest.BodyPublishers.ofString(body.toString(), StandardCharsets.UTF_8))
                    .build();
            HttpResponse<String> response =
                    http.send(request, HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
            if (response.statusCode() != 200) {
                LOGGER.debug("agent: result for {} answered {}", id, response.statusCode());
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (Exception e) {
            // The engine closed the socket (timed out, restarted): the caller
            // has already received its error.
            LOGGER.debug("agent: could not post result {}: {}", id, e.toString());
        }
    }

    // ------------------------------------------------------------------
    // State reporting
    // ------------------------------------------------------------------

    /** Posts the state when it changed or the 10 s heartbeat is due. */
    protected void reportStateIfDue() {
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
            JsonObject body = state.toJson();
            HttpRequest request = request(stateUrl)
                    .timeout(Duration.ofSeconds(5))
                    .POST(HttpRequest.BodyPublishers.ofString(body.toString(), StandardCharsets.UTF_8))
                    .build();
            HttpResponse<String> response =
                    http.send(request, HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
            if (response.statusCode() != 200) {
                LOGGER.debug("agent: state answered {}", response.statusCode());
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (Exception e) {
            LOGGER.debug("agent: could not post state: {}", e.toString());
        }
    }

    // ------------------------------------------------------------------
    // HTTP helpers
    // ------------------------------------------------------------------

    private HttpRequest.Builder request(String url) {
        return HttpRequest.newBuilder()
                .uri(URI.create(url))
                .header("content-type", "application/json")
                .header("x-espectral-client", "1")
                .header("x-espectral-bridge", token);
    }

    private static String string(JsonObject json, String name) {
        JsonElement el = json.get(name);
        return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isString()
                ? el.getAsString() : null;
    }
}
