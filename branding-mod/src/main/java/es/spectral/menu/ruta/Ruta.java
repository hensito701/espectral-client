package es.spectral.menu.ruta;

import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

import io.netty.util.AttributeKey;

import net.minecraft.client.Minecraft;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.cosmetics.CosmeticsService;
import es.spectral.menu.cosmetics.GrantVerifier;

/**
 * Ruta Espectral, client state + routing decisions.
 *
 * <p>Ruta is the MVP-only, opt-in-per-server Minecraft relay
 * ({@code ruta.uhce.top}); the wire format lives in {@link RutaProtocol}, the
 * pipeline handler in {@link RutaHandshakeHandler}. This class owns the public
 * API the multiplayer-list toggle icon calls ({@link #isAvailable()},
 * {@link #isEligible(String)}, {@link #isRouted(String)},
 * {@link #setRouted(String, boolean)}, {@link #isRoutedThroughRelay(String)})
 * and the per-thread routing decision that the {@code Connection} mixin
 * consumes.
 *
 * <p>Decision hand-off: both callers (the "Server Connector" thread of
 * {@code ConnectScreen} and the {@code ServerStatusPinger} pool thread) resolve
 * the typed {@code ServerAddress} immediately before {@code Connection.connect}.
 * {@link #noteAddress(String, int)} runs at the head of
 * {@code ServerNameResolver#resolveAddress} and stores the decision in a
 * {@link ThreadLocal} (5 s TTL, consumed once) so the connect path does no file
 * or grant work on the render thread.
 *
 * <p>Everything read from the render thread is in-memory: the toggle lives in
 * {@link ClientConfig} (volatile snapshot + immediate save) and the grant in
 * {@code CosmeticsService}.
 */
public final class Ruta {

    /** Channel attribute carrying the {@link RutaTarget} to a routed connection. */
    public static final AttributeKey<RutaTarget> TARGET = AttributeKey.valueOf("espectral_ruta_target");

    /** Routing states, as reported by {@link #decide(String)}. */
    public enum State {
        /** Not routed: connect directly, as vanilla does. */
        NONE,
        /** Routed through the relay. */
        ROUTE,
        /** The player asked for routing but may not use it: fail, never go direct. */
        BLOCKED
    }

    /** One routing decision. */
    public static final class Decision {
        public final State state;
        public final String key;
        /** Failure code when {@link #state} is {@link State#BLOCKED}. */
        public final String code;

        Decision(State state, String key, String code) {
            this.state = state;
            this.key = key;
            this.code = code;
        }
    }

    private static final Decision NONE =
            new Decision(State.NONE, null, null);

    private static final class Pending {
        final Decision decision;
        final long at;

        Pending(Decision decision, long at) {
            this.decision = decision;
            this.at = at;
        }
    }

    /** How long a noted decision stays valid (one connect follows within ms). */
    private static final long PENDING_TTL_MS = 5000L;
    /** A failed status ping keeps its Ruta text for this long. */
    private static final long FAILURE_TTL_MS = 20000L;
    /** Relay port verdict cache lifetime. */
    private static final long PORT_CACHE_MS = 60000L;

    private static final ThreadLocal<Pending> PENDING = new ThreadLocal<>();
    /** normalized server key -> wall clock of the last relay failure. */
    private static final Map<String, Long> FAILURES = new ConcurrentHashMap<>();
    private static volatile int relayPort;
    private static volatile long relayPortCheckedAt;

    private Ruta() {}

    // ------------------------------------------------------------------
    // Public API (toggle icon + callers)
    // ------------------------------------------------------------------

    /**
     * True when the player's own verified grant carries {@code mvp} and belongs
     * to the signed-in account. Cheap enough for a per-frame call: it reads the
     * worker thread's volatile snapshot.
     */
    public static boolean isAvailable() {
        CosmeticsService.ensureStarted();
        CosmeticsService.Own own = CosmeticsService.own();
        if (own == null || own.uuid() == null) return false;
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft != null && minecraft.getUser() != null
                && !own.uuid().equals(minecraft.getUser().getProfileId())) {
            return false;
        }
        GrantVerifier.Grant grant = own.grant();
        return grant != null && grant.hasFlag("mvp")
                && grant.expiresAt() * 1000L > System.currentTimeMillis();
    }

    /** Normalized config key for an address, or {@code null} when unparsable. */
    public static String key(String serverAddress) {
        return RutaProtocol.normalize(serverAddress);
    }

    /** Key for a host + port pair (the {@code ServerAddress} form). */
    public static String keyOf(String host, int port) {
        return RutaProtocol.keyOf(host, port);
    }

    /** Stored per-server toggle (always false for an unroutable address). */
    public static boolean isRouted(String serverAddress) {
        String key = RutaProtocol.normalize(serverAddress);
        if (key == null || !RutaProtocol.isEligible(key)) return false;
        return ClientConfig.getInstance().isRutaRouted(key);
    }

    /** Stores the per-server toggle and persists immediately. */
    public static void setRouted(String serverAddress, boolean routed) {
        String key = RutaProtocol.normalize(serverAddress);
        if (key == null) return;
        if (routed && !RutaProtocol.isEligible(key)) return;
        ClientConfig.getInstance().setRutaRouted(key, routed);
    }

    /**
     * False for LAN/loopback names and for loopback/private/link-local/CGNAT/
     * benchmark IP literals: the relay refuses those destinations anyway.
     * Espectral's own servers are eligible; the relay forwards the player's
     * real IP to them in a PROXY protocol v2 header.
     */
    public static boolean isEligible(String serverAddress) {
        return RutaProtocol.isEligible(serverAddress);
    }

    /**
     * The real routing condition: eligible AND switched on for that server AND
     * the player holds an MVP grant. This is what the toggle icon renders and
     * the connect path enforces.
     */
    public static boolean isRoutedThroughRelay(String serverAddress) {
        Decision decision = decide(serverAddress);
        return decision.state == State.ROUTE;
    }

    // ------------------------------------------------------------------
    // Decision
    // ------------------------------------------------------------------

    /** Routing decision for one server address (pure read, no I/O). */
    public static Decision decide(String serverAddress) {
        String key = RutaProtocol.normalize(serverAddress);
        if (key == null || !RutaProtocol.isEligible(key)) return NONE;
        if (!ClientConfig.getInstance().isRutaRouted(key)) return NONE;
        CosmeticsService.ensureStarted();
        if (!isAvailable()) {
            return new Decision(State.BLOCKED, key, blockCode());
        }
        return new Decision(State.ROUTE, key, null);
    }

    /** Why routing is unavailable although the toggle is on. */
    private static String blockCode() {
        CosmeticsService.Own own = CosmeticsService.own();
        GrantVerifier.Grant grant = own == null ? null : own.grant();
        if (grant == null) return "nogrant";
        if (!grant.hasFlag("mvp")) return "notmvp_local";
        return "expired";
    }

    /**
     * Records the decision for the address the calling thread is about to
     * connect. Called from the head of
     * {@code ServerNameResolver#resolveAddress}; the value is consumed once by
     * the {@code Connection} mixin.
     */
    public static void noteAddress(String host, int port) {
        String key = keyOf(host, port);
        if (key == null) {
            PENDING.remove();
            return;
        }
        Decision decision = decide(key);
        if (decision.state == State.NONE) {
            PENDING.remove();
            return;
        }
        PENDING.set(new Pending(decision, System.currentTimeMillis()));
    }

    /** Forgets any noted decision (thread reuse safety). */
    public static void clearNote() {
        PENDING.remove();
    }

    /**
     * Decision for the connection being opened on this thread, or {@code null}
     * when the next connection is a plain direct one.
     */
    public static Decision takePending() {
        Pending pending = PENDING.get();
        if (pending == null) return null;
        PENDING.remove();
        if (System.currentTimeMillis() - pending.at > PENDING_TTL_MS) return null;
        return pending.decision;
    }

    /**
     * Resolves the relay port to dial: {@code 25565}, else {@code 443}, else
     * {@code 0} when neither accepts a TCP connection (3 s each). The verdict
     * is cached for a minute so a burst of pings does one probe.
     */
    public static int relayPort() {
        long now = System.currentTimeMillis();
        int cached = relayPort;
        if (cached > 0 && now - relayPortCheckedAt < PORT_CACHE_MS) return cached;
        synchronized (Ruta.class) {
            now = System.currentTimeMillis();
            if (relayPort > 0 && now - relayPortCheckedAt < PORT_CACHE_MS) return relayPort;
            int port = probe(RutaProtocol.RELAY_PORT);
            if (port == 0) port = probe(RutaProtocol.RELAY_FALLBACK_PORT);
            relayPort = port;
            relayPortCheckedAt = now;
            return port;
        }
    }

    private static int probe(int port) {
        Socket socket = new Socket();
        try {
            socket.connect(new InetSocketAddress(InetAddress.getByName(RutaProtocol.RELAY_HOST), port),
                    RutaProtocol.CONNECT_TIMEOUT_MS);
            return port;
        } catch (Exception e) {
            return 0;
        } finally {
            try {
                socket.close();
            } catch (Exception ignored) {
                // nothing to do
            }
        }
    }

    /** {@link RutaTarget} for a routed connection to {@code dest}. */
    public static RutaTarget target(Decision decision, InetSocketAddress dest) {
        CosmeticsService.Own own = CosmeticsService.own();
        String grant = own != null ? own.grantJws() : null;
        String ip = dest.getAddress() != null
                ? dest.getAddress().getHostAddress().toLowerCase(Locale.ROOT)
                : dest.getHostString();
        return new RutaTarget(decision.key, ip, dest.getPort(), grant);
    }

    /** Resolves the relay host on the calling (non-render) thread. */
    public static InetAddress relayAddress() throws java.io.IOException {
        return InetAddress.getByName(RutaProtocol.RELAY_HOST);
    }

    // ------------------------------------------------------------------
    // Failure bookkeeping (status ping text)
    // ------------------------------------------------------------------

    /** Records that {@code key} failed through the relay with {@code code}. */
    public static void recordFailure(String key, String code) {
        if (key == null) return;
        FAILURES.put(key, System.currentTimeMillis());
        FAILURE_CODES.put(key, code == null ? "proto" : code);
    }

    private static final Map<String, String> FAILURE_CODES = new ConcurrentHashMap<>();

    /**
     * Failure code for {@code serverAddress} when a relay attempt failed
     * recently, else {@code null}. Drives the "failed via Ruta" status text.
     */
    public static String recentFailure(String serverAddress) {
        String key = RutaProtocol.normalize(serverAddress);
        if (key == null) return null;
        Long at = FAILURES.get(key);
        if (at == null) return null;
        if (System.currentTimeMillis() - at > FAILURE_TTL_MS) {
            FAILURES.remove(key);
            FAILURE_CODES.remove(key);
            return null;
        }
        String code = FAILURE_CODES.get(key);
        return code == null ? "proto" : code;
    }

    /** Forgets a recorded failure (a later successful ping clears it). */
    public static void clearFailure(String serverAddress) {
        String key = RutaProtocol.normalize(serverAddress);
        if (key == null) return;
        FAILURES.remove(key);
        FAILURE_CODES.remove(key);
    }
}
