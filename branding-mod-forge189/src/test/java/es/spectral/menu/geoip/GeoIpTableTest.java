package es.spectral.menu.geoip;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.fail;

import java.io.IOException;
import java.net.InetAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;

import org.junit.Test;

/**
 * The Java 8 {@code ESPGEO1} reader against a hand-built table: the format is
 * the launcher's contract, so the encodings are asserted byte for byte here
 * rather than only through the real monthly file.
 */
public class GeoIpTableTest {

    private static final byte[] V4_US_START = mapped(8, 8, 8, 0);
    private static final byte[] V4_US_END = mapped(8, 8, 8, 255);
    private static final byte[] V4_ES_START = mapped(217, 126, 0, 0);
    private static final byte[] V4_ES_END = mapped(217, 126, 0, 255);
    private static final byte[] V6_DE_START = v6(0x2a, 0x00, 0x14, 0x50);
    private static final byte[] V6_DE_END = v6End(0x2a, 0x00, 0x14, 0x50);

    /** Two v4 ranges (US, ES) and one v6 range (DE), countries ascending. */
    private static byte[] table() {
        byte[] codes = {'D', 'E', 'E', 'S', 'U', 'S'};
        int size = GeoIp.HEADER_LEN + codes.length + 3 * GeoIp.ENTRY_LEN;
        ByteBuffer out = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN);
        out.put(GeoIp.MAGIC);
        out.putInt(2);      // v4 ranges
        out.putInt(1);      // v6 ranges
        out.putInt(3);      // countries
        out.putInt(0);      // reserved
        out.put(codes);
        out.put(V4_US_START).put(V4_US_END).putShort((short) 2); // US
        out.put(V4_ES_START).put(V4_ES_END).putShort((short) 1); // ES
        out.put(V6_DE_START).put(V6_DE_END).putShort((short) 0); // DE
        return out.array();
    }

    private static byte[] mapped(int a, int b, int c, int d) {
        byte[] out = new byte[16];
        out[10] = (byte) 0xFF;
        out[11] = (byte) 0xFF;
        out[12] = (byte) a;
        out[13] = (byte) b;
        out[14] = (byte) c;
        out[15] = (byte) d;
        return out;
    }

    private static byte[] v6(int... prefix) {
        byte[] out = new byte[16];
        for (int i = 0; i < prefix.length; i++) {
            out[i] = (byte) prefix[i];
        }
        return out;
    }

    /** {@code 2a00:1450::/32} style range: the given prefix, then all ones. */
    private static byte[] v6End(int a, int b, int c, int d) {
        byte[] out = v6(a, b, c, d);
        for (int i = 4; i < 16; i++) {
            out[i] = (byte) 0xFF;
        }
        return out;
    }

    private static GeoIp.Table open(byte[] bytes) throws IOException {
        Path file = Files.createTempFile("espectral-geoip", ".bin");
        try {
            Files.write(file, bytes);
            return GeoIp.Table.open(file);
        } finally {
            Files.deleteIfExists(file);
        }
    }

    @Test
    public void readsHeaderCounts() throws IOException {
        GeoIp.Table table = open(table());
        assertEquals(3, table.countryCount());
        assertEquals(3, table.rangeCount());
    }

    @Test
    public void findsTheCoveringRange() throws IOException {
        GeoIp.Table table = open(table());
        assertEquals("US", table.country(InetAddress.getByName("8.8.8.8")));
        assertEquals("US", table.country(InetAddress.getByName("8.8.8.0")));
        assertEquals("US", table.country(InetAddress.getByName("8.8.8.255")));
        assertEquals("ES", table.country(InetAddress.getByName("217.126.0.60")));
        assertEquals("DE", table.country(InetAddress.getByName("2a00:1450:4001::1")));
    }

    @Test
    public void gapsAndPrivateSpaceHaveNoCountry() throws IOException {
        GeoIp.Table table = open(table());
        assertNull("between the US and ES ranges", table.country(InetAddress.getByName("8.8.9.0")));
        assertNull("before the first range", table.country(InetAddress.getByName("1.1.1.1")));
        assertNull("after the last v4 range", table.country(InetAddress.getByName("200.1.1.1")));
        assertNull(table.country(InetAddress.getByName("192.168.1.10")));
        assertNull(table.country(InetAddress.getByName("10.0.0.1")));
        assertNull(table.country(InetAddress.getByName("fd00::1")));
        assertNull(table.country(null));
    }

    @Test
    public void ipv4MappedInputUsesTheV4Section() throws IOException {
        GeoIp.Table table = open(table());
        // ::ffff:8.8.8.8 must behave exactly like 8.8.8.8.
        assertEquals("US", table.country(InetAddress.getByName("::ffff:8.8.8.8")));
    }

    @Test
    public void rejectsBrokenTables() throws IOException {
        byte[] bytes = table();
        byte[] badMagic = bytes.clone();
        badMagic[0] = 'X';
        assertBroken(badMagic, "magic");

        byte[] badLength = bytes.clone();
        byte[] short_ = new byte[badLength.length - 1];
        System.arraycopy(badLength, 0, short_, 0, short_.length);
        assertBroken(short_, "length");

        byte[] badReserved = bytes.clone();
        badReserved[20] = 1;
        assertBroken(badReserved, "reserved");
    }

    private static void assertBroken(byte[] bytes, String what) throws IOException {
        try {
            open(bytes);
            fail("accepted a table with a bad " + what);
        } catch (IOException expected) {
            // fine: the reader must refuse the file instead of answering nonsense
        }
    }

    @Test
    public void magicIsTheEspgeo1Signature() {
        assertArrayEquals(new byte[] {'E', 'S', 'P', 'G', 'E', 'O', '1', 0}, GeoIp.MAGIC);
    }
}
