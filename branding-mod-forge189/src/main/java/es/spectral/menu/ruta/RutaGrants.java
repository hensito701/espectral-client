package es.spectral.menu.ruta;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * The entitlement grant, read from the instance's
 * {@code config/espectral-cosmetics.json} (written by the launcher on every
 * launch, for every loader including Forge 1.8.9).
 *
 * <p>Read once and cached, refreshed only when the file's stamp changes: the
 * launcher rewrites that file when the grant is refreshed, so a long session
 * picks up a new grant without a restart. Read off the render thread only (the
 * row asks {@link #cached()}).
 *
 * <p>Java 8 has no Ed25519, so this class only decodes the payload
 * ({@link RutaGrant}); the relay verifies the signature before routing.
 */
public final class RutaGrants {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    /** Key holding the compact JWS in {@code espectral-cosmetics.json}. */
    static final String FILE = "espectral-cosmetics.json";

    private static volatile File configDir;
    private static volatile String cachedJws;
    private static volatile RutaGrant cachedGrant;
    private static volatile long lastStamp = -1L;
    private static volatile boolean missing;

    private RutaGrants() {}

    /** Points the reader at the game's config directory (set at mod init). */
    public static void setConfigDir(File dir) {
        configDir = dir;
        synchronized (RutaGrants.class) {
            lastStamp = -1L;
            missing = false;
        }
    }

    /** The cosmetics file for the current config dir. */
    public static Path file() {
        File dir = configDir;
        return dir == null ? null : new File(dir, FILE).toPath();
    }

    /**
     * The cached grant, or {@code null} when there is none. Never touches the
     * disk: safe on the render thread.
     */
    public static RutaGrant cached() {
        return cachedGrant;
    }

    /** The cached token verbatim, or {@code null}. Never log this value. */
    public static String cachedJws() {
        return cachedJws;
    }

    /**
     * Re-reads the file when its stamp changed. Called from the connect/ping
     * threads and from a background warm-up, never from the render thread.
     */
    public static synchronized void refresh() {
        Path path = file();
        if (path == null) return;
        long stamp;
        try {
            stamp = Files.exists(path) ? Files.getLastModifiedTime(path).toMillis() : -1L;
        } catch (IOException unreadable) {
            stamp = -1L;
        }
        if (stamp == lastStamp && (stamp >= 0L || missing)) return;
        lastStamp = stamp;
        if (stamp < 0L) {
            cachedJws = null;
            cachedGrant = null;
            missing = true;
            return;
        }
        missing = false;
        String jws;
        try {
            JsonElement parsed = new JsonParser().parse(
                    new String(Files.readAllBytes(path), StandardCharsets.UTF_8));
            if (!parsed.isJsonObject()) return;
            JsonObject root = parsed.getAsJsonObject();
            if (!root.has("grant") || !root.get("grant").isJsonPrimitive()) {
                cachedJws = null;
                cachedGrant = null;
                return;
            }
            jws = root.get("grant").getAsString();
        } catch (IOException unreadable) {
            LOGGER.warn("[ruta] cannot read {}: {}", path, unreadable.toString());
            return;
        } catch (RuntimeException malformed) {
            LOGGER.warn("[ruta] malformed {}: {}", path, malformed.toString());
            return;
        }
        RutaGrant grant = RutaGrant.parse(jws);
        cachedJws = jws == null || jws.isEmpty() ? null : jws;
        cachedGrant = grant;
    }

    /** The signed-in account as 32 lowercase hex, or {@code null} when offline/unknown. */
    public static String accountUuid32() {
        try {
            net.minecraft.client.Minecraft mc = net.minecraft.client.Minecraft.getMinecraft();
            if (mc == null || mc.getSession() == null) return null;
            UUID id = mc.getSession().getProfile() == null
                    ? null : mc.getSession().getProfile().getId();
            if (id == null) {
                String raw = mc.getSession().getPlayerID();
                return raw == null ? null : raw.replace("-", "").toLowerCase(java.util.Locale.ROOT);
            }
            return id.toString().replace("-", "").toLowerCase(java.util.Locale.ROOT);
        } catch (RuntimeException unavailable) {
            return null;
        }
    }
}
