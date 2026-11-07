package es.spectral.menu.ruta;

import java.util.ArrayList;
import java.util.List;

/**
 * Standalone harness for {@link RutaProtocol} — address normalization,
 * eligibility and the ERT1 wire format. Pure Java, no Minecraft, no Netty.
 *
 * <pre>
 *   javac -d build/ruta-test src/main/java/es/spectral/menu/ruta/RutaProtocol.java \
 *       tools/RutaProtocolTest.java
 *   java -cp build/ruta-test es.spectral.menu.ruta.RutaProtocolTest
 * </pre>
 */
public final class RutaProtocolTest {

    private static int checks;
    private static final List<String> FAILURES = new ArrayList<>();

    public static void main(String[] args) {
        // --- normalization -------------------------------------------------
        eq("mc.hypixel.net", RutaProtocol.normalize("mc.hypixel.net"), "bare host");
        eq("mc.hypixel.net", RutaProtocol.normalize("MC.Hypixel.NET"), "lowercased");
        eq("mc.hypixel.net", RutaProtocol.normalize("mc.hypixel.net:25565"), "default port dropped");
        eq("mc.hypixel.net:25566", RutaProtocol.normalize("mc.hypixel.net:25566"), "custom port kept");
        eq("mc.hypixel.net", RutaProtocol.normalize("  mc.hypixel.net.  "), "trimmed + trailing dot");
        eq("mc.hypixel.net", RutaProtocol.normalize("mc.hypixel.net:25565"), "explicit default port dropped");
        eq("10.0.0.5", RutaProtocol.normalize("10.0.0.5"), "ipv4 literal");
        eq("[::1]", RutaProtocol.normalize("[::1]"), "bracketed ipv6 literal");
        eq("[::1]:25566", RutaProtocol.normalize("[::1]:25566"), "bracketed ipv6 with port");
        eq("[2001:db8::1]", RutaProtocol.normalize("2001:DB8::1"), "bare ipv6 gets bracketed");
        eq(null, RutaProtocol.normalize(""), "empty");
        eq(null, RutaProtocol.normalize("   "), "blank");
        eq(null, RutaProtocol.normalize(null), "null");
        eq(null, RutaProtocol.normalize("host:0"), "port 0 rejected");
        eq(null, RutaProtocol.normalize("host:65536"), "port 65536 rejected");
        eq(null, RutaProtocol.normalize("host:abc"), "non-numeric port rejected");
        eq(null, RutaProtocol.normalize("[::1"), "unclosed bracket");

        // --- host/port/key helpers ----------------------------------------
        eq("mc.hypixel.net", RutaProtocol.hostOf("mc.hypixel.net:25566"), "hostOf custom port");
        eq(25566, RutaProtocol.portOf("mc.hypixel.net:25566"), "portOf custom");
        eq(RutaProtocol.DEFAULT_PORT, RutaProtocol.portOf("mc.hypixel.net"), "portOf default");
        eq("mc.hypixel.net", RutaProtocol.keyOf("Mc.Hypixel.Net", 25565), "keyOf default port");
        eq("mc.hypixel.net:25566", RutaProtocol.keyOf("MC.HYPIXEL.NET", 25566), "keyOf custom port");
        eq(null, RutaProtocol.keyOf(null, 25565), "keyOf null host");
        eq(null, RutaProtocol.keyOf("host", 0), "keyOf bad port");

        // --- eligibility ---------------------------------------------------
        ok(RutaProtocol.isEligible("mc.hypixel.net"), "public host eligible");
        ok(RutaProtocol.isEligible("play.example.com:25566"), "public host+port eligible");
        ok(RutaProtocol.isEligible("8.8.8.8"), "public ipv4 eligible");
        ok(RutaProtocol.isEligible("2001:4860:4860::8888"), "public ipv6 eligible");
        ok(RutaProtocol.isEligible("uhce.top"), "own apex domain eligible");
        ok(RutaProtocol.isEligible("mc.uhce.top"), "own subdomain eligible");
        ok(RutaProtocol.isEligible("24h.uhce.top:25568"), "own host with port eligible");
        ok(RutaProtocol.isEligible("espectral.es"), "legacy apex eligible");
        ok(RutaProtocol.isEligible("srv.espectral.es"), "legacy subdomain eligible");
        no(RutaProtocol.isEligible("localhost"), "localhost");
        no(RutaProtocol.isEligible("mc.localhost"), "*.localhost");
        no(RutaProtocol.isEligible("foo.local"), "*.local");
        no(RutaProtocol.isEligible("lan"), "lan");
        no(RutaProtocol.isEligible("127.0.0.1"), "loopback");
        no(RutaProtocol.isEligible("10.0.0.5"), "private 10/8");
        no(RutaProtocol.isEligible("192.168.1.10"), "private 192.168/16");
        no(RutaProtocol.isEligible("172.16.0.1"), "private 172.16/12");
        no(RutaProtocol.isEligible("169.254.1.1"), "link local");
        no(RutaProtocol.isEligible("100.64.0.1"), "cgnat");
        no(RutaProtocol.isEligible("198.18.0.1"), "benchmark");
        no(RutaProtocol.isEligible("0.0.0.0"), "this-network");
        no(RutaProtocol.isEligible("224.0.0.1"), "multicast");
        no(RutaProtocol.isEligible("::1"), "ipv6 loopback");
        no(RutaProtocol.isEligible("fe80::1"), "ipv6 link local");
        no(RutaProtocol.isEligible("fd00::1"), "ipv6 unique local");
        no(RutaProtocol.isEligible("[::1]:25566"), "bracketed ipv6 loopback");
        no(RutaProtocol.isEligible("::ffff:192.168.0.1"), "ipv4-mapped private");
        ok(RutaProtocol.isEligible("::ffff:8.8.8.8"), "ipv4-mapped public");

        // --- handshake line ------------------------------------------------
        String line = RutaProtocol.handshakeLine("a.b.c", "1.2.3.4", 25565);
        eq("ERT1 a.b.c 1.2.3.4 25565\n", line, "handshake line layout");
        ok(line.endsWith("\n"), "one line, newline terminated");
        eq("ERT1 g 2001:db8::1 25566\n",
                RutaProtocol.handshakeLine("g", "2001:db8::1", 25566), "ipv6 destination");
        throwsIAE(() -> RutaProtocol.handshakeLine(null, "1.2.3.4", 25565), "null grant");
        throwsIAE(() -> RutaProtocol.handshakeLine("", "1.2.3.4", 25565), "empty grant");
        throwsIAE(() -> RutaProtocol.handshakeLine("g", null, 25565), "null destination");
        throwsIAE(() -> RutaProtocol.handshakeLine("g", "1.2.3.4", 0), "port 0");
        throwsIAE(() -> RutaProtocol.handshakeLine("g", "1.2.3.4", 65536), "port 65536");
        throwsIAE(() -> RutaProtocol.handshakeLine("g", "[::1]", 25565), "bracketed destination");
        throwsIAE(() -> RutaProtocol.handshakeLine("g", "1.2.3.4 5", 25565), "space in destination");
        throwsIAE(() -> RutaProtocol.handshakeLine("g\n", "1.2.3.4", 25565), "newline in grant");
        StringBuilder huge = new StringBuilder();
        for (int i = 0; i < 5000; i++) huge.append('x');
        throwsIAE(() -> RutaProtocol.handshakeLine(huge.toString(), "1.2.3.4", 25565), "oversized line");

        // --- reply parsing --------------------------------------------------
        ok(RutaProtocol.parseReply("OK").ok, "OK accepted");
        ok(RutaProtocol.parseReply("OK\n").ok, "OK\\n accepted");
        ok(RutaProtocol.parseReply("OK\r\n").ok, "OK\\r\\n accepted");
        eq("proto", RutaProtocol.parseReply("").code, "empty reply");
        eq("proto", RutaProtocol.parseReply(null).code, "null reply");
        eq("proto", RutaProtocol.parseReply("OKAY").code, "unknown word");
        eq("proto", RutaProtocol.parseReply("err dest").code, "lowercase prefix rejected");
        eq("proto", RutaProtocol.parseReply("ERR").code, "bare ERR");
        eq("proto", RutaProtocol.parseReply("ERR nonsense").code, "unknown code");
        for (String code : new String[] {"proto", "auth", "expired", "notmvp", "dest",
                "limit", "connect", "busy"}) {
            RutaProtocol.Reply reply = RutaProtocol.parseReply("ERR " + code + "\n");
            ok(!reply.ok, "ERR " + code + " is an error");
            eq(code, reply.code, "ERR " + code + " code preserved");
        }
        ok(RutaProtocol.isKnownCode("busy"), "known code");
        ok(!RutaProtocol.isKnownCode("nope"), "unknown code rejected");

        // --- report ---------------------------------------------------------
        System.out.println(checks + " checks");
        if (FAILURES.isEmpty()) {
            System.out.println("all RutaProtocol checks passed");
        } else {
            for (String failure : FAILURES) {
                System.out.println("FAIL " + failure);
            }
            System.exit(1);
        }
    }

    private interface Body {
        void run();
    }

    private static void eq(Object expected, Object actual, String what) {
        checks++;
        if (expected == null ? actual == null : expected.equals(actual)) return;
        FAILURES.add(what + ": expected <" + expected + "> got <" + actual + ">");
    }

    private static void ok(boolean value, String what) {
        checks++;
        if (!value) FAILURES.add(what);
    }

    private static void no(boolean value, String what) {
        ok(!value, what + " must be false");
    }

    private static void throwsIAE(Body body, String what) {
        checks++;
        try {
            body.run();
            FAILURES.add(what + ": expected IllegalArgumentException");
        } catch (IllegalArgumentException expected) {
            // correct
        }
    }
}
