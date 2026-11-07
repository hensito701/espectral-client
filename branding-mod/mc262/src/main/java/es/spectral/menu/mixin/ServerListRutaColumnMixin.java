package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractSelectionList;
import net.minecraft.client.gui.screens.multiplayer.ServerSelectionList;

import es.spectral.menu.ruta.RutaRouteIcon;

/**
 * Paints the Ruta Espectral button column beside the multiplayer server list.
 *
 * <p>26.x splits rendering into an extraction pass, so the hook is on
 * {@code AbstractSelectionList.extractWidgetRenderState} at TAIL. Records are
 * emitted in call order, so the buttons land after the rows and scrollbar that
 * vanilla submitted. The {@code instanceof} guard keeps every other selection
 * list untouched. 26.3 shares this source unchanged.
 *
 * <p>Geometry and drawing live in {@link RutaRouteIcon}.
 */
@Mixin(AbstractSelectionList.class)
public abstract class ServerListRutaColumnMixin {

    @Inject(
            method = "extractWidgetRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$extractRutaColumn(GuiGraphicsExtractor gfx, int mouseX, int mouseY,
            float delta, CallbackInfo ci) {
        if ((Object) this instanceof ServerSelectionList list) {
            Minecraft minecraft = Minecraft.getInstance();
            if (minecraft == null || minecraft.font == null) {
                return;
            }
            RutaRouteIcon.paintColumn(gfx, minecraft.font, list, mouseX, mouseY);
        }
    }
}
