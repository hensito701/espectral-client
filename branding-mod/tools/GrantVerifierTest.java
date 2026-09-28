package es.spectral.menu.cosmetics;

import java.nio.charset.StandardCharsets;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.PrivateKey;
import java.security.PublicKey;
import java.security.Signature;
import java.util.Base64;
import java.util.UUID;

/**
 * Standalone harness for {@link GrantVerifier} — exercises every rejection
 * rule with a throwaway Ed25519 keypair plus a positive grant that also
 * verifies against the pinned production key is impossible (no private key),
 * so the happy path runs against the test key and the pinned key is exercised
 * for rejection. Run:
 *
 * <pre>
 *   javac -cp gson.jar -d build/grant-test \
 *       src/main/java/es/spectral/menu/cosmetics/GrantVerifier.java \
 *       tools/GrantVerifierTest.java
 *   java -cp "gson.jar;build/grant-test" es.spectral.menu.cosmetics.GrantVerifierTest
 * </pre>
 */
public final class GrantVerifierTest {

    private static int checks;
    private static int failures;

    public static void main(String[] args) throws Exception {
        KeyPairGenerator gen = KeyPairGenerator.getInstance("Ed25519");
        KeyPair kp = gen.generateKeyPair();
        KeyPair other = gen.generateKeyPair();
        String sub = "0123456789abcdef0123456789abcdef";
        long now = 1_800_000_000L;

        String good = sign(header("EdDSA", "eid1", "ESG"),
                payload(sub, now, now + 86400, "[\"supporter\",\"mvp\"]"), kp.getPrivate());

        ok(GrantVerifier.verify(good, sub, kp.getPublic(), now) != null,
                "valid grant accepted");
        GrantVerifier.Grant g = GrantVerifier.verify(good, sub, kp.getPublic(), now);
        ok(g != null && g.hasFlag("supporter") && g.hasFlag("mvp"), "flags parsed");
        ok(g != null && g.expiresAt() == now + 86400, "exp parsed");

        // signature path
        rej(sign(header("EdDSA", "eid1", "ESG"),
                payload(sub, now, now + 86400, "[]"), other.getPrivate()),
                sub, kp.getPublic(), now, "wrong key -> bad sig");
        rej(tamper(good), sub, kp.getPublic(), now, "tampered signature bytes");

        // header rules
        rej(sign(header("RS256", "eid1", "ESG"), payload(sub, now, now + 100, "[]"), kp.getPrivate()),
                sub, kp.getPublic(), now, "alg != EdDSA");
        rej(sign(header("EdDSA", "eid2", "ESG"), payload(sub, now, now + 100, "[]"), kp.getPrivate()),
                sub, kp.getPublic(), now, "unknown kid");
        rej(sign(header("EdDSA", "eid1", "JWT"), payload(sub, now, now + 100, "[]"), kp.getPrivate()),
                sub, kp.getPublic(), now, "typ != ESG");

        // payload rules
        rej(sign(header("EdDSA", "eid1", "ESG"),
                payloadRaw("{\"v\":1,\"iss\":\"https://id.espectral.es\",\"aud\":\"other\",\"sub\":\""
                        + sub + "\",\"flags\":[],\"iat\":" + now + ",\"exp\":" + (now + 100) + "}"),
                kp.getPrivate()), sub, kp.getPublic(), now, "aud mismatch");
        rej(sign(header("EdDSA", "eid1", "ESG"),
                payloadRaw("{\"v\":1,\"iss\":\"https://evil.example\",\"aud\":\"espectral-client\",\"sub\":\""
                        + sub + "\",\"flags\":[],\"iat\":" + now + ",\"exp\":" + (now + 100) + "}"),
                kp.getPrivate()), sub, kp.getPublic(), now, "iss mismatch");
        rej(sign(header("EdDSA", "eid1", "ESG"),
                payload(sub, now - 2000, now - 1, "[]"), kp.getPrivate()),
                sub, kp.getPublic(), now, "expired grant");
        rej(sign(header("EdDSA", "eid1", "ESG"),
                payload(sub, now + 600, now + 86400, "[]"), kp.getPrivate()),
                sub, kp.getPublic(), now, "iat > now+300");
        ok(GrantVerifier.verify(
                sign(header("EdDSA", "eid1", "ESG"),
                        payload(sub, now + 299, now + 86400, "[]"), kp.getPrivate()),
                sub, kp.getPublic(), now) != null, "iat within 300s skew accepted");

        // subject binding
        rej(sign(header("EdDSA", "eid1", "ESG"),
                payload("ffffffffffffffffffffffffffffffff", now, now + 100, "[]"), kp.getPrivate()),
                sub, kp.getPublic(), now, "sub != applied uuid");

        // envelope
        rej("not.a.jws.at.all", sub, kp.getPublic(), now, "malformed envelope");
        rej("..", sub, kp.getPublic(), now, "empty segments");
        rej(good, null, kp.getPublic(), now, "null subject");

        // uuid convenience overload must not accept a grant for a different uuid
        ok(GrantVerifier.verify(good, null) == null, "null uuid rejected");

        if (failures == 0) {
            System.out.println("GrantVerifierTest: " + checks + " checks, all passed");
        } else {
            System.out.println("GrantVerifierTest: " + failures + "/" + checks + " FAILED");
            System.exit(1);
        }
    }

    private static void ok(boolean cond, String name) {
        checks++;
        if (!cond) {
            failures++;
            System.out.println("FAIL(ok): " + name);
        }
    }

    private static void rej(String jws, String sub, PublicKey key, long now, String name) {
        checks++;
        if (GrantVerifier.verify(jws, sub, key, now) != null) {
            failures++;
            System.out.println("FAIL(rej): " + name);
        }
    }

    private static String header(String alg, String kid, String typ) {
        return "{\"alg\":\"" + alg + "\",\"kid\":\"" + kid + "\",\"typ\":\"" + typ + "\"}";
    }

    private static String payload(String sub, long iat, long exp, String flags) {
        return payloadRaw("{\"v\":1,\"iss\":\"https://id.espectral.es\",\"aud\":\"espectral-client\",\"sub\":\""
                + sub + "\",\"flags\":" + flags + ",\"iat\":" + iat + ",\"exp\":" + exp + "}");
    }

    private static String payloadRaw(String json) {
        return json;
    }

    private static String sign(String headerJson, String payloadJson, PrivateKey key) throws Exception {
        String h = b64(headerJson.getBytes(StandardCharsets.UTF_8));
        String p = b64(payloadJson.getBytes(StandardCharsets.UTF_8));
        Signature signer = Signature.getInstance("Ed25519");
        signer.initSign(key);
        signer.update((h + "." + p).getBytes(StandardCharsets.US_ASCII));
        return h + "." + p + "." + b64(signer.sign());
    }

    private static String b64(byte[] data) {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(data);
    }

    /** Flips one base64url char inside the signature segment. */
    private static String tamper(String jws) {
        int dot = jws.lastIndexOf('.');
        char[] chars = jws.toCharArray();
        int i = dot + 1;
        chars[i] = chars[i] == 'A' ? 'B' : 'A';
        return new String(chars);
    }
}
