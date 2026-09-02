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
 * Replaces the vanilla title-screen widget set with the Espectral menu.
 *
 * Uses a cancellable HEAD injection instead of {@code @Overwrite} so other
 * mods' injections into the vanilla {@code init()} body (e.g. ModMenu's
 * adjustRealmsHeight) keep a valid anchor; the vanilla body simply never runs
 * at runtime. Shared across both supported Minecraft versions.
 */
@Mixin(TitleScreen.class)
public abstract class TitleScreenInitMixin extends Screen {

    private static final org.slf4j.Logger LOGGER = org.slf4j.LoggerFactory.getLogger("espectral-menu");

    private TitleScreenInitMixin(Component title) {
        super(title);
    }

    @Inject(method = "init()V", at = @At("HEAD"), cancellable = true)
    private void espectralMenu$init(CallbackInfo ci) {
        // init() re-runs on window resize; clear like vanilla's init body
        // would, so the button set never stacks.
        this.clearWidgets();
        for (Button button : EspectralMenu.buttons(this)) {
            this.addRenderableWidget(button);
        }
        LOGGER.debug("Espectral menu: title-screen init replaced (4 vanilla-style buttons)");
        ci.cancel();
    }
}
