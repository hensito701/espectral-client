package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.ModifyArg;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractSelectionList;
import net.minecraft.client.gui.components.AbstractWidget;

import es.spectral.menu.ui.MenuSkin;

/**
 * Out-of-world menus: selection lists (servers, worlds, option rows) sit on
 * a dark glass band between accent hairlines instead of the dirt list
 * texture, and the selected row is outlined in the accent over translucent
 * glass rather than opaque black. The selection hook rewrites the colour of
 * the method's two fills (outline, then inner) because the row type is
 * protected and cannot appear in a handler signature.
 */
@Mixin(AbstractSelectionList.class)
public abstract class ListSkinMixin {

    @Inject(method = "extractListBackground(Lnet/minecraft/client/gui/GuiGraphicsExtractor;)V", at = @At("HEAD"), cancellable = true)
    private void espectral$glassList(GuiGraphicsExtractor gfx, CallbackInfo ci) {
        if (MenuSkin.skinWidgets()) {
            AbstractWidget list = (AbstractWidget) (Object) this;
            MenuSkin.paintListGlass(gfx, list.getX(), list.getY(), list.getRight(), list.getBottom());
            ci.cancel();
        }
    }

    @Inject(method = "extractListSeparators(Lnet/minecraft/client/gui/GuiGraphicsExtractor;)V", at = @At("HEAD"), cancellable = true)
    private void espectral$accentEdges(GuiGraphicsExtractor gfx, CallbackInfo ci) {
        if (MenuSkin.skinWidgets()) {
            AbstractWidget list = (AbstractWidget) (Object) this;
            MenuSkin.paintListEdges(gfx, list.getX(), list.getY(), list.getRight(), list.getBottom());
            ci.cancel();
        }
    }

    @ModifyArg(
            method = "extractSelection(Lnet/minecraft/client/gui/GuiGraphicsExtractor;Lnet/minecraft/client/gui/components/AbstractSelectionList$Entry;I)V",
            at = @At(value = "INVOKE", target = "Lnet/minecraft/client/gui/GuiGraphicsExtractor;fill(IIIII)V", ordinal = 0),
            index = 4
    )
    private int espectral$selectionOutline(int color) {
        return MenuSkin.skinWidgets() ? MenuSkin.selectionOutline(color) : color;
    }

    @ModifyArg(
            method = "extractSelection(Lnet/minecraft/client/gui/GuiGraphicsExtractor;Lnet/minecraft/client/gui/components/AbstractSelectionList$Entry;I)V",
            at = @At(value = "INVOKE", target = "Lnet/minecraft/client/gui/GuiGraphicsExtractor;fill(IIIII)V", ordinal = 1),
            index = 4
    )
    private int espectral$selectionFill(int color) {
        return MenuSkin.skinWidgets() ? MenuSkin.SELECT_FILL : color;
    }
}
