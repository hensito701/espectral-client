package es.spectral.menu.media;

import java.nio.ByteBuffer;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;


import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;

import es.spectral.menu.ClientConfig;

/**
 * The player state machine: queue, shuffle, repeat, transport, volume and the
 * now-playing toast. All of it runs on the client thread; the decode thread
 * only fills the ring and the sound thread only drains it.
 *
 * <p>Persistence uses the existing per-feature raw object
 * ({@code features.mediaplayer}) in {@code espectral-client.json}, so the
 * schema stays 2 and the launcher's reader keeps working untouched.
 */
public final class MediaPlayer {

    public static final String FEATURE_ID = "mediaplayer";
    public static final String REPEAT_OFF = "off";
    public static final String REPEAT_ALL = "all";
    public static final String REPEAT_ONE = "one";

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-media");
    private static final MediaPlayer INSTANCE = new MediaPlayer();

    /** Toast lifetime in ticks (3 s). */
    private static final int TOAST_TICKS = 60;

    public static MediaPlayer get() {
        return INSTANCE;
    }

    private final MediaLibrary library = new MediaLibrary();
    private final MediaRing ring = new MediaRing();
    private final MediaDecoder decoder = new MediaDecoder(ring);

    /** Playback order for the current session (ids), rebuilt when the source changes. */
    private final List<String> queue = new ArrayList<>();
    private int queueIndex = -1;
    private String sourcePlaylist = "";

    private volatile boolean playing;
    private volatile boolean paused;
    private volatile float volume = 0.7F;
    private boolean shuffle;
    private String repeat = REPEAT_OFF;
    private boolean duckMusic = true;
    private boolean toastEnabled = true;

    private MediaTrack current;
    private long startedAt;
    private long elapsedBaseMs;
    private boolean eofHandled;

    private String toastText = "";
    private int toastTicks;

    private MediaPlayer() {}

    // ---------------------------------------------------------------- state

    public MediaLibrary library() {
        return library;
    }

    public MediaTrack current() {
        return current;
    }

    public boolean isPlaying() {
        return playing;
    }

    public boolean isPaused() {
        return paused;
    }

    public float volume() {
        return volume;
    }

    public boolean shuffle() {
        return shuffle;
    }

    public String repeat() {
        return repeat;
    }

    public boolean duckMusic() {
        return duckMusic;
    }

    public boolean toastEnabled() {
        return toastEnabled;
    }

    public String currentPlaylistId() {
        return sourcePlaylist;
    }

    /** Elapsed playback time in ms, advanced while playing. */
    public long elapsedMs() {
        if (!playing || paused || current == null) {
            return Math.max(0L, elapsedBaseMs);
        }
        long wall = System.currentTimeMillis() - startedAt;
        long total = current.durationMs();
        long value = elapsedBaseMs + Math.max(0L, wall);
        return total > 0 ? Math.min(total, value) : Math.max(0L, value);
    }

    // -------------------------------------------------------------- settings

    /** Loads the persisted settings from the feature's raw object. */
    public void reloadSettings() {
        ClientConfig config = ClientConfig.getInstance();
        com.google.gson.JsonObject obj = config.featureRaw(FEATURE_ID);
        float v = 0.7F;
        boolean sh = false;
        String rep = REPEAT_OFF;
        boolean duck = true;
        boolean toast = true;
        if (obj != null) {
            if (obj.has("volume")) {
                try {
                    v = (float) Math.max(0.0, Math.min(1.0, obj.get("volume").getAsDouble()));
                } catch (Exception ignored) {
                    // keep the default
                }
            }
            if (obj.has("shuffle")) {
                sh = obj.get("shuffle").getAsBoolean();
            }
            if (obj.has("repeat")) {
                rep = obj.get("repeat").getAsString();
                if (!REPEAT_ALL.equals(rep) && !REPEAT_ONE.equals(rep)) {
                    rep = REPEAT_OFF;
                }
            }
            if (obj.has("duckMusic")) {
                duck = obj.get("duckMusic").getAsBoolean();
            }
            if (obj.has("toast")) {
                toast = obj.get("toast").getAsBoolean();
            }
        }
        this.volume = v;
        this.shuffle = sh;
        this.repeat = rep;
        this.duckMusic = duck;
        this.toastEnabled = toast;
        List<Playlist> loaded = config.getPlaylists(FEATURE_ID);
        this.playlists.clear();
        this.playlists.addAll(loaded);
    }

    /** Playlist snapshot (immutable entries) for the sub-screen. */
    public List<Playlist> playlists() {
        return List.copyOf(playlists);
    }

    public Playlist playlistById(String id) {
        for (Playlist playlist : playlists) {
            if (playlist.id().equals(id)) {
                return playlist;
            }
        }
        return null;
    }

    /** Creates a playlist named {@code name} and returns it, or {@code null} at the cap. */
    public Playlist createPlaylist(String name) {
        if (playlists.size() >= ClientConfig.MAX_PLAYLISTS) {
            return null;
        }
        Playlist created = new Playlist(ClientConfig.getInstance().newPlaylistId(name), name, List.of());
        playlists.add(created);
        persistPlaylists();
        return created;
    }

    public void renamePlaylist(String id, String name) {
        for (int i = 0; i < playlists.size(); i++) {
            Playlist playlist = playlists.get(i);
            if (playlist.id().equals(id)) {
                playlists.set(i, new Playlist(playlist.id(), name, playlist.tracks()));
                persistPlaylists();
                return;
            }
        }
    }

    public void deletePlaylist(String id) {
        if (playlists.removeIf(playlist -> playlist.id().equals(id))) {
            if (id.equals(sourcePlaylist)) {
                stop();
                sourcePlaylist = "";
            }
            persistPlaylists();
        }
    }

    public void addToPlaylist(String id, String trackId) {
        for (int i = 0; i < playlists.size(); i++) {
            Playlist playlist = playlists.get(i);
            if (!playlist.id().equals(id)) {
                continue;
            }
            List<String> tracks = new ArrayList<>(playlist.tracks());
            if (!tracks.contains(trackId)) {
                tracks.add(trackId);
            }
            playlists.set(i, new Playlist(playlist.id(), playlist.name(), List.copyOf(tracks)));
            persistPlaylists();
            return;
        }
    }

    public void removeFromPlaylist(String id, String trackId) {
        for (int i = 0; i < playlists.size(); i++) {
            Playlist playlist = playlists.get(i);
            if (!playlist.id().equals(id)) {
                continue;
            }
            List<String> tracks = new ArrayList<>(playlist.tracks());
            tracks.remove(trackId);
            playlists.set(i, new Playlist(playlist.id(), playlist.name(), List.copyOf(tracks)));
            persistPlaylists();
            return;
        }
    }

    /** Moves a track inside a playlist by {@code delta} (-1 up, +1 down). */
    public void moveInPlaylist(String id, int index, int delta) {
        for (int i = 0; i < playlists.size(); i++) {
            Playlist playlist = playlists.get(i);
            if (!playlist.id().equals(id)) {
                continue;
            }
            List<String> tracks = new ArrayList<>(playlist.tracks());
            int target = index + delta;
            if (index < 0 || index >= tracks.size() || target < 0 || target >= tracks.size()) {
                return;
            }
            String moved = tracks.remove(index);
            tracks.add(target, moved);
            playlists.set(i, new Playlist(playlist.id(), playlist.name(), List.copyOf(tracks)));
            persistPlaylists();
            return;
        }
    }

    private void persistPlaylists() {
        ClientConfig.getInstance().setPlaylists(FEATURE_ID, playlists);
    }

    /** Immutable playlist entry: stable id, display name, ordered track ids. */
    public record Playlist(String id, String name, List<String> tracks) {}

    private final List<Playlist> playlists = new ArrayList<>();

    private void saveSetting(String key, boolean value) {
        ClientConfig.getInstance().setFeatureFlag(FEATURE_ID, key, value);
    }

    private void saveSetting(String key, double value) {
        ClientConfig.getInstance().setFeatureFlag(FEATURE_ID, key, value);
    }

    private void saveSetting(String key, String value) {
        ClientConfig.getInstance().setFeatureFlag(FEATURE_ID, key, value);
    }

    public void setVolume(float value) {
        float clamped = Math.max(0.0F, Math.min(1.0F, value));
        if (Math.abs(clamped - this.volume) < 0.005F) {
            return;
        }
        this.volume = clamped;
        saveSetting("volume", (double) Math.round(clamped * 100.0) / 100.0);
        MediaAudio.get().applyVolume(clamped);
    }

    public void toggleShuffle() {
        this.shuffle = !this.shuffle;
        saveSetting("shuffle", this.shuffle);
        rebuildQueue();
    }

    public void cycleRepeat() {
        this.repeat = switch (this.repeat) {
            case REPEAT_ALL -> REPEAT_ONE;
            case REPEAT_ONE -> REPEAT_OFF;
            default -> REPEAT_ALL;
        };
        saveSetting("repeat", this.repeat);
    }

    public void setDuckMusic(boolean value) {
        this.duckMusic = value;
        saveSetting("duckMusic", value);
        if (value) {
            MediaAudio.get().duckVanillaMusic(playing && !paused);
        } else {
            MediaAudio.get().duckVanillaMusic(false);
        }
    }

    public void setToastEnabled(boolean value) {
        this.toastEnabled = value;
        saveSetting("toast", value);
    }

    // -------------------------------------------------------------- library

    /** Kicks a background rescan; {@code onChange} runs on the scan thread when done. */
    public void refreshLibrary(Runnable onChange) {
        library.scanAsync(onChange);
    }

    // ------------------------------------------------------------- transport

    /** Plays one track right away, keeping the current queue as the source. */
    public void playTrack(MediaTrack track) {
        if (track == null) {
            return;
        }
        this.current = track;
        this.queueIndex = queue.indexOf(track.id());
        startDecode(track);
        persistLast();
    }

    /** Re-derives the queue order from the active source after a shuffle flip. */
    private void rebuildQueue() {
        if (queue.isEmpty() || sourcePlaylist == null) {
            return;
        }
        String keep = current == null ? null : current.id();
        List<String> ids = new ArrayList<>();
        Playlist playlist = playlistById(sourcePlaylist);
        if (playlist != null) {
            ids.addAll(playlist.tracks());
        } else {
            for (MediaTrack track : library.tracks()) {
                ids.add(track.id());
            }
        }
        if (shuffle) {
            Collections.shuffle(ids);
        }
        queue.clear();
        queue.addAll(ids);
        queueIndex = keep == null ? -1 : queue.indexOf(keep);
    }

    /** Replaces the queue with {@code ids} (playlist or the whole library) and plays the first. */
    public void playQueue(String playlistId, List<String> ids) {
        this.sourcePlaylist = playlistId == null ? "" : playlistId;
        this.queue.clear();
        this.queue.addAll(ids);
        if (shuffle) {
            Collections.shuffle(this.queue);
        }
        this.queueIndex = -1;
        advance(1);
    }

    public void togglePlayPause() {
        if (current == null) {
            List<String> all = new ArrayList<>();
            for (MediaTrack track : library.tracks()) {
                all.add(track.id());
            }
            playQueue("", all);
            return;
        }
        if (paused) {
            resume();
        } else {
            pause();
        }
    }

    public void pause() {
        if (!playing || paused) {
            return;
        }
        this.elapsedBaseMs = elapsedMs();
        this.paused = true;
        MediaAudio.get().setPaused(true);
        MediaAudio.get().duckVanillaMusic(false);
    }

    public void resume() {
        if (!playing || !paused) {
            return;
        }
        this.paused = false;
        this.startedAt = System.currentTimeMillis();
        MediaAudio.get().setPaused(false);
        MediaAudio.get().duckVanillaMusic(duckMusic);
    }

    public void stop() {
        this.playing = false;
        this.paused = false;
        this.current = null;
        this.queueIndex = -1;
        this.elapsedBaseMs = 0L;
        decoder.stop();
        ring.clear();
        MediaAudio.get().setPaused(false);
        MediaAudio.get().stopStreaming();
        MediaAudio.get().duckVanillaMusic(false);
    }

    /** User skip to the next track; repeat-one does not hold a skip. */
    public void next() {
        advance(1);
    }

    /** The current track reached its end: repeat-one replays it, otherwise the queue moves on. */
    private void onTrackEnded() {
        if (REPEAT_ONE.equals(repeat) && current != null) {
            restartCurrent();
            return;
        }
        advance(1);
    }

    public void previous() {
        if (elapsedMs() > 3000L && current != null) {
            restartCurrent();
            return;
        }
        advance(-1);
    }

    private void restartCurrent() {
        if (current == null) {
            return;
        }
        startDecode(current);
        persistLast();
    }

    private void advance(int step) {
        if (queue.isEmpty()) {
            stop();
            return;
        }
        int size = queue.size();
        int index = queueIndex;
        for (int attempt = 0; attempt < size; attempt++) {
            index += step;
            if (index < 0) {
                if (REPEAT_OFF.equals(repeat) && !shuffle) {
                    stop();
                    return;
                }
                index = size - 1;
            } else if (index >= size) {
                if (REPEAT_OFF.equals(repeat) && !shuffle) {
                    stop();
                    return;
                }
                index = 0;
            }
            MediaTrack candidate = library.byId(queue.get(index));
            if (candidate != null) {
                this.queueIndex = index;
                this.current = candidate;
                startDecode(candidate);
                persistLast();
                return;
            }
        }
        // Every id in the queue is gone from disk: nothing left to play.
        LOGGER.info("Media queue is stale (files removed); stopping");
        stop();
    }

    private void startDecode(MediaTrack track) {
        this.playing = true;
        this.paused = false;
        this.eofHandled = false;
        this.elapsedBaseMs = 0L;
        this.startedAt = System.currentTimeMillis();
        Path path = track.path();
        decoder.open(path);
        MediaAudio.get().startStreaming(ring, decoder, volume);
        MediaAudio.get().duckVanillaMusic(duckMusic);
        showToast(track);
    }

    private void persistLast() {
        if (current != null) {
            saveSetting("lastTrack", current.id());
        }
    }

    // ------------------------------------------------------------- per tick

    /** Client tick: keys, toast timer, end-of-track, reload recovery. */
    public void onClientTick(Minecraft minecraft) {
        // One choke point like every other owned feature: the Suite gate and
        // the row's stored flag both switch playback off live.
        if (playing && !ClientConfig.getInstance().isFeatureEnabled(FEATURE_ID)) {
            stop();
        }
        MediaAudio.get().tick(minecraft);
        if (toastTicks > 0) {
            toastTicks--;
        }
        if (!playing || current == null) {
            return;
        }
        if (decoder.error() != null && !paused) {
            LOGGER.warn("Stopping playback after decode error: {}", decoder.error());
            stop();
            return;
        }
        if (decoder.atEof() && ring.available() == 0 && !paused && !eofHandled) {
            eofHandled = true;
            onTrackEnded();
        }
    }

    // --------------------------------------------------------------- toast

    private void showToast(MediaTrack track) {
        if (!toastEnabled) {
            return;
        }
        this.toastText = track.displayName();
        this.toastTicks = TOAST_TICKS;
    }

    public boolean toastVisible() {
        return toastTicks > 0 && !toastText.isEmpty();
    }

    public String toastText() {
        return toastText;
    }

    public int toastTicksLeft() {
        return toastTicks;
    }
}
