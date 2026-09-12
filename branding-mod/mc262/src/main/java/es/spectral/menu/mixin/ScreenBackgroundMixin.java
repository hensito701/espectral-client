package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.TitleScreen;

import es.spectral.menu.ui.SuiteConfirmScreen;
import es.spectral.menu.ui.SuitePainter;
import es.spectral.menu.ui.SuiteScreen;

/**
 * 26.2 only: paints the dark-glass Suite backdrop behind widgets but over the
 * vanilla panorama by injecting at the TAIL of {@code extractBackground}
 * (both the base implementation and the {@code TitleScreen} override declare
 * it with the descriptor below, verified by javap against 26.2). The
 * instanceof guards route each screen to its matching painter method; every
 * other screen keeps the vanilla background untouched.
 *
 * <p>Extends {@code Object}: a mixin may not name its own target
 * ({@code Screen}) as a superclass, so the instance is reached with a cast
 * and the font comes from the Minecraft instance.
 */
@Mixin({Screen.class, TitleScreen.class})
public abstract class ScreenBackgroundMixin {

    @Inject(
            method = "extractBackground(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$paintSuiteBackdrop(GuiGraphicsExtractor gfx,
            int mouseX, int mouseY, float partialTick, CallbackInfo ci) {
        Screen screen = (Screen) (Object) this;
        Minecraft minecraft = Minecraft.getInstance();
        Font font = minecraft != null ? minecraft.font : null;
        if (font == null) return;
        Object self = this;
        if (self instanceof SuiteScreen suite) {
            SuitePainter.paintSuite(gfx, suite, screen.width, screen.height, font);
        } else if (self instanceof SuiteConfirmScreen confirm) {
            SuitePainter.paintConfirm(gfx, confirm, screen.width, screen.height, font);
        } else if (self instanceof TitleScreen) {
            SuitePainter.paintTitle(gfx, screen.width, screen.height, font);
        }
    }
}
