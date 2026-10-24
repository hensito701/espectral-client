package es.spectral.menu.cosmetics.render;

import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.resources.Identifier;

/**
 * Bundled cosmetic textures under
 * {@code assets/espectral-menu/textures/cosmetics/} plus the vanilla render
 * types used to draw them. Everything here is vanilla {@link RenderTypes} (no
 * custom pipelines) so Sodium/Iris paths stay untouched; the files load lazily
 * through {@code TextureManager.getTexture} on first bind.
 */
public final class CosmeticTextures {

    private static final String NS = "espectral-menu";

    public static final Identifier MANTLE = tex("manto_espectral");
    public static final Identifier REVENANT = tex("capucha_errante");
    public static final Identifier REVENANT_HOOD = tex("capucha_errante_hood");
    public static final Identifier TATTERS = tex("jirones_ceniza");
    public static final Identifier WINGS = tex("alas_eco");
    public static final Identifier SCARF = tex("bufanda_llama_alma");
    public static final Identifier SCARF_TAIL = tex("bufanda_llama_alma_tail");
    public static final Identifier EARS = tex("orejas_vulpinas");
    public static final Identifier CAT_EARS = tex("orejas_felinas");

    public static final Identifier ELYTRA_VIDRIERA = tex("elitros_vidriera");
    /** Animated skins: {@link #FRAME_COUNT} files per skin. */
    public static final int FRAME_COUNT = 8;
    /** Ticks per animation frame (5 fps — visible but cheap). */
    public static final int FRAME_TICKS = 4;

    private static Identifier tex(String name) {
        return Identifier.fromNamespaceAndPath(NS, "textures/cosmetics/" + name + ".png");
    }

    private CosmeticTextures() {}

    /** Frame id for an animated elytra skin ({@code <base>_N.png}). */
    public static Identifier elytraFrame(String base, int frame) {
        return tex(base + "_" + frame);
    }

    /** Translucent, double-sided (cloth, wings). */
    public static RenderType cloth(Identifier id) {
        return RenderTypes.entityTranslucent(id, false);
    }

    /** Cutout boxes (ears, hood, scarf band, wrap bands). */
    public static RenderType cutout(Identifier id) {
        return RenderTypes.entityCutout(id);
    }

    /** Animation frame for an animated elytra skin at the given client tick. */
    public static int frameAt(int tick, boolean reducedMotion) {
        if (reducedMotion) return 0;
        return Math.floorMod(tick / FRAME_TICKS, FRAME_COUNT);
    }
}
