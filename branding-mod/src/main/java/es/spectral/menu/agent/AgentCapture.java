package es.spectral.menu.agent;

import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonObject;
import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.platform.NativeImage;

import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;

/**
 * {@code screenshot}: end-of-frame capture of the main render target (so
 * Iris/Sodium have already composited), written under
 * {@code <gameDir>/espectral-captures/} with an optional ≤1280 px preview.
 *
 * <p>The grab is issued at the tail of {@code Minecraft.renderFrame}: the tool
 * handler parks a {@link CompletableFuture} and the per-lane frame hook
 * fulfils it in that same frame, after the GUI pass. Vanilla's readback
 * consumer runs later on the GPU queue; it only copies a {@link NativeImage}
 * and hands the PNG encoding to a daemon executor, so a 4K frame never stalls
 * a tick.
 */
public final class AgentCapture {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    /** Preview width ceiling from the contract (the engine inlines it as base64). */
    public static final int PREVIEW_MAX_WIDTH = 1280;

    private static final DateTimeFormatter STAMP =
            DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss-SSS");

    private static final Object LOCK = new Object();
    /** Request awaiting the next end-of-frame, or null. */
    private static Request pending;
    private static boolean inFlight;

    /** One grabbed frame: the file paths plus pixel dimensions. */
    public record Grabbed(String imagePath, String previewPath, int width, int height) {}

    private static final class Request {
        final CompletableFuture<Grabbed> future = new CompletableFuture<>();
        final boolean preview;
        /** Frames still to skip (lets a resize/shader change settle). */
        int delayFrames = 1;

        Request(boolean preview) {
            this.preview = preview;
        }
    }

    private AgentCapture() {}

    /**
     * Requests a frame. Called on the render thread; the future completes once
     * the PNG(s) are on disk.
     */
    public static CompletableFuture<Grabbed> request(boolean wantPreview) {
        Request request = new Request(wantPreview);
        synchronized (LOCK) {
            if (pending != null) {
                request.future.completeExceptionally(
                        new AgentException("BUSY", "a screenshot is already in flight"));
                return request.future;
            }
            pending = request;
        }
        return request.future;
    }

    /** Frame hook: grabs when a request is parked and the frame is settled. */
    public static void onFrameEnd(Minecraft minecraft) {
        Request request;
        synchronized (LOCK) {
            request = pending;
            if (request == null || inFlight) return;
            if (request.delayFrames > 0) {
                request.delayFrames--;
                return;
            }
            inFlight = true;
            pending = null;
        }
        RenderTarget target = null;
        try {
            if (minecraft != null) {
                // Compat resolves the lane's accessor: gameRenderer's main
                // target on 26.x, Minecraft.getMainRenderTarget on 1.21.11.
                target = es.spectral.menu.Compat.mainRenderTarget(minecraft);
            }
        } catch (Throwable t) {
            LOGGER.debug("agent: no render target: {}", t.toString());
        }
        if (target == null) {
            fail(request, "FAILED", "the main render target is not available");
            return;
        }
        final Request done = request;
        try {
            Screenshot.takeScreenshot(target, image -> consume(done, image));
        } catch (Throwable t) {
            LOGGER.debug("agent: screenshot failed: {}", t.toString());
            fail(done, "FAILED", "screenshot readback failed: " + t);
        }
    }

    private static void consume(Request request, NativeImage image) {
        if (image == null) {
            fail(request, "FAILED", "the frame came back empty");
            return;
        }
        NativeImage copy;
        try {
            copy = copyOf(image);
        } catch (Throwable t) {
            fail(request, "FAILED", "could not copy the frame: " + t);
            return;
        } finally {
            try {
                image.close();
            } catch (Throwable ignored) {
                // already closed
            }
        }
        CompletableFuture.runAsync(() -> writeFiles(request, copy));
    }

    private static NativeImage copyOf(NativeImage source) {
        NativeImage copy = new NativeImage(source.getWidth(), source.getHeight(), false);
        copy.copyFrom(source);
        return copy;
    }

    private static void writeFiles(Request request, NativeImage image) {
        try {
            Path dir = capturesDir(Minecraft.getInstance());
            Files.createDirectories(dir);
            String stamp = LocalDateTime.now().format(STAMP);
            Path full = dir.resolve("shot-" + stamp + ".png");
            image.writeToFile(full);
            String previewPath = null;
            if (request.preview) {
                Path previewFile = dir.resolve("shot-" + stamp + "-preview.png");
                NativeImage scaled = downscale(image, PREVIEW_MAX_WIDTH);
                try {
                    scaled.writeToFile(previewFile);
                } finally {
                    scaled.close();
                }
                previewPath = relative(previewFile);
            }
            Grabbed grabbed = new Grabbed(relative(full), previewPath,
                    image.getWidth(), image.getHeight());
            request.future.complete(grabbed);
        } catch (Throwable t) {
            LOGGER.warn("agent: could not write capture: {}", t.toString());
            fail(request, "FAILED", "could not write the capture: " + t);
        } finally {
            try {
                image.close();
            } catch (Throwable ignored) {
                // already closed
            }
            synchronized (LOCK) {
                inFlight = false;
            }
        }
    }

    private static void fail(Request request, String code, String message) {
        synchronized (LOCK) {
            inFlight = false;
        }
        request.future.completeExceptionally(new AgentException(code, message));
    }

    /** Resize through stb (same pixel format required); returns a fresh image. */
    private static NativeImage downscale(NativeImage source, int maxWidth) {
        int width = source.getWidth();
        int height = source.getHeight();
        if (width <= maxWidth || width <= 0 || height <= 0) {
            NativeImage copy = new NativeImage(width, height, false);
            copy.copyFrom(source);
            return copy;
        }
        int targetHeight = Math.max(1, (int) Math.round(height * (maxWidth / (double) width)));
        NativeImage scaled = new NativeImage(maxWidth, targetHeight, false);
        source.resizeSubRectTo(0, 0, width, height, scaled);
        return scaled;
    }

    /** {@code <gameDir>/espectral-captures}. */
    public static Path capturesDir(Minecraft minecraft) {
        File gameDir = minecraft != null ? minecraft.gameDirectory : null;
        Path base = gameDir != null ? gameDir.toPath() : Path.of(".");
        return base.resolve(AgentRuntime.CAPTURES_DIR);
    }

    /** Contract form: gameDir-relative, forward slashes. */
    public static String relative(Path file) {
        Minecraft minecraft = Minecraft.getInstance();
        File gameDir = minecraft != null ? minecraft.gameDirectory : null;
        Path base = gameDir != null ? gameDir.toPath().toAbsolutePath().normalize()
                : Path.of(".").toAbsolutePath().normalize();
        Path abs = file.toAbsolutePath().normalize();
        try {
            return base.relativize(abs).toString().replace('\\', '/');
        } catch (IllegalArgumentException e) {
            return abs.toString().replace('\\', '/');
        }
    }

    /** Result payload for a completed grab. */
    public static JsonObject toJson(Grabbed grabbed) {
        JsonObject json = new JsonObject();
        json.addProperty("image_path", grabbed.imagePath());
        if (grabbed.previewPath() != null) {
            json.addProperty("preview_path", grabbed.previewPath());
        }
        json.addProperty("width", grabbed.width());
        json.addProperty("height", grabbed.height());
        return json;
    }

}
