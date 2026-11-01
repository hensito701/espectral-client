package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;

import es.spectral.menu.ui.MenuBall;

/**
 * 26.2/26.3 menu ball paint hook: screens draw through
 * {@code extractRenderState(GuiGraphicsExtractor, int, int, float)}, so tailing the base method
 * draws the ball over the screen's widgets. {@link MenuBall#paint} only acts
 * for the current out-of-world screen while the ball is loose.
 */
@Mixin(Screen.class)
public abstract class MenuBallPaintMixin {

    @Inject(
            method = "extractRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$paintMenuBall(GuiGraphicsExtractor gfx,
            int mouseX, int mouseY, float delta, CallbackInfo ci) {
        MenuBall.get().paint(gfx, (Screen) (Object) this);
    }
}
