package es.spectral.menu.agent;

import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.SocketAddress;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.network.Connection;

/**
 * Where the agent may act, and why.
 *
 * <p>The engine sends {@code {allowed_servers, blocked_hosts}} on every poll;
 * this class keeps the latest copy and classifies the current connection as
 * {@link Context#MENU} (no world), {@link Context#SINGLEPLAYER},
 * {@link Context#REPLAY}, {@link Context#LOCALHOST},
 * {@link Context#ALLOWLISTED}, {@link Context#ESPECTRAL} or
 * {@link Context#SERVER}.
 *
 * <p>Espectral's own hosts are blocked twice over: by domain suffix (the
 * engine's {@code blocked_hosts} list, which the allowlist can never
 * override) **and** by IP — a game connected to {@code 45.…} by raw address is
 * still an Espectral server. DNS answers are not known to the engine, so this
 * class resolves the blocked domains on a daemon thread and caches the
 * addresses; the render thread only ever reads the published snapshot, never
 * blocks on the network.
 */
public final class AgentPolicy {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    /** Fallback when no policy arrived yet (the engine sends this list). */
    private static final List<String> DEFAULT_BLOCKED_HOSTS =
            Collections.unmodifiableList(Arrays.asList("uhce.top", "espectral.es"));

    /** How often the resolver refreshes the blocked-host addresses. */
    private static final long RESOLVE_INTERVAL_MS = 5 * 60_000L;

    /** Connection context the tier table is evaluated against. */
    public enum Context {
        MENU,
        SINGLEPLAYER,
        REPLAY,
        LOCALHOST,
        ALLOWLISTED,
        ESPECTRAL,
        SERVER;

        public String wireName() {
            return name().toLowerCase(Locale.ROOT);
        }
    }

    private static volatile List<String> allowedServers = List.of();
    private static volatile List<String> blockedHosts = DEFAULT_BLOCKED_HOSTS;
    /** Literal addresses the blocked hosts resolved to (lowercased, no port). */
    private static volatile Set<String> blockedIps = Set.of();
    private static volatile boolean resolverStarted;
    /**
     * Last server this client was actually connected to. The client clears
     * {@code getCurrentServer()} the moment a kick lands, but the disconnect
     * screen is exactly where an agent must still be refused act tools on a
     * public server — so the address is latched while it is known.
     */
    private static volatile String lastServer;

    private AgentPolicy() {}

    // ------------------------------------------------------------------
    // Policy from the engine
    // ------------------------------------------------------------------

    /** Applies the {@code policy} block of a poll reply (null-safe). */
    public static void apply(JsonObject policy) {
        if (policy == null) return;
        List<String> allowed = stringList(policy.get("allowed_servers"));
        List<String> blocked = stringList(policy.get("blocked_hosts"));
        if (allowed != null) allowedServers = allowed;
        if (blocked != null && !blocked.isEmpty()) blockedHosts = blocked;
        startResolver();
    }

    private static List<String> stringList(JsonElement el) {
        if (el == null || !el.isJsonArray()) return null;
        JsonArray array = el.getAsJsonArray();
        List<String> out = new ArrayList<>(array.size());
        for (JsonElement item : array) {
            if (item.isJsonPrimitive() && item.getAsJsonPrimitive().isString()) {
                String v = item.getAsString().trim().toLowerCase(Locale.ROOT);
                if (!v.isEmpty()) out.add(v);
            }
        }
        return Collections.unmodifiableList(out);
    }

    public static List<String> allowedServers() {
        return allowedServers;
    }

    public static List<String> blockedHosts() {
        return blockedHosts;
    }

    public static Set<String> blockedIps() {
        return blockedIps;
    }

    /** Most recent observed remote server address (null before any join). */
    public static String lastServer() {
        return lastServer;
    }

    /** Latches the address the client is talking to (called on classify). */
    static void noteServer(String address) {
        if (address != null && !address.isEmpty()) lastServer = address;
    }

    /**
     * Latches the server named on the command line ({@code --quickPlayMultiplayer}):
     * a launch straight into a public server is disconnected before the first
     * state sample lands, and the client clears {@code getCurrentServer()} on
     * the kick. Without this an agent launched at a server would read "menu" on
     * the disconnect screen and could then act against it.
     */
    static void noteServerFromArguments() {
        String[] fromProcess = null;
        try {
            fromProcess = ProcessHandle.current().info().arguments().orElse(null);
        } catch (Throwable ignored) {
            // not available on this JVM
        }
        if (latchFrom(fromProcess)) return;
        // HotSpot exposes the full command line as sun.java.command even when
        // ProcessHandle.Info.arguments() is empty; split on spaces (host names
        // never contain them).
        String commandLine = System.getProperty("sun.java.command");
        if (commandLine != null) latchFrom(commandLine.split(" "));
    }

    private static boolean latchFrom(String[] args) {
        if (args == null) return false;
        for (int i = 0; i < args.length - 1; i++) {
            if ("--quickPlayMultiplayer".equals(args[i]) && !args[i + 1].isBlank()) {
                noteServer(args[i + 1]);
                LOGGER.info("agent: launch target latched ({})", args[i + 1]);
                return true;
            }
        }
        return false;
    }

    /** Drops the latch (a deliberate singleplayer/local join supersedes it). */
    static void clearLastServer() {
        lastServer = null;
    }

    // ------------------------------------------------------------------
    // Blocked-host address resolution (off-thread, cached)
    // ------------------------------------------------------------------

    /**
     * Starts the single resolver daemon. Kept lazy (called on the first poll
     * reply and by {@link AgentRelay#start()}) so agent mode off means no
     * thread at all.
     */
    public static synchronized void startResolver() {
        if (resolverStarted) return;
        resolverStarted = true;
        Thread thread = new Thread(AgentPolicy::resolveLoop, "espectral-agent-dns");
        thread.setDaemon(true);
        thread.start();
    }

    private static void resolveLoop() {
        while (true) {
            refreshBlockedIps();
            logSelfCheck();
            try {
                Thread.sleep(RESOLVE_INTERVAL_MS);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return;
            }
        }
    }

    private static void refreshBlockedIps() {
        Set<String> next = new LinkedHashSet<>();
        for (String host : blockedHosts) {
            String bare = bareHost(host);
            if (bare == null) continue;
            try {
                for (InetAddress address : InetAddress.getAllByName(bare)) {
                    next.add(address.getHostAddress().toLowerCase(Locale.ROOT));
                }
            } catch (Exception e) {
                LOGGER.debug("agent: could not resolve blocked host {}: {}", bare, e.toString());
            }
        }
        blockedIps = Collections.unmodifiableSet(next);
    }

    // ------------------------------------------------------------------
    // Classification
    // ------------------------------------------------------------------

    /** Classifies the current game state (render thread; never blocks). */
    public static Context classify(Minecraft minecraft) {
        if (minecraft == null) return Context.MENU;
        if (replayActive()) return Context.REPLAY;
        if (minecraft.hasSingleplayerServer() || minecraft.getSingleplayerServer() != null) {
            return Context.SINGLEPLAYER;
        }
        ServerData server = safeCurrentServer(minecraft);
        String remoteIp = remoteIp(minecraft);
        String address = server != null && server.ip != null && !server.ip.isEmpty()
                ? server.ip : remoteAddress(minecraft);
        if (address == null && remoteIp == null) return Context.MENU;
        noteServer(address != null ? address : remoteIp);
        return classify(address, remoteIp);
    }

    /**
     * Pure classifier for one endpoint (also used by the startup self-check
     * log line, so the decision rules are provable without a live server).
     *
     * @param address {@code host}, {@code host:port}, {@code [v6]:port} or a
     *                literal IP, as the client knows it; may be null
     * @param remoteIp literal address of the live socket, may be null
     */
    public static Context classify(String address, String remoteIp) {
        String host = bareHost(address);
        String portPart = portOf(address);
        String ip = normalizeIp(remoteIp);
        if (ip == null) ip = normalizeIp(host);

        // Espectral first: the allowlist can never override it.
        if (host != null && isBlockedDomain(host)) return Context.ESPECTRAL;
        Set<String> ips = blockedIps;
        if (ip != null && ips.contains(ip)) return Context.ESPECTRAL;
        if (host != null && ips.contains(normalizeIp(host))) return Context.ESPECTRAL;

        if (isLoopback(host) || isLoopback(ip)) return Context.LOCALHOST;
        if (host != null && isAllowlisted(host, portPart)) return Context.ALLOWLISTED;
        return Context.SERVER;
    }

    /** {@code main.uhce.top} → true; {@code notuhce.top} → false. */
    public static boolean isBlockedDomain(String host) {
        if (host == null) return false;
        String bare = bareHost(host);
        if (bare == null) return false;
        for (String blocked : blockedHosts) {
            String domain = bareHost(blocked);
            if (domain == null || domain.isEmpty()) continue;
            if (bare.equals(domain) || bare.endsWith("." + domain)) return true;
        }
        return false;
    }

    public static boolean isAllowlisted(String host, String port) {
        if (host == null) return false;
        String lowered = host.toLowerCase(Locale.ROOT);
        for (String entry : allowedServers()) {
            String entryHost = bareHost(entry);
            String entryPort = portOf(entry);
            if (entryHost == null) continue;
            if (!entryHost.equals(lowered)) continue;
            if (entryPort == null || entryPort.isEmpty() || entryPort.equals(port)) return true;
        }
        return false;
    }

    /**
     * Strips scheme, path, port and IPv6 brackets, lowercased.
     * {@code [::1]:25565} → {@code ::1}.
     */
    public static String bareHost(String address) {
        if (address == null) return null;
        String v = address.trim().toLowerCase(Locale.ROOT);
        if (v.isEmpty()) return null;
        int scheme = v.indexOf("://");
        if (scheme >= 0) v = v.substring(scheme + 3);
        int slash = v.indexOf('/');
        if (slash >= 0) v = v.substring(0, slash);
        if (v.startsWith("[")) {
            int close = v.indexOf(']');
            if (close > 0) return v.substring(1, close);
            return v.substring(1);
        }
        int colon = v.lastIndexOf(':');
        if (colon > 0 && v.indexOf(':') == colon) return v.substring(0, colon);
        return v;
    }

    /** Port of {@code host:port} / {@code [v6]:port}, or null. */
    public static String portOf(String address) {
        if (address == null) return null;
        String v = address.trim();
        int slash = v.indexOf('/');
        if (slash >= 0) v = v.substring(0, slash);
        if (v.startsWith("[")) {
            int close = v.indexOf(']');
            if (close > 0 && close + 1 < v.length() && v.charAt(close + 1) == ':') {
                return v.substring(close + 2);
            }
            return null;
        }
        int colon = v.lastIndexOf(':');
        if (colon > 0 && v.indexOf(':') == colon) return v.substring(colon + 1);
        return null;
    }

    private static String normalizeIp(String raw) {
        if (raw == null) return null;
        String v = raw.trim().toLowerCase(Locale.ROOT);
        if (v.isEmpty()) return null;
        // IPv4-mapped IPv6 (::ffff:1.2.3.4) — compare on the IPv4 form.
        if (v.startsWith("::ffff:")) v = v.substring(7);
        return v;
    }

    private static boolean isLoopback(String host) {
        if (host == null) return false;
        String h = normalizeIp(host);
        if (h == null) return false;
        if (h.equals("localhost") || h.equals("127.0.0.1") || h.equals("::1") || h.equals("0:0:0:0:0:0:0:1")) {
            return true;
        }
        // Any 127.0.0.0/8 address is loopback.
        return h.startsWith("127.") && h.indexOf(':') < 0;
    }

    private static ServerData safeCurrentServer(Minecraft minecraft) {
        try {
            return minecraft.getCurrentServer();
        } catch (Exception e) {
            return null;
        }
    }

    /** Literal address of the live socket (null when not connected). */
    private static String remoteIp(Minecraft minecraft) {
        String address = remoteAddress(minecraft);
        String host = bareHost(address);
        return host != null && normalizeIp(host) != null && host.indexOf(' ') < 0 ? host : null;
    }

    private static String remoteAddress(Minecraft minecraft) {
        try {
            ClientPacketListener listener = minecraft.getConnection();
            if (listener == null) return null;
            Connection connection = listener.getConnection();
            if (connection == null) return null;
            SocketAddress remote = connection.getRemoteAddress();
            if (remote instanceof InetSocketAddress inet) {
                InetAddress addr = inet.getAddress();
                String host = addr != null ? addr.getHostAddress() : inet.getHostString();
                return host + ":" + inet.getPort();
            }
            return remote != null ? remote.toString() : null;
        } catch (Exception e) {
            return null;
        }
    }

    // ------------------------------------------------------------------
    // Replay Mod (soft dependency, reflection)
    // ------------------------------------------------------------------

    private static volatile Boolean replayModPresent;

    /**
     * True while Replay Mod's replay viewer is open. Reflective on purpose:
     * Replay Mod is GPL-3 and never bundled, so the class may be absent.
     */
    public static boolean replayActive() {
        if (replayModPresent == null) {
            try {
                Class.forName("com.replaymod.replay.ReplayModReplay", false,
                        AgentPolicy.class.getClassLoader());
                replayModPresent = Boolean.TRUE;
            } catch (Throwable t) {
                replayModPresent = Boolean.FALSE;
            }
        }
        if (!replayModPresent) return false;
        try {
            Class<?> type = Class.forName("com.replaymod.replay.ReplayModReplay");
            Object instance = type.getField("instance").get(null);
            if (instance == null) return false;
            Object handler = instance.getClass().getField("replayHandler").get(instance);
            return handler != null;
        } catch (Throwable t) {
            return false;
        }
    }

    // ------------------------------------------------------------------
    // Self-check (startup proof of the classifier's decisions)
    // ------------------------------------------------------------------

    /**
     * Logs the classifier's verdict for the blocked hosts, their resolved IPs,
     * localhost and the first allowlisted host. The launcher's agent launch
     * log therefore carries evidence that a raw-IP connection to an Espectral
     * server is refused, with no need for a live server to connect to.
     */
    public static void logSelfCheck() {
        if (!AgentRuntime.enabled()) return;
        StringBuilder line = new StringBuilder("agent policy: blocked=");
        line.append(blockedHosts).append(" ips=").append(blockedIps);
        line.append(" allowed=").append(allowedServers);
        for (String host : blockedHosts) {
            String bare = bareHost(host);
            if (bare != null) {
                line.append(" | ").append(bare).append(" -> ").append(classify(bare, null).wireName());
            }
        }
        for (String ip : blockedIps) {
            line.append(" | ip ").append(ip).append(" -> ").append(classify(ip, ip).wireName());
        }
        line.append(" | localhost -> ").append(classify("localhost", "127.0.0.1").wireName());
        line.append(" | 127.0.0.1:25565 -> ").append(classify("127.0.0.1:25565", null).wireName());
        String allow = allowedServers.isEmpty() ? null : allowedServers.get(0);
        if (allow != null) {
            line.append(" | allowlisted ").append(allow).append(" -> ")
                    .append(classify(allow, null).wireName());
        }
        line.append(" | example.org -> ").append(classify("example.org", null).wireName());
        LOGGER.info(line.toString());
    }
}
