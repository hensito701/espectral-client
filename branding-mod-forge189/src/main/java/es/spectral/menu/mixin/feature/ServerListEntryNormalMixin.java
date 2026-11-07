package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.gui.GuiMultiplayer;
import net.minecraft.client.gui.ServerListEntryNormal;
import net.minecraft.client.multiplayer.ServerData;

import es.spectral.menu.serverlist.RowExtras;
import es.spectral.menu.serverlist.RowExtrasLayout;

/**
 * Ping number and country flag on the 1.8.9 multiplayer server entries.
 *
 * <p>{@code drawEntry} is injected at {@code TAIL}: vanilla has already drawn
 * the name, the MOTD, the player count, the status bars and its own hover text,
 * so the extras never overdraw anything and a hover we do not own keeps
 * vanilla's tooltip ({@code owner.setHoveringText} is the same call vanilla
 * makes there). Geometry, colours and the flag tooltip live in {@link RowExtras}.
 *
 * <p>{@code mousePressed} gives the reserved route-icon slot first refusal on a
 * click; nothing is installed by default, so the row behaves exactly as vanilla
 * until the Ruta Espectral feature plugs in.
 */
@Mixin(ServerListEntryNormal.class)
public abstract class ServerListEntryNormalMixin {

    @Shadow private ServerData server;

    @Shadow private GuiMultiplayer owner;

    @Unique private RowExtrasLayout espectral$layout = new RowExtrasLayout();

    @Inject(method = "drawEntry", at = @At("TAIL"))
    private void espectral$rowExtras(int slotIndex, int x, int y, int listWidth, int slotHeight,
            int mouseX, int mouseY, boolean isSelected, CallbackInfo ci) {
        // An out-of-date row shows vanilla's "Client/Server out of date!" in the
        // same band; leaving it alone keeps that message readable.
        if (server == null || server.isOnLAN() || server.version != RowExtras.PROTOCOL) return;
        String tooltip = RowExtras.draw(espectral$layout, server.pingToServer, server.populationInfo,
                server.serverIP, x, y, listWidth, mouseX, mouseY);
        if (tooltip != null) {
            owner.setHoveringText(tooltip);
        }
    }

    @Inject(method = "mousePressed", at = @At("HEAD"), cancellable = true)
    private void espectral$clickRowExtras(int slotIndex, int mouseX, int mouseY, int mouseButton,
            int relativeX, int relativeY, CallbackInfoReturnable<Boolean> cir) {
        // Args 2/3 are the absolute cursor position; the relative pair is only
        // convenient for the vanilla 32 px icon/swap band.
        if (server == null) return;
        if (RowExtras.click(espectral$layout, server.serverIP, mouseX, mouseY, mouseButton)) {
            cir.setReturnValue(Boolean.TRUE);
        }
    }
}
