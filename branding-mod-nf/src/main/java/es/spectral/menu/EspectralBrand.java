package es.spectral.menu;

import net.neoforged.fml.ModList;

/**
 * Builds the {@code minecraft:brand} string for the NeoForge companion:
 * {@code espectral:neoforge[:<version>]}, e.g. {@code espectral:neoforge:1.3.14}.
 * Mirrors the Fabric-side builder (separate build, same contract).
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

    /** Live brand for this build. */
    public static String brand() {
        return buildBrand("neoforge", modVersion());
    }

    /**
     * Mod container version ({@code neoforge.mods.toml}, expanded from the
     * launcher version at build time). Falls back to the jar manifest, then
     * null (the version segment is omitted, never faked).
     */
    public static String modVersion() {
        try {
            String v = ModList.get().getModContainerById(EspectralBrandMod.MOD_ID)
                    .map(c -> c.getModInfo().getVersion().toString())
                    .orElse(null);
            if (v != null && !v.isBlank()) return v.trim();
        } catch (Throwable ignored) {
            // Loader not booted (dev/test) — try the jar manifest below.
        }
        String impl = EspectralBrand.class.getPackage().getImplementationVersion();
        return (impl == null || impl.isBlank()) ? null : impl.trim();
    }
}
