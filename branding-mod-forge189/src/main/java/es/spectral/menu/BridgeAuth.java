package es.spectral.menu;

import java.net.URL;
import java.nio.charset.Charset;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Locale;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/**
 * Signed requests to the launcher's loopback bridge — Java 8 port of the
 * modern mod's {@code BridgeAuth} (engine {@code proof.rs}).
 *
 * <p>The per-launch token arrives in the {@code ESPECTRAL_BRIDGE_TOKEN}
 * environment variable (other users cannot read a process's environment, but
 * they can read its command line) and is never sent. Each request carries
 * {@code x-espectral-bridge-id} ({@code sha256(token)[..16]}), a fresh nonce
 * and {@code x-espectral-bridge-mac = hmac(token, "req\n" + METHOD + "\n" +
 * target + "\n" + nonce + "\n" + sha256(body))}. Every answer must carry
 * {@code x-espectral-proof = hmac(token, "res\n" + nonce + "\n" +
 * sha256(body))}; anything else on the port cannot produce it.
 */
public final class BridgeAuth {

    public static final String TOKEN_ENV = "ESPECTRAL_BRIDGE_TOKEN";
    public static final String ID_HEADER = "x-espectral-bridge-id";
    public static final String NONCE_HEADER = "x-espectral-nonce";
    public static final String MAC_HEADER = "x-espectral-bridge-mac";
    public static final String PROOF_HEADER = "x-espectral-proof";

    private static final Charset UTF8 = Charset.forName("UTF-8");
    private static final char[] HEX = "0123456789abcdef".toCharArray();
    private static final SecureRandom RANDOM = new SecureRandom();

    private final byte[] key;
    private final String id;

    public BridgeAuth(String token) {
        this.key = token.getBytes(UTF8);
        this.id = sha256Hex(key).substring(0, 16);
    }

    /** The launcher's token for this launch, or null when there is none. */
    public static String tokenFromEnv() {
        String token = System.getenv(TOKEN_ENV);
        return token == null || token.isEmpty() ? null : token;
    }

    public String id() {
        return id;
    }

    /** 16 random bytes as 32 lowercase hex. */
    public static String newNonce() {
        byte[] b = new byte[16];
        RANDOM.nextBytes(b);
        return hex(b);
    }

    /** Path plus {@code ?query}, exactly as sent on the request line. */
    public static String target(URL url) {
        String path = url.getPath() == null || url.getPath().isEmpty() ? "/" : url.getPath();
        return url.getQuery() == null ? path : path + "?" + url.getQuery();
    }

    public String requestMac(String method, String target, String nonce, byte[] body) {
        String msg = "req\n" + method.toUpperCase(Locale.ROOT) + "\n" + target + "\n" + nonce + "\n"
                + sha256Hex(body);
        return hmacHex(key, msg.getBytes(UTF8));
    }

    /** True when {@code proof} signs exactly {@code body} for {@code nonce}. */
    public boolean proofOk(String nonce, byte[] body, String proof) {
        if (proof == null) return false;
        String want = hmacHex(key, ("res\n" + nonce + "\n" + sha256Hex(body)).getBytes(UTF8));
        return MessageDigest.isEqual(want.getBytes(UTF8), proof.trim().toLowerCase(Locale.ROOT).getBytes(UTF8));
    }

    static String sha256Hex(byte[] data) {
        try {
            return hex(MessageDigest.getInstance("SHA-256").digest(data));
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    static String hmacHex(byte[] key, byte[] msg) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(key, "HmacSHA256"));
            return hex(mac.doFinal(msg));
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    private static String hex(byte[] bytes) {
        char[] out = new char[bytes.length * 2];
        for (int i = 0; i < bytes.length; i++) {
            out[i * 2] = HEX[(bytes[i] >> 4) & 0xf];
            out[i * 2 + 1] = HEX[bytes[i] & 0xf];
        }
        return new String(out);
    }
}
