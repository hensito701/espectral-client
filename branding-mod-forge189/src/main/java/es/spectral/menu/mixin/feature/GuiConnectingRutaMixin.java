package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.multiplayer.GuiConnecting;

import es.spectral.menu.ruta.Ruta;

/**
 * Ruta Espectral: remembers which server a join is for.
 *
 * <p>{@code GuiConnecting.connect(host, port)} starts every join and immediately
 * hands the work to a brand-new "Server Connector" thread, which cannot inherit
 * a {@link ThreadLocal}. The host is therefore noted here on the caller thread
 * and taken over once by that connector thread when
 * {@code NetworkManager.createNetworkManagerAndConnect} runs (see
 * {@link NetworkManagerRutaMixin}).
 */
@Mixin(GuiConnecting.class)
public abstract class GuiConnectingRutaMixin {

    @Inject(method = "connect", at = @At("HEAD"))
    private void espectral$noteJoinTarget(String host, int port, CallbackInfo ci) {
        Ruta.noteJoinTarget(host);
    }
}
