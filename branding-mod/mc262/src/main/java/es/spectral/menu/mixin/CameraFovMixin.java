package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import es.spectral.menu.ZoomEngine;
import net.minecraft.client.Camera;

/**
 * 26.2: FOV computation moved from GameRenderer to
 * {@code Camera#calculateFov(float)} (private; the result feeds
 * {@code Camera#getFov()}, which GameRenderer reads during extraction).
 * Scale its return value by the ZoomEngine factor while the native zoom is
 * engaged. HUD FOV is computed separately and intentionally untouched.
 */
@Mixin(Camera.class)
public abstract class CameraFovMixin {

    @Inject(
            method = "calculateFov(F)F",
            at = @At("RETURN"),
            cancellable = true
    )
    private void espectral$zoomFov(float partialTick, CallbackInfoReturnable<Float> cir) {
        float scale = ZoomEngine.getInstance().fovScale();
        if (scale < 0.999f) {
            cir.setReturnValue(cir.getReturnValueF() * scale);
        }
    }
}
