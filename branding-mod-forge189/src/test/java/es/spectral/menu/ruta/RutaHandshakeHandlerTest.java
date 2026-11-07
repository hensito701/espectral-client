package es.spectral.menu.ruta;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;

import java.nio.charset.StandardCharsets;

import org.junit.Test;

import io.netty.buffer.ByteBuf;
import io.netty.channel.embedded.EmbeddedChannel;

/**
 * The relay handshake handler on Netty's in-memory channel: the ERT1 line must
 * be the very first outbound bytes, the game's own writes must wait for
 * {@code OK}, and the reply line must never reach Minecraft's decoder.
 *
 * <p>{@code EmbeddedChannel} is always active, which is the state the handler
 * sees in game: 1.8.9 installs it on an already-connected channel.
 */
public class RutaHandshakeHandlerTest {

    private static final String GRANT = "header.payload.sig";

    private static RutaTarget target(String destIp, int port) {
        return new RutaTarget("mc.example.net", destIp, port, GRANT);
    }

    private static String drain(ByteBuf buf) {
        try {
            byte[] out = new byte[buf.readableBytes()];
            buf.readBytes(out);
            return new String(out, StandardCharsets.US_ASCII);
        } finally {
            buf.release();
        }
    }

    private static ByteBuf bytes(EmbeddedChannel channel, String text) {
        return channel.alloc().buffer().writeBytes(text.getBytes(StandardCharsets.US_ASCII));
    }

    @Test
    public void handshakeLineGoesOutFirstAndOnlyOnce() {
        EmbeddedChannel channel = new EmbeddedChannel(
                new RutaHandshakeHandler(target("1.2.3.4", 25565), null));
        ByteBuf out = (ByteBuf) channel.readOutbound();
        assertNotNull("the ERT1 line must be written on install", out);
        assertEquals("ERT1 header.payload.sig 1.2.3.4 25565\n", drain(out));
        assertNull("and never a second copy", channel.readOutbound());
        channel.finish();
    }

    @Test
    public void gameWritesWaitForOk() {
        EmbeddedChannel channel = new EmbeddedChannel(
                new RutaHandshakeHandler(target("1.2.3.4", 25565), null));
        drain((ByteBuf) channel.readOutbound()); // the ERT1 line

        ByteBuf handshake = channel.alloc().buffer();
        handshake.writeBytes(new byte[] {0x05, 0x00});
        // writeOutbound() asserts the message reaches the socket, which is
        // exactly what must NOT happen yet: go through the pipeline instead.
        channel.writeAndFlush(handshake);
        assertNull("nothing may reach the socket before OK", channel.readOutbound());

        channel.writeInbound(bytes(channel, "OK\n"));
        ByteBuf replayed = (ByteBuf) channel.readOutbound();
        assertNotNull("OK must release the buffered writes", replayed);
        assertEquals(0x05, replayed.readUnsignedByte());
        replayed.release();
        channel.finish();
    }

    @Test
    public void replyLineIsSwallowedAndTheRestOfTheBytesFlowThrough() {
        EmbeddedChannel channel = new EmbeddedChannel(
                new RutaHandshakeHandler(target("1.2.3.4", 25565), null));
        drain((ByteBuf) channel.readOutbound());

        channel.writeInbound(bytes(channel, "OK\nminecraft-bytes"));

        Object passThrough = channel.readInbound();
        assertNotNull("bytes after the reply belong to Minecraft", passThrough);
        assertEquals("minecraft-bytes", drain((ByteBuf) passThrough));
        assertNull("the reply line itself must not reach the decoder", channel.readInbound());
        channel.finish();
    }

    @Test
    public void errorReplyFailsTheSinkAndCloses() {
        final String[] code = new String[1];
        RutaSink sink = new RutaSink() {
            @Override
            public void rutaFailed(io.netty.channel.Channel channel, RutaTarget target, String reported) {
                code[0] = reported;
            }
        };
        EmbeddedChannel channel = new EmbeddedChannel(
                new RutaHandshakeHandler(target("10.0.0.5", 25565), sink));
        drain((ByteBuf) channel.readOutbound());

        ByteBuf pending = channel.alloc().buffer();
        pending.writeBytes(new byte[] {0x01, 0x02});
        channel.writeAndFlush(pending);

        channel.writeInbound(bytes(channel, "ERR dest\n"));
        assertEquals("dest", code[0]);
        assertEquals("a failed handshake closes the channel", false, channel.isOpen());
        assertNull("and never releases the buffered writes", channel.readOutbound());
        channel.finish();
    }

    @Test
    public void unknownRepliesReadAsProto() {
        final String[] code = new String[1];
        EmbeddedChannel channel = new EmbeddedChannel(new RutaHandshakeHandler(
                target("1.2.3.4", 25565), new RutaSink() {
                    @Override
                    public void rutaFailed(io.netty.channel.Channel channel, RutaTarget target, String reported) {
                        code[0] = reported;
                    }
                }));
        drain((ByteBuf) channel.readOutbound());
        channel.writeInbound(bytes(channel, "WAT\n"));
        assertEquals("proto", code[0]);
        channel.finish();
    }
}
