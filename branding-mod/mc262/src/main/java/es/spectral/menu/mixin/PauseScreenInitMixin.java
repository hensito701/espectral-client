package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

import es.spectral.menu.Compat;
import es.spectral.menu.ui.SuiteConfirmScreen;
import es.spectral.menu.ui.SuiteScreen;
import es.spectral.menu.ui.SuiteTheme;

/**
 * 26.2 only: appends the Espectral Client and Apoyar actions to the Esc
 * screen as one right-aligned row at the top-right corner (y = 6, widths
 * 120 + 100, gap 4), the same row as the title screen. Append-only RETURN
 * injection via {@code addRenderableWidget}: vanilla pause buttons and their
 * behaviour are never removed or reordered.
 */
@Mixin(PauseScreen.class)
public abstract class PauseScreenInitMixin extends Screen {

    private PauseScreenInitMixin(Component title) {
        super(title);
    }

    @Inject(method = "init()V", at = @At("RETURN"))
    private void espectral$appendSuiteActions(CallbackInfo ci) {
        Minecraft minecraft = Minecraft.getInstance();
        int rowX = SuiteTheme.actionRowX(this.width);
        int y = SuiteTheme.ACTION_Y;
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.pause.client").withStyle(ChatFormatting.GOLD),
                b -> Compat.open(minecraft, new SuiteScreen(this)))
                .bounds(rowX, y, SuiteTheme.ACTION_CLIENT_W, 20).build());
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.pause.support").withStyle(ChatFormatting.GOLD),
                b -> Compat.open(minecraft, SuiteConfirmScreen.support(this)))
                .bounds(rowX + SuiteTheme.ACTION_CLIENT_W + SuiteTheme.GAP, y,
                        SuiteTheme.ACTION_SUPPORT_W, 20).build());
    }
}
