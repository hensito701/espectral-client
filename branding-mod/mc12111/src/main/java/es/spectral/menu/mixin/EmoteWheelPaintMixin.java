package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;

import es.spectral.menu.ui.EmoteWheelScreen;

/**
 * 1.21.11 emote wheel paint hook: screens draw through
 * {@code render(GuiGraphics, int, int, float)}, and the shared
 * {@link EmoteWheelScreen} keeps its drawing lane-neutral, so this mixin
 * tails the base method and forwards to {@link EmoteWheelScreen#paint}.
 * Every other screen is untouched.
 */
@Mixin(Screen.class)
public abstract class EmoteWheelPaintMixin {

    @Inject(
            method = "render(Lnet/minecraft/client/gui/GuiGraphics;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$paintEmoteWheel(GuiGraphics gfx,
            int mouseX, int mouseY, float delta, CallbackInfo ci) {
        if ((Object) this instanceof EmoteWheelScreen wheel) {
            Minecraft minecraft = Minecraft.getInstance();
            wheel.paint(gfx, wheel.width, wheel.height, mouseX, mouseY,
                    minecraft != null ? minecraft.font : null);
        }
    }
}
