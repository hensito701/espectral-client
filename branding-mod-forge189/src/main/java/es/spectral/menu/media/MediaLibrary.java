package es.spectral.menu.media;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Collectors;
import java.util.stream.Stream;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

/**
 * The music folder as a sorted track list. Scanning walks the folder and reads
 * each file's header on a daemon thread, so the menus never stall on a large
 * or slow disk; the UI shows the previous list until the new one lands.
 */
public final class MediaLibrary {

    private static final Logger LOGGER = LogManager.getLogger("espectral-media");
    private static final int MAX_DEPTH = 6;
    private static final int MAX_TRACKS = 4000;

    private volatile List<MediaTrack> tracks = Collections.emptyList();
    private volatile boolean scanning;
    private Thread worker;

    /** The last completed scan; never {@code null}. */
    public List<MediaTrack> tracks() {
        return tracks;
    }

    public boolean isScanning() {
        return scanning;
    }

    /** Looks a track up by its playlist id; {@code null} when it is no longer on disk. */
    public MediaTrack byId(String id) {
        for (MediaTrack t : tracks) {
            if (t.id().equals(id)) {
                return t;
            }
        }
        return null;
    }

    /**
     * Starts a scan unless one is already running (concurrent calls collapse
     * into the one in flight). {@code onChange} runs on the scan thread once
     * the new list is published.
     */
    public synchronized void scanAsync(Runnable onChange) {
        if (scanning) {
            return;
        }
        scanning = true;
        Thread t = new Thread(() -> {
            try {
                tracks = scan(MediaPaths.musicDir());
            } finally {
                scanning = false;
            }
            if (onChange != null) {
                onChange.run();
            }
        }, "espectral-media-scan");
        t.setDaemon(true);
        worker = t;
        t.start();
    }

    /** Synchronous scan of {@code dir}; empty when it is missing or unreadable. */
    static List<MediaTrack> scan(Path dir) {
        if (dir == null || !Files.isDirectory(dir)) {
            return Collections.emptyList();
        }
        List<MediaTrack> out = new ArrayList<>();
        try (Stream<Path> walk = Files.walk(dir, MAX_DEPTH)) {
            List<Path> files = walk
                    .filter(Files::isRegularFile)
                    .filter(p -> Mp3Header.isPlayableName(p.getFileName().toString()))
                    .limit(MAX_TRACKS)
                    .collect(Collectors.toList());
            for (Path p : files) {
                String id = dir.relativize(p).toString().replace('\\', '/');
                String[] tags = Mp3Header.readTags(p);
                long duration = Mp3Header.durationMs(p);
                out.add(new MediaTrack(p, id, tags[0], tags[1], Math.max(0L, duration)));
            }
        } catch (IOException e) {
            LOGGER.warn("Could not scan music folder {}: {}", dir, e.toString());
        }
        out.sort(Comparator.comparing(MediaTrack::id, String.CASE_INSENSITIVE_ORDER));
        return Collections.unmodifiableList(out);
    }
}
