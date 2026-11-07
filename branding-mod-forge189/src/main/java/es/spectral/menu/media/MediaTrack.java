package es.spectral.menu.media;

import java.nio.file.Path;

/**
 * One playable file of the user's music folder.
 *
 * <p>{@link #id()} is the path relative to the music folder with {@code '/'}
 * separators: it is what playlists persist (the folder itself moves between
 * machines and launcher data dirs, so an absolute path must never be stored).
 * Title/artist come from the file's ID3 tags when present and fall back to the
 * file name; both are resolved off-thread by {@link MediaLibrary}.
 */
public final class MediaTrack {

    private final Path path;
    private final String id;
    private final String title;
    private final String artist;
    private final long durationMs;

    MediaTrack(Path path, String id, String title, String artist, long durationMs) {
        this.path = path;
        this.id = id;
        this.title = title;
        this.artist = artist;
        this.durationMs = durationMs;
    }

    public Path path() {
        return path;
    }

    /** Stable playlist key: relative path, {@code '/'}-separated. */
    public String id() {
        return id;
    }

    public String title() {
        return title;
    }

    public String artist() {
        return artist;
    }

    /** Total length in milliseconds, or {@code 0} when the header scan failed. */
    public long durationMs() {
        return durationMs;
    }

    /** {@code "Title — Artist"}, or just the title when untagged. */
    public String displayName() {
        return artist.isEmpty() ? title : title + " \u2014 " + artist;
    }
}
