package es.spectral.menu.media;

import java.awt.Desktop;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

import net.minecraft.client.Minecraft;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

/**
 * Where the user's music lives: <b>one folder shared by every instance</b>,
 * {@code <launcher data dir>/music}, passed in by the launcher as the
 * {@value #MUSIC_DIR_PROPERTY} JVM property (see {@code native/engine}'s
 * launch-arg assembly, which also creates it).
 *
 * <p>Without the property (a hand-made launch, curseforge, a dev run) it falls
 * back to {@code <mcDataDir>/music}, so the feature still works standalone.
 */
public final class MediaPaths {

    /** Must match {@code native/engine/src/launch.rs}. */
    public static final String MUSIC_DIR_PROPERTY = "espectral.music.dir";

    private static final Logger LOGGER = LogManager.getLogger("espectral-media");
    private static Path cached;

    private MediaPaths() {}

    /** The music folder, created on first call; never {@code null}. */
    public static synchronized Path musicDir() {
        if (cached != null) {
            return cached;
        }
        Path dir = null;
        String property = System.getProperty(MUSIC_DIR_PROPERTY);
        if (property != null && !property.trim().isEmpty()) {
            try {
                dir = Paths.get(property);
            } catch (Exception e) {
                LOGGER.warn("Ignoring invalid {}={}: {}", MUSIC_DIR_PROPERTY, property, e.toString());
            }
        }
        if (dir == null) {
            dir = Minecraft.getMinecraft().mcDataDir.toPath().resolve("music");
        }
        try {
            Files.createDirectories(dir);
        } catch (Exception e) {
            LOGGER.warn("Could not create music folder {}: {}", dir, e.toString());
        }
        cached = dir;
        return dir;
    }

    /** Opens the folder in the OS file manager; a no-op where the desktop API is absent. */
    public static void openFolder() {
        Path dir = musicDir();
        try {
            if (Desktop.isDesktopSupported()) {
                Desktop.getDesktop().open(dir.toFile());
            }
        } catch (Throwable e) {
            LOGGER.warn("Could not open music folder {}: {}", dir, e.toString());
        }
    }
}
