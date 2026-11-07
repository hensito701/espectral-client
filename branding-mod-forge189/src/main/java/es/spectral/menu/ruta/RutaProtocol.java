package es.spectral.menu.ruta;

import java.util.Locale;

/**
 * Pure (Minecraft-free, Java-8-safe) half of Ruta Espectral: address
 * normalization, eligibility, the {@code ERT1} handshake line and the relay's
 * one-line reply. Everything here is deterministic and unit-tested from
 * {@code tools/RutaProtocolTest.java}; the 1.8.9 port copies this class
 * verbatim.
 *
 * <p>Wire format (see {@code services/ruta-relay/PROTOCOL.md}):
 * {@code ERT1 <grant> <dest-ip> <dest-port>\n} immediately after the TCP
 * connect, then exactly one reply line: {@code OK\n} or {@code ERR <code>\n}.
 */
public final class RutaProtocol {

    /** Relay host (the only place the client names it). */
    public static final String RELAY_HOST = "ruta.uhce.top";
    /** Primary relay port (Minecraft's port). */
    public static final int RELAY_PORT = 25565;
    /** Fallback port for networks that block 25565. */
    public static final int RELAY_FALLBACK_PORT = 443;
    /** Minecraft's default server port; omitted from config keys. */
    public static final int DEFAULT_PORT = 25565;
    /** TCP connect timeout per relay port. */
    public static final int CONNECT_TIMEOUT_MS = 3000;
    /** Client-side bound on the OK/ERR reply (relay header timeout is 10 s). */
    public static final int HANDSHAKE_TIMEOUT_MS = 12000;
    /** Longest accepted reply line, {@code ERR <code>} included. */
    public static final int MAX_REPLY_BYTES = 64;
    /** Protocol cap for the whole handshake line. */
    public static final int MAX_LINE_BYTES = 4096;

    private static final String[] CODES = {
        "proto", "auth", "expired", "notmvp", "dest", "limit", "connect", "busy"
    };

    private RutaProtocol() {}

    /** One parsed relay reply: {@code ok} or an error code. */
    public static final class Reply {
        public final boolean ok;
        public final String code;

        private Reply(boolean ok, String code) {
            this.ok = ok;
            this.code = code;
        }

        static Reply ok() {
            return new Reply(true, "ok");
        }

        static Reply err(String code) {
            return new Reply(false, code);
        }
    }

    private static final Reply OK = Reply.ok();

    /**
     * Normalizes a server address exactly the way the config stores it:
     * lowercase host, port only when it is not {@link #DEFAULT_PORT}.
     * Accepts {@code host}, {@code host:port}, {@code [v6]}, {@code [v6]:port}
     * and bare IPv6 literals (no port). Returns {@code null} for anything that
     * cannot name a server.
     */
    public static String normalize(String raw) {
        if (raw == null) return null;
        String text = raw.trim();
        if (text.isEmpty()) return null;
        String host;
        String portText = null;
        if (text.charAt(0) == '[') {
            int close = text.indexOf(']');
            if (close < 0) return null;
            host = text.substring(1, close);
            String rest = text.substring(close + 1);
            if (!rest.isEmpty()) {
                if (rest.charAt(0) != ':') return null;
                portText = rest.substring(1);
            }
        } else {
            int first = text.indexOf(':');
            int last = text.lastIndexOf(':');
            if (first >= 0 && first == last) {
                host = text.substring(0, first);
                portText = text.substring(first + 1);
            } else {
                // No colon (hostname / IPv4) or several (bare IPv6, no port).
                host = text;
            }
        }
        host = trimDots(host.toLowerCase(Locale.ROOT));
        if (host.isEmpty()) return null;
        int port = DEFAULT_PORT;
        if (portText != null && !portText.isEmpty()) {
            port = parsePort(portText);
            if (port <= 0) return null;
        }
        // An IPv6 literal carries colons itself, so a port can only be attached
        // in the bracketed form ([::1]:25566). Keeping it that way makes the key
        // unambiguous, round-trips through hostOf/portOf, and matches the Fabric
        // lane byte for byte — both mods share this config file, and a toggle
        // written by one must be visible to the other.
        boolean v6 = host.indexOf(':') >= 0;
        if (port == DEFAULT_PORT) return v6 ? "[" + host + "]" : host;
        return v6 ? "[" + host + "]:" + port : host + ":" + port;
    }

    /**
     * Config key for a host + port pair, i.e. the {@code ServerAddress} form
     * the resolver hands out: lowercase host, port only when not 25565, IPv6
     * literals bracketed.
     */
    public static String keyOf(String host, int port) {
        if (host == null) return null;
        if (port < 1 || port > 65535) return null;
        String h = trimDots(host.trim().toLowerCase(Locale.ROOT));
        if (h.length() > 1 && h.charAt(0) == '[' && h.charAt(h.length() - 1) == ']') {
            h = h.substring(1, h.length() - 1);
        }
        if (h.isEmpty()) return null;
        boolean v6 = h.indexOf(':') >= 0;
        if (port == DEFAULT_PORT) return v6 ? "[" + h + "]" : h;
        return v6 ? "[" + h + "]:" + port : h + ":" + port;
    }

    /** Host part of a normalized key ({@code [::1]:25566} → {@code ::1}). */
    public static String hostOf(String normalized) {
        if (normalized == null) return null;
        if (normalized.charAt(0) == '[') {
            int close = normalized.indexOf(']');
            return close > 0 ? normalized.substring(1, close) : normalized;
        }
        int colon = normalized.lastIndexOf(':');
        if (colon >= 0 && normalized.indexOf(':') == colon) {
            return normalized.substring(0, colon);
        }
        return normalized;
    }

    /** Port of a normalized key, {@link #DEFAULT_PORT} when omitted. */
    public static int portOf(String normalized) {
        if (normalized == null) return -1;
        int colon;
        if (normalized.charAt(0) == '[') {
            int close = normalized.indexOf(']');
            if (close < 0) return DEFAULT_PORT;
            colon = normalized.indexOf(':', close);
        } else {
            colon = normalized.lastIndexOf(':');
            if (colon >= 0 && normalized.indexOf(':') != colon) return DEFAULT_PORT;
        }
        if (colon < 0) return DEFAULT_PORT;
        int port = parsePort(normalized.substring(colon + 1));
        return port > 0 ? port : DEFAULT_PORT;
    }

    private static String trimDots(String host) {
        int end = host.length();
        while (end > 0 && host.charAt(end - 1) == '.') {
            end--;
        }
        return host.substring(0, end);
    }

    private static int parsePort(String text) {
        if (text.length() == 0 || text.length() > 5) return -1;
        int value = 0;
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c < '0' || c > '9') return -1;
            value = value * 10 + (c - '0');
        }
        return value >= 1 && value <= 65535 ? value : -1;
    }

    /**
     * False for LAN/loopback names and for private/loopback/link-local/CGNAT/
     * benchmark IP literals. Espectral's own servers are eligible like any
     * public host: the relay sends the player's real IP to them in a PROXY
     * protocol v2 header, so AuthMe per-IP limits and AdvancedBan IP bans keep
     * working.
     */
    public static boolean isEligible(String serverAddress) {
        String key = normalize(serverAddress);
        if (key == null) return false;
        String host = hostOf(key);
        if (host == null || host.isEmpty()) return false;
        if (host.equals("localhost") || host.endsWith(".localhost")
                || host.equals("local") || host.endsWith(".local")
                || host.equals("lan") || host.endsWith(".lan")) {
            return false;
        }
        if (host.indexOf(':') >= 0) return eligibleV6(host);
        if (isIpv4(host)) return eligibleV4(host);
        return true;
    }

    private static boolean isIpv4(String host) {
        int dots = 0;
        for (int i = 0; i < host.length(); i++) {
            char c = host.charAt(i);
            if (c == '.') {
                dots++;
            } else if (c < '0' || c > '9') {
                return false;
            }
        }
        return dots == 3;
    }

    private static boolean eligibleV4(String host) {
        String[] parts = host.split("\\.", -1);
        if (parts.length != 4) return true;
        int[] octets = new int[4];
        for (int i = 0; i < 4; i++) {
            if (parts[i].isEmpty()) return true;
            octets[i] = Integer.parseInt(parts[i]);
        }
        int a = octets[0];
        int b = octets[1];
        if (a == 0 || a == 10 || a == 127) return false;
        if (a == 169 && b == 254) return false;
        if (a == 172 && b >= 16 && b <= 31) return false;
        if (a == 192 && b == 168) return false;
        if (a == 100 && b >= 64 && b <= 127) return false;
        if (a == 198 && (b == 18 || b == 19)) return false;
        if (a == 192 && b == 0 && (octets[2] == 0 || octets[2] == 2)) return false;
        if (a == 198 && b == 51 && octets[2] == 100) return false;
        if (a == 203 && b == 0 && octets[2] == 113) return false;
        if (a >= 224) return false;
        return true;
    }

    private static boolean eligibleV6(String host) {
        String h = host;
        int zone = h.indexOf('%');
        if (zone >= 0) h = h.substring(0, zone);
        if (h.equals("::") || h.equals("::1")) return false;
        if (h.startsWith("fe8") || h.startsWith("fe9") || h.startsWith("fea")
                || h.startsWith("feb")) {
            return false;
        }
        if (h.startsWith("fc") || h.startsWith("fd")) return false;
        if (h.startsWith("ff")) return false;
        if (h.startsWith("2001:db8")) return false;
        if (h.startsWith("::ffff:") || h.startsWith("::FFFF:")) {
            String tail = h.substring(7);
            return !isIpv4(tail) || eligibleV4(tail);
        }
        return true;
    }

    /**
     * Builds the handshake line, {@code \n} included. Validates every field so
     * a malformed grant or destination never reaches the relay as garbage
     * (the relay would answer {@code ERR proto}).
     */
    public static String handshakeLine(String grant, String destIp, int destPort) {
        if (grant == null || grant.isEmpty() || grant.length() > 3400) {
            throw new IllegalArgumentException("bad grant for ERT1");
        }
        for (int i = 0; i < grant.length(); i++) {
            char c = grant.charAt(i);
            if (c <= ' ' || c == '\n' || c == '\r') {
                throw new IllegalArgumentException("bad grant for ERT1");
            }
        }
        if (destIp == null || destIp.isEmpty() || destPort < 1 || destPort > 65535) {
            throw new IllegalArgumentException("bad destination for ERT1");
        }
        for (int i = 0; i < destIp.length(); i++) {
            char c = destIp.charAt(i);
            if (c <= ' ' || c == '[' || c == ']' || c == '\n' || c == '\r') {
                throw new IllegalArgumentException("bad destination ip for ERT1");
            }
        }
        String line = "ERT1 " + grant + " " + destIp + " " + destPort + "\n";
        if (line.length() > MAX_LINE_BYTES) {
            throw new IllegalArgumentException("ERT1 line too long");
        }
        return line;
    }

    /**
     * Parses the relay's one-line reply ({@code OK} or {@code ERR <code>}).
     * Anything unrecognized is a protocol violation; the client maps it to the
     * {@code proto} message.
     */
    public static Reply parseReply(String line) {
        if (line == null) return Reply.err("proto");
        String text = line;
        int end = text.length();
        while (end > 0) {
            char c = text.charAt(end - 1);
            if (c == '\n' || c == '\r' || c == ' ' || c == '\t') {
                end--;
            } else {
                break;
            }
        }
        text = text.substring(0, end);
        if (text.equals("OK")) return OK;
        if (!text.startsWith("ERR ")) return Reply.err("proto");
        String code = text.substring(4).trim();
        return Reply.err(isKnownCode(code) ? code : "proto");
    }

    public static boolean isKnownCode(String code) {
        if (code == null) return false;
        for (String known : CODES) {
            if (known.equals(code)) return true;
        }
        return false;
    }
}
