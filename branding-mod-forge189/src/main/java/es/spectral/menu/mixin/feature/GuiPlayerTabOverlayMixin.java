package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.gui.GuiPlayerTabOverlay;
import net.minecraft.network.NetworkManager;

import es.spectral.menu.ClientConfig;

/**
 * Tab list heads ({@code tabheads}): 1.8.9 only draws the head column when
 * the connection is encrypted (online mode) or integrated; with the feature
 * on the column is always drawn, so offline-mode servers show heads too.
 */
@Mixin(GuiPlayerTabOverlay.class)
public abstract class GuiPlayerTabOverlayMixin {

    @Redirect(method = "renderPlayerlist",
            at = @At(value = "INVOKE", target = "Lnet/minecraft/network/NetworkManager;getIsencrypted()Z"))
    private boolean espectral$alwaysHeads(NetworkManager manager) {
        return manager.getIsencrypted() || ClientConfig.getInstance().isFeatureEnabled("tabheads");
    }
}
