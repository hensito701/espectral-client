package es.spectral.menu.mixin;

import java.net.InetSocketAddress;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.ServerStatusPinger;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.network.chat.Component;
import net.minecraft.server.network.EventLoopGroupHolder;

import es.spectral.menu.ruta.Ruta;
import es.spectral.menu.ruta.RutaMessages;

/**
 * Status-ping side of Ruta Espectral (1.21.11, 26.2 and 26.3 share this file).
 *
 * <p>When a routed server's ping fails, vanilla replaces the row's MOTD with
 * "Can't connect to server." This tail injection swaps in a localized line that
 * names Ruta Espectral and the actual reason ({@code ERR <code>}, relay
 * unreachable, missing grant...). The standard unreachable state stays; only the
 * text says it failed via Ruta.
 *
 * <p>{@code pingLegacyServer} is skipped for routed servers: that pre-1.7
 * fallback dials the destination directly, which would silently bypass the relay
 * and reveal the player's IP.
 */
@Mixin(ServerStatusPinger.class)
public abstract class RutaPingMixin {

    @Inject(method = "pingServer", at = @At("HEAD"))
    private void espectral$rutaForgetStaleFailure(ServerData data, Runnable a, Runnable b,
            EventLoopGroupHolder holder, CallbackInfo ci) {
        // Each attempt starts clean: a previous relay failure must never
        // colour a later, unrelated verdict for the same row.
        if (data != null) Ruta.clearFailure(data.ip);
    }

    @Inject(method = "onPingFailed", at = @At("TAIL"))
    private void espectral$rutaFailureText(Component reason, ServerData data, CallbackInfo ci) {
        if (data == null) return;
        String code = Ruta.recentFailure(data.ip);
        if (code == null) return;
        data.motd = RutaMessages.status(code);
        data.status = Component.empty();
    }

    @Inject(method = "pingLegacyServer", at = @At("HEAD"), cancellable = true)
    private void espectral$rutaNoDirectLegacyPing(InetSocketAddress resolvedAddress,
            ServerAddress rawAddress, ServerData data, EventLoopGroupHolder eventLoopGroupHolder,
            CallbackInfo ci) {
        if (data == null || !Ruta.isRouted(data.ip)) return;
        // The direct legacy probe would defeat the relay (and expose the IP).
        ci.cancel();
    }
}
