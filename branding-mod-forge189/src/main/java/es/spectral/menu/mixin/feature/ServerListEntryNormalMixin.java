package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiMultiplayer;
import net.minecraft.client.gui.ServerListEntryNormal;
import net.minecraft.client.multiplayer.ServerData;

import es.spectral.menu.ruta.RutaIcon;
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
 * <p>The Ruta Espectral button column is painted from {@code drawEntry} via
 * {@link es.spectral.menu.ruta.RutaIcon}. Its clicks are taken on the screen by
 * {@code GuiMultiplayerRouteColumnMixin}, since a row never receives a click
 * outside its own x-range.
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
        String routeTip = RutaIcon.paintColumn(server, x, y, listWidth, mouseX, mouseY);
        if (routeTip != null) {
            owner.setHoveringText(routeTip);
        }
    }
}
