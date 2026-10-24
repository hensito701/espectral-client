package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.gui.components.PlayerTabOverlay;
import net.minecraft.client.multiplayer.ClientPacketListener;

import es.spectral.menu.ClientConfig;

/**
 * 26.2: forces the tab-list head column on for the {@code tabheads} feature.
 *
 * <p>Vanilla gates heads on {@code ClientPacketListener#onlineMode()} — the
 * flag from the login packet that says the server runs Mojang authentication.
 * Espectral (and most Spanish community servers) run offline mode behind
 * AuthMe, so that flag is false, the face column is dropped, and the tab list
 * shows names only.
 *
 * <p>Forcing the single call's result to {@code true} is enough: the same local
 * drives all three places vanilla uses it — the per-column width
 * ({@code +9} for the face), the {@code PlayerFaceRenderer} call, and the ping
 * icon offset — so the name text and the {@code ⚛}/{@code ★} badge (added
 * downstream by {@code PlayerTabOverlayBadgeMixin} to the display name) shift
 * together and never overlap the face.
 *
 * <p>Skins come from {@code PlayerInfo#getSkin()}, i.e. whatever the server
 * announced in the player-info packet or the client-side default skin; the mod
 * never fetches a skin itself. With the feature off the original value is
 * returned untouched, so rendering is vanilla-identical.
 */
@Mixin(PlayerTabOverlay.class)
public abstract class PlayerTabOverlayHeadsMixin {

    @Redirect(
            method = "extractRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;"
                    + "ILnet/minecraft/world/scores/Scoreboard;"
                    + "Lnet/minecraft/world/scores/Objective;)V",
            at = @At(
                    value = "INVOKE",
                    target = "Lnet/minecraft/client/multiplayer/ClientPacketListener;onlineMode()Z"
            )
    )
    private boolean espectral$forceTabHeads(ClientPacketListener connection) {
        if (ClientConfig.getInstance().isFeatureEnabled("tabheads")) return true;
        return connection.onlineMode();
    }
}
