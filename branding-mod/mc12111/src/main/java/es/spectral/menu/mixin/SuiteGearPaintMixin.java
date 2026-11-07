package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;

import es.spectral.menu.ui.SuiteGear;

/**
 * 1.21.11 gear hook: tails {@code Screen#render} and stamps the gear sprite
 * onto every Suite settings button (the buttons carry no label glyph at all).
 */
@Mixin(Screen.class)
public abstract class SuiteGearPaintMixin {

    @Inject(method = "render(Lnet/minecraft/client/gui/GuiGraphics;IIF)V", at = @At("TAIL"))
    private void espectral$paintGear(GuiGraphics gfx, int mouseX, int mouseY, float delta,
            CallbackInfo ci) {
        SuiteGear.paintFor(gfx, (Object) this);
    }
}
