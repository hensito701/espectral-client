package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.renderer.GlStateManager;
import net.minecraft.client.renderer.ItemRenderer;

import es.spectral.menu.ClientConfig;

/**
 * Low fire ({@code lowfire}): the first-person burning overlay is shifted
 * down in view space (push/translate before, pop after) so the flames block
 * less of the view. Off = vanilla-identical.
 */
@Mixin(ItemRenderer.class)
public abstract class ItemRendererMixin {

    /** Downward shift in overlay view units (same value as the Fabric mod). */
    @Unique private static final float FIRE_Y_OFFSET = -0.25f;
    @Unique private boolean espectral$lowered;

    @Inject(method = "renderFireInFirstPerson", at = @At("HEAD"))
    private void espectral$lowerFire(float partialTicks, CallbackInfo ci) {
        espectral$lowered = ClientConfig.getInstance().isFeatureEnabled("lowfire");
        if (espectral$lowered) {
            GlStateManager.pushMatrix();
            GlStateManager.translate(0f, FIRE_Y_OFFSET, 0f);
        }
    }

    @Inject(method = "renderFireInFirstPerson", at = @At("RETURN"))
    private void espectral$restoreFire(float partialTicks, CallbackInfo ci) {
        if (espectral$lowered) {
            GlStateManager.popMatrix();
            espectral$lowered = false;
        }
    }
}
