package es.spectral.menu;

import java.awt.image.BufferedImage;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.regex.Pattern;

import javax.imageio.ImageIO;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.GlStateManager;
import net.minecraft.client.renderer.texture.DynamicTexture;

/**
 * The user-selected title-menu background, from the launcher-managed pack
 * store (same contract as the Fabric mod): the launcher stages the selected
 * pack at {@code <gameDir>/config/espectral-menu-bg/<packId>/} with a
 * {@code pack.json} describing {@code type "image"} ({@code image.png}) or
 * {@code type "frames"} ({@code frames/NNNN.png} at {@code fps}), and records
 * the id in {@code personalization.background}. {@code -Despectral.menu.background}
 * (PNG file or pack directory) wins for dev runs. No selection (or an
 * unreadable pack) keeps the branded gradient.
 *
 * <p>Decoding never happens on the render thread: a single daemon worker
 * resolves the pack and decodes images ({@link ImageIO}); the render thread
 * only uploads into one {@link DynamicTexture}. Animated packs are streamed
 * through a bounded queue of {@link #RING} decoded frames, and reduced
 * motion shows the first frame only. {@link #releaseIfHeld()} frees the
 * texture once a world is loaded and no menu has drawn it for a second.
 */
public final class MenuBackground {

    private static final Logger LOGGER = LogManager.getLogger("espectral-menu");
    private static final Pattern PACK_ID_RE = Pattern.compile("^[A-Za-z0-9_-]{1,64}$");
    private static final int DEFAULT_FPS = 24;
    private static final int RING = 4;

    private static final ExecutorService WORKER = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "espectral-menu-bg");
        t.setDaemon(true);
        return t;
    });

    private enum Kind { IMAGE, FRAMES }

    private static final class Source {
        final Path path;
        final Kind kind;
        final int fps;
        final int frameCount;

        Source(Path path, Kind kind, int fps, int frameCount) {
            this.path = path;
            this.kind = kind;
            this.fps = fps;
            this.frameCount = frameCount;
        }
    }

    /** Decoded frame stamped with the generation that produced it. */
    private static final class Frame {
        final int gen;
        final BufferedImage img;

        Frame(int gen, BufferedImage img) {
            this.gen = gen;
            this.img = img;
        }
    }

    // Render-thread state.
    private static String sourceId;
    private static boolean requested;
    private static DynamicTexture texture;
    private static int width;
    private static int height;
    private static long readyAtMs;
    private static Source animated;
    private static long streamStartMs;
    private static long shownFrame = -1;
    private static long lastPaintMs;

    // Shared with the worker.
    private static final AtomicInteger generation = new AtomicInteger();
    private static volatile Frame pendingStill;
    private static volatile Source pendingStream;
    private static final BlockingQueue<Frame> frames = new ArrayBlockingQueue<Frame>(RING);

    private MenuBackground() {}

    private static String currentSourceId() {
        String override = System.getProperty("espectral.menu.background");
        if (override != null && !override.trim().isEmpty()) {
            return override.trim();
        }
        String packId = ClientConfig.getInstance().backgroundPack();
        return packId == null || packId.isEmpty() ? null : packId;
    }

    private static Source resolveSource(String id, File gameDir) {
        Path p = Paths.get(id);
        if (Files.isRegularFile(p)) {
            return new Source(p, Kind.IMAGE, 0, 1);
        }
        Source pack = readPack(p);
        if (pack != null) return pack;
        if (PACK_ID_RE.matcher(id).matches()) {
            return readPack(gameDir.toPath().resolve("config").resolve("espectral-menu-bg").resolve(id));
        }
        return null;
    }

    private static Source readPack(Path dir) {
        if (dir == null || !Files.isDirectory(dir) || !Files.isRegularFile(dir.resolve("pack.json"))) {
            return null;
        }
        try {
            JsonObject meta = new JsonParser().parse(
                    new String(Files.readAllBytes(dir.resolve("pack.json")), StandardCharsets.UTF_8)).getAsJsonObject();
            String type = meta.has("type") ? meta.get("type").getAsString() : "";
            if ("image".equals(type)) {
                Path img = dir.resolve("image.png");
                return Files.isRegularFile(img) ? new Source(img, Kind.IMAGE, 0, 1) : null;
            }
            if ("frames".equals(type)) {
                int count = meta.has("frame_count") ? meta.get("frame_count").getAsInt() : 0;
                int fps = meta.has("fps") ? meta.get("fps").getAsInt() : DEFAULT_FPS;
                if (count < 1 || !Files.isRegularFile(frameFile(dir, 0))) return null;
                return new Source(dir, Kind.FRAMES, Math.max(1, Math.min(60, fps)), count);
            }
        } catch (Exception e) {
            LOGGER.warn("[espectral] unreadable background pack {}: {}", dir, e.toString());
        }
        return null;
    }

    private static Path frameFile(Path dir, int index) {
        return dir.resolve("frames").resolve(String.format("%04d.png", index + 1));
    }

    /**
     * Called every menu frame: starts the load when the selected source
     * changes and uploads whatever the worker finished. Cheap when idle.
     */
    public static void request(Minecraft mc) {
        lastPaintMs = Minecraft.getSystemTime();
        String id;
        try {
            id = currentSourceId();
        } catch (Throwable t) {
            id = null;
        }
        if (id == null ? sourceId != null : !id.equals(sourceId)) {
            release();
            sourceId = id;
        }
        if (id != null && !requested) {
            requested = true;
            final int gen = generation.get();
            final String wanted = id;
            final File gameDir = mc.mcDataDir;
            final boolean reduced = ClientConfig.getInstance().reducedMotion();
            WORKER.submit(() -> load(wanted, gameDir, gen, reduced));
        }
        upload();
    }

    /** Worker: resolve, then decode the still image or stream frames until superseded. */
    private static void load(String id, File gameDir, int gen, boolean reduced) {
        try {
            Source src = resolveSource(id, gameDir);
            if (src == null || gen != generation.get()) return;
            if (src.kind == Kind.IMAGE || reduced) {
                Path file = src.kind == Kind.IMAGE ? src.path : frameFile(src.path, 0);
                BufferedImage img = ImageIO.read(file.toFile());
                if (img != null && gen == generation.get()) pendingStill = new Frame(gen, img);
                return;
            }
            pendingStream = src;
            int i = 0;
            while (gen == generation.get()) {
                BufferedImage img = ImageIO.read(frameFile(src.path, i).toFile());
                if (img == null) break;
                while (gen == generation.get() && !frames.offer(new Frame(gen, img), 100, TimeUnit.MILLISECONDS)) {
                    // render thread consumes at the pack's fps; wait for a free slot
                }
                i = (i + 1) % src.frameCount;
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (Exception e) {
            LOGGER.warn("[espectral] menu background {} failed: {}", id, e.toString());
        }
    }

    /** Render thread: upload the still image, or advance the stream on the pack's clock. */
    private static void upload() {
        int gen = generation.get();
        Frame still = pendingStill;
        if (still != null) {
            pendingStill = null;
            if (still.gen == gen) {
                put(still.img);
            }
        }
        Source stream = pendingStream;
        if (stream != null && animated == null) {
            animated = stream;
            streamStartMs = Minecraft.getSystemTime();
            shownFrame = -1;
        }
        if (animated == null) return;
        long due = (Minecraft.getSystemTime() - streamStartMs) * animated.fps / 1000L;
        if (shownFrame >= due && texture != null) return;
        Frame next = frames.poll();
        while (next != null && next.gen != gen) {
            next = frames.poll();
        }
        if (next != null) {
            put(next.img);
            shownFrame++;
        }
    }

    private static void put(BufferedImage img) {
        if (texture == null || width != img.getWidth() || height != img.getHeight()) {
            if (texture != null) texture.deleteGlTexture();
            texture = new DynamicTexture(img);
            width = img.getWidth();
            height = img.getHeight();
            readyAtMs = Minecraft.getSystemTime();
            return;
        }
        img.getRGB(0, 0, width, height, texture.getTextureData(), 0, width);
        texture.updateDynamicTexture();
    }

    public static boolean ready() {
        return texture != null && width > 0 && height > 0;
    }

    /** Binds the background texture (callers then draw with {@code Gfx.blitBound}). */
    public static void bind() {
        if (texture != null) GlStateManager.bindTexture(texture.getGlTextureId());
    }

    public static int width() {
        return width;
    }

    public static int height() {
        return height;
    }

    public static long readyAtMs() {
        return readyAtMs;
    }

    /** Client tick: free the texture once a world runs and no menu drew it for 1 s. */
    public static void releaseIfHeld(Minecraft mc) {
        if (mc.theWorld != null && texture != null && Minecraft.getSystemTime() - lastPaintMs > 1000L) {
            release();
            sourceId = null;
        }
    }

    /** Frees the texture and cancels the worker's current load/stream. */
    public static void release() {
        generation.incrementAndGet();
        requested = false;
        pendingStill = null;
        pendingStream = null;
        animated = null;
        frames.clear();
        if (texture != null) {
            texture.deleteGlTexture();
            texture = null;
        }
        width = 0;
        height = 0;
    }
}
