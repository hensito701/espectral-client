package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import com.mojang.blaze3d.platform.DisplayData;

import net.minecraft.client.Minecraft;

/**
 * An agent launch with an explicit window size ({@code --width/--height} plus
 * {@code -Despectral.agent.window}) gets exactly that size. The vanilla
 * constructor replaces the size with the player's
 * {@code overrideWidth/overrideHeight} options whenever both are set, so a
 * leftover override (1920x1080 in a test instance) silently turned a requested
 * 1080x1920 portrait window into a landscape one. The player's options file is
 * left untouched; plain launches keep vanilla behaviour.
 */
@Mixin(Minecraft.class)
public abstract class AgentWindowSizeMixin {

    @Redirect(method = "<init>", at = @At(value = "INVOKE",
            target = "Lcom/mojang/blaze3d/platform/DisplayData;withSize(II)Lcom/mojang/blaze3d/platform/DisplayData;"))
    private DisplayData espectral$agentWindowSize(DisplayData requested, int overrideWidth, int overrideHeight) {
        if (es.spectral.menu.agent.AgentRuntime.explicitWindow()) {
            return requested;
        }
        return requested.withSize(overrideWidth, overrideHeight);
    }
}
