package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;

import es.spectral.menu.ui.SuiteGear;

/**
 * 26.2 gear hook: tails {@code Screen#extractRenderState} and stamps the gear
 * sprite onto every Suite settings button (the buttons carry no label glyph).
 */
@Mixin(Screen.class)
public abstract class SuiteGearPaintMixin {

    @Inject(method = "extractRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V",
            at = @At("TAIL"))
    private void espectral$paintGear(GuiGraphicsExtractor gfx, int mouseX, int mouseY,
            float delta, CallbackInfo ci) {
        SuiteGear.paintFor(gfx, (Object) this);
    }
}
