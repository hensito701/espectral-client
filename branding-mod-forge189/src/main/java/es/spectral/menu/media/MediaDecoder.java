package es.spectral.menu.media;

import java.io.BufferedInputStream;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;

import javax.sound.sampled.AudioFormat;

import fr.delthas.javamp3.Sound;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

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

    private static final Logger LOGGER = LogManager.getLogger("espectral-media");

    private final MediaRing ring;
    private volatile Thread thread;
    private volatile boolean running;
    private volatile boolean eof;
    private volatile String error;
    private volatile AudioFormat format;

    MediaDecoder(MediaRing ring) {
        this.ring = ring;
    }

    /** Opens {@code path} and starts feeding the ring; replaces any running decode. */
    synchronized void open(Path path) {
        stop();
        ring.clear();
        eof = false;
        error = null;
        format = null;
        running = true;
        Thread t = new Thread(() -> decodeLoop(path), "espectral-media-decode");
        t.setDaemon(true);
        this.thread = t;
        t.start();
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

    /** The PCM format of the open file, or {@code null} until the header is parsed. */
    AudioFormat format() {
        return format;
    }

    private void decodeLoop(Path path) {
        byte[] chunk = new byte[16 * 1024];
        try (InputStream raw = Files.newInputStream(path);
                InputStream in = new BufferedInputStream(raw, 32 * 1024);
                Sound sound = new Sound(Mp3Header.skipId3v2(in))) {
            format = sound.getAudioFormat();
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
