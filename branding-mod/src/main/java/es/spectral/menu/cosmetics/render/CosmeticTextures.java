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
 *
 * <p>Model items bind {@code <id>.png} through {@link #texture(String)};
 * elytra skins keep their per-frame files because entity textures cannot
 * animate through {@code .mcmeta}.
 */
public final class CosmeticTextures {

    public static final String NAMESPACE = "espectral-menu";

    public static final Identifier ELYTRA_VIDRIERA = tex("elitros_vidriera");
    /** Animated skins: {@link #FRAME_COUNT} files each, resolved once so the render path allocates nothing. */
    public static final Identifier[] ELYTRA_NEBULOSA = frames("elitros_nebulosa");
    public static final Identifier[] ELYTRA_CENIZA = frames("elitros_ceniza");
    /** Frames per animated skin. */
    public static final int FRAME_COUNT = 8;
    /** Ticks per animation frame (5 fps — visible but cheap). */
    public static final int FRAME_TICKS = 4;

    private CosmeticTextures() {}

    /** Bundled texture of a model item ({@code textures/cosmetics/<id>.png}). */
    public static Identifier texture(String id) {
        return tex(id);
    }

    private static Identifier tex(String name) {
        return Identifier.fromNamespaceAndPath(NAMESPACE, "textures/cosmetics/" + name + ".png");
    }

    private static Identifier[] frames(String base) {
        Identifier[] out = new Identifier[FRAME_COUNT];
        for (int i = 0; i < FRAME_COUNT; i++) out[i] = tex(base + "_" + i);
        return out;
    }

    /** Frame of an animated skin for a client tick. */
    public static Identifier animatedFrame(Identifier[] frames, int tick, boolean reducedMotion) {
        return frames[frameAt(tick, reducedMotion)];
    }

    /** Translucent, double-sided (wings). */
    public static RenderType cloth(Identifier id) {
        return RenderTypes.entityTranslucent(id, false);
    }

    /** Cutout boxes (bandanas, ears, collars). */
    public static RenderType cutout(Identifier id) {
        return RenderTypes.entityCutout(id);
    }

    /** Animation frame for an animated elytra skin at the given client tick. */
    public static int frameAt(int tick, boolean reducedMotion) {
        if (reducedMotion) return 0;
        return Math.floorMod(tick / FRAME_TICKS, FRAME_COUNT);
    }
}
