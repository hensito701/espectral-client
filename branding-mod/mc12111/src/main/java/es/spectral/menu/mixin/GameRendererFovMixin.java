package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import es.spectral.menu.ZoomEngine;
import net.minecraft.client.Camera;
import net.minecraft.client.renderer.GameRenderer;

/**
 * 1.21.11: vanilla computes the camera FOV in
 * {@code GameRenderer#getFov(Camera, float, boolean)} (private). Scale its
 * return value by the ZoomEngine factor while the native zoom is engaged.
 */
@Mixin(GameRenderer.class)
public abstract class GameRendererFovMixin {

    @Inject(
            method = "getFov(Lnet/minecraft/client/Camera;FZ)F",
            at = @At("RETURN"),
            cancellable = true
    )
    private void espectral$zoomFov(Camera camera, float partialTick, boolean useFovSetting,
            CallbackInfoReturnable<Float> cir) {
        float scale = ZoomEngine.getInstance().fovScale();
        if (scale < 0.999f) {
            cir.setReturnValue(cir.getReturnValueF() * scale);
        }
    }
}
