package es.spectral.menu.mixin.feature;

import java.net.InetAddress;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.network.OldServerPinger;

import es.spectral.menu.geoip.GeoIp;
import es.spectral.menu.ruta.Ruta;

/**
 * Records the address the vanilla status ping resolved for a server row.
 *
 * <p>{@code OldServerPinger.ping} runs on its own pinger thread (a pool of 5),
 * resolves the saved address — SRV records included — and only then opens the
 * connection. That resolved address is the one to geolocate, so it is captured
 * here on the pinger thread and stored under the row's
 * {@code ServerData.serverIP}; the render thread then never performs DNS.
 *
 * <p>It also hands the row's address to Ruta Espectral, which decides on the
 * same thread whether this ping must go through the relay.
 *
 * <p>Address capture only: the ping itself is untouched (the redirect returns
 * exactly what vanilla resolved).
 */
@Mixin(OldServerPinger.class)
public abstract class OldServerPingerMixin {

    @Inject(method = "ping", at = @At("HEAD"))
    private void espectral$pingStarted(ServerData server, CallbackInfo ci) {
        GeoIp.getInstance().resolveStarted(server == null ? null : server.serverIP);
        // The pinger pool thread resolves here, then calls createNetworkManager
        // AndConnect on the same thread: both a country key and a Ruta key.
        Ruta.noteTypedHost(server == null ? null : server.serverIP);
    }

    @Redirect(method = "ping", at = @At(value = "INVOKE", remap = false,
            target = "Ljava/net/InetAddress;getByName(Ljava/lang/String;)Ljava/net/InetAddress;"))
    private InetAddress espectral$pingResolved(String host) throws java.net.UnknownHostException {
        InetAddress address = InetAddress.getByName(host);
        GeoIp.getInstance().resolved(address);
        return address;
    }
}
