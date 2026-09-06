package es.spectral.menu;

import net.neoforged.fml.common.Mod;

/**
 * Brand-only NeoForge companion: the full Espectral Menu stays Fabric-only,
 * this mod exists so NeoForge instances report {@code espectral:neoforge:<version>}
 * on the {@code minecraft:brand} handshake (see {@link EspectralBrand}).
 * Client and server safe — the brand mixin only applies on the client.
 */
@Mod(EspectralBrandMod.MOD_ID)
public final class EspectralBrandMod {

    public static final String MOD_ID = "espectral-brand";

    public EspectralBrandMod() {
        // Intentionally side-agnostic: no client classes touched here.
    }
}
