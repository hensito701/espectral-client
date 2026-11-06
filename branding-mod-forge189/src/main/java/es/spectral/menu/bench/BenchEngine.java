package es.spectral.menu.bench;

import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import net.minecraft.client.Minecraft;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.WorldSettings;
import net.minecraft.world.WorldType;

import es.spectral.menu.ui.EspectralTitleScreen;

/**
 * Locked-scene frame-time bench for 1.8.9 — the same file contract as the
 * 26.x lanes: {@code <gameDir>/espectral-bench.json} present at client init
 * arms it, absent leaves every hook on one static flag check.
 *
 * <p>Schema: {@code {"world": "<save folder>", "seed": N, "scenes": [{"id",
 * "duration_s", "steps": [{"cmd": "<server command>"} | {"wait_s": N}]}]}}.
 * Commands run through the integrated server as the op'd player
 * ({@code tp @p x y z yaw pitch}, {@code summon Cow x y z}, ...). Each scene
 * appends {@code scene_end} (p50/p95/p99/1% low frame times) to
 * {@code <gameDir>/bench-metrics.jsonl}; the last one is followed by
 * {@code done}.
 */
public final class BenchEngine {

    private static final Logger LOGGER = LogManager.getLogger("espectral-bench");
    private static final int FRAME_CAP = 1 << 17;

    private static volatile boolean armed;
    private static boolean recording;
    private static boolean finished;
    private static boolean enterAttempted;

    private static String world;
    private static long seed;
    private static final List<Scene> scenes = new ArrayList<Scene>();
    private static File metricsFile;

    private static int sceneIndex = -1;
    private static int stepIndex;
    private static long waitUntilMs;
    private static long recordEndMs;
    /** Allocated only when armed: an ordinary launch never pays for it. */
    private static long[] frames;
    private static int frameCount;
    private static long lastFrameNs;

    private BenchEngine() {}

    private static final class Scene {
        final String id;
        final int durationS;
        final List<JsonObject> steps;

        Scene(String id, int durationS, List<JsonObject> steps) {
            this.id = id;
            this.durationS = durationS;
            this.steps = steps;
        }
    }

    public static void maybeArm(File gameDir) {
        File spec = new File(gameDir, "espectral-bench.json");
        if (!spec.isFile()) return;
        try {
            JsonObject root = new JsonParser().parse(new String(Files.readAllBytes(spec.toPath()), StandardCharsets.UTF_8))
                    .getAsJsonObject();
            world = root.get("world").getAsString();
            seed = root.has("seed") ? root.get("seed").getAsLong() : 1337L;
            for (JsonElement el : root.getAsJsonArray("scenes")) {
                JsonObject o = el.getAsJsonObject();
                List<JsonObject> steps = new ArrayList<JsonObject>();
                if (o.has("steps")) {
                    JsonArray arr = o.getAsJsonArray("steps");
                    for (JsonElement s : arr) steps.add(s.getAsJsonObject());
                }
                scenes.add(new Scene(o.get("id").getAsString(), o.has("duration_s") ? o.get("duration_s").getAsInt() : 30, steps));
            }
            metricsFile = new File(gameDir, "bench-metrics.jsonl");
            frames = new long[FRAME_CAP];
            armed = !scenes.isEmpty();
            LOGGER.info("[espectral-bench] armed: world='{}' scenes={}", world, scenes.size());
        } catch (Exception e) {
            LOGGER.error("[espectral-bench] ignoring unreadable espectral-bench.json: {}", e.toString());
        }
    }

    /** Render tick END: one sample per rendered frame. */
    public static void onFrameEnd() {
        if (!recording) return;
        long now = System.nanoTime();
        if (lastFrameNs != 0L && frameCount < FRAME_CAP) {
            frames[frameCount++] = now - lastFrameNs;
        }
        lastFrameNs = now;
    }

    /** Client tick END. */
    public static void onTick(Minecraft mc) {
        if (!armed || finished) return;
        try {
            tick(mc);
        } catch (Exception e) {
            LOGGER.error("[espectral-bench] tick error, stopping", e);
            append("{\"type\":\"done\",\"error\":true}\n");
            finished = true;
        }
    }

    private static void tick(Minecraft mc) {
        long now = System.currentTimeMillis();
        if (mc.theWorld == null || mc.thePlayer == null) {
            if (!enterAttempted && mc.currentScreen instanceof EspectralTitleScreen) {
                enterAttempted = true;
                LOGGER.info("[espectral-bench] title screen at JVM uptime {} ms",
                        java.lang.management.ManagementFactory.getRuntimeMXBean().getUptime());
                if (mc.getSaveLoader().canLoadWorld(world)) {
                    LOGGER.info("[espectral-bench] opening world '{}'", world);
                } else {
                    LOGGER.info("[espectral-bench] creating world '{}' (seed {})", world, seed);
                }
                WorldSettings settings = new WorldSettings(seed, WorldSettings.GameType.CREATIVE, true, false, WorldType.DEFAULT);
                settings.enableCommands();
                mc.launchIntegratedServer(world, world, settings);
            }
            return;
        }
        if (sceneIndex < 0) sceneIndex = 0;
        Scene scene = scenes.get(sceneIndex);
        if (recording) {
            if (now >= recordEndMs) {
                recording = false;
                append(summary(scene.id, frameCount));
                sceneIndex++;
                stepIndex = 0;
                waitUntilMs = 0L;
                if (sceneIndex >= scenes.size()) {
                    append("{\"type\":\"done\"}\n");
                    finished = true;
                    LOGGER.info("[espectral-bench] all {} scenes complete", scenes.size());
                }
            }
            return;
        }
        if (now < waitUntilMs) return;
        if (stepIndex < scene.steps.size()) {
            JsonObject step = scene.steps.get(stepIndex++);
            if (step.has("wait_s")) {
                waitUntilMs = now + (long) (step.get("wait_s").getAsDouble() * 1000.0);
            } else if (step.has("cmd")) {
                MinecraftServer server = mc.getIntegratedServer();
                if (server != null) {
                    final String cmd = step.get("cmd").getAsString();
                    final MinecraftServer srv = server;
                    final String name = mc.thePlayer.getName();
                    server.addScheduledTask(new Runnable() {
                        @Override
                        public void run() {
                            srv.getCommandManager().executeCommand(srv.getConfigurationManager().getPlayerByUsername(name), cmd);
                        }
                    });
                }
            }
            return;
        }
        frameCount = 0;
        lastFrameNs = 0L;
        recordEndMs = now + scene.durationS * 1000L;
        recording = true;
        LOGGER.info("[espectral-bench] scene '{}' started ({}s)", scene.id, scene.durationS);
    }

    private static String summary(String id, int n) {
        long[] sorted = Arrays.copyOf(frames, n);
        Arrays.sort(sorted);
        double p50 = n > 0 ? sorted[rank(0.50, n)] / 1e6 : 0;
        double p95 = n > 0 ? sorted[rank(0.95, n)] / 1e6 : 0;
        double p99 = n > 0 ? sorted[rank(0.99, n)] / 1e6 : 0;
        int k = Math.max(1, (int) Math.round(n * 0.01));
        long worst = 0L;
        for (int i = Math.max(0, n - k); i < n; i++) worst += sorted[i];
        long total = 0L;
        for (int i = 0; i < n; i++) total += sorted[i];
        double avgFps = total > 0 ? n / (total / 1e9) : 0;
        String line = String.format(Locale.ROOT,
                "{\"type\":\"scene_end\",\"scene\":\"%s\",\"frames\":%d,\"avg_fps\":%.1f,\"p50_ms\":%.3f,\"p95_ms\":%.3f,\"p99_ms\":%.3f,\"low1pct_ms\":%.3f}\n",
                id, n, avgFps, p50, p95, p99, worst / (double) k / 1e6);
        LOGGER.info("[espectral-bench] {}", line.trim());
        return line;
    }

    private static int rank(double q, int n) {
        return Math.min(n - 1, Math.max(0, (int) Math.ceil(q * n) - 1));
    }

    private static void append(String line) {
        try (Writer w = new OutputStreamWriter(new FileOutputStream(metricsFile, true), StandardCharsets.UTF_8)) {
            w.write(line);
        } catch (Exception e) {
            LOGGER.error("[espectral-bench] metrics append failed: {}", e.toString());
        }
    }
}
