package es.spectral.menu.mixin.feature;

import java.io.IOException;
import java.net.InetAddress;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import io.netty.bootstrap.Bootstrap;
import io.netty.channel.Channel;
import io.netty.channel.ChannelFuture;
import io.netty.channel.ChannelFutureListener;

import net.minecraft.network.NetworkManager;

import es.spectral.menu.ruta.Ruta;
import es.spectral.menu.ruta.RutaHandshakeException;
import es.spectral.menu.ruta.RutaHandshakeHandler;
import es.spectral.menu.ruta.RutaMessages;
import es.spectral.menu.ruta.RutaSink;
import es.spectral.menu.ruta.RutaTarget;

/**
 * Ruta Espectral: sends a routed connection to the relay instead of the server.
 *
 * <p>1.8.9 opens every server connection in exactly one place,
 * {@code NetworkManager.createNetworkManagerAndConnect(InetAddress,int,boolean)};
 * both the join path ({@code GuiConnecting}'s connector thread) and the server
 * list status ping ({@code OldServerPinger.ping}) go through it. The typed
 * address is remembered by the {@code GuiConnecting}/{@code OldServerPinger}
 * injections, so the decision here uses the key the player actually typed.
 *
 * <p>When the row is routed, the {@code Bootstrap.connect} call is redirected to
 * the relay (25565, falling back to 443). Vanilla's own initializer still builds
 * the pipeline; {@link RutaHandshakeHandler} is inserted at its head before the
 * connect future completes, so the {@code ERT1} line is the connection's first
 * bytes, Minecraft's writes wait for {@code OK}, and the reply line never
 * reaches vanilla's decoder.
 *
 * <p>Never a silent direct fallback. A blocked route, an unreachable relay or an
 * unusable grant throws {@link RutaHandshakeException}, whose message is the
 * localized reason and which both call sites already render as their failure
 * text (the join screen prints {@code toString()}; the pinger catches every
 * {@code Exception}). The failure is also recorded so the server row shows the
 * reason next to the Ruta icon.
 */
@Mixin(NetworkManager.class)
public abstract class NetworkManagerRutaMixin {

    @Redirect(method = "createNetworkManagerAndConnect",
            at = @At(value = "INVOKE", remap = false,
                    target = "Lio/netty/bootstrap/Bootstrap;connect(Ljava/net/InetAddress;I)Lio/netty/channel/ChannelFuture;"))
    private ChannelFuture espectral$routeThroughRelay(Bootstrap bootstrap, InetAddress destination, int port) {
        Ruta.Decision decision = Ruta.decisionFor(destination, port);
        if (decision == null) {
            return bootstrap.connect(destination, port);
        }
        if (decision.state != Ruta.State.ROUTE) {
            throw new RutaHandshakeException(decision.code, RutaMessages.failureWithHint(decision.code));
        }

        RutaTarget target;
        try {
            target = Ruta.target(decision, destination, port);
            target.handshakeLine(); // validates grant + destination before dialling
        } catch (RuntimeException invalid) {
            String code = Ruta.isAvailable() ? "proto" : "nogrant";
            Ruta.recordFailure(decision.key, code);
            throw new RutaHandshakeException(code, RutaMessages.failureWithHint(code));
        }

        int relayPort = Ruta.relayPort();
        InetAddress relay;
        try {
            relay = relayPort == 0 ? null : Ruta.relayAddress();
        } catch (IOException unresolved) {
            relay = null;
        }
        if (relay == null) {
            Ruta.recordFailure(decision.key, "unreachable");
            throw new RutaHandshakeException("unreachable", RutaMessages.failureWithHint("unreachable"));
        }

        ChannelFuture future = bootstrap.connect(relay, relayPort);
        final RutaTarget routed = target;
        future.addListener(new ChannelFutureListener() {
            @Override
            public void operationComplete(ChannelFuture connected) throws Exception {
                if (!connected.isSuccess()) {
                    Ruta.recordFailure(routed.serverKey, "unreachable");
                    return; // the failed future carries the connect error already
                }
                Channel channel = connected.channel();
                if (channel.pipeline().context(RutaHandshakeHandler.NAME) == null) {
                    channel.pipeline().addFirst(RutaHandshakeHandler.NAME,
                            new RutaHandshakeHandler(routed, SINK));
                }
            }
        });
        return future;
    }

    /** Records relay failures so the server row can show the reason. */
    private static final RutaSink SINK = new RutaSink() {
        @Override
        public void rutaFailed(Channel channel, RutaTarget target, String code) {
            Ruta.recordFailure(target == null ? null : target.serverKey, code);
        }
    };
}
