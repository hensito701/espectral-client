package es.spectral.menu;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReferenceArray;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.client.renderer.texture.MipmapStrategy;
import net.minecraft.client.renderer.texture.ReloadableTexture;
import net.minecraft.client.renderer.texture.TextureContents;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.client.resources.metadata.texture.TextureMetadataSection;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.Util;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * The user-selected title-menu background, driven by the launcher-managed
 * pack store (Workstream 4).
 *
 * <p>The launcher stages the selected pack at
 * {@code <gameDir>/config/espectral-menu-bg/<packId>/} — {@code pack.json}
 * describing {@code type "image"} (a single {@code image.png}) or
 * {@code type "frames"} ({@code frames/NNNN.png} at {@code fps}) — and records
 * the choice in {@code personalization.background} of espectral-client.json,
 * read here through {@code ClientConfig.backgroundPack()}. The
 * {@code -Despectral.menu.background} override stays supported for dev runs
 * and accepts either a PNG file or a pack directory. No background selected
 * (or an unreadable pack) keeps the branded gradient fallback.
 *
 * <p>Still-image packs upload once as a {@link ReloadableTexture} so a
 * resource reload re-reads the file. Animated packs are STREAMED: a
 * background-executor loop decodes frames into a ring buffer of at most
 * {@link #RING} {@link NativeImage}s and the render thread uploads each frame
 * into a pooled {@link DynamicTexture} only when the timeline advances — no
 * decoding on the render thread, no unbounded resident frame set.
 * {@code reducedMotion()} renders only the first frame.
 *
 * <p>The texture outlives the title screen: every out-of-world menu draws it
 * ({@code MenuSkin}). {@link #releaseIfHeld()} — called each client tick
 * while a world is loaded — frees every texture and pending decode, so the
 * pack's GPU/native memory is held only while menus are on screen.
 *
 * <p>Both supported versions expose identical texture APIs, so this lives in
 * the shared source set; only the blit call is version-specific
 * ({@code Compat.uiBlitUv}).
 */
public final class MenuBackground {

    /** Texture id of the uploaded still-image background. */
    public static final Identifier ID = Identifier.fromNamespaceAndPath("espectral-menu", "menu_bg");

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-menu");

    /** Identifiers of the pooled frame textures for animated packs. */
    private static final Identifier[] FRAME_IDS = {
            Identifier.fromNamespaceAndPath("espectral-menu", "menu_bg_0"),
            Identifier.fromNamespaceAndPath("espectral-menu", "menu_bg_1"),
            Identifier.fromNamespaceAndPath("espectral-menu", "menu_bg_2"),
            Identifier.fromNamespaceAndPath("espectral-menu", "menu_bg_3"),
    };

    /** Decoded-frame lookahead; also the DynamicTexture pool size. */
    private static final int RING = 4;

    private static final int DEFAULT_FPS = 24;
    private static final java.util.regex.Pattern PACK_ID_RE =
            java.util.regex.Pattern.compile("^[A-Za-z0-9_-]{1,64}$");

    private enum Kind { IMAGE, FRAMES }
    private enum State { IDLE, LOADING, READY, ABSENT }

    /** What the loader resolved: file/dir + parsed pack meta. */
    private record Source(Path path, Kind kind, int fps, int frameCount, int w, int h) {}

    // ── Render-thread state ────────────────────────────────────────────────
    private static State state = State.IDLE;
    /** Pack id / override path the current source was resolved from (null = none). */
    private static String sourceId;
    /** True once {@link #resolveSource} finished for {@link #sourceId} (even to null). */
    private static boolean sourceResolved;
    /** True while an async {@link #resolveSource} for {@link #sourceId} is in flight. */
    private static boolean resolvePending;
    /** Resolved pack layout for {@link #sourceId}; null = no usable pack. */
    private static Source cachedSource;
    private static int width;
    private static int height;
    private static long readyAtMs;
    /**
     * Load generation: bumped on every release / source change so in-flight
     * resolves, uploads and stream producers started by an older source can
     * never publish into the new one. Read/written on the render thread;
     * producers read it atomically.
     */
    private static final AtomicInteger generation = new AtomicInteger();

    // ── Frame-stream state ────────────────────────────────────────────────
    /** A ring entry: decoded image stamped with its producing generation so
     * frames from a superseded stream are closed, never uploaded. */
    private record Frame(int gen, NativeImage img) {}
    /** Producer fills, render thread drains; entries owned by the ring. */
    private static final AtomicReferenceArray<Frame> ring = new AtomicReferenceArray<>(RING);
    /** Frames decoded into the ring (monotonic; unbounded so the stream loops). */
    private static final AtomicLong produced = new AtomicLong();
    /** Frames consumed by the render thread (monotonic). */
    private static final AtomicLong consumed = new AtomicLong();
    private static volatile boolean streaming;
    /** Decode failure on the producer thread; read on the render thread. */
    private static volatile Throwable streamError;
    private static final DynamicTexture[] frameTextures = new DynamicTexture[RING];
    private static Identifier currentFrameId;
    private static int fps = DEFAULT_FPS;
    private static int frameTotal;
    private static long streamStartMs;

    private MenuBackground() {}

    // ── Source resolution ──────────────────────────────────────────────────

    /**
     * Identity of the source to draw right now, or {@code null} for the
     * branded fallback. Allocation-free — called every frame; the expensive
     * Path/pack.json work happens only when this id CHANGES
     * ({@link #resolveSource}). {@code -Despectral.menu.background} wins
     * (PNG file or pack directory); otherwise the pack selected in
     * {@code personalization.background} staged at
     * {@code config/espectral-menu-bg/<packId>/}.
     */
    private static String currentSourceId(Minecraft minecraft) {
        String override = System.getProperty("espectral.menu.background");
        if (override != null && !override.isBlank()) {
            return override;
        }
        String packId = null;
        try {
            packId = ClientConfig.getInstance().backgroundPack();
        } catch (Throwable t) {
            // Config unreadable — fall through to the branded background.
        }
        return packId == null || packId.isEmpty() || minecraft.gameDirectory == null
                ? null
                : packId;
    }

    /** Resolve {@code id} to a pack layout once per change; null when unusable. */
    private static Source resolveSource(String id, Minecraft minecraft) {
        Path p = Path.of(id);
        if (Files.isRegularFile(p)) {
            return new Source(p, Kind.IMAGE, 0, 1, 0, 0);
        }
        Source pack = readPack(p);
        if (pack != null) {
            return pack;
        }
        // Not an override path: a pack id staged by the launcher.
        if (PACK_ID_RE.matcher(id).matches()) {
            return readPack(minecraft.gameDirectory.toPath()
                    .resolve("config").resolve("espectral-menu-bg").resolve(id));
        }
        return null;
    }

    /** Parse a staged pack directory; null when missing/invalid/empty. */
    private static Source readPack(Path dir) {
        if (dir == null || !Files.isDirectory(dir)) {
            return null;
        }
        Path metaFile = dir.resolve("pack.json");
        if (!Files.isRegularFile(metaFile)) {
            return null;
        }
        try {
            JsonObject meta = JsonParser.parseString(
                    Files.readString(metaFile, StandardCharsets.UTF_8)).getAsJsonObject();
            String type = meta.has("type") ? meta.get("type").getAsString() : "";
            if ("image".equals(type)) {
                Path img = dir.resolve("image.png");
                return Files.isRegularFile(img)
                        ? new Source(img, Kind.IMAGE, 0, 1, 0, 0)
                        : null;
            }
            if ("frames".equals(type)) {
                int frames = meta.has("frame_count") ? meta.get("frame_count").getAsInt() : 0;
                int f = meta.has("fps") ? meta.get("fps").getAsInt() : DEFAULT_FPS;
                int w = meta.has("width") ? meta.get("width").getAsInt() : 0;
                int h = meta.has("height") ? meta.get("height").getAsInt() : 0;
                if (frames < 1 || w <= 0 || h <= 0) {
                    return null;
                }
                f = Math.max(1, Math.min(60, f));
                Path first = frameFile(dir, 0);
                if (!Files.isRegularFile(first)) {
                    return null;
                }
                return new Source(dir, Kind.FRAMES, f, frames, w, h);
            }
        } catch (Exception e) {
            LOGGER.warn("[espectral] unreadable background pack {}: {}", dir, e.toString());
        }
        return null;
    }

    private static Path frameFile(Path dir, int index) {
        return dir.resolve("frames").resolve(String.format("%04d.png", index + 1));
    }

    // ── Lifecycle ──────────────────────────────────────────────────────────

    /**
     * Starts the load for the current source, then pumps the frame stream.
     * Safe to call every frame: the per-frame work is two field reads and an
     * id compare — Path/pack.json resolution runs only when the selected
     * pack changes (or the first time after a change).
     */
    public static void request(Minecraft minecraft) {
        if (minecraft == null) {
            return;
        }
        String id;
        try {
            id = currentSourceId(minecraft);
        } catch (Throwable t) {
            id = null;
        }
        if (!java.util.Objects.equals(id, sourceId)) {
            releaseInternal(minecraft);
            sourceId = id;
            sourceResolved = false;
            resolvePending = false;
            cachedSource = null;
        }
        if (id == null) {
            return; // no pack selected — branded gradient (state stays IDLE)
        }
        if (!sourceResolved) {
            // pack.json probing and directory reads happen on the background
            // executor; the render thread only applies the finished Source.
            if (!resolvePending) {
                resolvePending = true;
                final String wanted = id;
                final int gen = generation.get();
                CompletableFuture
                        .supplyAsync(() -> {
                            try {
                                return resolveSource(wanted, minecraft);
                            } catch (Throwable t) {
                                LOGGER.warn("[espectral] menu background resolve failed for {}: {}",
                                        wanted, t.toString());
                                return null;
                            }
                        }, Util.backgroundExecutor())
                        .whenCompleteAsync((resolved, err) -> acceptSource(wanted, gen, resolved), minecraft);
            }
            return;
        }
        final Source src = cachedSource;
        if (src == null) {
            return; // pack missing/unreadable — branded gradient (state stays IDLE)
        }
        if (state == State.IDLE) {
            state = State.LOADING;
            if (src.kind() == Kind.IMAGE) {
                final int gen = generation.get();
                CompletableFuture
                        .supplyAsync(() -> decodeImage(src.path()), Util.backgroundExecutor())
                        .whenCompleteAsync((found, error) -> upload(minecraft, gen, found, error), minecraft);
            } else {
                startStream(minecraft, src);
            }
        }
        pumpFrames(minecraft);
    }

    /** Render thread: apply an async resolution still belonging to this source. */
    private static void acceptSource(String id, int gen, Source resolved) {
        if (gen != generation.get() || !java.util.Objects.equals(id, sourceId)) {
            return; // source changed or released while resolving — drop it
        }
        cachedSource = resolved;
        resolvePending = false;
        sourceResolved = true;
    }

    /** Decoded still image waiting for upload. */
    private record Decoded(Path path, TextureContents contents) {}

    private static Decoded decodeImage(Path path) {
        try {
            return new Decoded(path, read(path));
        } catch (IOException | RuntimeException e) {
            LOGGER.warn("[espectral] menu background unreadable {}: {}", path, e.toString());
            return null;
        }
    }

    private static TextureContents read(Path path) throws IOException {
        try (InputStream in = Files.newInputStream(path)) {
            return new TextureContents(NativeImage.read(in),
                    new TextureMetadataSection(true, true, MipmapStrategy.AUTO,
                            TextureMetadataSection.DEFAULT_ALPHA_CUTOFF_BIAS));
        }
    }

    private static void upload(Minecraft minecraft, int gen, Decoded found, Throwable error) {
        if (gen != generation.get() || state != State.LOADING) {
            // Released or the source changed while decoding/uploading: this
            // texture no longer belongs to the current pack — free the
            // decoded contents instead of registering a stale texture.
            if (found != null) {
                found.contents().close();
            }
            return;
        }
        if (error != null || found == null) {
            state = State.ABSENT;
            if (error != null) {
                LOGGER.warn("[espectral] menu background load failed: {}", error.toString());
            } else {
                LOGGER.info("[espectral] no readable menu background; using the branded fallback");
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

    // ── Animated packs ─────────────────────────────────────────────────────

    /** Render thread: arm the stream state and spawn the decode producer. */
    private static void startStream(Minecraft minecraft, Source source) {
        boolean reduced;
        try {
            reduced = ClientConfig.getInstance().reducedMotion();
        } catch (Throwable t) {
            reduced = false;
        }
        fps = source.fps();
        frameTotal = reduced ? 1 : source.frameCount();
        width = source.w();
        height = source.h();
        streamStartMs = Util.getMillis();
        produced.set(0);
        consumed.set(0);
        streamError = null;
        currentFrameId = null;
        streaming = true;
        Path dir = source.path();
        int total = frameTotal;
        final int gen = generation.get();
        CompletableFuture.runAsync(() -> produce(dir, total, gen), Util.backgroundExecutor())
                .whenComplete((v, err) -> {
                    // A stale producer must not poison the stream that
                    // replaced it.
                    if (err != null && streamAlive(gen)) streamError = err;
                });
        LOGGER.info("[espectral] streaming background pack {} ({} frames @ {} fps{})",
                dir, source.frameCount(), fps, reduced ? ", reduced motion" : "");
    }

    /**
     * Producer (background thread): decodes frames cyclically, ahead of the
     * consumer by at most {@link #RING}; parks while the ring is full — the
     * consumer drains at fps, so a full ring simply waits for the next
     * consumed slot. {@code gen} ties the loop to the source that started it:
     * every publish point re-checks {@link #streamAlive} and any frame that
     * can no longer be delivered to the CURRENT stream is closed on the spot,
     * so a pack switch can neither leak NativeImages nor inject old-pack
     * frames into the new ring.
     */
    private static boolean streamAlive(int gen) {
        return streaming && gen == generation.get();
    }

    private static void produce(Path dir, int total, int gen) {
        for (long i = 0; streamAlive(gen); i++) {
            if (total == 1 && i > 0) {
                return; // single-frame / reduced-motion pack: frame 0 holds forever
            }
            while (streamAlive(gen) && produced.get() - consumed.get() >= RING) {
                try {
                    Thread.sleep(4);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    return;
                }
            }
            if (!streamAlive(gen)) {
                return;
            }
            NativeImage img;
            try (InputStream in = Files.newInputStream(frameFile(dir, (int) (i % total)))) {
                img = NativeImage.read(in);
            } catch (IOException | RuntimeException e) {
                if (streamAlive(gen)) {
                    streamError = e;
                }
                return;
            }
            if (!streamAlive(gen)) {
                img.close();
                return;
            }
            int slot = (int) (i % RING);
            Frame ours = new Frame(gen, img);
            for (int attempt = 0; attempt < RING + 1; attempt++) {
                if (ring.compareAndSet(slot, null, ours)) {
                    produced.incrementAndGet(); // publish after the slot write
                    if (!streamAlive(gen)) {
                        // Released between the store and this check: if the
                        // render thread did not already claim the slot, free
                        // it here — but only while it still holds OUR frame.
                        Frame orphan = ring.compareAndSet(slot, ours, null) ? ours : null;
                        if (orphan != null) {
                            orphan.img().close();
                        }
                        return;
                    }
                    img = null; // delivered to the ring
                    break;
                }
                // Slot taken: if it holds a dead generation's frame, evict
                // (close) it and retry — stale producers may land their
                // decoded frame just after release cleared the ring.
                Frame occupant = ring.get(slot);
                if (occupant != null && !streamAlive(occupant.gen())
                        && ring.compareAndSet(slot, occupant, null)) {
                    occupant.img().close();
                    continue;
                }
                // A live frame owns the slot: we were superseded mid-publish.
                break;
            }
            if (img != null) {
                img.close();
                return;
            }
        }
    }

    /**
     * Render thread: upload decoded frames up to the timeline position into
     * pooled DynamicTextures. No decoding or allocation here.
     */
    private static void pumpFrames(Minecraft minecraft) {
        if (!streaming || frameTotal <= 0) {
            return;
        }
        long elapsed = Util.getMillis() - streamStartMs;
        // Frame index of the timeline position; monotonic — the producer
        // decodes i % frameTotal, so the pack loops instead of freezing on
        // the last frame.
        long want = elapsed * fps / 1000;
        int gen = generation.get();
        TextureManager tm = minecraft.getTextureManager();
        while (consumed.get() <= want && consumed.get() < produced.get()) {
            long i = consumed.get();
            int slot = (int) (i % RING);
            Frame frame = ring.getAndSet(slot, null);
            consumed.incrementAndGet();
            if (frame == null) {
                continue;
            }
            NativeImage img = frame.img();
            if (frame.gen() != gen) {
                img.close(); // superseded stream's frame — reap, never upload
                continue;
            }
            DynamicTexture tex = frameTextures[slot];
            if (tex == null) {
                tex = new DynamicTexture(
                        () -> "espectral-menu background frame", img.getWidth(), img.getHeight(), true);
                frameTextures[slot] = tex;
                tm.register(FRAME_IDS[slot], tex);
            }
            tex.setPixels(img); // takes ownership; uploads below
            tex.upload();
            currentFrameId = FRAME_IDS[slot];
            if (state == State.LOADING) {
                readyAtMs = Util.getMillis();
                state = State.READY;
            }
        }
        if (streamError != null && currentFrameId == null && consumed.get() >= produced.get()) {
            state = State.ABSENT;
            LOGGER.warn("[espectral] background stream failed: {}", streamError.toString());
        }
    }

    /**
     * Frees every texture this loader owns and any decoded frames still in
     * the ring — render thread only.
     */
    public static void release() {
        releaseInternal(Minecraft.getInstance());
        sourceId = null;
        sourceResolved = false;
        cachedSource = null;
    }

    /** {@link #release()} when anything is loaded or loading; a field read otherwise. */
    public static void releaseIfHeld() {
        if (state != State.IDLE || sourceId != null || resolvePending || cachedSource != null) {
            release();
        }
    }

    private static void releaseInternal(Minecraft minecraft) {
        generation.incrementAndGet(); // retire every in-flight load/producer
        streaming = false;
        resolvePending = false;
        // Unwind whatever the producer may still be holding past the ring.
        produced.set(0);
        consumed.set(0);
        for (int i = 0; i < RING; i++) {
            Frame frame = ring.getAndSet(i, null);
            if (frame != null) {
                frame.img().close();
            }
            frameTextures[i] = null;
        }
        if (minecraft != null) {
            TextureManager tm = minecraft.getTextureManager();
            try {
                tm.release(ID);
            } catch (Throwable ignored) {
            }
            for (Identifier id : FRAME_IDS) {
                try {
                    tm.release(id);
                } catch (Throwable ignored) {
                }
            }
        }
        currentFrameId = null;
        streamError = null;
        state = State.IDLE;
        width = 0;
        height = 0;
        readyAtMs = 0;
    }

    // ── Draw-side API ──────────────────────────────────────────────────────

    /** True once a texture is uploaded and drawable. */
    public static boolean ready() {
        return state == State.READY;
    }

    /** Texture to blit this frame (per-frame id for animated packs). */
    public static Identifier texture() {
        return currentFrameId != null ? currentFrameId : ID;
    }

    public static int width() {
        return width;
    }

    public static int height() {
        return height;
    }

    /** {@link Util#getMillis()} timestamp of the first upload (reveal fade). */
    public static long readyAtMs() {
        return readyAtMs;
    }
}
