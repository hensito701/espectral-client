package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.GuiGraphics;

import es.spectral.menu.Compat;

/**
 * 1.21.11: vanilla renders the whole HUD in
 * {@code Gui#render(GuiGraphics, DeltaTracker)}. Draws the Espectral text
 * overlays (potion/coords/health/armor/fps-ping) on top via Compat.
 */
@Mixin(Gui.class)
public abstract class GuiHudMixin {

    @Inject(
            method = "render(Lnet/minecraft/client/gui/GuiGraphics;Lnet/minecraft/client/DeltaTracker;)V",
            at = @At("TAIL")
    )
    private void espectral$drawHudOverlays(GuiGraphics graphics, DeltaTracker deltaTracker, CallbackInfo ci) {
        Compat.drawHudOverlays(Minecraft.getInstance(), graphics);
    }
}
