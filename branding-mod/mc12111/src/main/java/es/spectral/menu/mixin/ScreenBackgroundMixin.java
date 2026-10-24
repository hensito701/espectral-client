package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;

import es.spectral.menu.ui.SuiteConfirmScreen;
import es.spectral.menu.ui.SuitePainter;
import es.spectral.menu.ui.SuiteScreen;
import es.spectral.menu.ui.SuiteSheet;

/**
 * 1.21.11 counterpart of 26.2's {@code ScreenBackgroundMixin}: paints the
 * dark-glass Suite sheet behind the widgets but over the vanilla background.
 * 1.21.11 declares {@code renderBackground(GuiGraphics, int, int, float)} on
 * {@code Screen}, so the TAIL of that method is the exact layer the vanilla
 * {@code render} pass draws widgets onto. The instanceof guards route each
 * screen to its painter; every other screen keeps the vanilla background.
 *
 * <p>{@code EspectralTitleScreen} overrides {@code renderBackground} with an
 * empty body and paints its own art through {@code TitleMenu}, so the title
 * band from 26.2 is intentionally not painted here.
 */
@Mixin(Screen.class)
public abstract class ScreenBackgroundMixin {

    @Inject(
            method = "renderBackground(Lnet/minecraft/client/gui/GuiGraphics;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$paintSuiteBackdrop(GuiGraphics gfx,
            int mouseX, int mouseY, float partialTick, CallbackInfo ci) {
        Screen screen = (Screen) (Object) this;
        Minecraft minecraft = Minecraft.getInstance();
        Font font = minecraft != null ? minecraft.font : null;
        if (font == null) return;
        if (screen instanceof SuiteScreen suite) {
            SuitePainter.paintSuite(gfx, suite, screen.width, screen.height, font);
        } else if (screen instanceof SuiteSheet sheet) {
            sheet.paintSheet(gfx, screen.width, screen.height, font);
        } else if (screen instanceof SuiteConfirmScreen confirm) {
            SuitePainter.paintConfirm(gfx, confirm, screen.width, screen.height, font);
        }
    }
}
