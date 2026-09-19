package es.spectral.menu.mixin;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

/**
 * Swaps the vanilla title screen for ours. 1.21.11 funnels screen-setting
 * through {@link Minecraft#setScreen}, so the swap happens there.
 */
@Mixin(Minecraft.class)
public class TitleSwapMixin {
    @ModifyVariable(method = "setScreen(Lnet/minecraft/client/gui/screens/Screen;)V",
            at = @At("HEAD"), argsOnly = true)
    private static Screen espectral$swap(Screen s) {
        return (s instanceof net.minecraft.client.gui.screens.TitleScreen)
                ? new es.spectral.menu.ui.EspectralTitleScreen()
                : s;
    }
}
