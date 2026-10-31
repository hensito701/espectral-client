package es.spectral.menu.mixin;

import java.nio.IntBuffer;

import org.lwjgl.system.MemoryStack;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import com.mojang.blaze3d.audio.DeviceList;
import com.mojang.blaze3d.audio.Library;

import es.spectral.menu.agent.AgentAudio;

/**
 * Agent launches run the sound engine on an OpenAL loopback device so the
 * game's mix can be recorded (see {@link AgentAudio}). Outside agent mode every
 * hook returns after one check and vanilla opens the system device as usual.
 */
@Mixin(Library.class)
public abstract class AgentAudioLibraryMixin {

    @Inject(method = "openDeviceOrFallback", at = @At("HEAD"), cancellable = true)
    private static void espectral$agentLoopback(String requested, String fallback,
            CallbackInfoReturnable<Long> cir) {
        long loopback = AgentAudio.openLoopback();
        if (loopback != 0L) {
            cir.setReturnValue(loopback);
        }
    }

    @Inject(method = "createAttributes", at = @At("RETURN"), cancellable = true)
    private void espectral$agentLoopbackFormat(MemoryStack stack, boolean hrtf,
            CallbackInfoReturnable<IntBuffer> cir) {
        if (AgentAudio.loopbackOpen()) {
            cir.setReturnValue(AgentAudio.attributes(stack, cir.getReturnValue()));
        }
    }

    @Inject(method = "init", at = @At("TAIL"))
    private void espectral$agentStartPump(String device, DeviceList devices, boolean hrtf,
            CallbackInfo ci) {
        AgentAudio.startPump();
    }

    @Inject(method = "cleanup", at = @At("HEAD"))
    private void espectral$agentStopPump(CallbackInfo ci) {
        AgentAudio.stopPump();
    }
}
