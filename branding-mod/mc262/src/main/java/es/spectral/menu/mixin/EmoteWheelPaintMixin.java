package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;

import es.spectral.menu.ui.EmoteWheelScreen;

/**
 * 26.2 emote wheel paint hook: screens draw through
 * {@code extractRenderState(GuiGraphicsExtractor, int, int, float)}, and the
 * shared {@link EmoteWheelScreen} keeps its drawing lane-neutral, so this
 * mixin tails the base method and forwards to
 * {@link EmoteWheelScreen#paint}. Mirrors {@code ScreenBackgroundMixin}'s
 * instanceof dispatch; every other screen is untouched.
 */
@Mixin(Screen.class)
public abstract class EmoteWheelPaintMixin {

    @Inject(
            method = "extractRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$paintEmoteWheel(GuiGraphicsExtractor gfx,
            int mouseX, int mouseY, float delta, CallbackInfo ci) {
        if ((Object) this instanceof EmoteWheelScreen wheel) {
            Minecraft minecraft = Minecraft.getInstance();
            wheel.paint(gfx, wheel.width, wheel.height, mouseX, mouseY,
                    minecraft != null ? minecraft.font : null);
        }
    }
}
