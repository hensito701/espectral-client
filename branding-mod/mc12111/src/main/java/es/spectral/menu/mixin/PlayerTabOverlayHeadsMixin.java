package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.gui.components.PlayerTabOverlay;
import net.minecraft.network.Connection;

import es.spectral.menu.ClientConfig;

/**
 * 1.21.11: forces the tab-list head column on for the {@code tabheads} feature.
 *
 * <p>This lane gates heads on {@code minecraft.isLocalServer() ||
 * connection.getConnection().isEncrypted()}. Espectral (and most Spanish
 * community servers) run offline mode behind AuthMe without transport
 * encryption, so both sides are false and the tab list drops the face column.
 *
 * <p>Forcing the {@code Connection#isEncrypted()} call to {@code true} makes the
 * whole condition true (the local-server case already short-circuits to true
 * before it). That single local drives all three uses vanilla makes of it — the
 * per-column width ({@code +9} for the face), the {@code PlayerFaceRenderer}
 * call, and the ping icon offset — so the name text and the {@code ⚛}/{@code ★}
 * badge (added downstream by {@code PlayerTabOverlayBadgeMixin} to the display
 * name) shift together and never overlap the face.
 *
 * <p>Skins come from {@code PlayerInfo#getSkin()}, i.e. whatever the server
 * announced in the player-info packet or the client-side default skin; the mod
 * never fetches a skin itself. With the feature off the original value is
 * returned untouched, so rendering is vanilla-identical.
 */
@Mixin(PlayerTabOverlay.class)
public abstract class PlayerTabOverlayHeadsMixin {

    @Redirect(
            method = "render(Lnet/minecraft/client/gui/GuiGraphics;"
                    + "ILnet/minecraft/world/scores/Scoreboard;"
                    + "Lnet/minecraft/world/scores/Objective;)V",
            at = @At(
                    value = "INVOKE",
                    target = "Lnet/minecraft/network/Connection;isEncrypted()Z"
            )
    )
    private boolean espectral$forceTabHeads(Connection connection) {
        if (ClientConfig.getInstance().isFeatureEnabled("tabheads")) return true;
        return connection.isEncrypted();
    }
}
