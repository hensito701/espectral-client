package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.gui.components.AbstractContainerWidget;
import net.minecraft.client.gui.screens.multiplayer.ServerSelectionList;
import net.minecraft.client.input.MouseButtonEvent;

import es.spectral.menu.ruta.RutaRouteIcon;

/**
 * Sends clicks on the Ruta Espectral button column to the toggle before vanilla
 * sees them.
 *
 * <p>Hooked at HEAD of {@code AbstractContainerWidget.mouseClicked}. Neither
 * {@code AbstractSelectionList} nor {@code ServerSelectionList} declares
 * {@code mouseClicked}, so this base declaration is what a list dispatches to. The
 * {@code instanceof} guard keeps every other container widget on vanilla's path.
 * A consumed click cancels the call, so the row does not join or select.
 */
@Mixin(AbstractContainerWidget.class)
public abstract class ServerListRutaClickMixin {

    @Inject(
            method = "mouseClicked(Lnet/minecraft/client/input/MouseButtonEvent;Z)Z",
            at = @At("HEAD"),
            cancellable = true
    )
    private void espectral$clickRutaColumn(MouseButtonEvent event, boolean doubleClick,
            CallbackInfoReturnable<Boolean> cir) {
        if ((Object) this instanceof ServerSelectionList list && event != null
                && RutaRouteIcon.clickColumn(list, event.x(), event.y(), event.button())) {
            cir.setReturnValue(true);
        }
    }
}
