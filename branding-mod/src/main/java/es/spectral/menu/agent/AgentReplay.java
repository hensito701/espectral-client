package es.spectral.menu.agent;

import java.io.BufferedOutputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import net.minecraft.client.Minecraft;

import es.spectral.menu.Compat;

/**
 * Cinematic clips through Replay Mod (GPL-3, soft dependency: reflection only,
 * never bundled): {@code replays}, {@code clip_render}, {@code clip_status}.
 *
 * <p>{@code clip_render} opens a finished {@code .mcpr}, builds a Simple Pathing
 * timeline from an orbit or a point path (1× replay time), and renders it
 * offline and frame-locked with Replay Mod's {@code VideoRenderer} — shaders
 * included — through our ffmpeg. A slow GPU only makes the render slower.
 *
 * <p>Replay Mod renders silently, so audio comes from a sidecar: while Replay
 * Mod records a world in an agent launch, the game's mix is copied to
 * {@code espectral-captures/replay-audio/<epoch ms>.pcm}; the clip's replay
 * time range is cut from it (aligned by the replay's start date) and muxed.
 */
public final class AgentReplay {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");
    private static final DateTimeFormatter STAMP = DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss-SSS");

    private static final String REPLAY_MOD = "com.replaymod.core.ReplayMod";
    private static final String REPLAY = "com.replaymod.replay.ReplayModReplay";
    private static final String SP_TIMELINE = "com.replaymod.simplepathing.SPTimeline";
    private static final String INTERPOLATOR = "com.replaymod.simplepathing.InterpolatorType";
    private static final String SETTINGS = "com.replaymod.render.RenderSettings";
    private static final String RENDERER = "com.replaymod.render.rendering.VideoRenderer";
    private static final String TIMELINE = "com.replaymod.replaystudio.pathing.path.Timeline";
    private static final String HANDLER = "com.replaymod.replay.ReplayHandler";
    private static final String COLOR = "com.replaymod.lib.de.johni0702.minecraft.gui.utils.lwjgl.ReadableColor";

    /** Seconds of video between generated orbit keyframes. */
    private static final double ORBIT_STEP_S = 0.25;
    /** Max wait for a replay world to load before rendering. */
    private static final long LOAD_TIMEOUT_MS = 90_000L;
    /** Sidecars older than this are deleted when a new one starts. */
    private static final long SIDECAR_KEEP_MS = 7L * 24 * 3600 * 1000;

    private static final Map<String, Job> JOBS = new ConcurrentHashMap<>();
    private static volatile Job current;

    private static AgentAudio.Sink sidecar;
    private static OutputStream sidecarOut;

    private AgentReplay() {}

    public static boolean available() {
        try {
            Class.forName(REPLAY);
            return true;
        } catch (ClassNotFoundException e) {
            return false;
        }
    }

    /** True while a clip is loading or rendering (the render thread belongs to it). */
    public static boolean busy() {
        Job job = current;
        return job != null && !job.future.isDone();
    }

    // ------------------------------------------------------------------
    // replays
    // ------------------------------------------------------------------

    public static JsonObject list(Minecraft minecraft) throws AgentException {
        requireReplayMod();
        JsonArray replays = new JsonArray();
        for (Path file : replayFiles(minecraft)) {
            JsonObject meta = meta(file);
            if (meta == null) continue;
            JsonObject entry = new JsonObject();
            entry.addProperty("name", stem(file));
            entry.addProperty("duration_s", meta.get("duration").getAsLong() / 1000.0);
            entry.addProperty("date", Instant.ofEpochMilli(meta.get("date").getAsLong()).toString());
            entry.addProperty("world", meta.has("serverName") ? meta.get("serverName").getAsString() : null);
            entry.addProperty("audio", sidecarFor(minecraft, meta.get("date").getAsLong()) != null);
            replays.add(entry);
        }
        JsonObject json = new JsonObject();
        json.add("replays", replays);
        return json;
    }

    private static List<Path> replayFiles(Minecraft minecraft) throws AgentException {
        Path dir = minecraft.gameDirectory.toPath().resolve("replay_recordings");
        List<Path> files = new ArrayList<>();
        if (!Files.isDirectory(dir)) return files;
        // Replay Mod keeps fresh recordings in a subfolder (replay_recordings/
        // recording/) and finished/organised ones at the top level; partial
        // saves are .mcpr.tmp / .mcpr.del folders, which isRegularFile skips.
        try (Stream<Path> entries = Files.walk(dir, 2)) {
            entries.filter(p -> p.getFileName().toString().endsWith(".mcpr") && Files.isRegularFile(p))
                    .sorted((a, b) -> b.getFileName().toString().compareTo(a.getFileName().toString()))
                    .forEach(files::add);
        } catch (IOException e) {
            throw new AgentException("FAILED", "cannot list replay_recordings: " + e.getMessage());
        }
        return files;
    }

    private static JsonObject meta(Path mcpr) {
        try (ZipFile zip = new ZipFile(mcpr.toFile())) {
            ZipEntry entry = zip.getEntry("metaData.json");
            if (entry == null) return null;
            try (InputStream in = zip.getInputStream(entry)) {
                JsonElement json = JsonParser.parseString(new String(in.readAllBytes(), StandardCharsets.UTF_8));
                JsonObject meta = json.getAsJsonObject();
                return meta.has("duration") && meta.has("date") ? meta : null;
            }
        } catch (Exception e) {
            return null;
        }
    }

    private static String stem(Path file) {
        String name = file.getFileName().toString();
        return name.endsWith(".mcpr") ? name.substring(0, name.length() - 5) : name;
    }

    // ------------------------------------------------------------------
    // Audio sidecar (render thread, every client tick)
    // ------------------------------------------------------------------

    /** Copies the mix while Replay Mod records a world in this agent launch. */
    public static void onClientTick(Minecraft minecraft) {
        boolean recordingWorld = minecraft.level != null && available() && !inReplay();
        if (recordingWorld && sidecar == null && AgentAudio.unavailableReason() == null) {
            startSidecar(minecraft);
        } else if (!recordingWorld && sidecar != null) {
            stopSidecar();
        }
    }

    private static void startSidecar(Minecraft minecraft) {
        try {
            Path dir = sidecarDir(minecraft);
            Files.createDirectories(dir);
            pruneSidecars(dir);
            long now = System.currentTimeMillis();
            sidecarOut = new BufferedOutputStream(Files.newOutputStream(dir.resolve(now + ".pcm")), 1 << 16);
            sidecar = AgentAudio.attach(sidecarOut);
        } catch (IOException e) {
            LOGGER.warn("agent: replay audio sidecar unavailable: {}", e.toString());
        }
    }

    private static void stopSidecar() {
        AgentAudio.detach(sidecar);
        sidecar = null;
        try {
            sidecarOut.close();
        } catch (IOException ignored) {
            // best effort
        }
        sidecarOut = null;
    }

    private static Path sidecarDir(Minecraft minecraft) {
        return AgentCapture.capturesDir(minecraft).resolve("replay-audio");
    }

    private static void pruneSidecars(Path dir) {
        long cutoff = System.currentTimeMillis() - SIDECAR_KEEP_MS;
        try (Stream<Path> files = Files.list(dir)) {
            files.filter(p -> sidecarStart(p) > 0 && sidecarStart(p) < cutoff).forEach(p -> {
                try {
                    Files.deleteIfExists(p);
                } catch (IOException ignored) {
                    // best effort
                }
            });
        } catch (IOException ignored) {
            // best effort
        }
    }

    private static long sidecarStart(Path file) {
        String name = file.getFileName().toString();
        if (!name.endsWith(".pcm")) return -1L;
        try {
            return Long.parseLong(name.substring(0, name.length() - 4));
        } catch (NumberFormatException e) {
            return -1L;
        }
    }

    /** The sidecar that started closest to the replay's start date (within 15 s). */
    private static Path sidecarFor(Minecraft minecraft, long replayDate) {
        Path dir = sidecarDir(minecraft);
        if (!Files.isDirectory(dir)) return null;
        try (Stream<Path> files = Files.list(dir)) {
            return files.filter(p -> sidecarStart(p) > 0 && Math.abs(sidecarStart(p) - replayDate) < 15_000L)
                    .min((a, b) -> Long.compare(Math.abs(sidecarStart(a) - replayDate),
                            Math.abs(sidecarStart(b) - replayDate)))
                    .orElse(null);
        } catch (IOException e) {
            return null;
        }
    }

    // ------------------------------------------------------------------
    // clip_render (render thread)
    // ------------------------------------------------------------------

    public static JsonObject start(Minecraft minecraft, AgentArgs args) throws AgentException {
        requireReplayMod();
        if (busy()) throw new AgentException("BUSY", "a clip is already rendering (job " + current.id + ")");
        if (AgentRecorder.recording()) throw new AgentException("BUSY", "a live recording is running; record_stop first");
        if (minecraft.level != null) {
            throw new AgentException("BAD_ARGS", "leave the world first (disconnect): a replay opens its own world, "
                    + "and Replay Mod only saves the recording of a world you left");
        }
        String name = args.requiredString("replay");
        Path file = null;
        for (Path candidate : replayFiles(minecraft)) {
            if (stem(candidate).equals(name)) file = candidate;
        }
        if (file == null) throw new AgentException("BAD_ARGS", "no replay '" + name + "' (see the replays tool)");
        JsonObject meta = meta(file);
        if (meta == null) throw new AgentException("FAILED", "unreadable replay " + name);
        long replayMs = meta.get("duration").getAsLong();

        String presetName = args.string("preset", "short");
        int width;
        int height;
        int fps;
        switch (presetName) {
            case "short" -> { width = 1080; height = 1920; fps = 60; }
            case "landscape" -> { width = 1920; height = 1080; fps = 60; }
            default -> throw new AgentException("BAD_ARGS", "clip presets: short, landscape");
        }
        fps = args.optInt("fps", fps);
        if (fps < 1 || fps > 120) throw new AgentException("BAD_ARGS", "fps must be 1…120");

        double startS = args.has("start_s") ? number(args, "start_s") : 0.0;
        double durationS = number(args, "duration_s");
        if (startS < 0 || durationS <= 0 || durationS > 120) {
            throw new AgentException("BAD_ARGS", "start_s ≥ 0 and 0 < duration_s ≤ 120");
        }
        long startMs = Math.round(startS * 1000);
        long durationMs = Math.round(durationS * 1000);
        if (startMs + durationMs > replayMs) {
            throw new AgentException("BAD_ARGS", "the replay is " + replayMs / 1000.0 + " s long; "
                    + "start_s + duration_s exceeds it");
        }
        List<double[]> keyframes = camera(args, durationMs);

        if (args.has("shaders")) {
            JsonObject shaderArgs = new JsonObject();
            shaderArgs.addProperty("pack", args.requiredString("shaders"));
            try {
                AgentShaders.applyNow(new AgentArgs(shaderArgs));
            } catch (AgentException e) {
                throw e;
            } catch (Exception e) {
                throw new AgentException("FAILED", "could not apply shaders: " + rootMessage(e));
            }
        }

        Path dir = AgentCapture.capturesDir(minecraft);
        String stamp = LocalDateTime.now().format(STAMP);
        Job job = new Job("clip-" + stamp, file, dir.resolve("clip-" + stamp + ".mp4"),
                dir.resolve("clip-" + stamp + ".video.mp4"), width, height, fps, startMs, durationMs,
                keyframes, args.bool("audio", true) ? sidecarFor(minecraft, meta.get("date").getAsLong()) : null,
                meta.get("date").getAsLong());
        job.audioWanted = args.bool("audio", true);
        try {
            Files.createDirectories(dir);
            Object replay = Class.forName(REPLAY).getField("instance").get(null);
            replay.getClass().getMethod("startReplay", File.class).invoke(replay, file.toFile());
        } catch (Exception e) {
            throw new AgentException("FAILED", "Replay Mod could not open " + name + ": " + rootMessage(e));
        }
        JOBS.put(job.id, job);
        current = job;
        job.loadingSince = System.currentTimeMillis();
        AgentFrames.after(1, () -> awaitWorld(minecraft, job));

        JsonObject json = new JsonObject();
        json.addProperty("job", job.id);
        json.addProperty("total_frames", (int) (durationMs * fps / 1000));
        return json;
    }

    /** Waits (frame by frame) for the replay world, then queues the render. */
    private static void awaitWorld(Minecraft minecraft, Job job) {
        try {
            if (handler() != null && minecraft.level != null && minecraft.player != null) {
                Object timeline = timeline(job);
                Object settings = settings(job);
                Object core = Class.forName(REPLAY_MOD).getField("instance").get(null);
                core.getClass().getMethod("runLaterWithoutLock", Runnable.class)
                        .invoke(core, (Runnable) () -> render(minecraft, job, settings, timeline));
                return;
            }
            if (System.currentTimeMillis() - job.loadingSince > LOAD_TIMEOUT_MS) {
                job.fail("the replay did not load within " + LOAD_TIMEOUT_MS / 1000 + " s");
                endReplay();
                return;
            }
            AgentFrames.after(5, () -> awaitWorld(minecraft, job));
        } catch (Throwable t) {
            job.fail("could not prepare the render: " + rootMessage(t));
            endReplay();
        }
    }

    private static void render(Minecraft minecraft, Job job, Object settings, Object timeline) {
        long started = System.currentTimeMillis();
        boolean ok = false;
        try {
            try {
                Compat.open(minecraft, null); // like Replay Mod's own render queue
            } catch (Throwable ignored) {
                // the renderer hides screens itself
            }
            Constructor<?> ctor = Class.forName(RENDERER).getConstructor(Class.forName(SETTINGS),
                    Class.forName(HANDLER), Class.forName(TIMELINE));
            Object renderer = ctor.newInstance(settings, handler(), timeline);
            job.renderer = renderer;
            job.state = "rendering";
            ok = (Boolean) renderer.getClass().getMethod("renderVideo").invoke(renderer);
            if ((Boolean) renderer.getClass().getMethod("hasFailed").invoke(renderer)) ok = false;
        } catch (Throwable t) {
            job.fail("Replay Mod render failed: " + rootMessage(t));
        } finally {
            job.renderSeconds = (System.currentTimeMillis() - started) / 1000.0;
            endReplay();
        }
        if (job.future.isDone()) return;
        if (!ok || !Files.isRegularFile(job.video)) {
            job.fail("Replay Mod reported a failed render (see the game log / export.log)");
            return;
        }
        CompletableFuture.runAsync(() -> finish(job));
    }

    /** Muxes the sidecar audio for the clip's replay range (off-thread). */
    private static void finish(Job job) {
        try {
            if (job.sidecar != null) {
                // Replay time t is sidecar time t + (replay start − sidecar start).
                double offsetS = (job.startMs + (job.replayDate - sidecarStart(job.sidecar))) / 1000.0;
                double durationS = job.durationMs / 1000.0;
                String filter = offsetS >= 0
                        ? "atrim=start=" + offsetS + ":duration=" + durationS + ",asetpts=PTS-STARTPTS"
                        : "adelay=" + Math.round(-offsetS * 1000) + ":all=1,atrim=duration=" + durationS;
                List<String> cmd = List.of(AgentRecorder.ffmpegPath(), "-hide_banner", "-loglevel", "error", "-y",
                        "-i", job.video.toString(),
                        "-f", "s16le", "-ar", Integer.toString(AgentAudio.SAMPLE_RATE),
                        "-ac", Integer.toString(AgentAudio.CHANNELS), "-i", job.sidecar.toString(),
                        "-filter_complex", "[1:a]" + filter + ",apad[a]", "-map", "0:v:0", "-map", "[a]",
                        "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest",
                        "-movflags", "+faststart", job.out.toString());
                Process mux = new ProcessBuilder(cmd).redirectErrorStream(true).start();
                String log = new String(mux.getInputStream().readAllBytes(), StandardCharsets.UTF_8);
                if (!mux.waitFor(120, TimeUnit.SECONDS) || mux.exitValue() != 0) {
                    throw new IOException("audio mux failed: " + log.strip());
                }
                Files.deleteIfExists(job.video);
                job.audioNote = "game (recorded alongside the replay)";
            } else {
                Files.move(job.video, job.out, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                job.audioNote = job.audioWanted
                        ? "none: no audio sidecar for this replay (it was recorded outside an agent launch "
                                + "or without the loopback audio tap)"
                        : "none: disabled (audio:false)";
            }
            job.state = "done";
            job.future.complete(null);
        } catch (Exception e) {
            job.fail("could not finish the clip: " + e.getMessage());
        }
    }

    // ------------------------------------------------------------------
    // clip_status (any thread: never queued behind a render)
    // ------------------------------------------------------------------

    public static CompletableFuture<JsonObject> status(AgentArgs args) {
        CompletableFuture<JsonObject> result = new CompletableFuture<>();
        Job job;
        int waitS;
        try {
            job = JOBS.get(args.requiredString("job"));
            waitS = Math.max(0, Math.min(25, args.optInt("wait_s", 0)));
        } catch (AgentException e) {
            result.completeExceptionally(e);
            return result;
        }
        if (job == null) {
            result.completeExceptionally(new AgentException("BAD_ARGS", "unknown job"));
            return result;
        }
        if (waitS == 0 || job.future.isDone()) {
            result.complete(job.toJson());
        } else {
            // A copy: completing it on timeout must not mark the job itself done.
            job.future.copy().completeOnTimeout(null, waitS, TimeUnit.SECONDS)
                    .whenComplete((v, e) -> result.complete(job.toJson()));
        }
        return result;
    }

    // ------------------------------------------------------------------
    // Replay Mod plumbing (reflection)
    // ------------------------------------------------------------------

    private static void requireReplayMod() throws AgentException {
        if (!available()) {
            throw new AgentException("NO_REPLAYMOD", "Replay Mod is not installed in this instance "
                    + "(enable capture tools for it in the launcher)");
        }
    }

    private static Object handler() throws Exception {
        Object replay = Class.forName(REPLAY).getField("instance").get(null);
        return replay == null ? null : replay.getClass().getMethod("getReplayHandler").invoke(replay);
    }

    private static boolean inReplay() {
        try {
            return handler() != null;
        } catch (Throwable t) {
            return false;
        }
    }

    private static void endReplay() {
        try {
            Object handler = handler();
            if (handler != null) handler.getClass().getMethod("endReplay").invoke(handler);
        } catch (Throwable t) {
            LOGGER.warn("agent: could not close the replay: {}", t.toString());
        }
    }

    /** Simple Pathing timeline: 1× replay time, one position keyframe per camera point. */
    private static Object timeline(Job job) throws Exception {
        Class<?> spClass = Class.forName(SP_TIMELINE);
        Object sp = spClass.getConstructor().newInstance();
        Class<?> interpolator = Class.forName(INTERPOLATOR);
        Object catmull = interpolator.getMethod("valueOf", String.class).invoke(null, "CATMULL_ROM");
        spClass.getMethod("setDefaultInterpolatorType", interpolator).invoke(sp, catmull);
        Method addTime = spClass.getMethod("addTimeKeyframe", long.class, int.class);
        addTime.invoke(sp, 0L, (int) job.startMs);
        addTime.invoke(sp, job.durationMs, (int) (job.startMs + job.durationMs));
        Method addPosition = spClass.getMethod("addPositionKeyframe", long.class, double.class, double.class,
                double.class, float.class, float.class, float.class, int.class);
        for (double[] k : job.keyframes) {
            addPosition.invoke(sp, (long) k[0], k[1], k[2], k[3], (float) k[4], (float) k[5], 0f, -1);
        }
        return spClass.getMethod("getTimeline").invoke(sp);
    }

    private static Object settings(Job job) throws Exception {
        Class<?> settings = Class.forName(SETTINGS);
        Class<?> method = Class.forName(SETTINGS + "$RenderMethod");
        Class<?> preset = Class.forName(SETTINGS + "$EncodingPreset");
        Class<?> aa = Class.forName(SETTINGS + "$AntiAliasing");
        Class<?> color = Class.forName(COLOR);
        // Same input template Replay Mod's presets use; H.264 CRF 18 out
        // (offline, so a slower preset than live recording is affordable).
        String exportArgs = "-y -f rawvideo -pix_fmt bgra -s %WIDTH%x%HEIGHT% -r %FPS% -i - %FILTERS%"
                + "-an -c:v libx264 -preset medium -crf 18 -pix_fmt yuv420p -movflags +faststart \"%FILENAME%\"";
        Constructor<?> ctor = settings.getConstructor(method, preset, int.class, int.class, int.class, int.class,
                File.class, boolean.class, boolean.class, boolean.class, boolean.class, boolean.class, color,
                int.class, int.class, boolean.class, boolean.class, boolean.class, aa, String.class, String.class,
                boolean.class);
        return ctor.newInstance(
                method.getMethod("valueOf", String.class).invoke(null, "DEFAULT"),
                preset.getMethod("valueOf", String.class).invoke(null, "MP4_CUSTOM"),
                job.width, job.height, job.fps, 20_000_000, job.video.toFile(),
                true, false, false, false, false, null, 360, 180, false, false, false,
                aa.getMethod("valueOf", String.class).invoke(null, "NONE"),
                AgentRecorder.ffmpegPath(), exportArgs, false);
    }

    /** Camera keyframes {videoMs, x, y, z, yaw, pitch} from an orbit or a path. */
    private static List<double[]> camera(AgentArgs args, long durationMs) throws AgentException {
        if (!args.has("camera")) throw new AgentException("BAD_ARGS", "camera {orbit|path} is required");
        JsonObject camera = args.asMap().get("camera").getAsJsonObject();
        List<double[]> keys = new ArrayList<>();
        try {
            if (camera.has("orbit")) {
                JsonObject orbit = camera.getAsJsonObject("orbit");
                JsonArray c = orbit.getAsJsonArray("center");
                double cx = c.get(0).getAsDouble();
                double cy = c.get(1).getAsDouble();
                double cz = c.get(2).getAsDouble();
                double radius = orbit.get("radius").getAsDouble();
                double height = orbit.get("height").getAsDouble();
                double degrees = orbit.has("degrees") ? orbit.get("degrees").getAsDouble() : 360.0;
                double startDeg = orbit.has("start_deg") ? orbit.get("start_deg").getAsDouble() : 0.0;
                if (radius <= 0) throw new AgentException("BAD_ARGS", "orbit.radius must be > 0");
                int steps = Math.max(2, (int) Math.ceil(durationMs / 1000.0 / ORBIT_STEP_S));
                double previousYaw = Double.NaN;
                for (int i = 0; i <= steps; i++) {
                    double f = i / (double) steps;
                    double angle = Math.toRadians(startDeg + degrees * f);
                    double x = cx + radius * Math.cos(angle);
                    double z = cz + radius * Math.sin(angle);
                    double y = cy + height;
                    double[] look = lookAt(x, y, z, cx, cy, cz, previousYaw);
                    previousYaw = look[0];
                    keys.add(new double[] {Math.round(durationMs * f), x, y, z, look[0], look[1]});
                }
            } else if (camera.has("path")) {
                JsonArray points = camera.getAsJsonObject("path").getAsJsonArray("points");
                if (points.size() < 2) throw new AgentException("BAD_ARGS", "path needs at least 2 points");
                for (int i = 0; i < points.size(); i++) {
                    JsonArray p = points.get(i).getAsJsonArray();
                    long t = Math.round(durationMs * (i / (double) (points.size() - 1)));
                    keys.add(new double[] {t, p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble(),
                            p.get(3).getAsDouble(), p.get(4).getAsDouble()});
                }
            } else {
                throw new AgentException("BAD_ARGS", "camera needs orbit or path");
            }
        } catch (AgentException e) {
            throw e;
        } catch (RuntimeException e) {
            throw new AgentException("BAD_ARGS", "bad camera: " + e.getMessage());
        }
        return keys;
    }

    /**
     * Minecraft yaw/pitch looking from (x,y,z) at the target (yaw 0 = +Z,
     * pitch > 0 = down). Yaw is unwrapped against the previous keyframe so the
     * interpolated orbit never spins the long way round.
     */
    private static double[] lookAt(double x, double y, double z, double tx, double ty, double tz, double previousYaw) {
        double dx = tx - x;
        double dy = ty - y;
        double dz = tz - z;
        double yaw = Math.toDegrees(Math.atan2(-dx, dz));
        double pitch = -Math.toDegrees(Math.atan2(dy, Math.sqrt(dx * dx + dz * dz)));
        if (!Double.isNaN(previousYaw)) {
            while (yaw - previousYaw > 180) yaw -= 360;
            while (yaw - previousYaw < -180) yaw += 360;
        }
        return new double[] {yaw, pitch};
    }

    private static double number(AgentArgs args, String name) throws AgentException {
        try {
            return args.asMap().get(name).getAsDouble();
        } catch (RuntimeException e) {
            throw new AgentException("BAD_ARGS", name + " must be a number");
        }
    }

    private static String rootMessage(Throwable t) {
        Throwable cause = t;
        while (cause instanceof InvocationTargetException && cause.getCause() != null) cause = cause.getCause();
        while (cause.getCause() != null && cause.getCause() != cause) cause = cause.getCause();
        return cause.toString();
    }

    private static final class Job {
        final String id;
        final Path replay;
        final Path out;
        final Path video;
        final int width;
        final int height;
        final int fps;
        final long startMs;
        final long durationMs;
        final List<double[]> keyframes;
        final Path sidecar;
        final long replayDate;
        final CompletableFuture<Void> future = new CompletableFuture<>();
        volatile String state = "loading";
        volatile Object renderer;
        volatile String error;
        volatile String audioNote;
        volatile double renderSeconds;
        boolean audioWanted;
        long loadingSince;

        Job(String id, Path replay, Path out, Path video, int width, int height, int fps, long startMs,
                long durationMs, List<double[]> keyframes, Path sidecar, long replayDate) {
            this.id = id;
            this.replay = replay;
            this.out = out;
            this.video = video;
            this.width = width;
            this.height = height;
            this.fps = fps;
            this.startMs = startMs;
            this.durationMs = durationMs;
            this.keyframes = keyframes;
            this.sidecar = sidecar;
            this.replayDate = replayDate;
        }

        void fail(String why) {
            error = why;
            state = "failed";
            future.complete(null);
            LOGGER.warn("agent: clip {} failed: {}", id, why);
        }

        JsonObject toJson() {
            JsonObject json = new JsonObject();
            json.addProperty("job", id);
            json.addProperty("state", state);
            int total = (int) (durationMs * fps / 1000);
            int done = "done".equals(state) ? total : 0;
            Object r = renderer;
            if (r != null && !"done".equals(state)) {
                try {
                    done = (Integer) r.getClass().getMethod("getFramesDone").invoke(r);
                    total = (Integer) r.getClass().getMethod("getTotalFrames").invoke(r);
                } catch (Exception ignored) {
                    // renderer not started yet
                }
            }
            json.addProperty("frames_done", done);
            json.addProperty("total_frames", total);
            json.addProperty("width", width);
            json.addProperty("height", height);
            json.addProperty("fps", fps);
            if ("done".equals(state)) {
                json.addProperty("video_path", AgentCapture.relative(out));
                json.addProperty("render_s", Math.round(renderSeconds * 10) / 10.0);
                json.addProperty("audio", audioNote);
            }
            if (error != null) json.addProperty("error", error);
            return json;
        }
    }
}
