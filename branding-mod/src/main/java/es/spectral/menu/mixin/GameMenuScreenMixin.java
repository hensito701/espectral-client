package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import es.spectral.menu.Compat;
import es.spectral.menu.ui.SuiteScreen;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

/**
 * Adds an Espectral Client entry to the vanilla in-game pause menu.
 * Registered for 1.21.11 only; 26.2 uses {@link PauseScreenInitMixin}, which
 * inserts the buttons into the vanilla column instead.
 */
@Mixin(PauseScreen.class)
public abstract class GameMenuScreenMixin extends Screen {

    private GameMenuScreenMixin(Component title) {
        super(title);
    }

    @Inject(method = "init()V", at = @At("RETURN"))
    private void espectralMenu$appendPauseButton(CallbackInfo ci) {
        int x = (this.width - 204) / 2;
        int y = Math.max(4, this.height - 28);
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.pause.client").withStyle(ChatFormatting.GOLD),
                button -> Compat.open(Minecraft.getInstance(), new SuiteScreen(this)))
                .bounds(x, y, 204, 20).build());
    }
}
