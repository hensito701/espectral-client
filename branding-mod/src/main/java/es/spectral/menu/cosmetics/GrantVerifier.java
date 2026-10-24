package es.spectral.menu.cosmetics;

import java.security.KeyFactory;
import java.security.PublicKey;
import java.security.Signature;
import java.security.spec.X509EncodedKeySpec;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.LinkedHashSet;
import java.util.Locale;
import java.util.Set;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Verifies compact-JWS entitlement grants (EdDSA/Ed25519, kid {@code eid1})
 * issued by {@code espectral-id}. Pure, allocation-light, and independent of
 * the Minecraft runtime so it can be exercised by a plain {@code javac}/{@code
 * java} harness.
 *
 * <p>Token: {@code b64url(header) "." b64url(payload) "." b64url(sig)}, the
 * signature covering the ASCII of the first two parts (no padding).
 * Header {@code {"alg":"EdDSA","kid":"eid1","typ":"ESG"}}; payload
 * {@code {"v":1,"iss":"https://id.uhce.top","aud":"espectral-client",
 * "sub":"<uuid32 lowercase hex>","flags":[...],"iat":s,"exp":s}}.
 *
 * <p>Two issuers are accepted: the canonical {@code https://id.uhce.top} and
 * the legacy {@code https://id.espectral.es} still minted by the service until
 * the ops {@code GRANT_ISS} flip. Nothing else is.
 *
 * <p>Rejects (returns {@code null}) on: malformed envelope, {@code alg} != EdDSA,
 * unknown {@code kid}, {@code typ} != ESG, bad signature, {@code aud} mismatch,
 * {@code iss} mismatch, {@code exp} &lt; now, {@code iat} &gt; now+300 s,
 * {@code sub} != the UUID the grant is applied to, or a non-32-hex {@code sub}.
 */
public final class GrantVerifier {

    /** Verified claim set. {@code flags} is an immutable lowercase set. */
    public record Grant(String subject, Set<String> flags, long expiresAt) {
        public boolean hasFlag(String flag) {
            return flags.contains(flag);
        }
    }

    /** Pinned service key identifier. */
    public static final String KEY_ID = "eid1";
    /** Pinned Ed25519 public key (SPKI DER, base64) for kid {@code eid1}. */
    private static final String SPKI_B64 =
            "MCowBQYDK2VwAyEA77kg5+/Nn6NvArCPuDvLYu3E4rvmYJHzaCGxwyvdlCI=";
    /** Maximum allowed {@code iat} skew into the future, seconds. */
    private static final long MAX_IAT_SKEW_S = 300L;

    private static final String REQUIRED_ALG = "EdDSA";
    private static final String REQUIRED_TYP = "ESG";
    /** Canonical issuer; the service flips GRANT_ISS to this after the client ships. */
    private static final String REQUIRED_ISS = "https://id.uhce.top";
    /** Issuer minted until that flip — accepted for backward compatibility. */
    private static final String LEGACY_ISS = "https://id.espectral.es";
    private static final String REQUIRED_AUD = "espectral-client";

    private static volatile PublicKey pinnedKey;

    private GrantVerifier() {}

    /** The pinned service public key (decoded once). */
    public static PublicKey pinnedKey() {
        PublicKey k = pinnedKey;
        if (k == null) {
            synchronized (GrantVerifier.class) {
                if (pinnedKey == null) {
                    try {
                        byte[] der = Base64.getDecoder().decode(SPKI_B64);
                        pinnedKey = KeyFactory.getInstance("Ed25519")
                                .generatePublic(new X509EncodedKeySpec(der));
                    } catch (Exception e) {
                        throw new IllegalStateException("pinned espectral-id key failed to decode", e);
                    }
                }
                k = pinnedKey;
            }
        }
        return k;
    }

    /**
     * Verifies {@code jws} for {@code subjectUuid} at the current wall clock
     * against the pinned key. Returns the grant or {@code null} on any failure.
     */
    public static Grant verify(String jws, java.util.UUID subjectUuid) {
        if (subjectUuid == null) return null;
        return verify(jws, subject(subjectUuid), pinnedKey(),
                System.currentTimeMillis() / 1000L);
    }

    /**
     * Test/verifier seam: explicit subject, key and clock. {@code subject} must
     * already be the 32-hex lowercase form.
     */
    public static Grant verify(String jws, String subject, PublicKey key, long nowSeconds) {
        if (jws == null || jws.isEmpty() || subject == null || key == null) return null;
        String[] parts = jws.split("\\.", -1);
        if (parts.length != 3) return null;
        if (parts[0].isEmpty() || parts[1].isEmpty() || parts[2].isEmpty()) return null;
        try {
            JsonObject header = parseJson(Base64.getUrlDecoder().decode(parts[0]));
            if (header == null) return null;
            if (!REQUIRED_ALG.equals(str(header, "alg"))) return null;
            if (!KEY_ID.equals(str(header, "kid"))) return null;
            if (!REQUIRED_TYP.equals(str(header, "typ"))) return null;

            byte[] signature = Base64.getUrlDecoder().decode(parts[2]);
            Signature verifier = Signature.getInstance("Ed25519");
            verifier.initVerify(key);
            verifier.update((parts[0] + "." + parts[1]).getBytes(StandardCharsets.US_ASCII));
            if (!verifier.verify(signature)) return null;

            JsonObject payload = parseJson(Base64.getUrlDecoder().decode(parts[1]));
            if (payload == null) return null;
            if (num(payload, "v", -1) != 1) return null;
            String iss = str(payload, "iss");
            if (!REQUIRED_ISS.equals(iss) && !LEGACY_ISS.equals(iss)) return null;
            if (!REQUIRED_AUD.equals(str(payload, "aud"))) return null;
            if (num(payload, "exp", Long.MIN_VALUE) < nowSeconds) return null;
            if (num(payload, "iat", Long.MAX_VALUE) > nowSeconds + MAX_IAT_SKEW_S) return null;

            String sub = str(payload, "sub");
            if (!subject.equals(sub)) return null;
            if (!isHex32(sub)) return null;

            Set<String> flags = new LinkedHashSet<>(2);
            JsonElement flagsEl = payload.get("flags");
            if (flagsEl != null && flagsEl.isJsonArray()) {
                for (JsonElement f : flagsEl.getAsJsonArray()) {
                    if (f.isJsonPrimitive() && f.getAsJsonPrimitive().isString()) {
                        String flag = f.getAsString().toLowerCase(Locale.ROOT);
                        if (flag.equals("supporter") || flag.equals("mvp")) flags.add(flag);
                    }
                }
            }
            return new Grant(sub, Set.copyOf(flags), num(payload, "exp", 0));
        } catch (Exception e) {
            return null;
        }
    }

    /** 32-hex lowercase subject form of a UUID. */
    public static String subject(java.util.UUID uuid) {
        return uuid.toString().replace("-", "");
    }

    private static JsonObject parseJson(byte[] bytes) {
        try {
            JsonElement el = JsonParser.parseString(new String(bytes, StandardCharsets.UTF_8));
            return el != null && el.isJsonObject() ? el.getAsJsonObject() : null;
        } catch (Exception e) {
            return null;
        }
    }

    private static String str(JsonObject obj, String key) {
        JsonElement el = obj.get(key);
        return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isString()
                ? el.getAsString() : null;
    }

    private static long num(JsonObject obj, String key, long fallback) {
        JsonElement el = obj.get(key);
        try {
            return el != null && el.isJsonPrimitive() && el.getAsJsonPrimitive().isNumber()
                    ? el.getAsLong() : fallback;
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    private static boolean isHex32(String s) {
        if (s == null || s.length() != 32) return false;
        for (int i = 0; i < 32; i++) {
            char c = s.charAt(i);
            if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) return false;
        }
        return true;
    }
}
