package es.spectral.menu.agent;

import java.io.File;
import java.io.FileOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.concurrent.CompletableFuture;

import javax.imageio.ImageIO;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.lwjgl.BufferUtils;
import org.lwjgl.opengl.GL11;
import org.lwjgl.opengl.GL12;

import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.OpenGlHelper;
import net.minecraft.client.renderer.texture.TextureUtil;
import net.minecraft.client.shader.Framebuffer;

import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.image.BufferedImage;
import java.nio.IntBuffer;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;

/**
 * {@code screenshot}: end-of-frame grab of the main framebuffer (vanilla's own
 * {@code ScreenShotHelper} read path, so the GUI pass is already composited),
 * written under {@code <gameDir>/espectral-captures/} with an optional ≤1280 px
 * preview.
 *
 * <p>The frame hook lives in {@code EntityRendererMixin} (tail of
 * {@code updateCameraAndRender}). The grab itself (glGetTexImage/glReadPixels)
 * runs on the render thread; PNG encoding and downscaling run on a daemon
 * thread, so a 4K frame never stalls a tick.
 */
public final class Capture {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    public static final int PREVIEW_MAX_SIDE = 1280;
    public static final int MIN_SIDE = 64;
    public static final int MAX_SIDE = 7680;
    public static final long MAX_PIXELS = 33_200_000L;

    private static final Object LOCK = new Object();
    private static Request pending;

    private Capture() {}

    /** One grabbed frame: game-dir-relative paths plus pixel dimensions. */
    public static final class Grabbed {
        public final String imagePath;
        public final String previewPath;
        public final int width;
        public final int height;

        Grabbed(String imagePath, String previewPath, int width, int height) {
            this.imagePath = imagePath;
            this.previewPath = previewPath;
            this.width = width;
            this.height = height;
        }
    }

    private static final class Request {
        final CompletableFuture<JsonObject> future = new CompletableFuture<JsonObject>();
        final boolean hud;
        final boolean preview;
        final int outWidth;
        final int outHeight;
        /** Set after the HUD was hidden for this grab. */
        boolean hudApplied;
        boolean savedHideGui;

        Request(boolean hud, boolean preview, int outWidth, int outHeight) {
            this.hud = hud;
            this.preview = preview;
            this.outWidth = outWidth;
            this.outHeight = outHeight;
        }
    }

    /**
     * Validates the args on the client thread and parks the request; the frame
     * hook fulfils {@code future}.
     */
    public static void request(AgentArgs args, CompletableFuture<JsonObject> future) {
        boolean hud;
        boolean preview;
        int width;
        int height;
        try {
            width = args.optInt("width", 0);
            height = args.optInt("height", 0);
            hud = args.bool("hud", true);
            preview = args.bool("preview", true);
            validateSize(width, height, args);
        } catch (AgentException e) {
            future.completeExceptionally(e);
            return;
        }
        Request request = new Request(hud, preview, width, height);
        synchronized (LOCK) {
            if (pending != null) {
                future.completeExceptionally(
                        new AgentException("BUSY", "a capture is already in flight"));
                return;
            }
            pending = request;
        }
        // The frame hook owns the future from here; give it a deadline so a
        // paused/resized client cannot wedge the relay.
        // Forward to the caller's future and drop the request once it settles.
        request.future.whenComplete(new java.util.function.BiConsumer<JsonObject, Throwable>() {
            @Override
            public void accept(JsonObject json, Throwable error) {
                clear(request);
                if (error != null) {
                    future.completeExceptionally(error);
                } else {
                    future.complete(json);
                }
            }
        });
    }

    private static void validateSize(int width, int height, AgentArgs args) throws AgentException {
        if (args.contains("width") != args.contains("height")) {
            throw new AgentException("BAD_ARGS", "give both width and height, or neither");
        }
        if (!args.contains("width")) return;
        if (width < MIN_SIDE || width > MAX_SIDE || height < MIN_SIDE || height > MAX_SIDE) {
            throw new AgentException("BAD_ARGS", "width and height must be "
                    + MIN_SIDE + "\u2026" + MAX_SIDE + " px");
        }
        if ((long) width * height > MAX_PIXELS) {
            throw new AgentException("BAD_ARGS", width + "x" + height + " exceeds "
                    + MAX_PIXELS + " pixels");
        }
    }

    private static void clear(Request request) {
        synchronized (LOCK) {
            if (pending == request) pending = null;
        }
    }

    /**
     * Called at the tail of {@code EntityRenderer.updateCameraAndRender}.
     * Runs on the render thread, after the GUI pass and before Display.update.
     */
    public static void onFrameEnd() {
        Request request;
        synchronized (LOCK) {
            request = pending;
        }
        if (request == null || request.future.isDone()) return;
        Minecraft minecraft = Minecraft.getMinecraft();
        if (minecraft == null) return;
        if (!request.hud) {
            if (!request.hudApplied) {
                // Hide the HUD, grab on the next frame so it is gone everywhere.
                request.savedHideGui = minecraft.gameSettings.hideGUI;
                minecraft.gameSettings.hideGUI = true;
                request.hudApplied = true;
                return;
            }
        }
        try {
            BufferedImage image = grab(minecraft);
            if (image == null) {
                request.future.completeExceptionally(
                        new AgentException("FAILED", "the frame came back empty"));
                return;
            }
            Request done = request;
            // One request = one grab: later frames must not grab again while
            // the writer thread is still encoding.
            clear(done);
            if (done.hudApplied) {
                // Restore the HUD in the same frame the grab happened: the
                // pixels are already read back, so the player sees no flicker.
                minecraft.gameSettings.hideGUI = done.savedHideGui;
                done.hudApplied = false;
            }
            Thread writer = new Thread(new Runnable() {
                @Override
                public void run() {
                    complete(done, image);
                }
            }, "espectral-capture-writer");
            writer.setDaemon(true);
            writer.start();
        } catch (Throwable t) {
            LOGGER.warn("agent: capture failed", t);
            if (request.hudApplied) {
                minecraft.gameSettings.hideGUI = request.savedHideGui;
            }
            request.future.completeExceptionally(new AgentException("FAILED", t.toString()));
        }
    }

    /** Render-thread readback, identical to vanilla ScreenShotHelper's path. */
    private static BufferedImage grab(Minecraft minecraft) {
        Framebuffer framebuffer = minecraft.getFramebuffer();
        int width = framebuffer.framebufferWidth;
        int height = framebuffer.framebufferHeight;
        if (OpenGlHelper.isFramebufferEnabled()) {
            width = framebuffer.framebufferTextureWidth;
            height = framebuffer.framebufferTextureHeight;
        }
        int pixels = width * height;
        IntBuffer buffer = BufferUtils.createIntBuffer(pixels);
        int[] values = new int[pixels];
        GL11.glPixelStorei(GL11.GL_PACK_ALIGNMENT, 1);
        GL11.glPixelStorei(GL11.GL_UNPACK_ALIGNMENT, 1);
        buffer.clear();
        if (OpenGlHelper.isFramebufferEnabled()) {
            GL11.glBindTexture(GL11.GL_TEXTURE_2D, framebuffer.framebufferTexture);
            GL11.glGetTexImage(GL11.GL_TEXTURE_2D, 0, GL12.GL_BGRA,
                    GL12.GL_UNSIGNED_INT_8_8_8_8_REV, buffer);
        } else {
            GL11.glReadPixels(0, 0, width, height, GL12.GL_BGRA,
                    GL12.GL_UNSIGNED_INT_8_8_8_8_REV, buffer);
        }
        buffer.get(values);
        TextureUtil.processPixelValues(values, width, height);

        if (OpenGlHelper.isFramebufferEnabled()) {
            BufferedImage image = new BufferedImage(framebuffer.framebufferWidth,
                    framebuffer.framebufferHeight, BufferedImage.TYPE_INT_RGB);
            int top = framebuffer.framebufferTextureHeight - framebuffer.framebufferHeight;
            for (int y = top; y < framebuffer.framebufferTextureHeight; ++y) {
                for (int x = 0; x < framebuffer.framebufferWidth; ++x) {
                    image.setRGB(x, y - top, values[y * framebuffer.framebufferTextureWidth + x]);
                }
            }
            return image;
        }
        BufferedImage image = new BufferedImage(width, height, BufferedImage.TYPE_INT_RGB);
        image.setRGB(0, 0, width, height, values, 0, width);
        return image;
    }

    /** Writer thread: encode the PNGs and answer the tool. */
    private static void complete(Request request, BufferedImage image) {
        try {
            Path dir = capturesDir();
            Files.createDirectories(dir);
            String stamp = new SimpleDateFormat("yyyyMMdd-HHmmss-SSS", Locale.ROOT).format(new Date());
            BufferedImage out = image;
            if (request.outWidth > 0) {
                out = resize(image, request.outWidth, request.outHeight);
            }
            Path full = dir.resolve("shot-" + stamp + ".png");
            ImageIO.write(out, "png", full.toFile());
            String previewPath = null;
            if (request.preview) {
                BufferedImage scaled = downscale(out, PREVIEW_MAX_SIDE);
                Path previewFile = dir.resolve("shot-" + stamp + "-preview.png");
                ImageIO.write(scaled, "png", previewFile.toFile());
                previewPath = relative(previewFile);
            }
            JsonObject json = new JsonObject();
            json.addProperty("image_path", relative(full));
            if (previewPath != null) json.addProperty("preview_path", previewPath);
            json.addProperty("width", out.getWidth());
            json.addProperty("height", out.getHeight());
            request.future.complete(json);
        } catch (Throwable t) {
            LOGGER.warn("agent: could not write capture", t);
            request.future.completeExceptionally(
                    new AgentException("FAILED", "could not write the capture: " + t));
        }
    }

    private static BufferedImage resize(BufferedImage source, int width, int height) {
        BufferedImage target = new BufferedImage(width, height, BufferedImage.TYPE_INT_RGB);
        Graphics2D graphics = target.createGraphics();
        try {
            graphics.setRenderingHint(RenderingHints.KEY_INTERPOLATION,
                    RenderingHints.VALUE_INTERPOLATION_BILINEAR);
            graphics.drawImage(source, 0, 0, width, height, null);
        } finally {
            graphics.dispose();
        }
        return target;
    }

    private static BufferedImage downscale(BufferedImage source, int maxSide) {
        int width = source.getWidth();
        int height = source.getHeight();
        int longest = Math.max(width, height);
        if (longest <= maxSide) return source;
        double factor = (double) maxSide / longest;
        int w = Math.max(1, (int) Math.round(width * factor));
        int h = Math.max(1, (int) Math.round(height * factor));
        return resize(source, w, h);
    }

    private static Path capturesDir() {
        Minecraft minecraft = Minecraft.getMinecraft();
        File gameDir = minecraft != null ? minecraft.mcDataDir : null;
        Path base = gameDir != null ? gameDir.toPath().toAbsolutePath().normalize()
                : Paths.get(".").toAbsolutePath().normalize();
        return base.resolve(AgentRuntime.CAPTURES_DIR);
    }

    /** Contract form: gameDir-relative, forward slashes. */
    private static String relative(Path file) {
        Minecraft minecraft = Minecraft.getMinecraft();
        File gameDir = minecraft != null ? minecraft.mcDataDir : null;
        Path base = gameDir != null ? gameDir.toPath().toAbsolutePath().normalize()
                : Paths.get(".").toAbsolutePath().normalize();
        Path abs = file.toAbsolutePath().normalize();
        try {
            return base.relativize(abs).toString().replace('\\', '/');
        } catch (IllegalArgumentException e) {
            return abs.toString().replace('\\', '/');
        }
    }
}
