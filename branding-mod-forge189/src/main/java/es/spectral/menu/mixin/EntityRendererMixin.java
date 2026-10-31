package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.renderer.EntityRenderer;

import es.spectral.menu.agent.Capture;

/**
 * End-of-frame hook for the {@code screenshot} tool: the tail of
 * {@code EntityRenderer.updateCameraAndRender} runs after the GUI pass and
 * before {@code Display.update}, so the readback sees the finished frame
 * (vanilla's own F2 path reads the framebuffer here too).
 */
@Mixin(EntityRenderer.class)
public abstract class EntityRendererMixin {

    @Inject(method = "updateCameraAndRender", at = @At("TAIL"))
    private void espectral$captureFrame(float partialTicks, long nanoTime, CallbackInfo ci) {
        Capture.onFrameEnd();
    }
}
