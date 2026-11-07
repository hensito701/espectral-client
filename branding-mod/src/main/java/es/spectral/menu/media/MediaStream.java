package es.spectral.menu.media;

import java.io.IOException;
import java.nio.ByteBuffer;

import javax.sound.sampled.AudioFormat;

import net.minecraft.client.sounds.AudioStream;

/**
 * The bridge between the decode thread and the game's OpenAL streaming channel.
 *
 * <p>Vanilla's {@code Channel.pumpBuffers} calls {@link #read} on the sound
 * engine thread and treats {@code null} as "nothing available this tick", so
 * this never blocks: it copies what the {@link MediaRing} holds and returns an
 * empty result when the decode thread has not caught up. The buffer is the
 * ring's own byte array (no per-call allocation) and is wrapped without
 * copying for the OpenAL upload.
 */
final class MediaStream implements AudioStream {

    private final MediaRing ring;
    private final AudioFormat format;
    /**
     * One second of 44.1 kHz stereo, i.e. vanilla's {@code streamingBufferSize}:
     * sized once so a pump is always satisfied from a single read.
     */
    private final byte[] scratch = new byte[176_400];

    MediaStream(MediaRing ring, AudioFormat format) {
        this.ring = ring;
        this.format = format;
    }

    @Override
    public AudioFormat getFormat() {
        return format;
    }

    @Override
    public ByteBuffer read(int size) throws IOException {
        int want = Math.min(size, scratch.length);
        int n = ring.read(scratch, 0, want);
        if (n <= 0) {
            // Nothing buffered yet: vanilla treats null as "no buffer this tick".
            return null;
        }
        ByteBuffer buffer = ByteBuffer.allocateDirect(n);
        buffer.put(scratch, 0, n);
        buffer.flip();
        return buffer;
    }

    /** The decode thread owns the decoder: closing the channel must not stop it. */
    @Override
    public void close() throws IOException {
        // Intentional no-op: playback continues across a sound-engine reload.
    }
}
