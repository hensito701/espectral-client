package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

import es.spectral.menu.bench.BenchEngine;

/**
 * Bench hooks (26.2 lane only): tick TAIL drives the scene state machine,
 * {@code renderFrame} TAIL records the end-of-frame timestamp. Both delegate
 * to {@link BenchEngine}, which returns on a single flag check when bench
 * mode is off (no allocation, no I/O off the render path).
 */
@Mixin(Minecraft.class)
public abstract class BenchMixin {

    @Inject(method = "tick()V", at = @At("TAIL"))
    private void espectralBench$onTick(CallbackInfo ci) {
        BenchEngine.onTick((Minecraft) (Object) this);
    }

    @Inject(method = "renderFrame(Z)V", at = @At("TAIL"))
    private void espectralBench$onFrame(boolean tick, CallbackInfo ci) {
        BenchEngine.onFrameEnd();
    }
}
