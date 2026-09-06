package es.spectral.menu;

import net.fabricmc.loader.api.FabricLoader;

/**
 * Builds the {@code minecraft:brand} string this client reports on the
 * configuration handshake: {@code espectral:<loader>[:<version>]}, e.g.
 * {@code espectral:fabric:1.3.14}. The loader token comes from the build
 * flavor ({@link Compat#brandLoader()}); the version is the mod container
 * version, which tracks the launcher release (see the root build.gradle).
 *
 * <p>Server-side matching is a case-insensitive {@code contains("espectral")};
 * loader/version segments are display-only. The brand is self-reported, so it
 * is identification for display/ad-suppression, never authentication.
 */
public final class EspectralBrand {

    private EspectralBrand() {}

    /**
     * Pure builder — no loader state, safe to unit-test.
     * A blank/null version is omitted (no trailing colon).
     */
    public static String buildBrand(String loader, String version) {
        StringBuilder sb = new StringBuilder("espectral:").append(loader);
        if (version != null && !version.isBlank()) {
            sb.append(':').append(version.trim());
        }
        return sb.toString();
    }

    /** Live brand for this build flavor. */
    public static String brand() {
        return buildBrand(Compat.brandLoader(), modVersion());
    }

    /**
     * Mod container version ({@code fabric.mod.json}, expanded from the
     * launcher version at build time). Falls back to the jar manifest, then
     * null (the version segment is omitted, never faked).
     */
    public static String modVersion() {
        try {
            String v = FabricLoader.getInstance().getModContainer("espectral-menu")
                    .map(c -> c.getMetadata().getVersion().getFriendlyString())
                    .orElse(null);
            if (v != null && !v.isBlank()) return v.trim();
        } catch (Throwable ignored) {
            // Loader not booted (dev/test) — try the jar manifest below.
        }
        String impl = EspectralBrand.class.getPackage().getImplementationVersion();
        return (impl == null || impl.isBlank()) ? null : impl.trim();
    }
}
