package es.spectral.menu.geoip;

import java.io.IOException;
import java.net.InetAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.FileChannel;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardOpenOption;

/**
 * Reader for the launcher's compact country table ({@code ESPGEO1}), memory
 * mapped once on a daemon thread and never written.
 *
 * <p>The launcher downloads DB-IP's monthly "IP to Country Lite" CSV, encodes
 * it as {@code cache/geoip/countries-YYYY-MM.bin} and passes the absolute path
 * as {@code -Despectral.geoip=<path>}. This class maps that file read-only the
 * first time a flag is asked for and then only binary-searches it; nothing
 * here touches the network, the disk, or the render thread's frame budget
 * after the first frame.
 *
 * <p><b>File layout</b> (little-endian header/indices, no padding):
 * <pre>
 * 0   8   magic  "ESPGEO1\0"
 * 8   4   u32 v4_count
 * 12  4   u32 v6_count
 * 16  4   u32 country_count
 * 20  4   u32 reserved (0)
 * 24  2*C ISO-3166 alpha-2 codes, ASCII, ascending
 * …   34*v4  start[16] end[16] u16 country index   (IPv4 ranges, ascending)
 * …   34*v6  same shape                            (IPv6 ranges, ascending)
 * </pre>
 * Addresses are 16-byte big-endian so one lexicographic compare is the numeric
 * compare; IPv4 is stored IPv4-mapped ({@code ::ffff:a.b.c.d}) so the same
 * search works for both families.
 *
 * <p>Anything missing, unreadable, truncated or mis-magicked turns the reader
 * permanently "off" ({@link #OFF}): no per-row retries, no flags, no log spam
 * beyond one warning line.
 *
 * <p>Note: anycast and DDoS-proxy-fronted servers report their <em>proxy's</em>
 * country — the table answers "where is this IP announced", not "where does the
 * operator live".
 */
public final class GeoIp {

    /** System property the launcher sets; absent means "no table". */
    public static final String PROPERTY = "espectral.geoip";

    private static final byte[] MAGIC = {'E', 'S', 'P', 'G', 'E', 'O', '1', 0};
    private static final int HEADER_LEN = 24;
    private static final int ENTRY_LEN = 34;
    private static final int COUNTRIES_OFF = HEADER_LEN;

    private enum State { IDLE, LOADING, READY, OFF }

    private static volatile State state = State.IDLE;
    private static volatile Table table;

    private GeoIp() {}

    /**
     * ISO-3166 alpha-2 code for {@code address}, or {@code null} when the
     * table has no entry (unknown range), the table is still loading, the table
     * could not be loaded, or the address is loopback/private/link-local/ULA —
     * none of those reach a public server and a flag would be a lie.
     */
    public static String country(InetAddress address) {
        if (address == null || !usable(address)) {
            return null;
        }
        Table t = table();
        if (t == null) {
            return null;
        }
        byte[] raw = address.getAddress();
        if (raw.length == 4) {
            return t.lookup(v4Mapped(raw), true);
        }
        if (raw.length != 16) {
            return null;
        }
        return t.lookup(raw, isV4Mapped(raw));
    }

    /** Whether the table finished loading; {@code false} while off/loading. */
    public static boolean ready() {
        return state == State.READY;
    }

    /** Whether table loading was attempted and failed (or no path was set). */
    public static boolean disabled() {
        return state == State.OFF;
    }

    /** Kicks the lazy load (idempotent; also happens on the first lookup). */
    public static void start() {
        table();
    }

    private static Table table() {
        State current = state;
        if (current == State.READY) {
            return table;
        }
        if (current == State.IDLE) {
            ensureLoading();
        }
        return null;
    }

    /** Starts the daemon loader exactly once (idempotent, cheap when off). */
    private static synchronized void ensureLoading() {
        if (state != State.IDLE) {
            return;
        }
        state = State.LOADING;
        Thread loader = new Thread(GeoIp::load, "espectral-geoip");
        loader.setDaemon(true);
        loader.setPriority(Thread.MIN_PRIORITY);
        loader.start();
    }

    private static void load() {
        String configured = System.getProperty(PROPERTY);
        if (configured == null || configured.isBlank()) {
            state = State.OFF;
            return;
        }
        Path path;
        try {
            path = Paths.get(configured);
        } catch (RuntimeException invalid) {
            state = State.OFF;
            return;
        }
        try (FileChannel channel = FileChannel.open(path, StandardOpenOption.READ)) {
            long size = channel.size();
            if (size < HEADER_LEN || size > Integer.MAX_VALUE) {
                state = State.OFF;
                return;
            }
            ByteBuffer mapped = channel.map(FileChannel.MapMode.READ_ONLY, 0, size)
                    .order(ByteOrder.LITTLE_ENDIAN);
            Table parsed = Table.parse(mapped);
            if (parsed == null) {
                state = State.OFF;
                return;
            }
            table = parsed;
            state = State.READY;
        } catch (IOException | RuntimeException unreadable) {
            state = State.OFF;
        }
    }

    // ── Address classification ─────────────────────────────────────────────

    private static boolean usable(InetAddress address) {
        if (address.isAnyLocalAddress() || address.isLoopbackAddress()
                || address.isLinkLocalAddress() || address.isSiteLocalAddress()
                || address.isMulticastAddress()) {
            return false;
        }
        byte[] raw = address.getAddress();
        if (raw.length == 4) {
            int first = raw[0] & 0xFF;
            // 0.0.0.0/8 "this network" and the limited broadcast address.
            return first != 0 && (raw[0] & 0xFF) != 0xFF;
        }
        // IPv6 unique-local fc00::/7 — the v6 analogue of RFC 1918.
        return (raw[0] & 0xFE) != 0xFC;
    }

    private static boolean isV4Mapped(byte[] raw) {
        for (int i = 0; i < 10; i++) {
            if (raw[i] != 0) {
                return false;
            }
        }
        return (raw[10] & 0xFF) == 0xFF && (raw[11] & 0xFF) == 0xFF;
    }

    private static byte[] v4Mapped(byte[] raw) {
        byte[] mapped = new byte[16];
        mapped[10] = (byte) 0xFF;
        mapped[11] = (byte) 0xFF;
        System.arraycopy(raw, 0, mapped, 12, 4);
        return mapped;
    }

    // ── Table ──────────────────────────────────────────────────────────────

    /** One validated, memory-mapped table. Immutable after construction. */
    private static final class Table {
        private final ByteBuffer buffer;
        private final int v4Count;
        private final int v6Count;
        private final int countryCount;
        private final int v4Base;
        private final int v6Base;

        private Table(ByteBuffer buffer, int v4Count, int v6Count, int countryCount) {
            this.buffer = buffer;
            this.v4Count = v4Count;
            this.v6Count = v6Count;
            this.countryCount = countryCount;
            this.v4Base = COUNTRIES_OFF + countryCount * 2;
            this.v6Base = v4Base + v4Count * ENTRY_LEN;
        }

        static Table parse(ByteBuffer buffer) {
            if (buffer.capacity() < HEADER_LEN) {
                return null;
            }
            for (int i = 0; i < MAGIC.length; i++) {
                if (buffer.get(i) != MAGIC[i]) {
                    return null;
                }
            }
            long v4 = Integer.toUnsignedLong(buffer.getInt(8));
            long v6 = Integer.toUnsignedLong(buffer.getInt(12));
            long countries = Integer.toUnsignedLong(buffer.getInt(16));
            long reserved = Integer.toUnsignedLong(buffer.getInt(20));
            if (reserved != 0 || countries > 0xFFFF || v4 > 0xFFFFFF || v6 > 0xFFFFFF) {
                return null;
            }
            long expected = HEADER_LEN + countries * 2 + (v4 + v6) * ENTRY_LEN;
            if (expected != buffer.capacity()) {
                return null;
            }
            return new Table(buffer, (int) v4, (int) v6, (int) countries);
        }

        String lookup(byte[] key, boolean useV4) {
            int base = useV4 ? v4Base : v6Base;
            int count = useV4 ? v4Count : v6Count;
            // Last entry whose start is <= key (partition point over starts).
            int lo = 0;
            int hi = count;
            while (lo < hi) {
                int mid = (lo + hi) >>> 1;
                if (compare(base + mid * ENTRY_LEN, key) <= 0) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            int index = lo - 1;
            if (index < 0) {
                return null;
            }
            int entry = base + index * ENTRY_LEN;
            if (compare(entry + 16, key) < 0) {
                return null;
            }
            int country = buffer.getShort(entry + 32) & 0xFFFF;
            if (country >= countryCount) {
                return null;
            }
            int at = COUNTRIES_OFF + country * 2;
            char hiChar = (char) (buffer.get(at) & 0xFF);
            char loChar = (char) (buffer.get(at + 1) & 0xFF);
            if (hiChar < 'A' || hiChar > 'Z' || loChar < 'A' || loChar > 'Z') {
                return null;
            }
            return new String(new char[] {hiChar, loChar});
        }

        /** Lexicographic compare of the 16 table bytes at {@code at} with {@code key}. */
        private int compare(int at, byte[] key) {
            for (int i = 0; i < 16; i++) {
                int a = buffer.get(at + i) & 0xFF;
                int b = key[i] & 0xFF;
                if (a != b) {
                    return a < b ? -1 : 1;
                }
            }
            return 0;
        }
    }
}
