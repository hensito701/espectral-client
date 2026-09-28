package es.spectral.menu.cosmetics;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.Collection;
import java.util.Collections;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;

/**
 * Background owner of the cosmetics state: the engine-written
 * {@code config/espectral-cosmetics.json} (hot-reloaded on mtime) and the
 * loopback bridge client that reports visible UUIDs and receives signed
 * presence. All file and HTTP work happens on one daemon thread; render/tick
 * threads only read volatile snapshots.
 *
 * <p>The bridge is active only when the launcher passed
 * {@code -Despectral.bridge.url} and {@code -Despectral.bridge.token}. The
 * client POSTs {@code {"uuids":[...]}} (32-hex lowercase) to
 * {@code <url>/visible} every 2 s while connected to a remote server, with
 * headers {@code x-espectral-client: 1} and {@code x-espectral-bridge: <token>}.
 * Each returned grant is verified locally (subject bound to the reported uuid);
 * {@code players} entries are presence regardless of grant validity, and
 * loadouts are filtered through flags + catalog + exclusivity before they ever
 * reach {@link Cosmetics}.
 */
public final class CosmeticsService {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    /** Bound on the presence cache (visible players only). */
    private static final int MAX_PEERS = 256;
    /** Bridge poll / state-file stat cadence. */
    private static final long TICK_MS = 2000L;
    private static final Duration HTTP_TIMEOUT = Duration.ofSeconds(2);

    /** Snapshot of the launcher-written state file. */
    public record Own(UUID uuid, GrantVerifier.Grant grant, boolean share,
            Loadout loadout, Set<String> hiddenPlayers, boolean emotes) {
        static final Own EMPTY = new Own(null, null, false, Loadout.EMPTY, Set.of(), false);
    }

    /** Verified presence for one visible peer. */
    private static final class Peer {
        final boolean present;
        final GrantVerifier.Grant grant;
        final Loadout loadout;
        Peer(boolean present, GrantVerifier.Grant grant, Loadout loadout) {
            this.present = present;
            this.grant = grant;
            this.loadout = loadout;
        }
    }

    private static volatile Own own = Own.EMPTY;

    /** UUID -> presence, bounded, access-ordered (evict oldest beyond 256). */
    private static final Map<String, Peer> PEERS =
            Collections.synchronizedMap(new LinkedHashMap<String, Peer>(64, 0.75f, true) {
                @Override
                protected boolean removeEldestEntry(Map.Entry<String, Peer> eldest) {
                    return size() > MAX_PEERS;
                }
            });

    /** Most recent visible-UUID snapshot offered by the game thread. */
    private static volatile Set<UUID> pending = Set.of();

    private static Thread worker;
    private static volatile boolean workerStarted;

    private CosmeticsService() {}

    /** Current own state (null-safe). */
    public static Own own() {
        return own;
    }

    /** Verified peer entry, or null when the uuid has no presence. */
    public static GrantVerifier.Grant peerGrant(UUID uuid) {
        if (uuid == null) return null;
        Peer p = PEERS.get(GrantVerifier.subject(uuid));
        return p != null ? p.grant : null;
    }

    public static Loadout peerLoadout(UUID uuid) {
        if (uuid == null) return null;
        Peer p = PEERS.get(GrantVerifier.subject(uuid));
        return p != null ? p.loadout : null;
    }

    /** True when the peer published presence (⚛) — valid grant or not. */
    public static boolean peerPresent(UUID uuid) {
        if (uuid == null) return false;
        Peer p = PEERS.get(GrantVerifier.subject(uuid));
        return p != null && p.present;
    }

    /**
     * Called from the client tick. Cheap: copies the visible-uuid set and arms
     * the daemon. No I/O on the game thread.
     */
    public static void onClientTick(Minecraft minecraft) {
        ensureWorker();
        boolean connected = minecraft != null && minecraft.getConnection() != null
                && minecraft.player != null && !minecraft.hasSingleplayerServer();
        if (!connected) {
            if (!pending.isEmpty()) pending = Set.of();
            if (!PEERS.isEmpty()) PEERS.clear();
            return;
        }
        try {
            Collection<UUID> ids = minecraft.getConnection().getOnlinePlayerIds();
            pending = ids == null ? Set.of() : Set.copyOf(ids);
        } catch (Exception e) {
            // leave the previous snapshot in place for this tick
        }
    }

    private static void ensureWorker() {
        if (workerStarted) return;
        synchronized (CosmeticsService.class) {
            if (workerStarted) return;
            workerStarted = true;
            worker = new Thread(CosmeticsService::loop, "espectral-cosmetics");
            worker.setDaemon(true);
            worker.start();
        }
    }

    private static void loop() {
        String bridgeUrl = System.getProperty("espectral.bridge.url");
        String bridgeToken = System.getProperty("espectral.bridge.token");
        HttpClient http = null;

        if (bridgeUrl != null && !bridgeUrl.isEmpty()
                && bridgeToken != null && !bridgeToken.isEmpty()) {
            try {
                http = HttpClient.newBuilder()
                        .connectTimeout(HTTP_TIMEOUT)
                        .build();
            } catch (Exception e) {
                LOGGER.warn("Bridge HTTP client unavailable: {}", e.getMessage());
                http = null;
            }
        }
        Path statePath = FabricLoader.getInstance().getConfigDir()
                .resolve("espectral-cosmetics.json");
        long knownMtime = -1L;
        while (true) {
            try {
                knownMtime = pollState(statePath, knownMtime);
                if (http != null) {
                    pollBridge(http, bridgeUrl, bridgeToken);
                }
            } catch (Throwable t) {
                LOGGER.debug("cosmetics service tick failed: {}", t.toString());
            }
            try {
                Thread.sleep(TICK_MS);
            } catch (InterruptedException ignored) {
                return;
            }
        }
    }

    /** Re-reads the state file when its mtime changed; returns the new mtime. */
    private static long pollState(Path path, long knownMtime) {
        try {
            if (!Files.exists(path)) {
                if (own != Own.EMPTY) own = Own.EMPTY;
                return -1L;
            }
            long mtime = Files.getLastModifiedTime(path).toMillis();
            if (mtime == knownMtime) return knownMtime;
            String content = Files.readString(path, StandardCharsets.UTF_8);
            Own parsed = parseOwn(content);
            own = parsed;
            return mtime;
        } catch (Exception e) {
            LOGGER.debug("cosmetics state read failed: {}", e.getMessage());
            return knownMtime;
        }
    }

    /**
     * Parses {@code espectral-cosmetics.json}
     * ({schema,uuid,grant,share,loadout,hidden_players,emotes}). A bad grant
     * becomes {@code null}; the loadout is filtered against the grant's flags.
     */
    private static Own parseOwn(String content) {
        try {
            JsonObject root = JsonParser.parseString(content).getAsJsonObject();
            UUID uuid = null;
            if (root.has("uuid") && root.get("uuid").isJsonPrimitive()) {
                uuid = uuidFromHex(root.get("uuid").getAsString());
            }
            GrantVerifier.Grant grant = null;
            if (uuid != null && root.has("grant") && root.get("grant").isJsonPrimitive()) {
                grant = GrantVerifier.verify(root.get("grant").getAsString(), uuid);
            }
            boolean share = root.has("share") && root.get("share").isJsonPrimitive()
                    && root.get("share").getAsBoolean();
            Map<String, String> bySlot = new LinkedHashMap<>(5);
            if (root.has("loadout") && root.get("loadout").isJsonObject()) {
                for (Map.Entry<String, JsonElement> e : root.getAsJsonObject("loadout").entrySet()) {
                    if (e.getValue().isJsonPrimitive()) {
                        bySlot.put(e.getKey(), e.getValue().getAsString());
                    }
                }
            }
            Set<String> flags = grant != null ? grant.flags() : Set.of();
            Loadout loadout = Catalog.get().filter(Catalog.get().fromMap(bySlot), flags);
            Set<String> hidden = new HashSet<>();
            if (root.has("hidden_players") && root.get("hidden_players").isJsonArray()) {
                for (JsonElement e : root.getAsJsonArray("hidden_players")) {
                    if (e.isJsonPrimitive()) hidden.add(e.getAsString().toLowerCase(Locale.ROOT));
                }
            }
            boolean emotes = !root.has("emotes")
                    || (root.get("emotes").isJsonPrimitive() && root.get("emotes").getAsBoolean());
            return new Own(uuid, grant, share, loadout, hidden, emotes);
        } catch (Exception e) {
            LOGGER.debug("cosmetics state parse failed: {}", e.getMessage());
            return Own.EMPTY;
        }
    }

    // ------------------------------------------------------------------
    // Bridge
    // ------------------------------------------------------------------

    private static void pollBridge(HttpClient http, String bridgeUrl, String bridgeToken) {
        Set<UUID> ids = pending;
        if (ids.isEmpty()) return;
        StringBuilder body = new StringBuilder(ids.size() * 40 + 16);
        body.append("{\"uuids\":[");
        boolean first = true;
        Set<String> subjects = new LinkedHashSet<>(ids.size());
        for (UUID id : ids) {
            if (id == null) continue;
            String s = GrantVerifier.subject(id);
            if (!subjects.add(s)) continue;
            if (!first) body.append(',');
            body.append('"').append(s).append('"');
            first = false;
        }
        body.append("]}");
        try {
            HttpRequest request = HttpRequest.newBuilder()
                    .uri(URI.create(bridgeUrl + "/visible"))
                    .timeout(HTTP_TIMEOUT)
                    .header("content-type", "application/json")
                    .header("x-espectral-client", "1")
                    .header("x-espectral-bridge", bridgeToken)
                    .POST(HttpRequest.BodyPublishers.ofString(body.toString(), StandardCharsets.UTF_8))
                    .build();
            HttpResponse<String> response =
                    http.send(request, HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
            if (response.statusCode() != 200) {
                LOGGER.debug("bridge /visible answered {}", response.statusCode());
                return;
            }
            applyBridgeResponse(subjects, response.body());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (Exception e) {
            LOGGER.debug("bridge poll failed: {}", e.getMessage());
        }
    }

    /**
     * Applies {@code {"players":[{uuid,grant,loadout}]}}. Requested subjects
     * absent from {@code players} are evicted (their presence expired).
     */
    private static void applyBridgeResponse(Set<String> requested, String body) {
        JsonObject root;
        try {
            root = JsonParser.parseString(body).getAsJsonObject();
        } catch (Exception e) {
            return;
        }
        Set<String> answered = new HashSet<>();
        JsonElement playersEl = root.get("players");
        if (playersEl != null && playersEl.isJsonArray()) {
            for (JsonElement el : playersEl.getAsJsonArray()) {
                if (!el.isJsonObject()) continue;
                JsonObject p = el.getAsJsonObject();
                UUID uuid = uuidFromHex(strOrNull(p, "uuid"));
                if (uuid == null) continue;
                String subject = GrantVerifier.subject(uuid);
                answered.add(subject);
                GrantVerifier.Grant grant = null;
                JsonElement grantEl = p.get("grant");
                if (grantEl != null && grantEl.isJsonPrimitive()) {
                    grant = GrantVerifier.verify(grantEl.getAsString(), subject,
                            GrantVerifier.pinnedKey(), System.currentTimeMillis() / 1000L);
                }
                Set<String> flags = grant != null ? grant.flags() : Set.of();
                Map<String, String> bySlot = new LinkedHashMap<>(5);
                if (p.has("loadout") && p.get("loadout").isJsonObject()) {
                    for (Map.Entry<String, JsonElement> e
                            : p.getAsJsonObject("loadout").entrySet()) {
                        if (e.getValue().isJsonPrimitive()) {
                            bySlot.put(e.getKey(), e.getValue().getAsString());
                        }
                    }
                }
                Loadout loadout = Catalog.get().filter(Catalog.get().fromMap(bySlot), flags);
                PEERS.put(subject, new Peer(true, grant, loadout));
            }
        }
        // Evict requested-but-absent subjects: their presence expired upstream.
        for (String subject : requested) {
            if (!answered.contains(subject)) {
                PEERS.remove(subject);
            }
        }
    }

    private static String strOrNull(JsonObject obj, String key) {
        JsonElement el = obj.get(key);
        return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isString()
                ? el.getAsString() : null;
    }

    /** Accepts dashed or 32-hex uuids; null on anything else. */
    private static UUID uuidFromHex(String s) {
        if (s == null) return null;
        try {
            if (s.length() == 36) return UUID.fromString(s);
            if (s.length() == 32) {
                return UUID.fromString(s.substring(0, 8) + "-" + s.substring(8, 12) + "-"
                        + s.substring(12, 16) + "-" + s.substring(16, 20) + "-" + s.substring(20));
            }
        } catch (IllegalArgumentException ignored) {
        }
        return null;
    }
}
