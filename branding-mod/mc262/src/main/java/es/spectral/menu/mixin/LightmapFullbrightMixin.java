package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.renderer.LightmapRenderStateExtractor;
import net.minecraft.client.renderer.state.LightmapRenderState;

import es.spectral.menu.FullbrightEngine;

/**
 * 26.2 fullbright: after vanilla fills the lightmap render state, a full
 * night-vision factor is written when the lightmap is being recomputed this
 * frame ({@code needsUpdate}, set every tick). See {@link FullbrightEngine}.
 */
@Mixin(LightmapRenderStateExtractor.class)
public abstract class LightmapFullbrightMixin {

    @Inject(
            method = "extract(Lnet/minecraft/client/renderer/state/LightmapRenderState;F)V",
            at = @At("TAIL")
    )
    private void espectral$fullbright(LightmapRenderState renderState, float partialTicks, CallbackInfo ci) {
        if (renderState.needsUpdate && FullbrightEngine.isEnabled()) {
            renderState.nightVisionEffectIntensity = FullbrightEngine.NIGHT_VISION;
        }
    }
}
