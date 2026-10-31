package es.spectral.menu.agent;

import java.util.function.Consumer;

import org.lwjgl.system.MemoryUtil;

import com.mojang.blaze3d.pipeline.RenderTarget;

import net.minecraft.client.Screenshot;

/**
 * Frame readback for live recording (1.21.11): vanilla's screenshot readback,
 * then one bulk copy out of the {@code NativeImage} (already top row first).
 */
public final class AgentReadback {

    private AgentReadback() {}

    /**
     * Copies the top-left {@code width}×{@code height} of {@code target}'s
     * colour texture into {@code dst} (tightly packed RGBA, top row first)
     * asynchronously; {@code done} runs once {@code dst} is filled. Render
     * thread only.
     */
    public static void read(RenderTarget target, int width, int height, byte[] dst, Runnable done,
            Consumer<Throwable> failed) {
        Screenshot.takeScreenshot(target, image -> {
            try {
                int stride = image.getWidth() * 4;
                int row = width * 4;
                for (int y = 0; y < height; y++) {
                    MemoryUtil.memByteBuffer(image.getPointer() + (long) y * stride, row).get(dst, y * row, row);
                }
                done.run();
            } catch (Throwable t) {
                failed.accept(t);
            } finally {
                image.close();
            }
        });
    }
}
