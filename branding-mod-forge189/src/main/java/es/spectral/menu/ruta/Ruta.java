package es.spectral.menu.ruta;

import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

import es.spectral.menu.ClientConfig;

/**
 * Ruta Espectral, client state + routing decisions (Forge 1.8.9).
 *
 * <p>Ruta is the MVP-only, opt-in-per-server Minecraft relay
 * ({@code ruta.uhce.top}); the wire format lives in {@link RutaProtocol}, the
 * pipeline handler in {@link RutaHandshakeHandler}. This class owns the API the
 * server-list toggle calls ({@link #isAvailable()}, {@link #isEligible},
 * {@link #isRouted(String)}, {@link #setRouted(String, boolean)}) and the
 * per-thread routing decision that the {@code NetworkManager} hook consumes.
 *
 * <p>Decision hand-off: both connect paths resolve the typed host first —
 * {@code OldServerPinger.ping} for the status ping, {@code GuiConnecting}'s
 * "Server Connector" thread for a join — and both then call
 * {@code NetworkManager.createNetworkManagerAndConnect}. The typed host is
 * remembered in a (5 s) {@link ThreadLocal} and combined with the port the
 * connect call carries, so the decision is made from the config key the player
 * actually typed and no file or grant work happens on the render thread.
 *
 * <p>Never falls back to a direct connection: when the player turned Ruta on
 * for a server and the attempt fails, the connection fails with a localized
 * reason (the point of the route is to keep the player's IP hidden).
 */
public final class Ruta {

    /** Feature id owning {@code features.ruta.servers}. */
    public static final String FEATURE_ID = "ruta";
    /** Key inside that feature object: array of normalized server keys. */
    public static final String SERVERS_KEY = "servers";

    /** Routing states. */
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

    private static final Decision NONE = new Decision(State.NONE, null, null);

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

    /** Typed host the current thread is about to connect to (see the mixins). */
    private static final ThreadLocal<String> TYPED_HOST = new ThreadLocal<String>();
    /**
     * Last join target noted on the render thread. A join runs on a brand-new
     * "Server Connector" thread, which cannot inherit a {@link ThreadLocal}, so
     * the typed host is handed over through this short-lived slot instead.
     */
    private static volatile String joinHost;
    private static volatile long joinHostAt;
    /** How long a handed-over join target stays believable. */
    private static final long JOIN_TTL_MS = 5000L;
    /** Decision for the next connect on this thread. */
    private static final ThreadLocal<Pending> PENDING = new ThreadLocal<Pending>();

    /** normalized server key -> wall clock of the last relay failure. */
    private static final Map<String, Long> FAILURES = new ConcurrentHashMap<String, Long>();
    private static final Map<String, String> FAILURE_CODES = new ConcurrentHashMap<String, String>();

    private static volatile int relayPort;
    private static volatile long relayPortCheckedAt;

    private Ruta() {}

    // ------------------------------------------------------------------
    // Public API (toggle icon)
    // ------------------------------------------------------------------

    /**
     * True when the player's own grant carries {@code mvp} and belongs to the
     * signed-in account. Cheap enough for a per-frame call: it reads the
     * cached grant only (the file is refreshed on connect/ping threads).
     */
    public static boolean isAvailable() {
        RutaGrant grant = RutaGrants.cached();
        if (grant == null) return false;
        String account = RutaGrants.accountUuid32();
        if (account == null) return false;
        return grant.allowsRuta(account, System.currentTimeMillis());
    }

    /** Normalized config key for an address, or {@code null} when unparsable. */
    public static String key(String serverAddress) {
        return RutaProtocol.normalize(serverAddress);
    }

    /** Key for a host + port pair (the resolved-connect form). */
    public static String keyOf(String host, int port) {
        return RutaProtocol.keyOf(host, port);
    }

    /** Stored per-server toggle (always false for an unroutable address). */
    public static boolean isRouted(String serverAddress) {
        String key = RutaProtocol.normalize(serverAddress);
        if (key == null || !RutaProtocol.isEligible(key)) return false;
        return isRoutedKey(key);
    }

    /** Whether the normalized key is switched on in the config. */
    public static boolean isRoutedKey(String key) {
        if (key == null) return false;
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
     * False for Espectral's own servers, LAN/loopback names and
     * loopback/private/link-local/CGNAT/benchmark IP literals: routing those
     * would break AuthMe per-IP limits and AdvancedBan IP bans, or cannot work
     * at all.
     */
    public static boolean isEligible(String serverAddress) {
        return RutaProtocol.isEligible(serverAddress);
    }

    /** Eligible AND switched on AND the player holds a usable MVP grant. */
    public static boolean isRoutedThroughRelay(String serverAddress) {
        return decide(serverAddress).state == State.ROUTE;
    }

    // ------------------------------------------------------------------
    // Decision
    // ------------------------------------------------------------------

    /** Routing decision for one server address (pure read of cached state). */
    public static Decision decide(String serverAddress) {
        String key = RutaProtocol.normalize(serverAddress);
        if (key == null || !RutaProtocol.isEligible(key)) return NONE;
        if (!isRoutedKey(key)) return NONE;
        if (!isAvailable()) return new Decision(State.BLOCKED, key, blockCode());
        return new Decision(State.ROUTE, key, null);
    }

    /** Why routing is unavailable although the toggle is on. */
    private static String blockCode() {
        RutaGrant grant = RutaGrants.cached();
        if (grant == null) return "nogrant";
        if (!grant.hasMvp()) return "notmvp_local";
        return "expired";
    }

    /** Records the typed host the calling thread is about to resolve/connect. */
    public static void noteTypedHost(String host) {
        if (host == null || host.trim().isEmpty()) {
            TYPED_HOST.remove();
        } else {
            TYPED_HOST.set(host.trim());
        }
    }

    /** Forgets the typed host (thread reuse safety). */
    public static void clearTypedHost() {
        TYPED_HOST.remove();
    }

    /**
     * Notes the host a join is about to connect to. Called on the caller thread
     * from {@code GuiConnecting.connect}; consumed once by the connector thread.
     */
    public static void noteJoinTarget(String host) {
        joinHost = host == null || host.trim().isEmpty() ? null : host.trim();
        joinHostAt = System.currentTimeMillis();
    }

    /** Takes (and clears) the handed-over join target when it is still fresh. */
    private static String takeJoinTarget() {
        String host = joinHost;
        if (host == null) return null;
        if (System.currentTimeMillis() - joinHostAt > JOIN_TTL_MS) {
            joinHost = null;
            return null;
        }
        joinHost = null;
        return host;
    }

    /**
     * Decision for the connection this thread is opening to
     * {@code resolved} on {@code port}: notes it, then returns it. {@code null}
     * means "connect directly" (the caller then leaves vanilla alone).
     *
     * <p>Consumes the typed host, so a thread reused by the connector pool never
     * inherits a stale key.
     */
    public static Decision decisionFor(InetAddress resolved, int port) {
        String typed = TYPED_HOST.get();
        TYPED_HOST.remove();
        if (typed == null || typed.trim().isEmpty()) {
            typed = takeJoinTarget();
        }
        String host = typed;
        if (host == null && resolved != null) {
            host = resolved.getHostAddress();
        }
        String key = RutaProtocol.keyOf(host, port);
        if (key == null) return null;
        Decision decision = decide(key);
        if (decision.state == State.NONE) return null;
        if (decision.state == State.ROUTE) {
            RutaGrants.refresh();
            // A route needs the token itself; refresh() may have just removed it.
            if (RutaGrants.cachedJws() == null) {
                return new Decision(State.BLOCKED, key, "nogrant");
            }
            if (!isAvailable()) return new Decision(State.BLOCKED, key, blockCode());
        }
        PENDING.set(new Pending(decision, System.currentTimeMillis()));
        return decision;
    }

    /** Forgets any noted decision (thread reuse safety). */
    public static void clearNote() {
        PENDING.remove();
    }

    /** Decision for the connection being opened on this thread, or {@code null}. */
    public static Decision takePending() {
        Pending pending = PENDING.get();
        if (pending == null) return null;
        PENDING.remove();
        if (System.currentTimeMillis() - pending.at > PENDING_TTL_MS) return null;
        return pending.decision;
    }

    /** True when the thread currently holds a ROUTE decision. */
    public static boolean hasPendingRoute() {
        Pending pending = PENDING.get();
        return pending != null && pending.decision.state == State.ROUTE;
    }

    /**
     * Resolves the relay port to dial: {@code 25565}, else {@code 443}, else
     * {@code 0} when neither accepts a TCP connection (3 s each). The verdict is
     * cached for a minute so a burst of pings does one probe. Blocking: called
     * on the connector/ping thread, never on the render thread.
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
        } catch (Exception unreachable) {
            return 0;
        } finally {
            try {
                socket.close();
            } catch (IOException ignored) {
                // nothing to do
            }
        }
    }

    /** {@link RutaTarget} for a routed connection to the resolved destination. */
    public static RutaTarget target(Decision decision, InetAddress dest, int port) {
        String ip = dest != null
                ? dest.getHostAddress().toLowerCase(Locale.ROOT)
                : "";
        return new RutaTarget(decision.key, ip, port, RutaGrants.cachedJws());
    }

    /** Resolves the relay host on the calling (non-render) thread. */
    public static InetAddress relayAddress() throws IOException {
        return InetAddress.getByName(RutaProtocol.RELAY_HOST);
    }

    // ------------------------------------------------------------------
    // Failure bookkeeping (status ping text + icon state)
    // ------------------------------------------------------------------

    /** Records that {@code key} failed through the relay with {@code code}. */
    public static void recordFailure(String key, String code) {
        if (key == null) return;
        FAILURES.put(key, System.currentTimeMillis());
        FAILURE_CODES.put(key, code == null ? "proto" : code);
    }

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
