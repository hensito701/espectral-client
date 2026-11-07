package es.spectral.menu.geoip;

import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.util.Locale;
import java.util.concurrent.ConcurrentHashMap;

import net.minecraft.client.multiplayer.resolver.ServerAddress;

/**
 * Host → resolved address, captured where vanilla performs the DNS/SRV
 * resolution for its status ping.
 *
 * <p>{@code ServerStatusPinger.pingServer} runs on the server-list thread pool
 * and resolves the configured address through {@code ServerNameResolver}
 * (SRV-aware). The {@code ServerNameResolverMixin} records the resulting
 * address here, so the render thread can map a row to a country with a plain
 * map lookup — it never performs DNS itself.
 *
 * <p>Keys are {@code "<host>:<port>"} lowercased, exactly as
 * {@link ServerAddress#parseString(String)} normalizes the {@code ServerData.ip}
 * string, so the recorded key and the row's lookup key always agree.
 */
public final class ServerAddresses {

    private static final ConcurrentHashMap<String, InetAddress> BY_KEY = new ConcurrentHashMap<>();
    private static final int MAX_ENTRIES = 1024;

    private ServerAddresses() {}

    /** Normalized key for a host/port pair. */
    public static String key(String host, int port) {
        return host.toLowerCase(Locale.ROOT) + ':' + port;
    }

    /** Records a resolution performed off the render thread. */
    public static void record(String host, int port, InetSocketAddress resolved) {
        if (host == null || host.isEmpty() || resolved == null) {
            return;
        }
        InetAddress address = resolved.getAddress();
        if (address == null) {
            return;
        }
        if (BY_KEY.size() > MAX_ENTRIES) {
            BY_KEY.clear();
        }
        BY_KEY.put(key(host, port), address);
    }

    /**
     * The address vanilla last resolved for a server list entry
     * ({@code ServerData.ip}), or {@code null} while the ping has not resolved
     * it yet. Never performs a lookup.
     */
    public static InetAddress forServerIp(String serverIp) {
        if (serverIp == null || serverIp.isEmpty()) {
            return null;
        }
        ServerAddress address;
        try {
            address = ServerAddress.parseString(serverIp);
        } catch (RuntimeException invalid) {
            return null;
        }
        if (address == null || address.getHost().isEmpty()) {
            return null;
        }
        return BY_KEY.get(key(address.getHost(), address.getPort()));
    }
}
