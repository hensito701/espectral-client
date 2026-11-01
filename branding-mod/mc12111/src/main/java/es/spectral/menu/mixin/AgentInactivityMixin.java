package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.Options;
import net.minecraft.client.renderer.GameRenderer;

/**
 * Agent launches never pause on lost focus (1.21.11 lane; 26.x has its own
 * {@code pauseIfInactive} hook). On 1.21.11 the check lives in
 * {@code GameRenderer.render}: an unfocused window with
 * {@code pauseOnLostFocus} opens the pause menu, which froze hidden agent
 * windows and every scene being recorded. The player's option is left
 * untouched; it only reads as off inside this check during agent launches.
 */
@Mixin(GameRenderer.class)
public abstract class AgentInactivityMixin {

    @Redirect(method = "render", at = @At(value = "FIELD",
            target = "Lnet/minecraft/client/Options;pauseOnLostFocus:Z"))
    private boolean espectral$agentNeverPause(Options options) {
        return !es.spectral.menu.agent.AgentRuntime.enabled() && options.pauseOnLostFocus;
    }
}
