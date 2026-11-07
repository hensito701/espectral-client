package es.spectral.menu.geoip;

import java.io.IOException;
import java.net.InetAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.MappedByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardOpenOption;
import java.util.concurrent.ConcurrentHashMap;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

/**
 * Country lookup for the multiplayer server list, backed by the launcher's
 * {@code ESPGEO1} table ({@code -Despectral.geoip=<path>}).
 *
 * <p>The launcher converts DB-IP's monthly "IP to Country Lite" CSV into this
 * fixed-width binary table and only advertises it once it validated; this class
 * maps that file read-only on a daemon thread the first time a row asks for a
 * country, and never touches the disk or DNS again. Absent property, missing
 * file or a table failing validation all mean one permanent "off" state: rows
 * then draw no flags and nothing retries the load.
 *
 * <p>Attribution is mandatory (data licence CC BY 4.0): "IP Geolocation by
 * DB-IP", shown in the flag tooltip. Anycast and DDoS-protected servers resolve
 * to their proxy's country — the table answers "where is this address
 * announced", not "where is the operator".
 *
 * <p>Addresses are recorded from the vanilla status ping, which is the one
 * place that already resolved the host (SRV records included), so the render
 * thread never performs DNS. Keys are {@code ServerData.serverIP} verbatim.
 */
public final class GeoIp {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    /** JVM property the launcher sets to the absolute table path. */
    public static final String PROPERTY = "espectral.geoip";

    /** {@code ESPGEO1\0}, 8 bytes. */
    public static final byte[] MAGIC = {'E', 'S', 'P', 'G', 'E', 'O', '1', 0};
    /** magic + v4 count + v6 count + country count + reserved. */
    public static final int HEADER_LEN = 24;
    /** start[16] + end[16] + country index (u16 LE). */
    public static final int ENTRY_LEN = 34;

    private static final GeoIp INSTANCE = new GeoIp();
    /** Stored value for "looked up, no country": a host is never resolved twice. */
    private static final String NONE = "";
    /** Bound on remembered servers; the list is small, this is only a safety net. */
    private static final int CACHE_MAX = 2048;

    private final ConcurrentHashMap<String, String> byServer = new ConcurrentHashMap<String, String>();
    /** Addresses resolved before the table finished loading. */
    private final ConcurrentHashMap<String, InetAddress> pending = new ConcurrentHashMap<String, InetAddress>();
    /** The row whose status ping is running on this thread (see {@link #resolveStarted}). */
    private final ThreadLocal<String> resolving = new ThreadLocal<String>();

    private volatile Table table;
    private volatile boolean off;
    private boolean started;

    private GeoIp() {}

    public static GeoIp getInstance() {
        return INSTANCE;
    }

    /**
     * Country for a server row, or {@code null} while unknown.
     *
     * <p>Never blocks and never resolves: a row whose status ping has not
     * finished yet simply draws no flag until it has.
     */
    public String country(String serverKey) {
        if (serverKey == null) return null;
        String cached = byServer.get(serverKey);
        if (cached == null) {
            Table mapped = table(); // starts the load on first use; never blocks on I/O
            if (mapped == null) return null;
            InetAddress address = pending.remove(serverKey);
            if (address != null) {
                store(serverKey, mapped.country(address));
                cached = byServer.get(serverKey);
            }
            if (cached == null) return null;
        }
        return NONE.equals(cached) ? null : cached;
    }

    /**
     * The vanilla status ping for {@code serverKey} is about to resolve its
     * host on this thread; the address handed to
     * {@code NetworkManager.createNetworkManagerAndConnect} belongs to it.
     */
    public void resolveStarted(String serverKey) {
        if (serverKey == null) {
            resolving.remove();
        } else {
            resolving.set(serverKey);
        }
    }

    /** The address vanilla just resolved for the thread's current ping, if any. */
    public void resolved(InetAddress address) {
        String key = resolving.get();
        resolving.remove();
        record(key, address);
    }

    /** Record the address the vanilla status ping resolved for {@code serverKey}. */
    public void record(String serverKey, InetAddress address) {
        if (serverKey == null || address == null || off) return;
        Table mapped = table();
        if (mapped == null) {
            pending.put(serverKey, address); // the loader drains this once mapped
            return;
        }
        store(serverKey, mapped.country(address));
    }

    private void store(String key, String code) {
        if (byServer.size() > CACHE_MAX) byServer.clear();
        byServer.put(key, code == null ? NONE : code);
    }

    /**
     * The mapped table, or {@code null} while the daemon thread is loading it
     * (or forever when geolocation is off). Never blocks on I/O.
     */
    public Table table() {
        Table mapped = table;
        if (mapped != null) return mapped;
        if (off) return null;
        synchronized (this) {
            if (table == null && !off && !started) {
                started = true;
                startLoader();
            }
        }
        return table;
    }

    private void startLoader() {
        Thread thread = new Thread(new Runnable() {
            @Override
            public void run() {
                load();
            }
        }, "Espectral GeoIP");
        thread.setDaemon(true);
        thread.start();
    }

    private void load() {
        String property = System.getProperty(PROPERTY);
        if (property == null || property.trim().isEmpty()) {
            off = true; // no table advertised: stay off, no per-row retry
            return;
        }
        Table loaded;
        try {
            loaded = Table.open(Paths.get(property.trim()));
        } catch (IOException broken) {
            LOGGER.warn("[geoip] cannot read {}: {}", property, broken.toString());
            off = true;
            return;
        } catch (RuntimeException broken) {
            LOGGER.warn("[geoip] unusable table {}: {}", property, broken.toString());
            off = true;
            return;
        }
        table = loaded;
        for (String key : pending.keySet()) {
            InetAddress address = pending.remove(key);
            if (address != null) store(key, loaded.country(address));
        }
    }

    // -----------------------------------------------------------------------
    // Table
    // -----------------------------------------------------------------------

    /**
     * One {@code ESPGEO1} file: a 24-byte header, {@code 2 * country_count}
     * bytes of ascending ISO codes, then the IPv4 entries and the IPv6 entries.
     * An entry is {@code start[16] end[16] u16 LE country index} with 16-byte
     * big-endian addresses; IPv4 is stored IPv4-mapped ({@code ::ffff:a.b.c.d})
     * so a single lexicographic compare is also the numeric one.
     */
    public static final class Table {

        private final MappedByteBuffer buffer;
        private final String[] countries;
        private final int v4Count;
        private final int v6Count;
        private final int v4Offset;
        private final int v6Offset;

        private Table(MappedByteBuffer buffer, String[] countries, int v4Count, int v6Count,
                int v4Offset, int v6Offset) {
            this.buffer = buffer;
            this.countries = countries;
            this.v4Count = v4Count;
            this.v6Count = v6Count;
            this.v4Offset = v4Offset;
            this.v6Offset = v6Offset;
        }

        /** Map and validate a table file, ready for {@link #country(InetAddress)}. */
        public static Table open(Path path) throws IOException {
            FileChannel channel = FileChannel.open(path, StandardOpenOption.READ);
            try {
                long size = channel.size();
                if (size < HEADER_LEN) {
                    throw new IOException("truncated header (" + size + " bytes)");
                }
                if (size > Integer.MAX_VALUE) {
                    throw new IOException("table too large (" + size + " bytes)");
                }
                MappedByteBuffer buffer = channel.map(FileChannel.MapMode.READ_ONLY, 0, size);
                buffer.order(ByteOrder.LITTLE_ENDIAN);
                for (int i = 0; i < MAGIC.length; i++) {
                    if (buffer.get(i) != MAGIC[i]) {
                        throw new IOException("bad magic (not an ESPGEO1 table)");
                    }
                }
                long v4 = ((long) buffer.getInt(8)) & 0xFFFFFFFFL;
                long v6 = ((long) buffer.getInt(12)) & 0xFFFFFFFFL;
                long count = ((long) buffer.getInt(16)) & 0xFFFFFFFFL;
                if (buffer.getInt(20) != 0) {
                    throw new IOException("unsupported table version (reserved word is not 0)");
                }
                long expected = HEADER_LEN + count * 2 + (v4 + v6) * ENTRY_LEN;
                if (expected != size) {
                    throw new IOException("bad length: " + size + " bytes, header says " + expected);
                }
                if (count > 0xFFFFL) {
                    throw new IOException("too many countries (" + count + ")");
                }
                int countryCount = (int) count;
                String[] codes = new String[countryCount];
                for (int i = 0; i < countryCount; i++) {
                    char first = (char) (buffer.get(HEADER_LEN + i * 2) & 0xFF);
                    char second = (char) (buffer.get(HEADER_LEN + i * 2 + 1) & 0xFF);
                    codes[i] = new String(new char[] {first, second});
                }
                int v4Offset = HEADER_LEN + countryCount * 2;
                int v6Offset = v4Offset + (int) v4 * ENTRY_LEN;
                return new Table(buffer, codes, (int) v4, (int) v6, v4Offset, v6Offset);
            } finally {
                channel.close();
            }
        }

        public int countryCount() {
            return countries.length;
        }

        public int rangeCount() {
            return v4Count + v6Count;
        }

        /** ISO-3166 alpha-2 of the range covering {@code address}, else {@code null}. */
        public String country(InetAddress address) {
            if (address == null) return null;
            byte[] raw = address.getAddress();
            if (raw.length != 4 && raw.length != 16) return null;
            boolean v4 = raw.length == 4 || isMapped(raw);
            byte[] key = v4 && raw.length == 4 ? mapped(raw) : raw;
            int offset = v4 ? v4Offset : v6Offset;
            int count = v4 ? v4Count : v6Count;

            int index = upperBound(offset, count, key) - 1;
            if (index < 0) return null;
            int entry = offset + index * ENTRY_LEN;
            if (compare(entry + 16, key) < 0) return null; // key > end
            int code = buffer.getShort(entry + 32) & 0xFFFF;
            return code < countries.length ? countries[code] : null;
        }

        /** IPv4 octets as the IPv4-mapped IPv6 form ({@code ::ffff:a.b.c.d}). */
        private static byte[] mapped(byte[] v4) {
            byte[] out = new byte[16];
            out[10] = (byte) 0xFF;
            out[11] = (byte) 0xFF;
            out[12] = v4[0];
            out[13] = v4[1];
            out[14] = v4[2];
            out[15] = v4[3];
            return out;
        }

        private static boolean isMapped(byte[] v6) {
            for (int i = 0; i < 10; i++) {
                if (v6[i] != 0) return false;
            }
            return v6[10] == (byte) 0xFF && v6[11] == (byte) 0xFF;
        }

        /** Index of the first start greater than {@code key}. */
        private int upperBound(int base, int count, byte[] key) {
            int low = 0;
            int high = count;
            while (low < high) {
                int mid = (low + high) >>> 1;
                if (compare(base + mid * ENTRY_LEN, key) <= 0) {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            return low;
        }

        /** Unsigned big-endian compare of the 16 bytes at {@code offset} against {@code key}. */
        private int compare(int offset, byte[] key) {
            for (int i = 0; i < 16; i++) {
                int left = buffer.get(offset + i) & 0xFF;
                int right = key[i] & 0xFF;
                if (left != right) return left < right ? -1 : 1;
            }
            return 0;
        }
    }
}
