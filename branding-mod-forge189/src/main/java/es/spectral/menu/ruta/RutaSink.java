package es.spectral.menu.ruta;

import io.netty.channel.Channel;

/**
 * Where a failed relay handshake is reported. The Minecraft side records the
 * disconnect reason so the join path shows a localized screen and the ping
 * path shows a localized status line; the test harness just records it.
 */
public interface RutaSink {

    /**
     * Called once per failed relay connection, on the Netty IO thread, right
     * before the channel is closed. {@code code} is one of the relay codes or
     * one of the client-only ones ({@code unreachable}, {@code nogrant},
     * {@code notmvp}, {@code timeout}, {@code proto}).
     */
    void rutaFailed(Channel channel, RutaTarget target, String code);
}
