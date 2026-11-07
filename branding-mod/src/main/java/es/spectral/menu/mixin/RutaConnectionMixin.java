package es.spectral.menu.mixin;

import java.net.InetAddress;
import java.net.InetSocketAddress;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import io.netty.bootstrap.Bootstrap;
import io.netty.channel.ChannelFuture;
import io.netty.channel.ChannelHandlerContext;

import net.minecraft.network.Connection;
import net.minecraft.network.DisconnectionDetails;

import es.spectral.menu.ruta.Ruta;
import es.spectral.menu.ruta.RutaHandshakeHandler;
import es.spectral.menu.ruta.RutaMessages;
import es.spectral.menu.ruta.RutaSink;
import es.spectral.menu.ruta.RutaTarget;

/**
 * Ruta Espectral routing hook on {@link Connection}.
 *
 * <p>Vanilla's {@code Connection.connect(InetSocketAddress, EventLoopGroupHolder,
 * Connection)} builds the Netty bootstrap and dials
 * {@code Bootstrap.connect(InetAddress, int)}. {@link Ruta#takePending()}
 * returns the decision noted by {@code ServerNameResolverMixin} for this thread,
 * so a routed connection dials the relay instead and carries the resolved
 * destination + grant in a channel attribute. The Minecraft pipeline is
 * unchanged: the handshake line, the {@code OK} wait and the outbound buffering
 * all happen in {@link RutaHandshakeHandler}, which is inserted at the head of
 * the pipeline in {@code channelActive} (i.e. before the game writes anything).
 *
 * <p>A routed server whose player is not allowed to use the relay throws a
 * {@link es.spectral.menu.ruta.RutaHandshakeException} here: the connector and
 * the pinger both surface it as a normal connection failure, and there is
 * deliberately no silent fall-back to a direct connection.
 *
 * <p>Same target method and descriptor on 1.21.11, 26.2 and 26.3, so the file is
 * shared by every lane.
 */
@Mixin(Connection.class)
public abstract class RutaConnectionMixin implements RutaSink {

    @Shadow private DisconnectionDetails disconnectionDetails;

    /** Channel attribute carrying the routing target between redirect and active. */
    private static final io.netty.util.AttributeKey<RutaTarget> ESPECTRAL$TARGET = Ruta.TARGET;

    @Redirect(
            method = "connect(Ljava/net/InetSocketAddress;Lnet/minecraft/server/network/EventLoopGroupHolder;Lnet/minecraft/network/Connection;)Lio/netty/channel/ChannelFuture;",
            at = @At(value = "INVOKE", remap = false,
                    target = "Lio/netty/bootstrap/Bootstrap;connect(Ljava/net/InetAddress;I)Lio/netty/channel/ChannelFuture;")
    )
    private static ChannelFuture espectral$rutaConnect(Bootstrap bootstrap, InetAddress address, int port) {
        Ruta.Decision decision = Ruta.takePending();
        if (decision == null || decision.state == Ruta.State.NONE) {
            return bootstrap.connect(address, port);
        }
        if (decision.state == Ruta.State.BLOCKED) {
            String message = RutaMessages.failure(decision.code).getString();
            Ruta.recordFailure(decision.key, decision.code);
            throw new es.spectral.menu.ruta.RutaHandshakeException(decision.code, message);
        }
        int relayPort = Ruta.relayPort();
        if (relayPort == 0) {
            Ruta.recordFailure(decision.key, "unreachable");
            throw new es.spectral.menu.ruta.RutaHandshakeException("unreachable",
                    RutaMessages.failure("unreachable").getString());
        }
        InetSocketAddress destination = new InetSocketAddress(address, port);
        RutaTarget target = Ruta.target(decision, destination);
        try {
            InetAddress relay = Ruta.relayAddress();
            bootstrap.attr(ESPECTRAL$TARGET, target);
            return bootstrap.connect(relay, relayPort);
        } catch (java.io.IOException error) {
            Ruta.recordFailure(decision.key, "unreachable");
            throw new es.spectral.menu.ruta.RutaHandshakeException("unreachable",
                    RutaMessages.failure("unreachable").getString());
        }
    }

    @Inject(method = "channelActive", at = @At("TAIL"))
    private void espectral$rutaInstall(ChannelHandlerContext ctx, CallbackInfo ci) {
        RutaTarget target = ctx.channel().attr(ESPECTRAL$TARGET).getAndSet(null);
        if (target == null) return;
        if (ctx.pipeline().context(RutaHandshakeHandler.NAME) == null) {
            ctx.pipeline().addFirst(RutaHandshakeHandler.NAME,
                    new RutaHandshakeHandler(target, this));
        }
    }

    @Override
    public void rutaFailed(io.netty.channel.Channel channel, RutaTarget target, String code) {
        Ruta.recordFailure(target == null ? null : target.serverKey, code);
        this.disconnectionDetails = new DisconnectionDetails(RutaMessages.failure(code));
    }
}
