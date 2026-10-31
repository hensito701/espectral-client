package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

/**
 * Agent launches never pause on lost focus: the agent drives a game the human
 * is not looking at (often a hidden window), and singleplayer pausing the
 * moment the window is unfocused would freeze the scene it is recording. The
 * player's {@code pauseOnLostFocus} option is left untouched.
 */
@Mixin(Minecraft.class)
public abstract class AgentInactivityMixin {

    @Inject(method = "pauseIfInactive", at = @At("HEAD"), cancellable = true)
    private void espectral$agentNeverPause(CallbackInfo ci) {
        if (es.spectral.menu.agent.AgentRuntime.enabled()) {
            ci.cancel();
        }
    }
}
