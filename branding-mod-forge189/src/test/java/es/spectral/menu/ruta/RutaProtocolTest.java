package es.spectral.menu.ruta;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

/**
 * The pure half of Ruta Espectral: address normalization, eligibility, the
 * {@code ERT1} line and the relay reply. These are the rules the relay enforces
 * too, so a drift here would silently break routing.
 */
public class RutaProtocolTest {

    @Test
    public void normalizeDropsTheDefaultPortAndLowercases() {
        assertEquals("mc.example.net", RutaProtocol.normalize("MC.Example.NET"));
        assertEquals("mc.example.net", RutaProtocol.normalize("mc.example.net:25565"));
        assertEquals("mc.example.net:25566", RutaProtocol.normalize(" mc.example.net:25566 "));
        assertEquals("mc.example.net", RutaProtocol.normalize("mc.example.net."));
        assertEquals("1.2.3.4", RutaProtocol.normalize("1.2.3.4:25565"));
        // IPv6 keys are bracketed: the only form that keeps host/port
        // round-tripping, and the form the Fabric lane writes into the same file.
        assertEquals("[::1]", RutaProtocol.normalize("[::1]"));
        assertEquals("[::1]:25566", RutaProtocol.normalize("[::1]:25566"));
        // A bare literal is a host, never host:port (that spelling IS a v6
        // address), so it keeps the default port.
        assertEquals("[::1:25566]", RutaProtocol.normalize("::1:25566"));
        assertEquals("[2a00:1450::1]", RutaProtocol.normalize("2a00:1450::1"));
        assertNull(RutaProtocol.normalize(""));
        assertNull(RutaProtocol.normalize(null));
        assertNull(RutaProtocol.normalize("host:0"));
        assertNull(RutaProtocol.normalize("host:70000"));
        assertNull(RutaProtocol.normalize("host:abc"));
        assertNull(RutaProtocol.normalize("[::1]x"));
    }

    @Test
    public void keyOfCombinesHostAndPort() {
        assertEquals("mc.example.net", RutaProtocol.keyOf("mc.example.net", 25565));
        assertEquals("mc.example.net:25566", RutaProtocol.keyOf("MC.Example.net", 25566));
        assertEquals("[::1]", RutaProtocol.keyOf("::1", 25565));
        assertEquals("[::1]", RutaProtocol.keyOf("[::1]", 25565));
        assertEquals("[::1]:25566", RutaProtocol.keyOf("::1", 25566));
        assertEquals("[::1]:25566", RutaProtocol.keyOf("[::1]", 25566));
        assertEquals("[2a00:1450:4001::1]", RutaProtocol.keyOf("2a00:1450:4001::1", 25565));
        assertEquals("[2a00:1450:4001::1]:25566",
                RutaProtocol.keyOf("2a00:1450:4001::1", 25566));
        assertNull(RutaProtocol.keyOf(null, 25565));
        assertNull(RutaProtocol.keyOf("", 25565));
        assertNull(RutaProtocol.keyOf("host", 0));
    }

    @Test
    public void hostOfAndPortOfSplitTheKey() {
        assertEquals("mc.example.net", RutaProtocol.hostOf("mc.example.net"));
        assertEquals("mc.example.net", RutaProtocol.hostOf("mc.example.net:25566"));
        assertEquals("2a00:1450::1", RutaProtocol.hostOf("[2a00:1450::1]"));
        assertEquals("::1", RutaProtocol.hostOf("[::1]:25566"));
        assertEquals(25565, RutaProtocol.portOf("mc.example.net"));
        assertEquals(25566, RutaProtocol.portOf("mc.example.net:25566"));
        assertEquals(25565, RutaProtocol.portOf("[::1]"));
        assertEquals(25566, RutaProtocol.portOf("[::1]:25566"));
    }

    @Test
    public void espectralAndLocalServersAreNeverRouted() {
        assertFalse(RutaProtocol.isEligible("uhce.top"));
        assertFalse(RutaProtocol.isEligible("play.uhce.top"));
        assertFalse(RutaProtocol.isEligible("espectral.es"));
        assertFalse(RutaProtocol.isEligible("mc.espectral.es"));
        assertFalse(RutaProtocol.isEligible("localhost"));
        assertFalse(RutaProtocol.isEligible("printer.local"));
        assertFalse(RutaProtocol.isEligible("lan"));
        assertFalse(RutaProtocol.isEligible(""));
        assertNull(RutaProtocol.normalize(""));
    }

    @Test
    public void privateAndSpecialAddressesAreNotEligible() {
        for (String ip : new String[] {
            "0.0.0.0", "10.0.0.5", "100.64.0.1", "127.0.0.1", "169.254.1.1",
            "172.16.0.1", "172.31.255.255", "192.0.0.1", "192.0.2.1", "192.168.1.10",
            "198.18.0.1", "198.51.100.1", "203.0.113.1", "224.0.0.1", "255.255.255.255",
            "::1", "::", "fc00::1", "fd12:3456::1", "fe80::1", "ff02::1", "2001:db8::1",
            "::ffff:192.168.0.1", "10.0.0.5:25565",
        }) {
            assertFalse(ip, RutaProtocol.isEligible(ip));
        }
    }

    @Test
    public void publicAddressesAndHostnamesAreEligible() {
        for (String ip : new String[] {
            "8.8.8.8", "150.136.181.209", "95.17.83.60", "203.0.114.1",
            "2a00:1450:4001::1", "::ffff:8.8.8.8", "mc.hypixel.net", "play.example.net:25566",
        }) {
            assertTrue(ip, RutaProtocol.isEligible(ip));
        }
    }

    @Test
    public void handshakeLineIsOneAsciiLine() {
        String line = RutaProtocol.handshakeLine("a.b.c", "150.136.181.209", 25565);
        assertEquals("ERT1 a.b.c 150.136.181.209 25565\n", line);
        assertEquals('E', line.charAt(0));
    }

    @Test
    public void handshakeLineRejectsGarbage() {
        assertBadGrant(new String[] {null, "", repeat("x", 3401), "has space?"}, "a.b.c", 25565);
        assertBadDest(new String[] {null, "", "1.2.3.4 x", "[::1]", "line\nbreak"}, "a.b.c");
    }

    private static void assertBadGrant(String[] grants, String ip, int port) {
        for (String grant : grants) {
            try {
                RutaProtocol.handshakeLine(grant, ip, port);
                fail("accepted grant " + grant + " / dest " + ip);
            } catch (IllegalArgumentException expected) {
                // the relay would answer ERR proto; refuse locally instead
            }
        }
    }

    private static void assertBadDest(String[] destinations, String grant) {
        for (String dest : destinations) {
            try {
                RutaProtocol.handshakeLine(grant, dest, 25565);
                fail("accepted destination " + dest);
            } catch (IllegalArgumentException expected) {
                // fine
            }
        }
        for (int port : new int[] {0, -1, 65536}) {
            try {
                RutaProtocol.handshakeLine(grant, "1.2.3.4", port);
                fail("accepted port " + port);
            } catch (IllegalArgumentException expected) {
                // fine
            }
        }
    }

    private static String repeat(String text, int times) {
        StringBuilder out = new StringBuilder(text.length() * times);
        for (int i = 0; i < times; i++) out.append(text);
        return out.toString();
    }

    @Test
    public void replyParsing() {
        assertTrue(RutaProtocol.parseReply("OK\n").ok);
        assertTrue(RutaProtocol.parseReply("OK").ok);
        assertTrue(RutaProtocol.parseReply("OK\r\n").ok);

        assertEquals("dest", RutaProtocol.parseReply("ERR dest\n").code);
        assertEquals("busy", RutaProtocol.parseReply("ERR busy\n").code);
        assertEquals("proto", RutaProtocol.parseReply("GARBAGE").code);
        assertEquals("proto", RutaProtocol.parseReply("ERR nope\n").code);
        assertEquals("proto", RutaProtocol.parseReply(null).code);
        assertFalse(RutaProtocol.parseReply("ERR limit\n").ok);
    }

    @Test
    public void knownCodes() {
        for (String code : new String[] {
            "proto", "auth", "expired", "notmvp", "dest", "limit", "connect", "busy",
        }) {
            assertTrue(code, RutaProtocol.isKnownCode(code));
        }
        assertFalse(RutaProtocol.isKnownCode("nope"));
        assertFalse(RutaProtocol.isKnownCode(null));
    }

    @Test
    public void messagesMapUnknownCodesToProto() {
        assertEquals("dest", RutaMessages.key("dest"));
        assertEquals("timeout", RutaMessages.key("timeout"));
        assertEquals("proto", RutaMessages.key("weird"));
        assertEquals("proto", RutaMessages.key(null));
        assertNotNull(RutaMessages.key("auth"));
    }
}
