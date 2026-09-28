package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.gui.components.PlayerTabOverlay;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.network.chat.Component;

import es.spectral.menu.cosmetics.Cosmetics;

/**
 * Prepends the ⚛/★ badge to tab-list display names. The returned component is
 * the server/team-decided name with a leading sibling — LuckPerms prefixes and
 * team colours stay authoritative.
 */
@Mixin(PlayerTabOverlay.class)
public abstract class PlayerTabOverlayBadgeMixin {

    @Inject(
            method = "getNameForDisplay(Lnet/minecraft/client/multiplayer/PlayerInfo;)"
                    + "Lnet/minecraft/network/chat/Component;",
            at = @At("RETURN"),
            cancellable = true)
    private void espectral$badgeTabName(PlayerInfo info,
            CallbackInfoReturnable<Component> cir) {
        if (info == null || info.getProfile() == null || info.getProfile().id() == null) return;
        Component decorated = Cosmetics.decorateNameTag(
                cir.getReturnValue(), info.getProfile().id());
        if (decorated != cir.getReturnValue()) {
            cir.setReturnValue(decorated);
        }
    }
}
