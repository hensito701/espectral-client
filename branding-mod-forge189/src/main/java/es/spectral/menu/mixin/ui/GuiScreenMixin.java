package es.spectral.menu.mixin.ui;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiScreen;

import es.spectral.menu.ui.MenuSkin;

/** Out-of-world menus: the Espectral art replaces the dirt options background. */
@Mixin(GuiScreen.class)
public abstract class GuiScreenMixin {

    @Shadow public int width;
    @Shadow public int height;

    @Inject(method = "drawBackground", at = @At("HEAD"), cancellable = true)
    private void espectral$backdrop(int tint, CallbackInfo ci) {
        if (MenuSkin.skinWidgets()) {
            MenuSkin.paintBackdrop(width, height);
            ci.cancel();
        }
    }
}
