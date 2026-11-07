package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiMultiplayer;
import net.minecraft.client.gui.ServerListEntryNormal;
import net.minecraft.client.gui.ServerSelectionList;
import net.minecraft.client.multiplayer.ServerData;

import es.spectral.menu.ruta.RutaIcon;

/**
 * Takes clicks on the Ruta Espectral button column to the right of the multiplayer
 * server rows.
 *
 * <p>Why the screen and not the row: vanilla dispatches a click to a row only inside
 * the row's own x-range, and the button sits outside it, so the row never receives
 * the click. Here the screen resolves the row from the pointer's y, and cancels
 * {@code mouseClicked} when the button consumed the click so the list does not act.
 */
@Mixin(GuiMultiplayer.class)
public abstract class GuiMultiplayerRouteColumnMixin {

    @Shadow
    private ServerSelectionList serverListSelector;

    @Inject(method = "mouseClicked(III)V", at = @At("HEAD"), cancellable = true)
    private void espectral$routeColumnClick(int mouseX, int mouseY, int mouseButton, CallbackInfo ci) {
        ServerSelectionList list = this.serverListSelector;
        if (list == null || mouseY < list.top || mouseY >= list.bottom) return;

        // Mirrors GuiSlot's row layout: rows start at top + 4 - scroll + headerPadding.
        int listWidth = list.getListWidth();
        int rowX = list.left + list.width / 2 - listWidth / 2 + 2;
        int base = list.top + 4 - list.getAmountScrolled() + list.headerPadding;
        int index = Math.floorDiv(mouseY - base, list.slotHeight);
        int rowY = base + index * list.slotHeight;
        GuiMultiplayer screen = (GuiMultiplayer) (Object) this;
        if (index < 0 || index >= screen.getServerList().countServers()) return;
        if (!RutaIcon.onButton(rowX, rowY, listWidth, mouseX, mouseY)) return;

        Object entry = list.getListEntry(index);
        if (!(entry instanceof ServerListEntryNormal)) return;
        ServerData server = ((ServerListEntryNormal) entry).getServerData();
        if (RutaIcon.clickColumn(server, mouseButton)) {
            ci.cancel();
        }
    }
}
