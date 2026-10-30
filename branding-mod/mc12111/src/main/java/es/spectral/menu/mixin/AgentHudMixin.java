package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;

/**
 * 1.21.11 counterpart of the shared {@code AgentHudMixin}: this lane renders
 * screens through {@code Screen#render(GuiGraphics, int, int, float)}, so the
 * "AGENT PAUSED (F8)" chip is appended there.
 */
@Mixin(Screen.class)
public abstract class AgentHudMixin {

    @Inject(
            method = "render(Lnet/minecraft/client/gui/GuiGraphics;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$agentHud(GuiGraphics gfx, int mouseX, int mouseY,
            float partialTick, CallbackInfo ci) {
        if (!es.spectral.menu.agent.AgentRuntime.enabled()) return;
        Minecraft minecraft = Minecraft.getInstance();
        es.spectral.menu.agent.AgentHud.render(gfx, minecraft != null ? minecraft.font : null);
    }
}
