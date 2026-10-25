package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyArg;

import net.minecraft.client.renderer.LightTexture;

import es.spectral.menu.FullbrightEngine;

/**
 * 1.21.11 fullbright: {@code updateLightTexture} writes the lightmap uniform
 * block with {@code Std140Builder.putFloat} — the fourth float (ordinal 3) is
 * the night-vision factor (ambient, sky factor, block factor, night vision,
 * …; checked against the 1.21.11 bytecode). It is raised to full while
 * fullbright is on. See {@link FullbrightEngine}.
 */
@Mixin(LightTexture.class)
public abstract class LightmapFullbrightMixin {

    @ModifyArg(
            method = "updateLightTexture(F)V",
            at = @At(value = "INVOKE",
                    target = "Lcom/mojang/blaze3d/buffers/Std140Builder;putFloat(F)Lcom/mojang/blaze3d/buffers/Std140Builder;",
                    ordinal = 3),
            index = 0
    )
    private float espectral$fullbright(float nightVision) {
        return FullbrightEngine.isEnabled() ? FullbrightEngine.NIGHT_VISION : nightVision;
    }
}
