package es.spectral.menu.media;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import net.fabricmc.loader.api.FabricLoader;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Where the user's music lives: <b>one folder shared by every instance</b>,
 * {@code <launcher data dir>/music}, passed in by the launcher as the
 * {@value #MUSIC_DIR_PROPERTY} JVM property (see {@code native/engine}'s
 * launch-arg assembly, which also creates it).
 *
 * <p>Without the property (a hand-made launch, curseforge, a dev run) it falls
 * back to {@code <gameDir>/music}, so the feature still works standalone.
 */
public final class MediaPaths {

    /** Must match {@code native/engine/src/launch.rs}. */
    public static final String MUSIC_DIR_PROPERTY = "espectral.music.dir";

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-media");
    private static Path cached;

    private MediaPaths() {}

    /** The music folder, created on first call; never {@code null}. */
    public static synchronized Path musicDir() {
        if (cached != null) {
            return cached;
        }
        Path dir = null;
        String property = System.getProperty(MUSIC_DIR_PROPERTY);
        if (property != null && !property.isBlank()) {
            try {
                dir = Path.of(property);
            } catch (Exception e) {
                LOGGER.warn("Ignoring invalid {}={}: {}", MUSIC_DIR_PROPERTY, property, e.toString());
            }
        }
        if (dir == null) {
            dir = FabricLoader.getInstance().getGameDir().resolve("music");
        }
        try {
            Files.createDirectories(dir);
        } catch (Exception e) {
            LOGGER.warn("Could not create music folder {}: {}", dir, e.toString());
        }
        cached = dir;
        return dir;
    }

    /** Opens the folder in the OS file manager through the game's own helper. */
    public static void openFolder() {
        Path dir = musicDir();
        try {
            es.spectral.menu.Compat.openPath(dir);
        } catch (Throwable e) {
            LOGGER.warn("Could not open music folder {}: {}", dir, e.toString());
        }
    }
}
