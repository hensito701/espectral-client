package es.spectral.menu.agent;

import java.io.BufferedOutputStream;
import java.io.File;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonObject;
import com.mojang.blaze3d.pipeline.RenderTarget;

import net.minecraft.client.Minecraft;

import es.spectral.menu.Compat;

/**
 * {@code record_start} / {@code record_stop}: live capture of what the game
 * renders, with the game's own audio.
 *
 * <p>Video: at every end of frame the main render target (Iris composite
 * included) is read back asynchronously when the wall clock has reached the
 * next output frame slot, then a writer thread streams raw RGBA into ffmpeg
 * (H.264 yuv420p CRF 18). Slots the game did not render in time are filled
 * with the previous frame, so the file always runs at the requested constant
 * frame rate in step with the wall clock — and the audio. {@code dropped}
 * counts those repeats; {@code achieved_fps} is the unique-frame rate.
 *
 * <p>Size: presets {@code short}/{@code landscape} render the world at their
 * own size for the whole recording (the same {@code Window.setWidth} route as
 * custom-size screenshots, 26.x only); {@code evidence} records the window.
 *
 * <p>Audio: raw PCM from {@link AgentAudio} while recording, muxed as AAC at
 * stop. When the loopback tap is unavailable the video is still produced and
 * {@code audio} says why.
 */
public final class AgentRecorder {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");
    private static final DateTimeFormatter STAMP = DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss-SSS");

    /** Readbacks allowed in flight + queued before frames are skipped. */
    private static final int MAX_PENDING = 6;

    private record Preset(int width, int height, int fps, boolean hud, int maxSeconds) {}

    private static Session active;
    /** Result of a recording that stopped on its own (time limit), for the next record_stop. */
    private static CompletableFuture<JsonObject> finished;

    private AgentRecorder() {}

    public static boolean recording() {
        return active != null;
    }

    /** True while a recording renders at its own size (custom-size screenshots must wait). */
    public static boolean sized() {
        Session session = active;
        return session != null && session.saved != null;
    }

    private static Preset preset(Minecraft minecraft, String name) throws AgentException {
        return switch (name) {
            case "short" -> new Preset(1080, 1920, 60, false, 60);
            case "landscape" -> new Preset(1920, 1080, 60, false, 300);
            case "evidence" -> new Preset(0, 0, 30, true, 600);
            default -> throw new AgentException("BAD_ARGS",
                    "unknown preset '" + name + "' (short, landscape, evidence)");
        };
    }

    // ------------------------------------------------------------------
    // record_start (render thread)
    // ------------------------------------------------------------------

    public static JsonObject start(Minecraft minecraft, AgentArgs args) throws AgentException {
        if (active != null) throw new AgentException("BUSY", "a recording is already running; record_stop first");
        Preset preset = preset(minecraft, args.string("preset", "evidence"));
        int width = args.optInt("width", preset.width());
        int height = args.optInt("height", preset.height());
        if (args.contains("width") != args.contains("height")) {
            throw new AgentException("BAD_ARGS", "give both width and height, or neither");
        }
        int fps = args.optInt("fps", preset.fps());
        if (fps < 1 || fps > 120) throw new AgentException("BAD_ARGS", "fps must be 1…120");
        boolean wantAudio = args.bool("audio", true);
        boolean hud = args.bool("hud", preset.hud());

        var window = minecraft.getWindow();
        boolean sized = width > 0 && height > 0
                && (width != window.getWidth() || height != window.getHeight());
        if (width <= 0 || height <= 0) {
            width = window.getWidth();
            height = window.getHeight();
        }
        if (width < AgentCapture.MIN_SIDE || height < AgentCapture.MIN_SIDE
                || width > AgentCapture.MAX_SIDE || height > AgentCapture.MAX_SIDE) {
            throw new AgentException("BAD_ARGS", "width and height must be "
                    + AgentCapture.MIN_SIDE + "…" + AgentCapture.MAX_SIDE + " px");
        }
        if (sized && !Compat.supportsSizedCapture()) {
            throw new AgentException("UNSUPPORTED",
                    "recording at a custom size needs Minecraft 26.2 or newer; use preset evidence");
        }
        if (sized && Compat.getScreen(minecraft) != null) {
            throw new AgentException("BAD_ARGS", "a sized recording renders the world view; "
                    + "close the open screen first (key escape)");
        }
        // yuv420p needs even sides.
        width &= ~1;
        height &= ~1;

        String ffmpeg = ffmpegPath();
        Path dir = AgentCapture.capturesDir(minecraft);
        String stamp = LocalDateTime.now().format(STAMP);
        Session session = new Session(dir.resolve("rec-" + stamp + ".mp4"),
                dir.resolve("rec-" + stamp + ".video.mp4"), dir.resolve("rec-" + stamp + ".pcm"),
                dir.resolve("rec-" + stamp + ".ffmpeg.log"), ffmpeg, width, height, fps,
                preset.maxSeconds());
        try {
            Files.createDirectories(dir);
            session.startEncoder();
        } catch (IOException e) {
            throw new AgentException("NO_FFMPEG", "could not start ffmpeg ('" + ffmpeg + "'): "
                    + e.getMessage());
        }

        if (sized) {
            session.saved = new int[] {window.getWidth(), window.getHeight(), window.getGuiScale()};
            window.setWidth(width);
            window.setHeight(height);
            window.setGuiScale(window.calculateScale(minecraft.options.guiScale().get(),
                    minecraft.isEnforceUnicode()));
        }
        session.hudWas = Compat.hudHidden(minecraft);
        Compat.setHudHidden(minecraft, !hud);

        if (wantAudio) {
            String reason = AgentAudio.unavailableReason();
            if (reason == null) {
                try {
                    session.audioOut = new BufferedOutputStream(Files.newOutputStream(session.pcm), 1 << 16);
                    session.audioNote = "game";
                } catch (IOException e) {
                    session.audioNote = "none: " + e.getMessage();
                }
            } else {
                session.audioNote = "none: " + reason;
            }
        } else {
            session.audioNote = "none: disabled (audio:false)";
        }
        session.startNs = System.nanoTime();
        if (session.audioOut != null) session.audioSink = AgentAudio.attach(session.audioOut);
        active = session;
        finished = null;

        JsonObject json = new JsonObject();
        json.addProperty("video_path", AgentCapture.relative(session.out));
        json.addProperty("width", width);
        json.addProperty("height", height);
        json.addProperty("fps", fps);
        json.addProperty("audio", session.audioNote);
        return json;
    }

    // ------------------------------------------------------------------
    // Frames (render thread, end of frame)
    // ------------------------------------------------------------------

    public static void onFrameEnd(Minecraft minecraft) {
        Session session = active;
        if (session == null) return;
        long now = System.nanoTime();
        if (now - session.startNs > session.maxSeconds * 1_000_000_000L) {
            LOGGER.info("agent: recording reached its {} s limit, stopping", session.maxSeconds);
            finished = stopNow(minecraft);
            return;
        }
        if (session.failed != null) return;
        session.gameFrames++;
        long slot = (now - session.startNs) * session.fps / 1_000_000_000L;
        if (slot <= session.lastSlot) return;
        RenderTarget target = Compat.mainRenderTarget(minecraft);
        if (target == null || target.width < session.width || target.height < session.height) {
            return; // e.g. window resized smaller mid-recording: that slot repeats
        }
        if (session.pending.get() >= MAX_PENDING) {
            session.encoderBusy++;
            return; // encoder behind: the slot repeats the previous frame
        }
        session.lastSlot = slot;
        session.pending.incrementAndGet();
        final long frameSlot = slot;
        byte[] bytes = session.pool.poll();
        if (bytes == null) bytes = new byte[session.frameBytes];
        final byte[] frame = bytes;
        try {
            AgentReadback.read(target, session.width, session.height, frame,
                    () -> session.accept(frameSlot, frame),
                    error -> session.reject(frame, "frame readback failed: " + error));
        } catch (Throwable t) {
            session.reject(frame, "frame readback failed: " + t);
        }
    }

    // ------------------------------------------------------------------
    // record_stop
    // ------------------------------------------------------------------

    /** Stops on the render thread; the future completes once the MP4 is muxed. */
    public static CompletableFuture<JsonObject> stop(Minecraft minecraft) {
        CompletableFuture<JsonObject> result = new CompletableFuture<>();
        minecraft.execute(() -> {
            if (active == null) {
                CompletableFuture<JsonObject> done = finished;
                finished = null;
                if (done != null) {
                    done.whenComplete((json, error) -> {
                        if (error != null) result.completeExceptionally(error);
                        else result.complete(json);
                    });
                } else {
                    result.completeExceptionally(new AgentException("BAD_ARGS", "no recording is running"));
                }
                return;
            }
            stopNow(minecraft).whenComplete((json, error) -> {
                if (error != null) result.completeExceptionally(error);
                else result.complete(json);
            });
        });
        return result;
    }

    private static CompletableFuture<JsonObject> stopNow(Minecraft minecraft) {
        Session session = active;
        active = null;
        long stopNs = System.nanoTime();
        long audioFrames = AgentAudio.detach(session.audioSink);
        if (session.saved != null) {
            var window = minecraft.getWindow();
            window.setWidth(session.saved[0]);
            window.setHeight(session.saved[1]);
            window.setGuiScale(session.saved[2]);
        }
        Compat.setHudHidden(minecraft, session.hudWas);
        long lastSlot = Math.max(0L, (stopNs - session.startNs) * session.fps / 1_000_000_000L - 1);
        return CompletableFuture.supplyAsync(() -> {
            try {
                return session.finish(lastSlot, (stopNs - session.startNs) / 1e9, audioFrames);
            } catch (AgentException e) {
                throw new java.util.concurrent.CompletionException(e);
            }
        });
    }

    // ------------------------------------------------------------------

    static String ffmpegPath() {
        String configured = System.getProperty("espectral.ffmpeg");
        if (configured != null && !configured.isBlank() && new File(configured).canExecute()) {
            return configured;
        }
        return "ffmpeg";
    }

    /** One H.264 encoder: ffmpeg args before the input, and output args. */
    private record Encoder(String name, List<String> beforeInput, List<String> output) {}

    /**
     * Software fallback. {@code superfast}: the game and the encoder share the
     * CPU during a live recording, so speed beats compression here.
     */
    private static final Encoder X264 = new Encoder("libx264", List.of(),
            List.of("-c:v", "libx264", "-preset", "superfast", "-crf", "18", "-pix_fmt", "yuv420p"));

    private static volatile Encoder encoder = X264;
    private static boolean probed;

    /**
     * Picks a hardware H.264 encoder when ffmpeg can open one (VAAPI, NVENC,
     * AMF, Quick Sync, VideoToolbox) — a live recording then costs the game
     * almost no CPU. Runs once, off-thread, at relay start; until it finishes
     * (or when nothing works) recordings use libx264.
     */
    static synchronized void probeEncodersAsync() {
        if (probed) return;
        probed = true;
        Thread thread = new Thread(() -> {
            String ffmpeg = ffmpegPath();
            for (Encoder candidate : hardwareCandidates()) {
                if (probe(ffmpeg, candidate)) {
                    encoder = candidate;
                    LOGGER.info("agent: live recordings use {}", candidate.name());
                    return;
                }
            }
            LOGGER.info("agent: no hardware H.264 encoder; live recordings use libx264");
        }, "espectral-agent-encoder-probe");
        thread.setDaemon(true);
        thread.start();
    }

    private static List<Encoder> hardwareCandidates() {
        String os = System.getProperty("os.name", "").toLowerCase(java.util.Locale.ROOT);
        List<Encoder> list = new ArrayList<>();
        if (os.contains("mac")) {
            list.add(new Encoder("h264_videotoolbox", List.of(),
                    List.of("-c:v", "h264_videotoolbox", "-q:v", "65", "-pix_fmt", "yuv420p")));
            return list;
        }
        list.add(new Encoder("h264_nvenc", List.of(),
                List.of("-c:v", "h264_nvenc", "-preset", "p4", "-cq", "20", "-pix_fmt", "yuv420p")));
        if (os.contains("win")) {
            list.add(new Encoder("h264_amf", List.of(), List.of("-c:v", "h264_amf", "-rc", "cqp",
                    "-qp_i", "20", "-qp_p", "20", "-pix_fmt", "yuv420p")));
        } else if (new File("/dev/dri/renderD128").exists()) {
            // RGBA→NV12 on the GPU (scale_vaapi): a third of the CPU of swscale.
            list.add(new Encoder("h264_vaapi", List.of("-vaapi_device", "/dev/dri/renderD128"),
                    List.of("-vf", "hwupload,scale_vaapi=format=nv12", "-c:v", "h264_vaapi", "-qp", "20")));
        }
        list.add(new Encoder("h264_qsv", List.of(),
                List.of("-c:v", "h264_qsv", "-global_quality", "20", "-pix_fmt", "nv12")));
        return list;
    }

    /** True when ffmpeg encodes a few frames with {@code candidate}. */
    private static boolean probe(String ffmpeg, Encoder candidate) {
        List<String> cmd = new ArrayList<>(List.of(ffmpeg, "-hide_banner", "-loglevel", "error"));
        cmd.addAll(candidate.beforeInput());
        // RGBA like the real input, so the encoder's filter chain is probed too.
        cmd.addAll(List.of("-f", "lavfi", "-i", "color=black:s=320x240:r=30:d=0.3,format=rgba"));
        cmd.addAll(candidate.output());
        cmd.addAll(List.of("-frames:v", "5", "-f", "null", "-"));
        try {
            Process process = new ProcessBuilder(cmd).redirectErrorStream(true)
                    .redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
            if (!process.waitFor(10, TimeUnit.SECONDS)) {
                process.destroyForcibly();
                return false;
            }
            return process.exitValue() == 0;
        } catch (IOException e) {
            return false;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    private static final class Session {
        final Path out;
        final Path video;
        final Path pcm;
        final Path log;
        final String ffmpeg;
        final int width;
        final int height;
        final int fps;
        final int maxSeconds;
        final int frameBytes;
        String encoderName;
        final java.util.concurrent.atomic.AtomicInteger pending = new java.util.concurrent.atomic.AtomicInteger();
        final BlockingQueue<Object[]> queue = new ArrayBlockingQueue<>(MAX_PENDING + 2);
        final BlockingQueue<byte[]> pool = new ArrayBlockingQueue<>(MAX_PENDING + 3);
        Process process;
        OutputStream stdin;
        Thread writer;
        int[] saved;
        boolean hudWas;
        OutputStream audioOut;
        AgentAudio.Sink audioSink;
        String audioNote;
        long startNs;
        long lastSlot = -1L;
        /** Frames the game rendered while recording, and slots skipped because the encoder lagged. */
        long gameFrames;
        long encoderBusy;
        volatile String failed;
        // writer-thread state
        long nextSlot;
        long unique;
        long repeats;
        byte[] previous;
        volatile long endSlot = Long.MAX_VALUE;

        Session(Path out, Path video, Path pcm, Path log, String ffmpeg, int width, int height, int fps,
                int maxSeconds) {
            this.out = out;
            this.video = video;
            this.pcm = pcm;
            this.log = log;
            this.ffmpeg = ffmpeg;
            this.width = width;
            this.height = height;
            this.fps = fps;
            this.maxSeconds = maxSeconds;
            this.frameBytes = width * height * 4;
        }

        void startEncoder() throws IOException {
            Encoder chosen = encoder;
            encoderName = chosen.name();
            List<String> cmd = new ArrayList<>(List.of(ffmpeg, "-hide_banner", "-loglevel", "error", "-y"));
            cmd.addAll(chosen.beforeInput());
            cmd.addAll(List.of("-f", "rawvideo", "-pix_fmt", "rgba", "-s", width + "x" + height,
                    "-framerate", Integer.toString(fps), "-i", "-", "-an"));
            cmd.addAll(chosen.output());
            cmd.addAll(List.of("-movflags", "+faststart", video.toString()));
            process = new ProcessBuilder(cmd).redirectErrorStream(true)
                    .redirectOutput(log.toFile()).start();
            stdin = process.getOutputStream();
            writer = new Thread(this::writeLoop, "espectral-agent-record");
            writer.setDaemon(true);
            writer.start();
        }

        /** Readback done (render thread): hand the frame to the writer. */
        void accept(long slot, byte[] frame) {
            if (!queue.offer(new Object[] {slot, frame})) {
                pool.offer(frame);
                encoderBusy++; // writer behind: the slot repeats the previous frame
            }
            pending.decrementAndGet();
        }

        void reject(byte[] frame, String why) {
            failed = why;
            pool.offer(frame);
            pending.decrementAndGet();
        }

        private void writeLoop() {
            try {
                while (true) {
                    Object[] item = queue.poll(100, TimeUnit.MILLISECONDS);
                    if (item == null) {
                        // Stopped and drained: nothing more can arrive.
                        if (endSlot != Long.MAX_VALUE && pending.get() == 0 && queue.isEmpty()) break;
                        continue;
                    }
                    long slot = (Long) item[0];
                    byte[] frame = (byte[]) item[1];
                    if (slot > endSlot) {
                        pool.offer(frame);
                        continue;
                    }
                    // Repeat the previous frame for slots the game skipped.
                    while (previous != null && nextSlot < slot) {
                        stdin.write(previous);
                        repeats++;
                        nextSlot++;
                    }
                    stdin.write(frame);
                    unique++;
                    nextSlot = slot + 1;
                    if (previous != null) pool.offer(previous);
                    previous = frame;
                }
                // Pad to the stop time so video and audio end together.
                while (previous != null && nextSlot <= endSlot) {
                    stdin.write(previous);
                    repeats++;
                    nextSlot++;
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            } catch (IOException e) {
                failed = "ffmpeg stopped accepting frames: " + e.getMessage();
            } finally {
                try {
                    stdin.close();
                } catch (IOException ignored) {
                    // ffmpeg already gone
                }
            }
        }

        JsonObject finish(long lastSlot, double seconds, long audioFrames) throws AgentException {
            endSlot = lastSlot;
            try {
                // Wait for in-flight readbacks (they complete on the render thread).
                long deadline = System.nanoTime() + 5_000_000_000L;
                while (pending.get() > 0 && System.nanoTime() < deadline) Thread.sleep(10);
                writer.join(30_000);
                if (!process.waitFor(120, TimeUnit.SECONDS)) {
                    process.destroyForcibly();
                    throw new AgentException("FAILED", "ffmpeg did not finish encoding");
                }
                if (audioOut != null) audioOut.close();
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new AgentException("FAILED", "interrupted while finishing the recording");
            } catch (IOException e) {
                audioNote = "none: " + e.getMessage();
            }
            if (process.exitValue() != 0 || !Files.isRegularFile(video)) {
                throw new AgentException("FAILED", "ffmpeg failed (exit " + process.exitValue() + "): "
                        + tail(log));
            }
            try {
                if (audioOut != null && audioFrames > 0) {
                    mux();
                    Files.deleteIfExists(video);
                } else {
                    if (audioOut != null) audioNote = "none: no audio frames were rendered";
                    Files.move(video, out, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                }
                Files.deleteIfExists(pcm);
                Files.deleteIfExists(log);
            } catch (IOException | InterruptedException e) {
                throw new AgentException("FAILED", "could not finish " + out.getFileName() + ": " + e.getMessage());
            }
            JsonObject json = new JsonObject();
            json.addProperty("video_path", AgentCapture.relative(out));
            json.addProperty("duration_s", Math.round(seconds * 100) / 100.0);
            json.addProperty("frames", unique + repeats);
            json.addProperty("dropped", repeats);
            json.addProperty("achieved_fps", seconds > 0 ? Math.round(unique / seconds * 10) / 10.0 : 0);
            // Why frames repeat: the game rendering slower than fps, or the encoder lagging.
            json.addProperty("game_fps", seconds > 0 ? Math.round(gameFrames / seconds * 10) / 10.0 : 0);
            json.addProperty("encoder_busy", encoderBusy);
            json.addProperty("encoder", encoderName);
            json.addProperty("width", width);
            json.addProperty("height", height);
            json.addProperty("fps", fps);
            json.addProperty("audio", audioNote);
            if (failed != null) json.addProperty("warning", failed);
            return json;
        }

        private void mux() throws IOException, InterruptedException, AgentException {
            List<String> cmd = List.of(ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
                    "-i", video.toString(),
                    "-f", "s16le", "-ar", Integer.toString(AgentAudio.SAMPLE_RATE),
                    "-ac", Integer.toString(AgentAudio.CHANNELS), "-i", pcm.toString(),
                    "-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k",
                    "-shortest", "-movflags", "+faststart", out.toString());
            Process mux = new ProcessBuilder(cmd).redirectErrorStream(true).redirectOutput(log.toFile()).start();
            if (!mux.waitFor(120, TimeUnit.SECONDS)) {
                mux.destroyForcibly();
                throw new AgentException("FAILED", "ffmpeg did not finish muxing the audio");
            }
            if (mux.exitValue() != 0) {
                throw new AgentException("FAILED", "audio mux failed: " + tail(log));
            }
        }

        private static String tail(Path file) {
            try {
                String text = Files.readString(file).strip();
                return text.length() > 400 ? text.substring(text.length() - 400) : text;
            } catch (IOException e) {
                return "(no ffmpeg log)";
            }
        }
    }
}
