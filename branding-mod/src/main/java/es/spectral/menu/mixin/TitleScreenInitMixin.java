package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import es.spectral.menu.EspectralMenu;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.network.chat.Component;

/**
 * Appends the Espectral bottom action row to the title screen.
 *
 * <p>Append-only RETURN injection: the vanilla {@code init()} body runs first
 * and keeps its widgets (navigation, splash, panorama, icon row), so other
 * mods' injections into it stay valid too. Shared across both supported
 * Minecraft versions.
 */
@Mixin(TitleScreen.class)
public abstract class TitleScreenInitMixin extends Screen {

    private static final org.slf4j.Logger LOGGER = org.slf4j.LoggerFactory.getLogger("espectral-menu");

    private TitleScreenInitMixin(Component title) {
        super(title);
    }

    @Inject(method = "init()V", at = @At("RETURN"))
    private void espectralMenu$appendActions(CallbackInfo ci) {
        for (Button button : EspectralMenu.buttons(this)) {
            this.addRenderableWidget(button);
        }
        LOGGER.debug("Espectral menu: title-screen action row appended (2 buttons)");
    }
}
