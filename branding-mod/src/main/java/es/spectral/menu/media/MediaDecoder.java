package es.spectral.menu.media;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;

import javax.sound.sampled.AudioFormat;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import fr.delthas.javamp3.Sound;

/**
 * Streaming MP3 decode thread: reads {@link Sound} frames into a
 * {@link MediaRing} and nothing else.
 *
 * <p>Memory is bounded by the ring plus one decoded MPEG frame
 * ({@code Sound} pulls exactly one frame per internal read), so a 10-minute
 * track costs the same heap as a 10-second one. The thread parks on the ring
 * while it is full, so a paused player costs no CPU.
 */
final class MediaDecoder {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-media");

    private final MediaRing ring;
    private volatile Thread thread;
    private volatile boolean running;
    private volatile boolean eof;
    private volatile String error;
    /** Real format of the open file, read from its first MPEG frame. */
    private volatile AudioFormat format;

    MediaDecoder(MediaRing ring) {
        this.ring = ring;
    }

    /**
     * Opens {@code path} and starts feeding the ring; replaces any running
     * decode. The MPEG header is read here (one frame) so {@link #format()}
     * is known before the OpenAL channel is created: the output format must
     * match the file (48 kHz and mono files are common), or playback would
     * run at the wrong speed. One buffered frame read is cheap enough for the
     * client thread — the full decode stays on the worker.
     */
    synchronized void open(Path path) {
        stop();
        ring.clear();
        eof = false;
        error = null;
        Sound sound;
        try {
            InputStream in = new java.io.BufferedInputStream(Files.newInputStream(path), 32 * 1024);
            sound = new Sound(in);
        } catch (Throwable e) {
            this.error = e.toString();
            LOGGER.warn("Cannot open {}: {}", path.getFileName(), e.toString());
            return;
        }
        this.format = sound.getAudioFormat();
        running = true;
        Thread t = new Thread(() -> decodeLoop(sound, path), "espectral-media-decode");
        t.setDaemon(true);
        this.thread = t;
        t.start();
    }

    /** Format of the open file, or {@code null} when nothing is open. */
    AudioFormat format() {
        return format;
    }

    /** Stops the decode thread (waits briefly so a close never races the file handle). */
    synchronized void stop() {
        running = false;
        Thread t = thread;
        thread = null;
        if (t != null) {
            t.interrupt();
            try {
                t.join(400L);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
    }

    boolean atEof() {
        return eof;
    }

    String error() {
        return error;
    }

    private void decodeLoop(Sound sound, Path path) {
        byte[] chunk = new byte[16 * 1024];
        try (Sound open = sound) {
            while (running) {
                if (ring.awaitRoom(250) <= 0) {
                    continue;
                }
                int n = sound.read(chunk, 0, chunk.length);
                if (n < 0) {
                    eof = true;
                    break;
                }
                int written = 0;
                while (written < n && running) {
                    int w = ring.write(chunk, written, n - written);
                    written += w;
                    if (w == 0) {
                        ring.awaitRoom(250);
                    }
                }
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (Throwable e) {
            if (running) {
                error = e.toString();
                LOGGER.warn("MP3 decode failed for {}: {}", path.getFileName(), e.toString());
            }
        } finally {
            if (running) {
                eof = true;
            }
        }
    }
}
