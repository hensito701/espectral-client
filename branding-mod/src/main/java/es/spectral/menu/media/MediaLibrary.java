package es.spectral.menu.media;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * The library: every {@code .mp3} found under {@link MediaPaths#musicDir()}.
 *
 * <p>Scanning happens on one daemon thread. Results are published as an
 * immutable snapshot in a volatile field, so the render thread and the sound
 * thread read a plain field — no lock, no allocation per frame. Until the
 * first scan finishes the list is empty (the screen shows "scanning…").
 */
public final class MediaLibrary {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-media");

    /** Immutable published snapshot; replaced on each completed scan. */
    private volatile List<MediaTrack> tracks = List.of();
    private volatile boolean scanning;
    private Thread worker;

    public List<MediaTrack> tracks() {
        return tracks;
    }

    public boolean isScanning() {
        return scanning;
    }

    /**
     * Starts a background scan. Concurrent calls are collapsed into the
     * running one ({@code onChange} of a dropped request is not invoked).
     * {@code onChange} runs on the scan thread: screens only flip a flag and
     * repaint on their own tick.
     */
    public synchronized void scanAsync(Runnable onChange) {
        if (scanning) {
            return;
        }
        scanning = true;
        worker = new Thread(() -> {
            try {
                this.tracks = scan(MediaPaths.musicDir());
            } catch (Throwable e) {
                LOGGER.warn("Music scan failed: {}", e.toString());
            } finally {
                scanning = false;
            }
            if (onChange != null) {
                onChange.run();
            }
        }, "espectral-media-scan");
        worker.setDaemon(true);
        worker.start();
    }

    private static List<MediaTrack> scan(Path dir) {
        List<MediaTrack> out = new ArrayList<>();
        if (dir == null || !Files.isDirectory(dir)) {
            return List.of();
        }
        try (java.util.stream.Stream<Path> walk = Files.walk(dir, 6)) {
            walk.filter(Files::isRegularFile)
                    .filter(p -> Mp3Header.isPlayableName(p.getFileName().toString()))
                    .limit(4000)
                    .forEach(p -> {
                        String id = dir.relativize(p).toString().replace('\\', '/');
                        String[] tags = Mp3Header.readTags(p);
                        long duration = Mp3Header.durationMs(p);
                        out.add(new MediaTrack(p, id, tags[0], tags[1], Math.max(0L, duration)));
                    });
        } catch (Exception e) {
            LOGGER.warn("Music folder walk failed: {}", e.toString());
        }
        out.sort(Comparator.comparing(MediaTrack::id, String.CASE_INSENSITIVE_ORDER));
        return List.copyOf(out);
    }

    /** Track with this playlist id, or {@code null} when the file disappeared. */
    public MediaTrack byId(String id) {
        if (id == null || id.isEmpty()) {
            return null;
        }
        for (MediaTrack track : tracks) {
            if (track.id().equals(id)) {
                return track;
            }
        }
        return null;
    }
}
