package es.spectral.menu.agent;

import java.nio.ByteBuffer;
import java.util.function.Consumer;

import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.textures.GpuTexture;

/**
 * Frame readback for live recording (26.3, renderpearl). Same GPU copy as
 * vanilla's {@code Screenshot.takeScreenshot}, but the mapped rows are bulk
 * copied (bottom-up GL rows → top-down) instead of vanilla's per-pixel
 * {@code setPixelABGR} loop, which alone halved the game's fps while recording.
 */
public final class AgentReadback {

    /** Reused readback buffers: allocating 8 MB of GPU memory per frame costs fps. */
    private static final java.util.ArrayDeque<GpuBuffer> FREE = new java.util.ArrayDeque<>();
    private static long freeSize;

    private AgentReadback() {}

    private static GpuBuffer acquire(long size) {
        if (size != freeSize) {
            while (!FREE.isEmpty()) FREE.pop().close();
            freeSize = size;
        }
        GpuBuffer buffer = FREE.poll();
        return buffer != null ? buffer : RenderSystem.getDevice().createBuffer(() -> "espectral agent recording",
                GpuBuffer.USAGE_MAP_READ | GpuBuffer.USAGE_COPY_DST, size);
    }

    private static void release(GpuBuffer buffer) {
        if (buffer.size() == freeSize && FREE.size() < 4) FREE.push(buffer);
        else buffer.close();
    }

    /**
     * Copies the top-left {@code width}×{@code height} of {@code target}'s
     * colour texture into {@code dst} (tightly packed RGBA, top row first)
     * asynchronously; {@code done} runs on the render thread once {@code dst}
     * is filled. Render thread only.
     */
    public static void read(RenderTarget target, int width, int height, byte[] dst, Runnable done,
            Consumer<Throwable> failed) {
        GpuTexture texture = target.getColorTexture();
        if (texture == null) {
            failed.accept(new IllegalStateException("incomplete framebuffer"));
            return;
        }
        int fullWidth = target.width;
        int fullHeight = target.height;
        int pixel = texture.getFormat().blockSize();
        GpuBuffer buffer = acquire((long) fullWidth * fullHeight * pixel);
        RenderSystem.getDevice().createCommandEncoder().copyTextureToBuffer(texture, buffer, 0L, () -> {
            try (GpuBufferSlice.MappedView view = buffer.map(true, false)) {
                ByteBuffer data = view.data();
                int row = width * 4;
                for (int y = 0; y < height; y++) {
                    data.get((fullHeight - 1 - y) * fullWidth * pixel, dst, y * row, row);
                }
                done.run();
            } catch (Throwable t) {
                failed.accept(t);
            } finally {
                release(buffer);
            }
        }, 0);
    }
}
