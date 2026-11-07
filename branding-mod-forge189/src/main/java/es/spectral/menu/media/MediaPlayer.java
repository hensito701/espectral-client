package es.spectral.menu.media;

import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.List;
import java.util.Locale;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import es.spectral.menu.ClientConfig;

/**
 * Bring-your-own MP3 player: a library scanned from the music folder, user
 * playlists persisted in {@code features.mediaplayer}, and one streaming
 * session at a time. Playback state lives here; audio is {@link MediaAudio}.
 */
public final class MediaPlayer {

    public static final String FEATURE_ID = "mediaplayer";
    public static final String REPEAT_OFF = "off";
    public static final String REPEAT_ALL = "all";
    public static final String REPEAT_ONE = "one";

    /** Now-playing toast lifetime in client ticks (3 seconds). */
    static final int TOAST_TICKS = 60;
    /** Previous-track press restarts the song once it has played past this point. */
    private static final long RESTART_AFTER_MS = 3000L;

    private static final Logger LOGGER = LogManager.getLogger("espectral-media");
    private static final MediaPlayer INSTANCE = new MediaPlayer();

    private final MediaLibrary library = new MediaLibrary();
    private final MediaRing ring = new MediaRing();
    private final MediaDecoder decoder = new MediaDecoder(ring);

    private List<String> queue = new ArrayList<String>();
    private int queueIndex = -1;
    private String sourcePlaylist = "";
    private List<Playlist> playlists = new ArrayList<Playlist>();
    private boolean loaded;

    private volatile boolean playing;
    private volatile boolean paused;
    private volatile float volume = 0.7F;
    private volatile boolean shuffle;
    private volatile String repeat = REPEAT_OFF;
    private volatile boolean duckMusic = true;
    private volatile boolean toastEnabled = true;
    private volatile MediaTrack current;
    private boolean eofHandled;

    private volatile String toastText = "";
    private volatile int toastTicks;

    private MediaPlayer() {
    }

    /** The shared player; the first call loads settings and starts the library scan. */
    public static synchronized MediaPlayer get() {
        if (!INSTANCE.loaded) {
            INSTANCE.init();
        }
        return INSTANCE;
    }

    private void init() {
        loaded = true;
        reloadSettings();
        MediaAudio.registerMusicGate();
        library.scanAsync(null);
    }

    // --- read accessors ----------------------------------------------------

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

    public synchronized String currentPlaylistId() {
        return sourcePlaylist;
    }

    /** Milliseconds of the current track the audio line has actually consumed. */
    public long elapsedMs() {
        return playing && current != null ? MediaAudio.get().playedMs() : 0L;
    }

    // --- settings ----------------------------------------------------------

    /** Reads volume, shuffle, repeat, ducking, toast and playlists from the config. */
    public synchronized void reloadSettings() {
        ClientConfig config = ClientConfig.getInstance();
        JsonObject raw = config.featureRaw(FEATURE_ID);
        if (raw != null) {
            if (has(raw, "volume")) {
                volume = clamp01((float) raw.get("volume").getAsDouble());
            }
            shuffle = bool(raw, "shuffle", false);
            String mode = has(raw, "repeat") ? raw.get("repeat").getAsString() : REPEAT_OFF;
            repeat = REPEAT_ALL.equals(mode) || REPEAT_ONE.equals(mode) ? mode : REPEAT_OFF;
            duckMusic = bool(raw, "duckMusic", true);
            toastEnabled = bool(raw, "toast", true);
        }
        MediaAudio.get().applyVolume(volume);
        playlists = new ArrayList<Playlist>(config.getPlaylists(FEATURE_ID));
    }

    public synchronized void setVolume(float value) {
        float clamped = clamp01(value);
        if (Math.abs(clamped - volume) < 0.005F) {
            return;
        }
        volume = clamped;
        saveSetting("volume", Math.round(clamped * 100F) / 100.0);
        MediaAudio.get().applyVolume(clamped);
    }

    public synchronized void toggleShuffle() {
        shuffle = !shuffle;
        saveSetting("shuffle", shuffle);
        rebuildQueue();
    }

    public synchronized void cycleRepeat() {
        if (REPEAT_OFF.equals(repeat)) {
            repeat = REPEAT_ALL;
        } else if (REPEAT_ALL.equals(repeat)) {
            repeat = REPEAT_ONE;
        } else {
            repeat = REPEAT_OFF;
        }
        saveSetting("repeat", repeat);
    }

    public synchronized void setDuckMusic(boolean value) {
        duckMusic = value;
        saveSetting("duckMusic", value);
        MediaAudio.get().duckVanillaMusic(value && playing && !paused);
    }

    public synchronized void setToastEnabled(boolean value) {
        toastEnabled = value;
        saveSetting("toast", value);
    }

    /** Rescans the music folder on a background thread; {@code onChange} runs when the scan ends. */
    public void refreshLibrary(Runnable onChange) {
        library.scanAsync(onChange);
    }

    // --- playlists ---------------------------------------------------------

    public synchronized List<Playlist> playlists() {
        return Collections.unmodifiableList(new ArrayList<Playlist>(playlists));
    }

    public synchronized Playlist playlistById(String id) {
        int i = indexOfPlaylist(id);
        return i < 0 ? null : playlists.get(i);
    }

    /** Creates an empty playlist, or returns {@code null} when the name is empty or the list is full. */
    public synchronized Playlist createPlaylist(String name) {
        String clean = cleanName(name);
        if (clean.isEmpty() || playlists.size() >= ClientConfig.MAX_PLAYLISTS) {
            return null;
        }
        Playlist created = new Playlist(ClientConfig.getInstance().newPlaylistId(clean), clean,
                Collections.<String>emptyList());
        playlists.add(created);
        persistPlaylists();
        return created;
    }

    public synchronized boolean renamePlaylist(String id, String name) {
        int i = indexOfPlaylist(id);
        String clean = cleanName(name);
        if (i < 0 || clean.isEmpty()) {
            return false;
        }
        Playlist old = playlists.get(i);
        playlists.set(i, new Playlist(old.id(), clean, old.tracks()));
        persistPlaylists();
        return true;
    }

    public synchronized void deletePlaylist(String id) {
        for (Iterator<Playlist> it = playlists.iterator(); it.hasNext();) {
            if (it.next().id().equals(id)) {
                it.remove();
            }
        }
        if (id.equals(sourcePlaylist)) {
            stop();
            sourcePlaylist = "";
        }
        persistPlaylists();
    }

    /** Appends a library track to a playlist; false when already present, full, or unknown. */
    public synchronized boolean addToPlaylist(String playlistId, String trackId) {
        int i = indexOfPlaylist(playlistId);
        if (i < 0 || trackId == null) {
            return false;
        }
        Playlist old = playlists.get(i);
        if (old.tracks().contains(trackId) || old.tracks().size() >= ClientConfig.MAX_PLAYLIST_TRACKS) {
            return false;
        }
        List<String> tracks = new ArrayList<String>(old.tracks());
        tracks.add(trackId);
        playlists.set(i, new Playlist(old.id(), old.name(), tracks));
        persistPlaylists();
        if (playlistId.equals(sourcePlaylist)) {
            rebuildQueue();
        }
        return true;
    }

    public synchronized void removeFromPlaylist(String playlistId, String trackId) {
        int i = indexOfPlaylist(playlistId);
        if (i < 0) {
            return;
        }
        Playlist old = playlists.get(i);
        List<String> tracks = new ArrayList<String>(old.tracks());
        if (!tracks.remove(trackId)) {
            return;
        }
        playlists.set(i, new Playlist(old.id(), old.name(), tracks));
        persistPlaylists();
        if (playlistId.equals(sourcePlaylist)) {
            rebuildQueue();
        }
    }

    /** Moves the track at {@code index} by {@code delta} (-1 up, +1 down); false when out of range. */
    public synchronized boolean moveInPlaylist(String playlistId, int index, int delta) {
        int i = indexOfPlaylist(playlistId);
        if (i < 0) {
            return false;
        }
        Playlist old = playlists.get(i);
        int to = index + delta;
        if (index < 0 || index >= old.tracks().size() || to < 0 || to >= old.tracks().size()) {
            return false;
        }
        List<String> tracks = new ArrayList<String>(old.tracks());
        Collections.swap(tracks, index, to);
        playlists.set(i, new Playlist(old.id(), old.name(), tracks));
        persistPlaylists();
        if (playlistId.equals(sourcePlaylist)) {
            rebuildQueue();
        }
        return true;
    }

    // --- transport ---------------------------------------------------------

    /** Starts {@code ids} as the queue (from a playlist, or the whole library when {@code playlistId} is empty). */
    public synchronized void playQueue(String playlistId, List<String> ids) {
        sourcePlaylist = playlistId == null ? "" : playlistId;
        queue = new ArrayList<String>(ids);
        if (shuffle) {
            Collections.shuffle(queue);
        }
        if (queue.isEmpty()) {
            stop();
            return;
        }
        queueIndex = 0;
        MediaTrack track = library.byId(queue.get(0));
        if (track == null) {
            advance(1);
            return;
        }
        current = track;
        startDecode(track);
        persistLast();
    }

    /** Plays a single track from the library; it stays the current track while the queue is kept. */
    public synchronized void playTrack(MediaTrack track) {
        if (track == null) {
            return;
        }
        current = track;
        queueIndex = queue.indexOf(track.id());
        startDecode(track);
        persistLast();
    }

    public synchronized void togglePlayPause() {
        if (current == null) {
            playQueue("", allTrackIds());
            return;
        }
        if (paused) {
            resume();
        } else {
            pause();
        }
    }

    private void pause() {
        if (!playing || paused) {
            return;
        }
        paused = true;
        MediaAudio.get().setPaused(true);
        MediaAudio.get().duckVanillaMusic(false);
    }

    private void resume() {
        if (!playing || !paused) {
            return;
        }
        paused = false;
        MediaAudio.get().setPaused(false);
        MediaAudio.get().duckVanillaMusic(duckMusic);
    }

    public synchronized void stop() {
        playing = false;
        paused = false;
        current = null;
        queueIndex = -1;
        decoder.stop();
        ring.clear();
        MediaAudio.get().setPaused(false);
        MediaAudio.get().stopStreaming();
        MediaAudio.get().duckVanillaMusic(false);
    }

    public synchronized void next() {
        advance(1);
    }

    /** Restarts the track when it is well in; otherwise steps back in the queue. */
    public synchronized void previous() {
        if (elapsedMs() > RESTART_AFTER_MS && current != null) {
            restartCurrent();
        } else {
            advance(-1);
        }
    }

    /** Called once per client tick: keeps audio gain current, advances on finish, stops on error. */
    public synchronized void onClientTick(Minecraft mc) {
        MediaAudio.get().tick(mc);
        if (toastTicks > 0) {
            toastTicks--;
        }
        if (!playing || current == null) {
            return;
        }
        String failure = decoder.error();
        if (failure == null) {
            failure = MediaAudio.get().error();
        }
        if (failure != null) {
            LOGGER.warn("Stopping playback after decode error: {}", failure);
            stop();
            return;
        }
        if (!paused && !eofHandled && MediaAudio.get().finished()) {
            eofHandled = true;
            if (REPEAT_ONE.equals(repeat)) {
                restartCurrent();
            } else {
                advance(1);
            }
        }
    }

    // --- toast -------------------------------------------------------------

    public boolean toastVisible() {
        return toastTicks > 0 && !toastText.isEmpty();
    }

    public String toastText() {
        return toastText;
    }

    public int toastTicksLeft() {
        return toastTicks;
    }

    // --- internals ---------------------------------------------------------

    private void restartCurrent() {
        if (current == null) {
            return;
        }
        startDecode(current);
        persistLast();
    }

    /**
     * Moves the queue cursor by {@code step}, honouring repeat and shuffle, and starts the first track that
     * still exists on disk. Stops when the queue ends (repeat off) or nothing in it plays.
     */
    private void advance(int step) {
        if (queue.isEmpty()) {
            stop();
            return;
        }
        int size = queue.size();
        int index = queueIndex;
        for (int attempt = 0; attempt < size; attempt++) {
            index += step;
            if (index < 0 || index >= size) {
                if (REPEAT_OFF.equals(repeat) && !shuffle) {
                    stop();
                    return;
                }
                index = index < 0 ? size - 1 : 0;
            }
            MediaTrack candidate = library.byId(queue.get(index));
            if (candidate != null) {
                queueIndex = index;
                current = candidate;
                startDecode(candidate);
                persistLast();
                return;
            }
        }
        LOGGER.info("Media queue is stale (files removed); stopping");
        stop();
    }

    private void startDecode(MediaTrack track) {
        playing = true;
        paused = false;
        eofHandled = false;
        MediaAudio audio = MediaAudio.get();
        audio.stopStreaming();
        decoder.open(track.path());
        audio.startStreaming(ring, decoder, volume);
        audio.duckVanillaMusic(duckMusic);
        showToast(track);
    }

    private void rebuildQueue() {
        if (queue.isEmpty()) {
            return;
        }
        String keep = current == null ? null : current.id();
        Playlist source = sourcePlaylist.isEmpty() ? null : playlistById(sourcePlaylist);
        List<String> ids = source != null ? new ArrayList<String>(source.tracks()) : allTrackIds();
        if (shuffle) {
            Collections.shuffle(ids);
        }
        queue = ids;
        queueIndex = keep == null ? -1 : queue.indexOf(keep);
    }

    private List<String> allTrackIds() {
        List<String> ids = new ArrayList<String>();
        for (MediaTrack track : library.tracks()) {
            ids.add(track.id());
        }
        return ids;
    }

    private void persistLast() {
        MediaTrack track = current;
        if (track != null) {
            saveSetting("lastTrack", track.id());
        }
    }

    private void showToast(MediaTrack track) {
        if (!toastEnabled) {
            return;
        }
        toastText = track.displayName();
        toastTicks = TOAST_TICKS;
    }

    private void persistPlaylists() {
        ClientConfig.getInstance().setPlaylists(FEATURE_ID, playlists);
    }

    private void saveSetting(String key, boolean value) {
        ClientConfig.getInstance().setFeatureFlag(FEATURE_ID, key, value);
    }

    private void saveSetting(String key, double value) {
        ClientConfig.getInstance().setFeatureFlag(FEATURE_ID, key, value);
    }

    private void saveSetting(String key, String value) {
        ClientConfig.getInstance().setFeatureFlag(FEATURE_ID, key, value);
    }

    private int indexOfPlaylist(String id) {
        for (int i = 0; i < playlists.size(); i++) {
            if (playlists.get(i).id().equals(id)) {
                return i;
            }
        }
        return -1;
    }

    private static String cleanName(String name) {
        if (name == null) {
            return "";
        }
        String trimmed = name.trim();
        return trimmed.length() <= ClientConfig.MAX_PLAYLIST_NAME
                ? trimmed : trimmed.substring(0, ClientConfig.MAX_PLAYLIST_NAME);
    }

    private static boolean has(JsonObject object, String key) {
        JsonElement element = object.get(key);
        return element != null && element.isJsonPrimitive();
    }

    private static boolean bool(JsonObject object, String key, boolean fallback) {
        return has(object, key) ? object.get(key).getAsBoolean() : fallback;
    }

    private static float clamp01(float value) {
        return Math.max(0F, Math.min(1F, value));
    }

    /** A named, ordered list of library track ids, stored in the feature config. */
    public static final class Playlist {
        private final String id;
        private final String name;
        private final List<String> tracks;

        public Playlist(String id, String name, List<String> tracks) {
            this.id = id;
            this.name = name;
            this.tracks = Collections.unmodifiableList(new ArrayList<String>(tracks));
        }

        public String id() {
            return id;
        }

        public String name() {
            return name;
        }

        public List<String> tracks() {
            return tracks;
        }
    }
}
