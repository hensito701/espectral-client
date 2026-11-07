package es.spectral.menu.ruta;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.nio.charset.StandardCharsets;
import java.util.Base64;

import org.junit.Test;

/**
 * Grant claim decoding. Java 8 has no Ed25519, so the client only reads the
 * payload for UI gating; the signature is the relay's job. These tests pin the
 * claims the gating depends on, including the "no token, no UI" cases.
 */
public class RutaGrantTest {

    /** Builds {@code header.payload.signature} with a dummy signature. */
    private static String token(String payloadJson) {
        Base64.Encoder enc = Base64.getUrlEncoder().withoutPadding();
        String header = enc.encodeToString("{\"alg\":\"EdDSA\",\"kid\":\"eid1\",\"typ\":\"ESG\"}"
                .getBytes(StandardCharsets.US_ASCII));
        String payload = enc.encodeToString(payloadJson.getBytes(StandardCharsets.UTF_8));
        return header + "." + payload + ".AAAAAAAA";
    }

    private static final String UUID = "0123456789abcdef0123456789abcdef";

    @Test
    public void readsTheClaimsGatingNeeds() {
        long exp = System.currentTimeMillis() / 1000L + 3600L;
        RutaGrant grant = RutaGrant.parse(token("{\"v\":1,\"iss\":\"https://id.uhce.top\","
                + "\"aud\":\"espectral-client\",\"sub\":\"" + UUID + "\","
                + "\"flags\":[\"mvp\",\"donator\"],\"iat\":" + (exp - 3600L) + ",\"exp\":" + exp + "}"));
        assertNotNull(grant);
        assertEquals(UUID, grant.subject());
        assertTrue(grant.hasMvp());
        assertEquals(exp, grant.expiresAt());
        assertTrue(grant.allowsRuta(UUID, System.currentTimeMillis()));
        assertFalse("another account's grant", grant.allowsRuta(
                "ffffffffffffffffffffffffffffffff", System.currentTimeMillis()));
        assertFalse("expired", grant.allowsRuta(UUID, exp * 1000L + 1L));
    }

    @Test
    public void rejectsMalformedOrNonEsgTokens() {
        assertNull(RutaGrant.parse(null));
        assertNull(RutaGrant.parse(""));
        assertNull(RutaGrant.parse("not-a-jws"));
        assertNull(RutaGrant.parse("a.b"));
        assertNull(RutaGrant.parse("a.!!!.c"));
        assertNull("v must be 1", RutaGrant.parse(token("{\"v\":2,\"sub\":\"" + UUID + "\","
                + "\"flags\":[\"mvp\"],\"exp\":" + (System.currentTimeMillis() / 1000L + 60L) + "}")));
        assertNull("sub must be 32 hex", RutaGrant.parse(token("{\"v\":1,\"sub\":\"nope\","
                + "\"flags\":[\"mvp\"],\"exp\":" + (System.currentTimeMillis() / 1000L + 60L) + "}")));
        assertNull("exp is required", RutaGrant.parse(token("{\"v\":1,\"sub\":\"" + UUID + "\","
                + "\"flags\":[\"mvp\"]}")));
    }

    @Test
    public void flagsAreCaseInsensitiveAndOptional() {
        long exp = System.currentTimeMillis() / 1000L + 60L;
        RutaGrant upper = RutaGrant.parse(token("{\"v\":1,\"sub\":\"" + UUID + "\","
                + "\"flags\":[\"MVP\"],\"exp\":" + exp + "}"));
        assertNotNull(upper);
        assertTrue(upper.hasMvp());

        RutaGrant plain = RutaGrant.parse(token("{\"v\":1,\"sub\":\"" + UUID + "\","
                + "\"exp\":" + exp + "}"));
        assertNotNull(plain);
        assertFalse("no flags list means no mvp", plain.hasMvp());
        assertFalse(plain.allowsRuta(UUID, System.currentTimeMillis()));
    }

    @Test
    public void uuidCheck() {
        assertTrue(RutaGrant.isUuid32(UUID));
        assertFalse(RutaGrant.isUuid32(null));
        assertFalse(RutaGrant.isUuid32("01234567-89ab-cdef-0123-456789abcdef"));
        assertFalse(RutaGrant.isUuid32("0123456789ABCDEF0123456789ABCDEF"));
        assertFalse(RutaGrant.isUuid32("0123456789abcdef0123456789abcde"));
    }

    @Test
    public void toStringNeverLeaksTheToken() {
        long exp = System.currentTimeMillis() / 1000L + 60L;
        String jws = token("{\"v\":1,\"sub\":\"" + UUID + "\",\"flags\":[\"mvp\"],\"exp\":" + exp + "}");
        RutaGrant grant = RutaGrant.parse(jws);
        assertNotNull(grant);
        String text = grant.toString();
        assertTrue(text.contains(UUID));
        assertFalse("the bearer token must never be printed", text.contains(jws));
        assertFalse(text.contains("AAAAAAAA"));
    }
}
