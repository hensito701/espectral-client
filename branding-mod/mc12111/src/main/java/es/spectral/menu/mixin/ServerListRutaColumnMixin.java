package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.AbstractSelectionList;
import net.minecraft.client.gui.screens.multiplayer.ServerSelectionList;

import es.spectral.menu.ruta.RutaRouteIcon;

/**
 * Paints the Ruta Espectral button column beside the multiplayer server list.
 *
 * <p>Hooked on the list's own render method rather than on each row, so the
 * column is drawn once per frame instead of once per row. The tail injection
 * runs after vanilla has drawn the rows and scrollbar, so the buttons sit on top.
 * The {@code instanceof} guard keeps every other selection list untouched. The
 * target is the shared base class because {@code renderWidget} is declared there
 * and not overridden by {@code ServerSelectionList}.
 *
 * <p>Geometry and drawing live in {@link RutaRouteIcon}.
 */
@Mixin(AbstractSelectionList.class)
public abstract class ServerListRutaColumnMixin {

    @Inject(
            method = "renderWidget(Lnet/minecraft/client/gui/GuiGraphics;IIF)V",
            at = @At("TAIL")
    )
    private void espectral$paintRutaColumn(GuiGraphics gfx, int mouseX, int mouseY,
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
