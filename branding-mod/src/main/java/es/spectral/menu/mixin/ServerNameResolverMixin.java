package es.spectral.menu.mixin;

import java.net.InetSocketAddress;
import java.util.Optional;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.multiplayer.resolver.ResolvedServerAddress;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.client.multiplayer.resolver.ServerNameResolver;

import es.spectral.menu.geoip.ServerAddresses;

/**
 * Records the address vanilla resolves for a server status ping (SRV included)
 * so the multiplayer list can map a row to a country later without a second
 * DNS query — and never on the render thread.
 *
 * <p>{@code ServerStatusPinger.pingServer} runs on the server-list thread pool
 * and resolves the configured {@code ServerData.ip} through this method. The
 * injection sits on RETURN so it observes the address that was actually
 * dialled, keyed by the <em>configured</em> host:port (the redirect target's
 * port can differ when DNS SRV is in play).
 *
 * <p>Identical class and signature on 1.21.11 and 26.2/26.3, so this mixin is
 * shared by every lane.
 */
@Mixin(ServerNameResolver.class)
public abstract class ServerNameResolverMixin {

    @Inject(
            method = "resolveAddress",
            at = @At("HEAD")
    )
    private void espectral$rutaArm(ServerAddress address, CallbackInfoReturnable<Optional<ResolvedServerAddress>> cir) {
        if (address == null) {
            es.spectral.menu.ruta.Ruta.clearNote();
            return;
        }
        es.spectral.menu.ruta.Ruta.noteAddress(address.getHost(), address.getPort());
    }

    @Inject(
            method = "resolveAddress",
            at = @At("RETURN")
    )
    private void espectral$recordResolvedAddress(ServerAddress address,
            CallbackInfoReturnable<Optional<ResolvedServerAddress>> cir) {
        if (address == null) {
            return;
        }
        Optional<ResolvedServerAddress> resolved = cir.getReturnValue();
        if (resolved == null) {
            return;
        }
        resolved.ifPresent(value -> {
            InetSocketAddress socketAddress = value.asInetSocketAddress();
            if (socketAddress != null) {
                ServerAddresses.record(address.getHost(), address.getPort(), socketAddress);
            }
        });
    }
}
