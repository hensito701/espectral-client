package es.spectral.menu.mixin.feature;

import java.util.Arrays;

import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.renderer.EntityRenderer;
import net.minecraft.client.renderer.texture.DynamicTexture;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.ZoomEngine;

/**
 * Fullbright and zoom hooks in {@code EntityRenderer}:
 * <ul>
 *   <li>{@code updateLightmap}: when the lightmap was recomputed this frame
 *       and {@code fullbright} is on, every entry is replaced with full white
 *       (the night-vision look); vanilla recomputes it every tick, so turning
 *       the feature off restores the vanilla lightmap within a tick.</li>
 *   <li>{@code getFOVModifier}: the world FOV (not the hand's) is scaled by
 *       {@link ZoomEngine#fovScale()}.</li>
 * </ul>
 */
@Mixin(EntityRenderer.class)
public abstract class EntityRendererFeatureMixin {

    @Shadow private boolean lightmapUpdateNeeded;
    @Shadow @Final private int[] lightmapColors;
    @Shadow @Final private DynamicTexture lightmapTexture;

    @Unique private boolean espectral$lightmapRecomputed;

    @Inject(method = "updateLightmap", at = @At("HEAD"))
    private void espectral$lightmapHead(float partialTicks, CallbackInfo ci) {
        espectral$lightmapRecomputed = lightmapUpdateNeeded;
    }

    @Inject(method = "updateLightmap", at = @At("TAIL"))
    private void espectral$fullbright(float partialTicks, CallbackInfo ci) {
        if (espectral$lightmapRecomputed && ClientConfig.getInstance().isFeatureEnabled("fullbright")) {
            Arrays.fill(lightmapColors, 0xFFFFFFFF);
            lightmapTexture.updateDynamicTexture();
        }
    }

    @Inject(method = "getFOVModifier", at = @At("RETURN"), cancellable = true)
    private void espectral$zoom(float partialTicks, boolean useFOVSetting, CallbackInfoReturnable<Float> cir) {
        if (useFOVSetting) {
            float scale = ZoomEngine.fovScale();
            if (scale != 1.0f) cir.setReturnValue(cir.getReturnValue() * scale);
        }
    }
}
