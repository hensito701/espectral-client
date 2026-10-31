package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import com.mojang.blaze3d.platform.FramerateLimitTracker;

/**
 * Agent launches are never fps-throttled for inactivity: an agent sends no
 * mouse/keyboard input for minutes while it records, and a hidden or unfocused
 * window would otherwise drop to 10–30 fps (AFK / iconified / menu limits) —
 * ruining live recordings and slowing every capture. The configured max fps
 * still applies.
 */
@Mixin(FramerateLimitTracker.class)
public abstract class AgentThrottleMixin {

    @Inject(method = "getThrottleReason", at = @At("HEAD"), cancellable = true)
    private void espectral$agentNoThrottle(
            CallbackInfoReturnable<FramerateLimitTracker.FramerateThrottleReason> cir) {
        if (es.spectral.menu.agent.AgentRuntime.enabled()) {
            cir.setReturnValue(FramerateLimitTracker.FramerateThrottleReason.NONE);
        }
    }
}
