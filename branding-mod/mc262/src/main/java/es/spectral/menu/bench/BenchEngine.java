package es.spectral.menu.bench;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.Arrays;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicLong;

import javax.management.Notification;
import javax.management.NotificationEmitter;
import javax.management.NotificationListener;
import javax.management.openmbean.CompositeData;

import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryUsage;

import com.mojang.blaze3d.platform.Window;
import com.sun.management.GarbageCollectionNotificationInfo;
import com.sun.management.GcInfo;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.core.HolderLookup;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.world.Difficulty;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;

/**
 * Workstream 2 bench driver: locked-scene frame-time instrumentation.
 *
 * <p>Armed only when {@code <gameDir>/espectral-bench.json} exists at client
 * init (see {@link BenchClient}); otherwise every entry point below returns
 * on a single static flag check with no allocation and no I/O.
 *
 * <p>State machine (all driven from the client tick, never blocking it):
 * wait for the title screen, enter-or-create the bench world, run each
 * scene's steps in order ({@code tp}/{@code look} on the client player,
 * {@code summon} through the integrated server, {@code wait_s} as a tick
 * deadline), then record per-frame deltas for {@code duration_s} and append
 * {@code scene_start}/{@code scene_end} lines plus a final {@code done} to
 * {@code bench-metrics.jsonl} from a single background writer thread.
 */
public final class BenchEngine {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-bench");

    /** Preallocated frame buffer: 65536 samples (~18 min at 60 fps). */
    private static final int FRAME_CAP = 1 << 16;
    /** Give world creation/loading this long before declaring the world missing. */
    private static final long ENTER_TIMEOUT_MS = 180_000L;

    private static volatile boolean armed;
    private static volatile boolean recording;

    private static BenchSpec spec;
    private static File metricsFile;
    private static ExecutorService writer;

    private static boolean enterAttempted;
    private static long enterStartMs;
    private static boolean finished;
    private static int sceneIndex = -1;
    private static int stepIndex;
    private static long stepWaitUntilMs;
    private static long recordEndMs;
    private static long recordStartEpochMs;

    private static final long[] FRAME_BUF = new long[FRAME_CAP];
    private static int frameCount;
    private static long lastFrameNs;
    private static boolean overflowLogged;
    private static long firstBlockerMs;
    /** Last observed heap-used-after-GC across all collectors (bytes). -1 = none yet. */
    private static final AtomicLong LAST_HEAP_AFTER_GC = new AtomicLong(-1L);
    private static boolean gcListenerInstalled;
    private BenchEngine() {}

    /**
     * Called once from the lane entrypoint. Reads and parses the bench file a
     * single time; absent file (or unparseable content) leaves the engine
     * fully inert.
     */
    public static synchronized void maybeArm(File benchFile, File metrics) {
        if (benchFile == null || !benchFile.isFile()) {
            LOGGER.debug("[espectral-bench] no espectral-bench.json, bench hooks inert");
            return;
        }
        try {
            String json = Files.readString(benchFile.toPath(), StandardCharsets.UTF_8);
            BenchSpec parsed = BenchSpec.parse(
                    com.google.gson.JsonParser.parseString(json).getAsJsonObject());
            spec = parsed;
            metricsFile = metrics;
            writer = Executors.newSingleThreadExecutor(r -> {
                Thread t = new Thread(r, "espectral-bench-writer");
                t.setDaemon(true);
                return t;
            });
            installGcListener();
            armed = true;
            LOGGER.info("[espectral-bench] armed: world='{}' seed={} scenes={} (metrics -> {})",
                    parsed.world, parsed.seed, parsed.scenes.size(), metrics.getAbsolutePath());
        } catch (Exception e) {
            LOGGER.error("[espectral-bench] ignoring unreadable espectral-bench.json: {}", e.toString());
        }
    }

    /** Client-tick driver; single volatile check when disarmed. */
    public static void onTick(Minecraft minecraft) {
        if (!armed || finished) {
            return;
        }
        try {
            onTickInner(minecraft);
        } catch (Exception e) {
            LOGGER.error("[espectral-bench] tick error, idling", e);
            finishQuietly();
        }
    }

    /** End-of-frame hook (render thread); inert unless a window is open. */
    public static void onFrameEnd() {
        if (!recording) {
            return;
        }
        long now = System.nanoTime();
        // done on the render thread: two field stores, no allocation
        if (lastFrameNs == 0L) {
            lastFrameNs = now;
            return;
        }
        long delta = now - lastFrameNs;
        lastFrameNs = now;
        if (frameCount < FRAME_CAP) {
            FRAME_BUF[frameCount++] = delta;
        } else if (!overflowLogged) {
            overflowLogged = true;
            LOGGER.warn("[espectral-bench] frame buffer full ({}), dropping further samples this scene",
                    FRAME_CAP);
        }
    }

    private static void onTickInner(Minecraft minecraft) {
        if (minecraft == null) {
            return;
        }
        long nowMs = System.currentTimeMillis();
        boolean inWorld = minecraft.level != null && minecraft.player != null;

        if (!inWorld) {
            if (!enterAttempted) {
                Screen screen = minecraft.gui != null ? minecraft.gui.screen() : null;
                if (screen instanceof TitleScreen) {
                    enterAttempted = true;
                    enterStartMs = nowMs;
                    attemptEnter(minecraft);
                } else if (screen != null && minecraft.isGameLoadFinished()) {
                    // Fresh gameDirs open on the narrator/accessibility
                    // onboarding instead of the title and would wait for input
                    // forever; bench mode is input-free, so skip to the title
                    // once the dialog has settled.
                    if (firstBlockerMs == 0L) {
                        firstBlockerMs = nowMs;
                    } else if (nowMs - firstBlockerMs > 5000L) {
                        LOGGER.info("[espectral-bench] skipping '{}' blocker, showing title",
                                screen.getClass().getSimpleName());
                        minecraft.setScreenAndShow(new TitleScreen());
                    }
                } else if (screen == null) {
                    firstBlockerMs = 0L;
                }
            } else if (nowMs - enterStartMs > ENTER_TIMEOUT_MS) {
                LOGGER.error("[espectral-bench] world missing: '{}' (enter timed out)", spec.world);
                emitDone(nowMs);
                finished = true;
            } else if (nowMs - enterStartMs > 30_000L) {
                // A launched enter leaves the title within seconds (loading
                // screen); still sitting on it means vanilla aborted the
                // enter (it logs and swallows creation failures itself).
                Screen screen = minecraft.gui != null ? minecraft.gui.screen() : null;
                if (screen instanceof TitleScreen) {
                    LOGGER.error("[espectral-bench] world missing: '{}' (enter aborted)", spec.world);
                    emitDone(nowMs);
                    finished = true;
                }
            }
            return;
        }

        if (sceneIndex < 0) {
            sceneIndex = 0;
            stepIndex = 0;
            stepWaitUntilMs = 0L;
        }
        if (sceneIndex >= spec.scenes.size()) {
            return;
        }
        BenchSpec.Scene scene = spec.scenes.get(sceneIndex);

        if (recording) {
            if (nowMs >= recordEndMs) {
                endScene(minecraft, scene, nowMs);
                sceneIndex++;
                stepIndex = 0;
                stepWaitUntilMs = 0L;
                if (sceneIndex >= spec.scenes.size()) {
                    emitDone(nowMs);
                    finished = true;
                    LOGGER.info("[espectral-bench] all {} scenes complete", spec.scenes.size());
                }
            }
            return;
        }

        if (nowMs < stepWaitUntilMs) {
            return;
        }
        if (stepIndex < scene.steps.size()) {
            runStep(minecraft, scene.steps.get(stepIndex), nowMs);
            stepIndex++;
            return;
        }
        beginScene(scene, nowMs);
    }

    private static void attemptEnter(Minecraft minecraft) {
        String folder = spec.world;
        try {
            if (minecraft.getLevelSource() == null) {
                LOGGER.error("[espectral-bench] world missing: '{}' (level source unavailable)", folder);
                emitDone(System.currentTimeMillis());
                finished = true;
                return;
            }
            if (minecraft.getLevelSource().levelExists(folder)) {
                LOGGER.info("[espectral-bench] opening world '{}'", folder);
                minecraft.createWorldOpenFlows().openWorld(folder, () -> {});
                return;
            }
            LOGGER.info("[espectral-bench] world '{}' not found, creating it (seed {})", folder, spec.seed);
            createWorld(minecraft, folder);
        } catch (Exception e) {
            LOGGER.error("[espectral-bench] world missing: '{}' ({})", folder, e.toString());
            emitDone(System.currentTimeMillis());
            finished = true;
        }
    }

    private static void createWorld(Minecraft minecraft, String folder) {
        LevelSettings settings = new LevelSettings(folder, GameType.CREATIVE,
                new LevelSettings.DifficultySettings(Difficulty.NORMAL, false, false),
                true,
                new WorldDataConfiguration(DataPackConfig.DEFAULT, FeatureFlags.DEFAULT_FLAGS));
        WorldOptions options = new WorldOptions(spec.seed, true, false);
        java.util.function.Function<HolderLookup.Provider, WorldDimensions> dimensions =
                WorldPresets::createNormalWorldDimensions;
        Screen area = minecraft.gui != null ? minecraft.gui.screen() : null;
        minecraft.createWorldOpenFlows().createFreshLevel(folder, settings, options, dimensions, area);
    }

    private static void runStep(Minecraft minecraft, BenchSpec.Step step, long nowMs) {
        switch (step.kind) {
            case WAIT -> stepWaitUntilMs = nowMs + (long) (step.waitS * 1000.0);
            case TP -> {
                if (minecraft.player != null) {
                    minecraft.player.teleportTo(step.vec[0], step.vec[1], step.vec[2]);
                }
            }
            case LOOK -> {
                if (minecraft.player != null) {
                    minecraft.player.setYRot((float) step.vec[0]);
                    minecraft.player.setXRot((float) step.vec[1]);
                }
            }
            case SUMMON -> {
                var server = minecraft.getSingleplayerServer();
                if (server == null) {
                    LOGGER.warn("[espectral-bench] summon skipped (no integrated server yet)");
                    break;
                }
                var source = server.createCommandSourceStack()
                        .withPermission(PermissionSet.ALL_PERMISSIONS)
                        .withSuppressedOutput();
                String pos = fmt(step.vec[0]) + " " + fmt(step.vec[1]) + " " + fmt(step.vec[2]);
                for (int i = 0; i < step.count; i++) {
                    String cmd = "summon " + step.entity + " " + pos;
                    server.execute(() -> {
                        try {
                            server.getCommands().performPrefixedCommand(source, cmd);
                        } catch (Exception e) {
                            LOGGER.warn("[espectral-bench] summon failed: {}", e.toString());
                        }
                    });
                }
            }
        }
    }

    private static void beginScene(BenchSpec.Scene scene, long nowMs) {
        frameCount = 0;
        lastFrameNs = 0L;
        overflowLogged = false;
        recordStartEpochMs = nowMs;
        recordEndMs = nowMs + scene.durationS * 1000L;
        recording = true;
        final String id = scene.id;
        final long ts = nowMs;
        writer.execute(() -> append("{\"type\":\"scene_start\",\"scene\":" + quote(id) + ",\"ts\":" + ts + "}\n"));
        LOGGER.info("[espectral-bench] scene '{}' started ({}s)", id, scene.durationS);
    }

    private static void endScene(Minecraft minecraft, BenchSpec.Scene scene, long nowMs) {
        recording = false;
        int n = frameCount;
        double[] stats = summarize(n);
        long heapBytes = LAST_HEAP_AFTER_GC.get();
        if (heapBytes < 0) {
            heapBytes = ManagementFactory.getMemoryMXBean().getHeapMemoryUsage().getUsed();
        }
        double heapMb = heapBytes / (1024.0 * 1024.0);
        final String line = "{\"type\":\"scene_end\",\"scene\":" + quote(scene.id)
                + ",\"frames\":" + n
                + ",\"p50_ms\":" + f3(stats[0])
                + ",\"p95_ms\":" + f3(stats[1])
                + ",\"p99_ms\":" + f3(stats[2])
                + ",\"low1pct_ms\":" + f3(stats[3])
                + ",\"over50\":" + (long) stats[4]
                + ",\"over100\":" + (long) stats[5]
                + ",\"heap_after_gc_mb\":" + f3(heapMb)
                + ",\"ts\":" + nowMs + "}\n";
        writer.execute(() -> append(line));
        LOGGER.info("[espectral-bench] scene '{}' ended: frames={} p50={}ms p95={}ms p99={}ms low1%={}ms over50={} over100={} heap={}MB",
                scene.id, n, f3(stats[0]), f3(stats[1]), f3(stats[2]), f3(stats[3]),
                (long) stats[4], (long) stats[5], f3(heapMb));
        // Quiet the render thread's reference; buffer is reused next scene.
        Window window = minecraft.getWindow();
        if (window == null) {
            LOGGER.debug("[espectral-bench] no window handle at scene end (headless?)");
        }
    }

    /**
     * Returns {p50, p95, p99, low1pctMean, over50, over100} in ms (counts as
     * whole doubles). Empty window yields zeros, never NaN.
     */
    static double[] summarize(int n) {
        if (n <= 0) {
            return new double[6];
        }
        long[] sorted = Arrays.copyOf(FRAME_BUF, n);
        Arrays.sort(sorted);
        double p50 = sorted[rank(0.50, n)] / 1_000_000.0;
        double p95 = sorted[rank(0.95, n)] / 1_000_000.0;
        double p99 = sorted[rank(0.99, n)] / 1_000_000.0;
        int k = Math.max(1, (int) Math.round(n * 0.01));
        long worst = 0L;
        for (int i = n - k; i < n; i++) {
            worst += sorted[i];
        }
        double low1 = worst / (double) k / 1_000_000.0;
        long over50 = 0L;
        long over100 = 0L;
        for (int i = 0; i < n; i++) {
            if (sorted[i] > 50_000_000L) {
                over50++;
            }
            if (sorted[i] > 100_000_000L) {
                over100++;
            }
        }
        return new double[] {p50, p95, p99, low1, over50, over100};
    }

    private static int rank(double q, int n) {
        return Math.min(n - 1, Math.max(0, (int) Math.ceil(q * n) - 1));
    }

    private static void emitDone(long nowMs) {
        final long ts = nowMs;
        if (writer != null) {
            writer.execute(() -> append("{\"type\":\"done\",\"ts\":" + ts + "}\n"));
        }
    }

    private static void finishQuietly() {
        try {
            emitDone(System.currentTimeMillis());
        } catch (Exception ignored) {
            // Last resort: never throw out of the tick hook.
        }
        finished = true;
    }

    private static void append(String line) {
        try {
            Files.writeString(metricsFile.toPath(), line, StandardCharsets.UTF_8,
                    java.nio.file.StandardOpenOption.CREATE,
                    java.nio.file.StandardOpenOption.APPEND);
        } catch (Exception e) {
            LOGGER.error("[espectral-bench] metrics append failed: {}", e.toString());
        }
    }

    private static void installGcListener() {
        if (gcListenerInstalled) {
            return;
        }
        NotificationListener listener = (Notification notification, Object handback) -> {
            try {
                if (!GarbageCollectionNotificationInfo.GARBAGE_COLLECTION_NOTIFICATION
                        .equals(notification.getType())) {
                    return;
                }
                GarbageCollectionNotificationInfo info = GarbageCollectionNotificationInfo.from(
                        (CompositeData) notification.getUserData());
                GcInfo gcInfo = info.getGcInfo();
                long heap = 0L;
                for (Map.Entry<String, MemoryUsage> e : gcInfo.getMemoryUsageAfterGc().entrySet()) {
                    String name = e.getKey();
                    if (name.contains("Metaspace") || name.contains("Compressed")
                            || name.contains("CodeHeap") || name.contains("Code Cache")) {
                        continue;
                    }
                    heap += e.getValue().getUsed();
                }
                LAST_HEAP_AFTER_GC.set(heap);
            } catch (Exception ignored) {
                // Notification thread: never propagate.
            }
        };
        for (GarbageCollectorMXBean bean : ManagementFactory.getGarbageCollectorMXBeans()) {
            if (bean instanceof NotificationEmitter emitter) {
                try {
                    emitter.addNotificationListener(listener, null, null);
                } catch (Exception e) {
                    LOGGER.warn("[espectral-bench] GC listener install failed: {}", e.toString());
                }
            }
        }
        gcListenerInstalled = true;
    }

    private static String quote(String s) {
        return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }

    private static String f3(double v) {
        return String.format(Locale.ROOT, "%.3f", v);
    }

    private static String fmt(double v) {
        return String.format(Locale.ROOT, "%.3f", v);
    }
}
