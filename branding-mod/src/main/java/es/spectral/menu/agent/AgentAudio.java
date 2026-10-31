package es.spectral.menu.agent;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.nio.IntBuffer;

import javax.sound.sampled.AudioFormat;
import javax.sound.sampled.AudioSystem;
import javax.sound.sampled.SourceDataLine;

import org.lwjgl.openal.ALC10;
import org.lwjgl.openal.SOFTLoopback;
import org.lwjgl.system.MemoryStack;
import org.lwjgl.system.MemoryUtil;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * The game's own audio mix, tapped in-process for agent recordings.
 *
 * <p>Agent launches open the sound engine on an OpenAL Soft <b>loopback</b>
 * device ({@code ALC_SOFT_loopback}) instead of the system output: nothing is
 * played by OpenAL itself; a pump thread renders the mix in 10 ms blocks
 * ({@code alcRenderSamplesSOFT}), forwards them to the speakers through Java
 * Sound — so the player still hears the game — and, while a recording runs,
 * also to the recording's audio file. The blocking speaker write paces the
 * pump in real time; without an output line it paces itself on the clock.
 * Portable (same code on Windows/Linux/macOS) and sample-aligned with what the
 * game played, unlike any OS-level capture.
 *
 * <p>Wired by the 26.x {@code AgentAudioLibraryMixin}. Disabled with
 * {@code -Despectral.agent.audio=0}; any failure falls back to the normal
 * device (recordings then report {@code audio: "none"}).
 */
public final class AgentAudio {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    public static final int SAMPLE_RATE = 48_000;
    public static final int CHANNELS = 2;
    /** 16-bit signed little-endian interleaved stereo. */
    public static final int BYTES_PER_FRAME = 4;
    /** Frames per pump block (10 ms). */
    private static final int BLOCK_FRAMES = SAMPLE_RATE / 100;

    private static volatile long device;
    private static volatile boolean running;
    private static Thread pump;
    /** Live copies of the mix: a live recording and/or a replay audio sidecar. */
    private static final java.util.List<Sink> SINKS = new java.util.concurrent.CopyOnWriteArrayList<>();
    private static volatile String failure;

    private AgentAudio() {}

    /** Loopback device for {@code Library.init}, or 0 to open the real device. */
    public static long openLoopback() {
        if (!AgentRuntime.enabled() || "0".equals(System.getProperty("espectral.agent.audio"))) {
            return 0L;
        }
        try {
            if (!ALC10.alcIsExtensionPresent(0L, "ALC_SOFT_loopback")) {
                failure = "OpenAL has no ALC_SOFT_loopback";
                return 0L;
            }
            long handle = SOFTLoopback.alcLoopbackOpenDeviceSOFT((ByteBuffer) null);
            if (handle == 0L) {
                failure = "could not open an OpenAL loopback device";
                return 0L;
            }
            if (!SOFTLoopback.alcIsRenderFormatSupportedSOFT(handle, SAMPLE_RATE,
                    SOFTLoopback.ALC_STEREO_SOFT, SOFTLoopback.ALC_SHORT_SOFT)) {
                ALC10.alcCloseDevice(handle);
                failure = "loopback device rejects 48 kHz stereo 16-bit";
                return 0L;
            }
            device = handle;
            failure = null;
            LOGGER.info("agent: sound engine on an OpenAL loopback device (game audio is capturable)");
            return handle;
        } catch (Throwable t) {
            failure = "loopback unavailable: " + t;
            LOGGER.warn("agent: {}", failure);
            return 0L;
        }
    }

    /** True between a successful {@link #openLoopback} and {@code Library.cleanup}. */
    public static boolean loopbackOpen() {
        return device != 0L;
    }

    /**
     * Context attributes for the loopback device: the render format first
     * (mandatory for loopback contexts), then vanilla's own attributes.
     */
    public static IntBuffer attributes(MemoryStack stack, IntBuffer vanilla) {
        int pairs = 0;
        if (vanilla != null) {
            for (int i = vanilla.position(); i + 1 < vanilla.limit() && vanilla.get(i) != 0; i += 2) {
                pairs++;
            }
        }
        IntBuffer out = stack.callocInt(6 + pairs * 2 + 1);
        out.put(SOFTLoopback.ALC_FORMAT_CHANNELS_SOFT).put(SOFTLoopback.ALC_STEREO_SOFT);
        out.put(SOFTLoopback.ALC_FORMAT_TYPE_SOFT).put(SOFTLoopback.ALC_SHORT_SOFT);
        out.put(ALC10.ALC_FREQUENCY).put(SAMPLE_RATE);
        for (int i = 0; i < pairs * 2; i++) {
            out.put(vanilla.get(vanilla.position() + i));
        }
        out.put(0);
        return out.flip();
    }

    /** Starts the pump once the loopback context exists ({@code Library.init} tail). */
    public static synchronized void startPump() {
        if (device == 0L || running) return;
        running = true;
        long handle = device;
        pump = new Thread(() -> pumpLoop(handle), "espectral-agent-audio");
        pump.setDaemon(true);
        pump.setPriority(Thread.MAX_PRIORITY);
        pump.start();
    }

    /** Stops the pump before the device closes ({@code Library.cleanup} head). */
    public static synchronized void stopPump() {
        running = false;
        Thread thread = pump;
        pump = null;
        if (thread != null) {
            try {
                thread.join(2000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
        device = 0L;
    }

    /** Null when game audio can be recorded, else why not. */
    public static String unavailableReason() {
        if (running && device != 0L) return null;
        return failure != null ? failure : "the game's sound engine is not on the loopback device";
    }

    /** One copy of the mix into a raw s16le 48 kHz stereo stream. */
    public static final class Sink {
        final OutputStream out;
        volatile long frames;
        /** Wall clock (epoch ms) of the first frame written. */
        public final long startedAtMs = System.currentTimeMillis();

        Sink(OutputStream out) {
            this.out = out;
        }
    }

    /** Starts copying the mix into {@code out} from the next 10 ms block. */
    public static Sink attach(OutputStream out) {
        Sink sink = new Sink(out);
        SINKS.add(sink);
        return sink;
    }

    /** Stops copying; returns the frames written since {@link #attach}. */
    public static long detach(Sink sink) {
        if (sink == null) return 0L;
        SINKS.remove(sink);
        return sink.frames;
    }

    private static void pumpLoop(long handle) {
        ByteBuffer native_ = MemoryUtil.memAlloc(BLOCK_FRAMES * BYTES_PER_FRAME);
        byte[] block = new byte[BLOCK_FRAMES * BYTES_PER_FRAME];
        SourceDataLine line = openLine();
        long start = System.nanoTime();
        long rendered = 0L;
        try {
            while (running) {
                native_.clear();
                SOFTLoopback.alcRenderSamplesSOFT(handle, native_, BLOCK_FRAMES);
                native_.get(block);
                for (Sink sink : SINKS) {
                    try {
                        sink.out.write(block);
                        sink.frames += BLOCK_FRAMES;
                    } catch (IOException e) {
                        SINKS.remove(sink);
                        LOGGER.warn("agent: recording audio write failed: {}", e.toString());
                    }
                }
                rendered += BLOCK_FRAMES;
                if (line != null) {
                    line.write(block, 0, block.length); // blocks: real-time pacing
                } else {
                    long due = start + rendered * 1_000_000_000L / SAMPLE_RATE;
                    long wait = due - System.nanoTime();
                    if (wait > 0) {
                        java.util.concurrent.locks.LockSupport.parkNanos(wait);
                    }
                }
            }
        } catch (Throwable t) {
            failure = "audio pump stopped: " + t;
            LOGGER.warn("agent: {}", failure);
            running = false;
        } finally {
            if (line != null) {
                line.stop();
                line.close();
            }
            MemoryUtil.memFree(native_);
        }
    }

    /** Speaker output so the player keeps hearing the game; null when unavailable. */
    private static SourceDataLine openLine() {
        try {
            AudioFormat format = new AudioFormat(SAMPLE_RATE, 16, CHANNELS, true, false);
            SourceDataLine line = AudioSystem.getSourceDataLine(format);
            // ~80 ms buffer: low enough latency for play, enough to ride GC pauses.
            line.open(format, SAMPLE_RATE * BYTES_PER_FRAME * 8 / 100);
            line.start();
            return line;
        } catch (Throwable t) {
            LOGGER.info("agent: no speaker output for the loopback mix ({}); audio stays capturable",
                    t.toString());
            return null;
        }
    }
}
