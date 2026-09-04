package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.Hud;

import es.spectral.menu.Compat;

/**
 * 26.2: the HUD extraction pipeline builds render state in
 * {@code Hud#extractRenderState(GuiGraphicsExtractor, DeltaTracker)} (there is
 * no {@code GuiGraphics} anymore). Submits the Espectral text overlays
 * (potion/coords/health/armor/fps-ping) on top via Compat.
 */
@Mixin(Hud.class)
public abstract class HudOverlayMixin {

    @Inject(
            method = "extractRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;"
                    + "Lnet/minecraft/client/DeltaTracker;)V",
            at = @At("TAIL")
    )
    private void espectral$drawHudOverlays(GuiGraphicsExtractor extractor, DeltaTracker deltaTracker,
            CallbackInfo ci) {
        Compat.drawHudOverlays(Minecraft.getInstance(), extractor);
    }
}
