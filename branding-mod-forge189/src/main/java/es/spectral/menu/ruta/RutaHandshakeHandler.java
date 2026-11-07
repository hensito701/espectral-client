package es.spectral.menu.ruta;

import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.Queue;
import java.util.concurrent.TimeUnit;

import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import io.netty.channel.Channel;
import io.netty.channel.ChannelDuplexHandler;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelPromise;
import io.netty.util.ReferenceCountUtil;
import io.netty.util.concurrent.ScheduledFuture;

/**
 * First handler on a relayed connection's pipeline (Netty only — the relay
 * harness drives this exact class).
 *
 * <p>Before the relay answers {@code OK} nothing Minecraft writes may reach the
 * socket: the {@code ERT1 ...\n} line has to be the first bytes. This handler
 * therefore
 * <ul>
 *   <li>writes the handshake line as soon as it is added (the channel is
 *       already active at that point),</li>
 *   <li>buffers every outbound message with its promise and swallows
 *       {@code flush} until {@code OK},</li>
 *   <li>accumulates inbound bytes until a newline, capped at
 *       {@link RutaProtocol#MAX_REPLY_BYTES} (Minecraft's decoder never sees the
 *       reply line),</li>
 *   <li>on {@code OK}: replays the buffered writes, flushes, removes itself;</li>
 *   <li>on {@code ERR <code>}, a malformed reply, EOF or the handshake timeout:
 *       fails the buffered promises, reports the code through {@link RutaSink}
 *       and closes the channel. It never falls back to a direct connection.</li>
 * </ul>
 *
 * <p>1.8.9 runs Netty 4.0.23; this class only uses API present there
 * ({@code ChannelDuplexHandler}, {@code ChannelPromise}, {@code ScheduledFuture}),
 * so it compiles and runs unchanged on 4.0 and 4.1.
 */
public final class RutaHandshakeHandler extends ChannelDuplexHandler {

    /** Pipeline name, for logs, the harness and debugging. */
    public static final String NAME = "espectral_ruta";

    private static final class Pending {
        final Object message;
        final ChannelPromise promise;

        Pending(Object message, ChannelPromise promise) {
            this.message = message;
            this.promise = promise;
        }
    }

    private final RutaTarget target;
    private final RutaSink sink;
    private final Queue<Pending> pending = new ArrayDeque<Pending>(8);
    private final StringBuilder reply = new StringBuilder(RutaProtocol.MAX_REPLY_BYTES);
    private ScheduledFuture<?> timeout;
    /** True until OK/ERR: outbound is buffered, inbound is protocol only. */
    private boolean waiting = true;
    /** True once the ERT1 line went out (exactly once, never twice). */
    private boolean sent;

    public RutaHandshakeHandler(RutaTarget target, RutaSink sink) {
        this.target = target;
        this.sink = sink;
    }

    // --- lifecycle --------------------------------------------------------

    @Override
    public void handlerAdded(ChannelHandlerContext ctx) throws Exception {
        super.handlerAdded(ctx);
        try {
            this.timeout = ctx.executor().schedule(new Runnable() {
                @Override
                public void run() {
                    fail(ctx, "timeout");
                }
            }, RutaProtocol.HANDSHAKE_TIMEOUT_MS, TimeUnit.MILLISECONDS);
        } catch (IllegalStateException notRegistered) {
            // Netty's EmbeddedChannel adds handlers before it registers, so
            // the unit tests have no executor here. In game the channel is
            // always registered (we are added after the connect completed).
        }
        // 1.8.9 adds this handler AFTER the connect completed (the relay pipe is
        // already open), so the active event never reaches us: send right here.
        // When it is added before activation the line waits for channelActive.
        if (ctx.channel().isActive()) {
            sendLine(ctx);
        }
    }

    @Override
    public void channelActive(ChannelHandlerContext ctx) throws Exception {
        // Only reached when the handler was installed before the connect
        // finished. Writing here — with the channel open — is what the relay
        // needs: a line queued before activation can end up behind the game's
        // first bytes, which is exactly the "ERR proto" the live relay answered.
        sendLine(ctx);
        super.channelActive(ctx);
    }

    /** Writes the {@code ERT1} line exactly once, as the first outbound bytes. */
    private void sendLine(ChannelHandlerContext ctx) {
        if (this.sent || !this.waiting) return;
        this.sent = true;
        String line = this.target.handshakeLine();
        ctx.writeAndFlush(Unpooled.copiedBuffer(line, StandardCharsets.US_ASCII));
    }

    @Override
    public void handlerRemoved(ChannelHandlerContext ctx) throws Exception {
        cancelTimeout();
        super.handlerRemoved(ctx);
    }

    @Override
    public void channelInactive(ChannelHandlerContext ctx) throws Exception {
        if (this.waiting) {
            fail(ctx, "unreachable");
            return;
        }
        super.channelInactive(ctx);
    }

    @Override
    public void exceptionCaught(ChannelHandlerContext ctx, Throwable cause) throws Exception {
        if (this.waiting) {
            fail(ctx, "unreachable");
            return;
        }
        super.exceptionCaught(ctx, cause);
    }

    // --- outbound ---------------------------------------------------------

    @Override
    public void write(ChannelHandlerContext ctx, Object msg, ChannelPromise promise) throws Exception {
        if (this.waiting) {
            this.pending.add(new Pending(msg, promise));
            return;
        }
        ctx.write(msg, promise);
    }

    @Override
    public void flush(ChannelHandlerContext ctx) throws Exception {
        if (this.waiting) {
            return; // flushed together with the buffered writes
        }
        ctx.flush();
    }

    // --- inbound ----------------------------------------------------------

    @Override
    public void channelRead(ChannelHandlerContext ctx, Object msg) throws Exception {
        if (!this.waiting) {
            ctx.fireChannelRead(msg);
            return;
        }
        ByteBuf leftover = null;
        try {
            if (!(msg instanceof ByteBuf)) {
                return;
            }
            ByteBuf buf = (ByteBuf) msg;
            while (buf.isReadable()) {
                byte b = buf.readByte();
                if (b == '\n') {
                    int start = 0;
                    while (start < this.reply.length() && this.reply.charAt(start) == '\r') {
                        start++;
                    }
                    if (start > 0) {
                        this.reply.delete(0, start);
                    }
                    answer(ctx);
                    if (buf.isReadable()) {
                        // Bytes that followed the reply belong to Minecraft.
                        leftover = buf.retain();
                    }
                    return;
                }
                if (this.reply.length() >= RutaProtocol.MAX_REPLY_BYTES) {
                    fail(ctx, "proto");
                    return;
                }
                this.reply.append((char) (b & 0xFF));
            }
        } finally {
            ReferenceCountUtil.release(msg);
            if (leftover != null) {
                // fireChannelRead starts at the head, so the frame decoder
                // resolves these bytes like any other socket input.
                ByteBuf rest = leftover;
                leftover = null;
                ctx.channel().pipeline().fireChannelRead(rest);
            }
        }
    }

    private void answer(ChannelHandlerContext ctx) {
        RutaProtocol.Reply parsed = RutaProtocol.parseReply(this.reply.toString());
        if (!parsed.ok) {
            fail(ctx, parsed.code);
            return;
        }
        this.waiting = false;
        cancelTimeout();
        if (ctx.pipeline().context(this) != null) {
            ctx.pipeline().remove(this);
        }
        Pending item;
        while ((item = this.pending.poll()) != null) {
            ctx.write(item.message, item.promise);
        }
        ctx.flush();
    }

    private void fail(ChannelHandlerContext ctx, String code) {
        if (!this.waiting) return;
        this.waiting = false;
        cancelTimeout();
        // The handler must run on plain Netty too (the harness and, in game,
        // the connector thread before the resource manager is up): a missing
        // Minecraft runtime degrades the message, never the close.
        String message;
        try {
            message = RutaMessages.failureWithHint(code);
        } catch (Throwable noMinecraft) {
            message = "Ruta Espectral: " + code;
        }
        RutaHandshakeException error = new RutaHandshakeException(code, message);
        Pending item;
        while ((item = this.pending.poll()) != null) {
            ReferenceCountUtil.release(item.message);
            if (item.promise != null) {
                try {
                    item.promise.tryFailure(error);
                } catch (Throwable ignored) {
                    // a listener threw; closing is what matters
                }
            }
        }
        if (this.sink != null) {
            try {
                this.sink.rutaFailed(ctx.channel(), this.target, code);
            } catch (Throwable ignored) {
                // never mask the close
            }
        }
        if (ctx.channel().isOpen()) {
            ctx.close();
        }
    }

    private void cancelTimeout() {
        if (this.timeout != null) {
            this.timeout.cancel(false);
            this.timeout = null;
        }
    }

    /** True while the relay has not answered yet (writes are buffered). */
    public boolean isWaiting() {
        return this.waiting;
    }

    /** Destination this connection is routed to. */
    public RutaTarget target() {
        return this.target;
    }
}
