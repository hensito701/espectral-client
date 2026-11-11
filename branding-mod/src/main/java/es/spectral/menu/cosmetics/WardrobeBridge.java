package es.spectral.menu.cosmetics;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicInteger;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import es.spectral.menu.BridgeAuth;

/**
 * Client for the launcher's {@code POST /api/bridge/cosmetics} route (engine
 * {@code loopback.rs} + {@code cosmetics::bridge_view}): the wardrobe the
 * in-game Cosméticos screen edits.
 *
 * <p>{@code {}} reads the view of the account that owns this launch;
 * {@code {"patch": {...}}} applies a {@code PUT /api/cosmetics/:uuid} patch
 * (same validation and error codes) and answers the resulting view. Requests
 * are signed with {@link BridgeAuth} and an answer without a valid launcher
 * proof is ignored.
 *
 * <p>All I/O runs on one daemon thread; the game thread only reads the
 * volatile snapshot ({@link #wardrobe()}, {@link #serial()}) and never blocks.
 * A refusal keeps the last good snapshot and exposes its code, so the screen
 * can show a short message and stay consistent with the launcher's state.
 */
public final class WardrobeBridge {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");
    private static final Duration HTTP_TIMEOUT = Duration.ofSeconds(5);

    /** One catalog entry of a slot ({@code requires} is {@code mvp} today). */
    public record Item(String id, String requires, String es, String en) {}

    /** One slot with its selectable items, in catalog order. */
    public record Slot(String id, List<Item> items) {}

    /** The engine's wardrobe answer (immutable). */
    public record Wardrobe(List<Slot> slots, Map<String, String> loadout, boolean share,
            boolean emotes, boolean mvp, Set<String> flags) {}

    /** A queued exchange: {@code patch == null} is a read. */
    private record Op(JsonObject patch) {}

    /** Last good view, or null before the first successful answer. */
    private static volatile Wardrobe wardrobe;
    /** Bumped after every answer (view, refusal or transport failure). */
    private static volatile int serial;
    /** Language key of the last refusal / transport problem, null when the last exchange succeeded. */
    private static volatile String errorKey;
    /** Engine error code behind {@link #errorKey} (generic message argument). */
    private static volatile String errorCode;
    /** False until an exchange was proven to come from this launcher. */
    private static volatile boolean reachable;
    /** Exchanges queued or in flight (the screen disables the chips meanwhile). */
    private static final AtomicInteger INFLIGHT = new AtomicInteger();

    private static final LinkedBlockingQueue<Op> QUEUE = new LinkedBlockingQueue<>();
    private static volatile boolean started;

    private WardrobeBridge() {}

    /** True when the launcher passed both the bridge URL and the launch token. */
    public static boolean configured() {
        String url = System.getProperty("espectral.bridge.url");
        return url != null && !url.isEmpty() && BridgeAuth.tokenFromEnv() != null;
    }

    public static Wardrobe wardrobe() {
        return wardrobe;
    }

    public static int serial() {
        return serial;
    }

    /** True while a request is queued or in flight. */
    public static boolean busy() {
        return INFLIGHT.get() > 0;
    }

    /** True when the last exchange was answered by this launcher. */
    public static boolean reachable() {
        return reachable;
    }

    /** Language key of the last refusal, or null. */
    public static String errorKey() {
        return errorKey;
    }

    /** Engine error code behind {@link #errorKey()}, or null. */
    public static String errorCode() {
        return errorCode;
    }

    /** Reads the current wardrobe. Cheap; the answer arrives on the worker thread. */
    public static void requestLoad() {
        submit(new Op(null));
    }

    /** Applies {@code patch} (share / emotes / loadout) and re-reads the view. */
    public static void requestPatch(JsonObject patch) {
        if (patch == null || patch.size() == 0) return;
        submit(new Op(patch.deepCopy()));
    }

    private static void submit(Op op) {
        if (!configured()) {
            markUnreachable();
            return;
        }
        ensureStarted();
        INFLIGHT.incrementAndGet();
        if (!QUEUE.offer(op)) {
            INFLIGHT.decrementAndGet();
            markUnreachable();
        }
    }

    private static synchronized void ensureStarted() {
        if (started) return;
        started = true;
        Thread worker = new Thread(WardrobeBridge::loop, "espectral-wardrobe");
        worker.setDaemon(true);
        worker.start();
    }

    private static void loop() {
        HttpClient http;
        try {
            http = HttpClient.newBuilder().connectTimeout(HTTP_TIMEOUT).build();
        } catch (Exception e) {
            LOGGER.warn("Wardrobe bridge HTTP client unavailable: {}", e.getMessage());
            markUnreachable();
            return;
        }
        String base = System.getProperty("espectral.bridge.url");
        String token = BridgeAuth.tokenFromEnv();
        if (base == null || base.isEmpty() || token == null) {
            markUnreachable();
            return;
        }
        BridgeAuth auth = new BridgeAuth(token);
        while (true) {
            Op op;
            try {
                op = QUEUE.take();
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return;
            }
            try {
                exchange(http, base, auth, op);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                markUnreachable();
                return;
            } catch (Exception e) {
                LOGGER.debug("wardrobe bridge exchange failed: {}", e.toString());
                markUnreachable();
            } finally {
                INFLIGHT.decrementAndGet();
                serial++;
            }
        }
    }

    private static void exchange(HttpClient http, String base, BridgeAuth auth, Op op)
            throws Exception {
        JsonObject body = new JsonObject();
        if (op.patch() != null) body.add("patch", op.patch());
        byte[] bytes = body.toString().getBytes(StandardCharsets.UTF_8);
        URI uri = URI.create(base + "/cosmetics");
        String nonce = BridgeAuth.newNonce();
        HttpRequest request = HttpRequest.newBuilder()
                .uri(uri)
                .timeout(HTTP_TIMEOUT)
                .header("content-type", "application/json")
                .header("x-espectral-client", "1")
                .header(BridgeAuth.ID_HEADER, auth.id())
                .header(BridgeAuth.NONCE_HEADER, nonce)
                .header(BridgeAuth.MAC_HEADER,
                        auth.requestMac("POST", BridgeAuth.target(uri), nonce, bytes))
                .POST(HttpRequest.BodyPublishers.ofByteArray(bytes))
                .build();
        HttpResponse<byte[]> response = http.send(request, HttpResponse.BodyHandlers.ofByteArray());
        String proof = response.headers().firstValue(BridgeAuth.PROOF_HEADER).orElse(null);
        if (!auth.proofOk(nonce, response.body(), proof)) {
            // Something else answered on the port (or no launcher at all).
            markUnreachable();
            return;
        }
        String text = new String(response.body(), StandardCharsets.UTF_8);
        // A valid proof proves this launcher answered, refusals included.
        reachable = true;
        if (response.statusCode() == 200) {
            applyView(text);
            return;
        }
        String code = errorCodeOf(text);
        if ("BRIDGE_AUTH".equals(code) || "BRIDGE_OUTDATED".equals(code)) {
            // This game's mod and the launcher disagree: the same restart fixes both.
            markUnreachable();
            return;
        }
        errorCode = code;
        errorKey = errorKeyOf(code);
    }

    private static void applyView(String text) {
        try {
            JsonObject root = JsonParser.parseString(text).getAsJsonObject();
            List<Slot> slots = new ArrayList<>();
            JsonElement slotsEl = root.get("slots");
            if (slotsEl != null && slotsEl.isJsonArray()) {
                for (JsonElement el : slotsEl.getAsJsonArray()) {
                    if (!el.isJsonObject()) continue;
                    JsonObject s = el.getAsJsonObject();
                    String id = str(s, "slot");
                    if (id == null) continue;
                    List<Item> items = new ArrayList<>();
                    JsonElement itemsEl = s.get("items");
                    if (itemsEl != null && itemsEl.isJsonArray()) {
                        for (JsonElement ie : itemsEl.getAsJsonArray()) {
                            if (!ie.isJsonObject()) continue;
                            JsonObject io = ie.getAsJsonObject();
                            String itemId = str(io, "id");
                            if (itemId == null) continue;
                            Map<String, String> names = new LinkedHashMap<>();
                            JsonElement nameEl = io.get("name");
                            if (nameEl != null && nameEl.isJsonObject()) {
                                for (Map.Entry<String, JsonElement> n
                                        : nameEl.getAsJsonObject().entrySet()) {
                                    if (n.getValue().isJsonPrimitive()) {
                                        names.put(n.getKey(), n.getValue().getAsString());
                                    }
                                }
                            }
                            items.add(new Item(itemId, str(io, "requires"),
                                    names.get("es"), names.get("en")));
                        }
                    }
                    slots.add(new Slot(id, List.copyOf(items)));
                }
            }
            Map<String, String> loadout = new LinkedHashMap<>();
            JsonElement loadoutEl = root.get("loadout");
            if (loadoutEl != null && loadoutEl.isJsonObject()) {
                for (Map.Entry<String, JsonElement> e : loadoutEl.getAsJsonObject().entrySet()) {
                    JsonElement v = e.getValue();
                    if (v != null && v.isJsonPrimitive() && v.getAsJsonPrimitive().isString()
                            && !v.getAsString().isEmpty()) {
                        loadout.put(e.getKey(), v.getAsString());
                    }
                }
            }
            Set<String> flags = new LinkedHashSet<>();
            JsonElement flagsEl = root.get("flags");
            if (flagsEl != null && flagsEl.isJsonArray()) {
                for (JsonElement f : flagsEl.getAsJsonArray()) {
                    if (f.isJsonPrimitive()) flags.add(f.getAsString());
                }
            }
            boolean mvp = bool(root, "mvp");
            wardrobe = new Wardrobe(List.copyOf(slots), Collections.unmodifiableMap(loadout),
                    bool(root, "share"), bool(root, "emotes"), mvp, Set.copyOf(flags));
            errorKey = null;
            errorCode = null;
            reachable = true;
        } catch (Exception e) {
            LOGGER.debug("wardrobe view parse failed: {}", e.getMessage());
            markUnreachable();
        }
    }

    private static void markUnreachable() {
        reachable = false;
        errorCode = null;
        errorKey = null;
    }

    /** Language key for an engine error code (the screen shows it as one line). */
    public static String errorKeyOf(String code) {
        if (code == null) return "espectral.cosmetics.err.generic";
        return switch (code) {
            case "FLAG_REQUIRED" -> "espectral.cosmetics.err.mvp";
            case "EXCLUSIVE_CONFLICT" -> "espectral.cosmetics.err.exclusive";
            case "UNKNOWN_COSMETIC", "WRONG_SLOT", "BAD_LOADOUT", "BAD_PATCH" ->
                    "espectral.cosmetics.err.invalid";
            case "BRIDGE_NO_ACCOUNT" -> "espectral.cosmetics.err.noaccount";
            default -> "espectral.cosmetics.err.generic";
        };
    }

    private static String errorCodeOf(String text) {
        try {
            JsonObject root = JsonParser.parseString(text).getAsJsonObject();
            JsonElement err = root.get("error");
            if (err != null && err.isJsonObject()) {
                return str(err.getAsJsonObject(), "code");
            }
        } catch (Exception ignored) {
        }
        return null;
    }

    private static String str(JsonObject obj, String key) {
        JsonElement el = obj.get(key);
        return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isString()
                ? el.getAsString() : null;
    }

    private static boolean bool(JsonObject obj, String key) {
        JsonElement el = obj.get(key);
        return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isBoolean()
                && el.getAsBoolean();
    }
}
