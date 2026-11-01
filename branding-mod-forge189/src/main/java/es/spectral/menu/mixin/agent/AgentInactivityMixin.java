package es.spectral.menu.mixin.agent;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.renderer.EntityRenderer;
import net.minecraft.client.settings.GameSettings;

import es.spectral.menu.agent.AgentRuntime;

/**
 * Agent launches never pause on lost focus (same rule as the Fabric mod). On
 * 1.8.9 the check is in {@code EntityRenderer.updateCameraAndRender}: an
 * inactive window with {@code pauseOnLostFocus} opens the pause menu after
 * 500 ms, so a hidden agent window kept reopening it. The player's option is
 * left untouched; it only reads as off inside this check during agent
 * launches.
 */
@Mixin(EntityRenderer.class)
public abstract class AgentInactivityMixin {

    @Redirect(method = "updateCameraAndRender", at = @At(value = "FIELD",
            target = "Lnet/minecraft/client/settings/GameSettings;pauseOnLostFocus:Z"))
    private boolean espectral$agentNeverPause(GameSettings settings) {
        return !AgentRuntime.enabled() && settings.pauseOnLostFocus;
    }
}
