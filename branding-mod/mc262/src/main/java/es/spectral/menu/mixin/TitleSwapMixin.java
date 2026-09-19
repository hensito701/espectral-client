package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

import net.minecraft.client.gui.screens.Screen;

/**
 * 26.2 only: swaps the vanilla title screen for the Espectral main menu.
 * Screen-setting funnels through {@code Gui.setScreen}, so an
 * {@code argsOnly} HEAD tweak of its parameter redirects every
 * {@code TitleScreen} open (boot, disconnect, Gui back-stack) to
 * {@link es.spectral.menu.ui.EspectralTitleScreen}; every other screen
 * passes through untouched.
 */
@Mixin(net.minecraft.client.gui.Gui.class)
public abstract class TitleSwapMixin {

    @ModifyVariable(
            method = "setScreen(Lnet/minecraft/client/gui/screens/Screen;)V",
            at = @At("HEAD"),
            argsOnly = true
    )
    private static Screen espectral$swap(Screen s) {
        return (s instanceof net.minecraft.client.gui.screens.TitleScreen)
                ? new es.spectral.menu.ui.EspectralTitleScreen() : s;
    }
}
