package es.spectral.menu.mixin.agent;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.Minecraft;
import net.minecraft.client.settings.GameSettings;

import es.spectral.menu.agent.AgentRuntime;

/**
 * Window size for agent launches on 1.8.9: an explicit agent window
 * ({@code --width/--height} plus {@code -Despectral.agent.window}) wins over
 * the player's {@code overrideWidth/overrideHeight} options, which
 * {@code startGame} would otherwise apply. The options file is not changed.
 */
@Mixin(Minecraft.class)
public abstract class AgentWindowMixin {

    @Redirect(method = "startGame", at = @At(value = "FIELD",
            target = "Lnet/minecraft/client/settings/GameSettings;overrideWidth:I"))
    private int espectral$agentWindowSize(GameSettings settings) {
        return AgentRuntime.explicitWindow() ? 0 : settings.overrideWidth;
    }
}
