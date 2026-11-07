package es.spectral.menu.ruta;

import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.Locale;
import java.util.Set;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The claims this client reads out of the entitlement grant, for <em>UI
 * gating only</em>.
 *
 * <p>Java 8 ships no Ed25519, so the signature cannot be checked in game: the
 * payload is decoded and inspected, and the relay performs the real
 * verification against the pinned {@code eid1} key before it routes a single
 * byte. Nothing here is a security boundary — it only decides whether the
 * toggle is offered and whether a connection is attempted at all; a forged
 * grant buys the player a {@code ERR auth} from the relay, never a working
 * route.
 *
 * <p>Token: {@code b64url(header).b64url(payload).b64url(sig)}; payload
 * {@code {"v":1,"iss":…,"aud":"espectral-client","sub":"<uuid32>",
 * "flags":[…],"iat":s,"exp":s}}. The checked claims mirror
 * {@code GrantVerifier}: {@code sub} must equal the signed-in account,
 * {@code flags} must contain {@code mvp}, {@code exp} must be in the future.
 */
public final class RutaGrant {

    /** Flag the relay (and this client) require for Ruta. */
    public static final String FLAG_MVP = "mvp";

    private final String subject;
    private final Set<String> flags;
    private final long expiresAt;

    private RutaGrant(String subject, Set<String> flags, long expiresAt) {
        this.subject = subject;
        this.flags = flags;
        this.expiresAt = expiresAt;
    }

    /** Parsed claims, or {@code null} when the token is not a readable ESG grant. */
    public static RutaGrant parse(String jws) {
        if (jws == null) return null;
        String[] parts = jws.trim().split("\\.", -1);
        if (parts.length != 3 || parts[1].isEmpty()) return null;
        JsonObject claims;
        try {
            byte[] json = Base64.getUrlDecoder().decode(parts[1]);
            JsonElement parsed = new JsonParser().parse(new String(json, StandardCharsets.UTF_8));
            if (!parsed.isJsonObject()) return null;
            claims = parsed.getAsJsonObject();
        } catch (RuntimeException malformed) {
            return null;
        }
        // v must be 1, like GrantVerifier and the relay.
        if (!claims.has("v") || !claims.get("v").isJsonPrimitive()
                || claims.get("v").getAsInt() != 1) {
            return null;
        }
        String subject = claims.has("sub") && claims.get("sub").isJsonPrimitive()
                ? claims.get("sub").getAsString().toLowerCase(Locale.ROOT)
                : null;
        if (subject == null || !isUuid32(subject)) return null;
        long exp = claims.has("exp") && claims.get("exp").isJsonPrimitive()
                ? claims.get("exp").getAsLong()
                : -1L;
        if (exp <= 0L) return null;
        Set<String> flags = new LinkedHashSet<String>();
        if (claims.has("flags") && claims.get("flags").isJsonArray()) {
            JsonArray array = claims.getAsJsonArray("flags");
            for (int i = 0; i < array.size(); i++) {
                JsonElement flag = array.get(i);
                if (flag.isJsonPrimitive()) {
                    flags.add(flag.getAsString().toLowerCase(Locale.ROOT));
                }
            }
        }
        return new RutaGrant(subject, Collections.unmodifiableSet(flags), exp);
    }

    /** 32 lowercase hex digits, the form {@code espectral-id} mints. */
    public static boolean isUuid32(String value) {
        if (value == null || value.length() != 32) return false;
        for (int i = 0; i < 32; i++) {
            char c = value.charAt(i);
            boolean hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
            if (!hex) return false;
        }
        return true;
    }

    /** The account (32-hex uuid) this grant was minted for. */
    public String subject() {
        return this.subject;
    }

    /** Whether the grant carries {@code mvp}. */
    public boolean hasMvp() {
        return this.flags.contains(FLAG_MVP);
    }

    /** Expiry, epoch seconds. */
    public long expiresAt() {
        return this.expiresAt;
    }

    /** Whether the grant is still inside its validity window at {@code nowMillis}. */
    public boolean isValidAt(long nowMillis) {
        return this.expiresAt * 1000L > nowMillis;
    }

    /** True when the grant is this account's, carries {@code mvp} and has not expired. */
    public boolean allowsRuta(String accountUuid32, long nowMillis) {
        return this.subject.equalsIgnoreCase(accountUuid32)
                && hasMvp()
                && isValidAt(nowMillis);
    }

    @Override
    public String toString() {
        // Never render the token itself: it is a bearer credential.
        return "RutaGrant{" + this.subject + ", mvp=" + hasMvp() + ", exp=" + this.expiresAt + "}";
    }
}
