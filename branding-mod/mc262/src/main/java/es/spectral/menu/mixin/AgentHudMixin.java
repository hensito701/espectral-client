package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;

/**
 * Draws the "AGENT PAUSED (F8)" chip over whatever screen is open (26.2/26.3
 * extraction pipeline). Returns on one flag check when agent mode is off or
 * when the agent is not paused, so normal play renders nothing extra.
 */
@Mixin(Screen.class)
public abstract class AgentHudMixin {

    @Inject(
            method = "extractRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$agentHud(GuiGraphicsExtractor gfx, int mouseX, int mouseY,
            float partialTick, CallbackInfo ci) {
        if (!es.spectral.menu.agent.AgentRuntime.enabled()) return;
        Minecraft minecraft = Minecraft.getInstance();
        es.spectral.menu.agent.AgentHud.render(gfx, minecraft != null ? minecraft.font : null);
    }
}
