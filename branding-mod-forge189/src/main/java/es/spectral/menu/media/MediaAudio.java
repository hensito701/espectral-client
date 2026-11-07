package es.spectral.menu.media;

import javax.sound.sampled.AudioFormat;
import javax.sound.sampled.AudioSystem;
import javax.sound.sampled.DataLine;
import javax.sound.sampled.SourceDataLine;

import net.minecraft.client.Minecraft;
import net.minecraft.client.audio.ISound;
import net.minecraft.client.audio.SoundCategory;
import net.minecraft.client.audio.SoundHandler;
import net.minecraftforge.client.event.sound.PlaySoundEvent;
import net.minecraftforge.common.MinecraftForge;
import net.minecraftforge.fml.common.eventhandler.SubscribeEvent;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

/**
 * Plays the media ring through a Java Sound {@link SourceDataLine} on a daemon thread.
 *
 * <p>1.8.9 has no streaming channel pool on its sound engine, so the mod owns
 * its own line. That line never touches Minecraft's sound system, so it keeps
 * playing through resource reloads and device changes. Gain is software:
 * media volume times the game's master volume, applied per chunk.
 *
 * <p>Vanilla music ducking uses the forge {@link PlaySoundEvent}: while media is
 * audible, music sounds are cancelled as they start and the one already
 * playing is stopped. Nothing is written to {@code options.txt}.
 */
public final class MediaAudio {

    private static final Logger LOGGER = LogManager.getLogger("espectral-media");
    private static final MediaAudio INSTANCE = new MediaAudio();

    /** Line buffer depth; also bounds how far playback progress can lag the decoder. */
    private static final int BUFFER_MILLIS = 250;
    private static final int CHUNK_FRAMES = 1024;
    private static final int JOIN_MILLIS = 400;

    private volatile Session session;
    private volatile boolean paused;
    private volatile boolean ducking;
    private volatile float mediaVolume = 0.7F;
    private volatile float masterVolume = 1F;
    private volatile float gain = 0.7F;
    private volatile ISound lastVanillaMusic;

    private MediaAudio() {
    }

    public static MediaAudio get() {
        return INSTANCE;
    }

    /** Hooks the vanilla music gate onto the forge bus; call once during client init. */
    public static void registerMusicGate() {
        MinecraftForge.EVENT_BUS.register(INSTANCE);
    }

    /** Starts streaming {@code decoder}'s output from {@code ring}, replacing any running stream. */
    synchronized void startStreaming(MediaRing ring, MediaDecoder decoder, float volume) {
        stopStreaming();
        mediaVolume = clamp01(volume);
        recomputeGain();
        paused = false;
        Session s = new Session(ring, decoder);
        session = s;
        Thread t = new Thread(() -> pump(s), "espectral-media-audio");
        t.setDaemon(true);
        s.thread = t;
        t.start();
    }

    /** Stops the running stream immediately; the line is flushed so a blocked write returns. */
    synchronized void stopStreaming() {
        Session s = session;
        session = null;
        if (s == null) {
            return;
        }
        s.running = false;
        SourceDataLine line = s.line;
        if (line != null && line.isOpen()) {
            line.stop();
            line.flush();
        }
        Thread t = s.thread;
        if (t != null) {
            t.interrupt();
            try {
                t.join(JOIN_MILLIS);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
    }

    void applyVolume(float volume) {
        mediaVolume = clamp01(volume);
        recomputeGain();
    }

    /** Pauses or resumes the line; a paused line stops consuming, so the ring fills and the decoder parks. */
    void setPaused(boolean value) {
        paused = value;
        Session s = session;
        SourceDataLine line = s == null ? null : s.line;
        if (line == null || !line.isOpen()) {
            return;
        }
        try {
            if (value) {
                line.stop();
            } else {
                line.start();
            }
        } catch (IllegalStateException closedByStop) {
            // The stream was stopped between the check and the call; nothing left to pause.
        }
    }

    /** Called once per client tick: keeps the gain in step with the game's master volume. */
    void tick(Minecraft mc) {
        masterVolume = mc.gameSettings.getSoundLevel(SoundCategory.MASTER);
        recomputeGain();
    }

    /** Whether the stream has played to its end (the line drained after EOF). */
    boolean finished() {
        Session s = session;
        return s != null && s.finished;
    }

    /** Decode-independent error from the audio thread (for example, no sound device), or {@code null}. */
    String error() {
        Session s = session;
        return s == null ? null : s.error;
    }

    /** Milliseconds of audio the line has actually consumed: written bytes minus bytes still queued. */
    long playedMs() {
        Session s = session;
        if (s == null || s.frameBytes <= 0) {
            return 0L;
        }
        SourceDataLine line = s.line;
        long queued = 0L;
        if (line != null && line.isOpen()) {
            queued = Math.max(0, s.bufferBytes - line.available());
        }
        long played = Math.max(0L, s.written - queued);
        return (played / s.frameBytes) * 1000L / (long) s.frameRate;
    }

    /**
     * Ducks vanilla music while media is audible. Turning it on while media plays stops any
     * music already sounding; turning it off leaves vanilla music to its own scheduler.
     */
    void duckVanillaMusic(boolean on) {
        ducking = on;
        if (on && audible()) {
            silenceVanillaMusic();
        }
    }

    /** Forge hook: cancels vanilla music starts while ducking applies. */
    @SubscribeEvent
    public void onPlaySound(PlaySoundEvent event) {
        if (event.result == null || !isMusic(event)) {
            return;
        }
        if (audible()) {
            event.result = null;
            return;
        }
        lastVanillaMusic = event.sound;
    }

    private boolean audible() {
        Session s = session;
        return ducking && s != null && s.running && !s.finished && !paused;
    }

    private void silenceVanillaMusic() {
        ISound music = lastVanillaMusic;
        if (music == null) {
            return;
        }
        SoundHandler handler = Minecraft.getMinecraft().getSoundHandler();
        if (handler.isSoundPlaying(music)) {
            handler.stopSound(music);
        }
    }

    private static boolean isMusic(PlaySoundEvent event) {
        if (event.category == SoundCategory.MUSIC) {
            return true;
        }
        return event.sound != null && event.sound.getSoundLocation().getResourcePath().startsWith("music.");
    }

    private void recomputeGain() {
        gain = mediaVolume * masterVolume;
    }

    private void pump(Session s) {
        SourceDataLine line = null;
        try {
            AudioFormat format = awaitFormat(s);
            if (format == null) {
                return;
            }
            if (format.getEncoding() != AudioFormat.Encoding.PCM_SIGNED || format.getSampleSizeInBits() != 16) {
                s.error = "unsupported PCM format: " + format;
                LOGGER.warn("Media playback stopped: {}", s.error);
                return;
            }
            int frameBytes = format.getFrameSize();
            int bufferBytes = (int) (format.getFrameRate() * BUFFER_MILLIS / 1000F) * frameBytes;
            line = (SourceDataLine) AudioSystem.getLine(new DataLine.Info(SourceDataLine.class, format));
            line.open(format, bufferBytes);
            s.bigEndian = format.isBigEndian();
            s.frameRate = format.getFrameRate();
            s.frameBytes = frameBytes;
            s.bufferBytes = line.getBufferSize();
            s.line = line;
            if (paused) {
                line.stop();
            } else {
                line.start();
            }

            byte[] buf = new byte[frameBytes * CHUNK_FRAMES];
            while (s.running) {
                int n = s.ring.read(buf, 0, buf.length);
                if (n == 0) {
                    if (s.decoder.atEof() && s.ring.available() == 0) {
                        break;
                    }
                    s.ring.awaitData(20);
                    continue;
                }
                applyGain(buf, n, gain, s.bigEndian);
                int off = 0;
                while (off < n && s.running) {
                    int w = line.write(buf, off, n - off);
                    off += w;
                    s.written += w;
                }
            }
            if (s.running) {
                line.drain();
                s.finished = true;
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (Throwable e) {
            if (s.running) {
                s.error = e.toString();
                LOGGER.warn("Media audio failed: {}", e.toString());
            }
        } finally {
            if (line != null) {
                line.stop();
                line.close();
            }
        }
    }

    private static AudioFormat awaitFormat(Session s) throws InterruptedException {
        while (s.running) {
            AudioFormat format = s.decoder.format();
            if (format != null) {
                return format;
            }
            if (s.decoder.error() != null || s.decoder.atEof()) {
                return null;
            }
            Thread.sleep(5L);
        }
        return null;
    }

    /** Scales 16-bit signed PCM in place by {@code g}, clamping to the sample range. */
    private static void applyGain(byte[] buf, int n, float g, boolean bigEndian) {
        for (int i = 0; i + 1 < n; i += 2) {
            int hi = bigEndian ? buf[i] : buf[i + 1];
            int lo = bigEndian ? buf[i + 1] : buf[i];
            int sample = (short) ((hi << 8) | (lo & 0xFF));
            int scaled = Math.round(sample * g);
            if (scaled > Short.MAX_VALUE) {
                scaled = Short.MAX_VALUE;
            } else if (scaled < Short.MIN_VALUE) {
                scaled = Short.MIN_VALUE;
            }
            if (bigEndian) {
                buf[i] = (byte) (scaled >> 8);
                buf[i + 1] = (byte) scaled;
            } else {
                buf[i] = (byte) scaled;
                buf[i + 1] = (byte) (scaled >> 8);
            }
        }
    }

    private static float clamp01(float v) {
        return Math.max(0F, Math.min(1F, v));
    }

    /** One streaming session: a ring, its decoder, and the line that drains the ring. */
    private static final class Session {
        final MediaRing ring;
        final MediaDecoder decoder;
        volatile boolean running = true;
        volatile boolean finished;
        volatile String error;
        volatile SourceDataLine line;
        volatile int bufferBytes;
        volatile int frameBytes;
        volatile float frameRate = 1F;
        volatile boolean bigEndian;
        volatile long written;
        Thread thread;

        Session(MediaRing ring, MediaDecoder decoder) {
            this.ring = ring;
            this.decoder = decoder;
        }
    }
}
