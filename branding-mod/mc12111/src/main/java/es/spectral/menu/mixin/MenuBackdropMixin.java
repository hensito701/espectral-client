package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;

import es.spectral.menu.ui.MenuSkin;

/**
 * Out-of-world menus show the Espectral menu art instead of vanilla's
 * panorama + blur + dirt overlay ({@code MenuSkin}). {@code Screen#renderBackground}
 * only paints the panorama when no level is loaded, so hooking the three
 * steps it calls keeps the in-world (transparent) path untouched. Screens
 * that override {@code renderBackground} entirely — our title screen, loading
 * screens — are unaffected.
 */
@Mixin(Screen.class)
public abstract class MenuBackdropMixin {

    @Inject(method = "renderPanorama(Lnet/minecraft/client/gui/GuiGraphics;F)V", at = @At("HEAD"), cancellable = true)
    private void espectral$menuArt(GuiGraphics gfx, float partialTick, CallbackInfo ci) {
        Screen screen = (Screen) (Object) this;
        MenuSkin.paintBackdrop(gfx, screen.width, screen.height);
        ci.cancel();
    }

    /** The art stays sharp, like on the title screen (blur would smear it). */
    @Inject(method = "renderBlurredBackground(Lnet/minecraft/client/gui/GuiGraphics;)V", at = @At("HEAD"), cancellable = true)
    private void espectral$noBlurOverArt(GuiGraphics gfx, CallbackInfo ci) {
        if (MenuSkin.consumeBackdrop()) {
            ci.cancel();
        }
    }

    @Inject(method = "renderMenuBackground(Lnet/minecraft/client/gui/GuiGraphics;IIII)V", at = @At("HEAD"), cancellable = true)
    private void espectral$menuDim(GuiGraphics gfx, int x, int y, int width, int height, CallbackInfo ci) {
        if (MenuSkin.outOfWorld()) {
            MenuSkin.paintMenuDim(gfx, x, y, width, height);
            ci.cancel();
        }
    }
}
