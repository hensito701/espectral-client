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

    /** Preview longest-side ceiling (the engine inlines previews as base64). */
    public static final int PREVIEW_MAX_SIDE = 1280;

    /** Custom-size limits from the contract: each side, and total pixels. */
    public static final int MIN_SIDE = 64;
    public static final int MAX_SIDE = 7680;
    public static final long MAX_PIXELS = 33_200_000L;

    /**
     * Frames rendered at a custom size before the grab. Iris rebuilds its
     * targets on the first one; temporal passes (TAA in Complementary) need a
     * few more before the frame is clean.
     */
    private static final int SIZED_SETTLE_FRAMES = 6;

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
        /** Requested size, or 0×0 for the window size. */
        final int width;
        final int height;
        final boolean hud;
        /** Frames still to skip (lets a resize/shader change settle). */
        int delayFrames = 1;
        /** What the capture changed (size/HUD), restored right after the grab. */
        Saved saved;

        Request(boolean preview, int width, int height, boolean hud) {
            this.preview = preview;
            this.width = width;
            this.height = height;
            this.hud = hud;
        }

        boolean sized() {
            return width > 0 && height > 0;
        }
    }

    /**
     * What a capture changed on the render path, restored right after the grab.
     * Sizes go through {@code Window.setWidth/setHeight} — the same route
     * vanilla's panorama capture uses — so the camera projection, the main
     * target and Iris's targets all follow on the next frames without
     * {@code resizeGui()} re-initialising the open screen.
     */
    private record Saved(int width, int height, int guiScale, boolean hideGui, boolean resized) {}

    private AgentCapture() {}

    /**
     * Requests a frame. The future completes once the PNG(s) are on disk.
     * {@code width/height} 0 = window size; {@code hud=false} hides the HUD
     * (like F1) for the grab only.
     */
    public static CompletableFuture<Grabbed> request(boolean wantPreview, int width, int height,
            boolean hud) {
        Request request = new Request(wantPreview, width, height, hud);
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
        }
        if (request.saved == null && (request.sized() || !request.hud)) {
            try {
                request.saved = apply(minecraft, request);
            } catch (AgentException e) {
                synchronized (LOCK) {
                    pending = null;
                }
                request.future.completeExceptionally(e);
                return;
            } catch (Throwable t) {
                synchronized (LOCK) {
                    pending = null;
                }
                fail(request, "FAILED", "could not prepare the capture: " + t);
                return;
            }
            request.delayFrames = request.sized() ? SIZED_SETTLE_FRAMES : 1;
            return;
        }
        synchronized (LOCK) {
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
            restore(minecraft, request.saved);
            fail(request, "FAILED", "the main render target is not available");
            return;
        }
        if (request.sized() && (target.width != request.width || target.height != request.height)) {
            // The window was resized mid-capture (the resize event overwrote
            // the override): never hand back a frame of the wrong size.
            restore(minecraft, request.saved);
            fail(request, "FAILED", "the frame came out " + target.width + "x" + target.height
                    + " instead of " + request.width + "x" + request.height
                    + " (window resized during the capture?); retry");
            return;
        }
        final Request done = request;
        try {
            Screenshot.takeScreenshot(target, image -> consume(done, image));
        } catch (Throwable t) {
            LOGGER.debug("agent: screenshot failed: {}", t.toString());
            fail(done, "FAILED", "screenshot readback failed: " + t);
        } finally {
            // The readback is already encoded against this frame's texture;
            // the next frame renders at the window size again.
            restore(minecraft, request.saved);
        }
    }

    /** Switches the render path to the requested size / HUD state. Render thread. */
    private static Saved apply(Minecraft minecraft, Request request) throws AgentException {
        if (minecraft == null) throw new AgentException("FAILED", "no client");
        var window = minecraft.getWindow();
        int guiScale = window.getGuiScale();
        boolean hideGui = es.spectral.menu.Compat.hudHidden(minecraft);
        if (request.sized()) {
            if (!es.spectral.menu.Compat.supportsSizedCapture()) {
                throw new AgentException("UNSUPPORTED",
                        "custom-size capture needs Minecraft 26.2 or newer; capture at the window size");
            }
            if (es.spectral.menu.Compat.getScreen(minecraft) != null) {
                throw new AgentException("BAD_ARGS",
                        "custom-size capture renders the world view; close the open screen first "
                                + "(key escape) or capture at the window size");
            }
            if (AgentRecorder.sized()) {
                throw new AgentException("BUSY",
                        "a recording is rendering at its own size; record_stop first, or capture "
                                + "without width/height");
            }
        }
        Saved saved = new Saved(window.getWidth(), window.getHeight(), guiScale, hideGui,
                request.sized());
        if (request.sized()) {
            window.setWidth(request.width);
            window.setHeight(request.height);
            // The HUD lays itself out from the gui-scaled size each frame, so a
            // scale computed for the capture size keeps it proportionate.
            window.setGuiScale(window.calculateScale(minecraft.options.guiScale().get(),
                    minecraft.isEnforceUnicode()));
        }
        if (!request.hud) {
            es.spectral.menu.Compat.setHudHidden(minecraft, true);
        }
        return saved;
    }

    private static void restore(Minecraft minecraft, Saved saved) {
        if (minecraft == null || saved == null) return;
        try {
            var window = minecraft.getWindow();
            if (saved.resized()) {
                window.setWidth(saved.width());
                window.setHeight(saved.height());
                window.setGuiScale(saved.guiScale());
            }
            es.spectral.menu.Compat.setHudHidden(minecraft, saved.hideGui());
        } catch (Throwable t) {
            LOGGER.warn("agent: could not restore the window after a capture: {}", t.toString());
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
                NativeImage scaled = downscale(image, PREVIEW_MAX_SIDE);
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

    /**
     * Resize through stb (same pixel format required); returns a fresh image
     * whose longest side is at most {@code maxSide} — a 1080×1920 portrait
     * preview is 720×1280, so previews stay small enough to inline.
     */
    private static NativeImage downscale(NativeImage source, int maxSide) {
        int width = source.getWidth();
        int height = source.getHeight();
        int longest = Math.max(width, height);
        if (longest <= maxSide || width <= 0 || height <= 0) {
            NativeImage copy = new NativeImage(width, height, false);
            copy.copyFrom(source);
            return copy;
        }
        double scale = maxSide / (double) longest;
        int targetWidth = Math.max(1, (int) Math.round(width * scale));
        int targetHeight = Math.max(1, (int) Math.round(height * scale));
        NativeImage scaled = new NativeImage(targetWidth, targetHeight, false);
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
