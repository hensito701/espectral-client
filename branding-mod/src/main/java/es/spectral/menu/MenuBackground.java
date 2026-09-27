package es.spectral.menu;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletableFuture;

import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.MipmapStrategy;
import net.minecraft.client.renderer.texture.ReloadableTexture;
import net.minecraft.client.renderer.texture.TextureContents;
import net.minecraft.client.resources.metadata.texture.TextureMetadataSection;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.Util;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * The user-supplied title-menu background, loaded once per game session.
 *
 * <p>The PNG is decoded on {@link Util#backgroundExecutor()} (never on the
 * render thread, per the v2 product rules) and uploaded on the render thread
 * as a {@link ReloadableTexture}, so a later resource reload re-reads the file
 * instead of dropping the texture. The texture is sampled linearly with
 * clamped edges: the title menu pans it by sub-pixel UV offsets, which
 * nearest filtering would turn into shimmer.
 *
 * <p>Both supported versions expose identical texture APIs, so this lives in
 * the shared source set; only the blit call is version-specific
 * ({@code Compat.uiBlitUv}).
 */
public final class MenuBackground {

    /** Texture id of the uploaded background. */
    public static final Identifier ID = Identifier.fromNamespaceAndPath("espectral-menu", "menu_bg");

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-menu");

    private enum State { IDLE, LOADING, READY, ABSENT }

    // Written only on the render thread (request + upload callback).
    private static State state = State.IDLE;
    private static int width;
    private static int height;
    private static long readyAtMs;

    private MenuBackground() {}

    /** Where a background PNG is looked for, in priority order. */
    private static List<Path> candidates(Minecraft minecraft) {
        List<Path> out = new ArrayList<>(3);
        String override = System.getProperty("espectral.menu.background");
        if (override != null && !override.isBlank()) {
            out.add(Path.of(override));
        }
        if (minecraft.gameDirectory != null) {
            Path game = minecraft.gameDirectory.toPath();
            out.add(game.resolve("config").resolve("espectral-menu-background.png"));
            out.add(game.resolve("espectral-menu-background.png"));
        }
        return out;
    }

    /**
     * Starts the one-time asynchronous load. Safe to call every frame; only
     * the first call on a fresh session does any work.
     */
    public static void request(Minecraft minecraft) {
        if (state != State.IDLE || minecraft == null) {
            return;
        }
        state = State.LOADING;
        List<Path> paths = candidates(minecraft);
        CompletableFuture
                .supplyAsync(() -> decodeFirst(paths), Util.backgroundExecutor())
                .whenCompleteAsync((found, error) -> upload(minecraft, found, error), minecraft);
    }

    /** Decoded source waiting for upload. */
    private record Decoded(Path path, TextureContents contents) {}

    private static Decoded decodeFirst(List<Path> paths) {
        for (Path p : paths) {
            if (!Files.isRegularFile(p)) {
                continue;
            }
            try {
                return new Decoded(p, read(p));
            } catch (IOException | RuntimeException e) {
                LOGGER.warn("[espectral] menu background unreadable {}: {}", p, e.toString());
            }
        }
        return null;
    }

    private static TextureContents read(Path path) throws IOException {
        try (InputStream in = Files.newInputStream(path)) {
            return new TextureContents(NativeImage.read(in),
                    new TextureMetadataSection(true, true, MipmapStrategy.AUTO,
                            TextureMetadataSection.DEFAULT_ALPHA_CUTOFF_BIAS));
        }
    }

    private static void upload(Minecraft minecraft, Decoded found, Throwable error) {
        if (error != null || found == null) {
            state = State.ABSENT;
            if (error != null) {
                LOGGER.warn("[espectral] menu background load failed: {}", error.toString());
            } else {
                LOGGER.info("[espectral] no menu background PNG; using the branded fallback");
            }
            return;
        }
        Path path = found.path();
        try {
            int w = found.contents().image().getWidth();
            int h = found.contents().image().getHeight();
            ReloadableTexture texture = new ReloadableTexture(ID) {
                @Override
                public TextureContents loadContents(ResourceManager resources) throws IOException {
                    return read(path);
                }
            };
            texture.apply(found.contents()); // uploads and closes the image
            minecraft.getTextureManager().register(ID, texture);
            width = w;
            height = h;
            readyAtMs = Util.getMillis();
            state = State.READY;
            LOGGER.info("[espectral] menu background loaded {} ({}x{})", path, w, h);
        } catch (RuntimeException e) {
            found.contents().close();
            state = State.ABSENT;
            LOGGER.warn("[espectral] menu background upload failed {}: {}", path, e.toString());
        }
    }

    /** True once the texture is uploaded and drawable. */
    public static boolean ready() {
        return state == State.READY;
    }

    public static int width() {
        return width;
    }

    public static int height() {
        return height;
    }

    /** {@link Util#getMillis()} timestamp of the upload (drives the reveal fade). */
    public static long readyAtMs() {
        return readyAtMs;
    }
}
