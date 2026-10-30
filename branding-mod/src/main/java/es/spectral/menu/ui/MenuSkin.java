package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.util.Util;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;
import es.spectral.menu.MenuBackground;

/**
 * The Espectral main-menu look carried into every vanilla menu opened outside
 * a world (Multiplayer, Singleplayer, Options and their sub-screens, add /
 * edit server, …). Painting only — the per-version mixins
 * ({@code MenuBackdropMixin}, {@code ButtonSkinMixin}, {@code SliderSkinMixin},
 * {@code ListSkinMixin}) route vanilla's draw calls here, so layouts, widgets
 * and input stay 100% vanilla.
 *
 * <ul>
 *   <li>Backdrop: the player's menu background (or the branded charcoal) with
 *       the same slow Ken Burns drift as the title screen, replacing the
 *       panorama + blur + dirt overlay.</li>
 *   <li>Buttons/sliders: the title screen's dark-glass buttons — hairline
 *       accent edge, eased hover with the growing accent bar.</li>
 *   <li>Lists: a dark glass band between accent hairlines instead of the
 *       dirt list texture; the selected row gets an accent outline.</li>
 * </ul>
 *
 * <p>In a world everything stays vanilla. Our own Suite screens keep their
 * themed buttons (they only inherit the backdrop behind their glass sheet).
 * All drawing goes through {@code Compat.ui*} with the version's graphics
 * object passed as {@code Object}.
 */
public final class MenuSkin {

    /** Branded charcoal under the art (and the whole fallback). */
    public static final int BASE = 0xFF0B0D10;
    /** Title-screen button glass, idle → hot. */
    public static final int BTN_FILL = 0x8C090B0E;
    public static final int BTN_FILL_HOT = 0xC8151A21;
    private static final int BTN_FILL_OFF = 0x59090B0E;
    private static final int BTN_EDGE_OFF = 0x1FFFFFFF;
    /** Behind list rows: dark enough for server MOTDs over any photo. */
    private static final int LIST_GLASS = 0x9E07090C;
    /** Replaces vanilla's opaque black selected-row fill. */
    public static final int SELECT_FILL = 0xB8050608;
    /** Replaces the full-screen {@code menu_background} texture. */
    private static final int MENU_DIM = 0x4D000000;

    /** Set by the backdrop hook, consumed by the blur hook of the same frame. */
    private static boolean backdropPainted;
    /**
     * {@link Util#getMillis()} of the last frame that painted the menu art.
     * The level exists BEFORE the menus that draw the art are gone: joining a
     * server shows {@code LevelLoadingScreen}, which paints the panorama (our
     * art) with a level already loaded. Releasing on "level != null" alone
     * freed the texture every tick while that screen re-requested it every
     * frame, so each join decoded, uploaded and deleted a full-screen texture
     * over and over — at the exact moment the world's first chunks hit the
     * GPU (seen right before native driver crashes on 26.3 + AMD).
     */
    private static long lastArtPaintMs;
    /** Art unpainted this long with a world loaded → free it. */
    private static final long RELEASE_AFTER_MS = 1000L;

    private MenuSkin() {}

    // ── Gates ───────────────────────────────────────────────────────────────

    /** True on the menus before/after a world (no level loaded). */
    public static boolean outOfWorld() {
        Minecraft minecraft = Minecraft.getInstance();
        return minecraft != null && minecraft.level == null;
    }

    /** True when vanilla widgets on the current screen get the Espectral skin. */
    public static boolean skinWidgets() {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null || minecraft.level != null) {
            return false;
        }
        Screen screen = Compat.getScreen(minecraft);
        return screen != null && !screen.getClass().getName().startsWith("es.spectral.");
    }

    /**
     * Client tick: frees the background textures once a world is loaded AND
     * no screen has painted them for {@link #RELEASE_AFTER_MS} — the menus
     * that draw them are gone until the next disconnect, which re-requests
     * (and re-decodes) the pack.
     */
    public static void onClientTick(Minecraft minecraft) {
        if (minecraft != null && minecraft.level != null
                && Util.getMillis() - lastArtPaintMs > RELEASE_AFTER_MS) {
            MenuBackground.releaseIfHeld();
        }
    }

    // ── Backdrop ────────────────────────────────────────────────────────────

    /**
     * Cover-fit menu art with the slow Ken Burns drift (~±3% zoom over 28 s,
     * 40 s / 52 s Lissajous pan) plus an optional parallax offset in
     * [-1, 1]. The clock runs from the upload time, so moving between the
     * title screen and other menus continues the same motion seamlessly.
     * Falls back to the branded charcoal gradient while no pack is ready.
     */
    public static void paintArt(Object gfx, int width, int height, long now, boolean reduced,
            float parX, float parY) {
        // Drives the async pack resolution/upload and advances animated
        // packs; per-frame cost is an id compare when nothing changed.
        lastArtPaintMs = now;
        MenuBackground.request(Minecraft.getInstance());
        Compat.uiFill(gfx, 0, 0, width, height, BASE);
        if (!MenuBackground.ready() || MenuBackground.width() <= 0 || MenuBackground.height() <= 0) {
            Compat.uiFillGradient(gfx, 0, 0, width, height, 0x00000000, 0x66000000);
            return;
        }
        float bw = MenuBackground.width();
        float bh = MenuBackground.height();
        // Under reduced motion the Ken Burns clock is frozen at t = 0 so the
        // cover-fit UV window stays put; only the upload reveal fades.
        float t = reduced ? 0f : (now - MenuBackground.readyAtMs()) / 1000f;
        float zoom = 1.08f + 0.03f * (float) Math.sin(t * (Math.PI * 2 / 28.0));
        float scale = Math.max(width / bw, height / bh) * zoom;
        float uw = Math.min(1f, width / scale / bw);
        float vh = Math.min(1f, height / scale / bh);
        float px = clamp01(0.5f + 0.32f * (float) Math.sin(t * (Math.PI * 2 / 40.0)) + 0.14f * parX);
        float py = clamp01(0.5f + 0.32f * (float) Math.cos(t * (Math.PI * 2 / 52.0)) + 0.14f * parY);
        float u0 = (1f - uw) * px;
        float v0 = (1f - vh) * py;
        Compat.uiBlitUv(gfx, MenuBackground.texture(), 0, 0, width, height, u0, v0, u0 + uw, v0 + vh);
        // Reveal: a one-shot fade-in once the upload lands.
        float reveal = reduced ? 1f
                : easeOutCubic(clamp01(((now - MenuBackground.readyAtMs()) / 1000f) / 0.9f));
        if (reveal < 1f) {
            Compat.uiFill(gfx, 0, 0, width, height, withAlpha(BASE, 1f - reveal));
        }
    }

    /** Replaces the vanilla panorama on out-of-world menus. */
    public static void paintBackdrop(Object gfx, int width, int height) {
        paintArt(gfx, width, height, Util.getMillis(), ClientConfig.getInstance().reducedMotion(), 0f, 0f);
        // Header and footer shade so titles and button rows read over any photo.
        Compat.uiFillGradient(gfx, 0, 0, width, 40, 0x8C000000, 0x00000000);
        Compat.uiFillGradient(gfx, 0, Math.max(0, height - 72), width, height, 0x00000000, 0xA6000000);
        backdropPainted = true;
    }

    /** True once per painted backdrop: the art replaces vanilla's blur. */
    public static boolean consumeBackdrop() {
        boolean painted = backdropPainted;
        backdropPainted = false;
        return painted;
    }

    /** Light uniform dim in place of the {@code menu_background} texture. */
    public static void paintMenuDim(Object gfx, int x, int y, int w, int h) {
        Compat.uiFill(gfx, x, y, x + w, y + h, MENU_DIM);
    }

    // ── Lists ───────────────────────────────────────────────────────────────

    public static void paintListGlass(Object gfx, int x0, int y0, int x1, int y1) {
        Compat.uiFill(gfx, x0, y0, x1, y1, LIST_GLASS);
    }

    /** Accent hairlines above and below the list, fading out at both ends. */
    public static void paintListEdges(Object gfx, int x0, int top, int x1, int bottom) {
        hairline(gfx, x0, top - 1, x1);
        hairline(gfx, x0, bottom, x1);
    }

    /** Outline colour for the selected row (vanilla passes white when focused). */
    public static int selectionOutline(int vanillaColor) {
        return vanillaColor == -1 ? (SuiteTheme.accent() | 0xFF000000) : SuiteTheme.panelEdge();
    }

    private static void hairline(Object gfx, int x0, int y, int x1) {
        int rgb = SuiteTheme.accent() & 0xFFFFFF;
        int seg = Math.max(1, (x1 - x0) / 5);
        Compat.uiFill(gfx, x0, y, x0 + seg, y + 1, argb(0.18f, rgb));
        Compat.uiFill(gfx, x0 + seg, y, x1 - seg, y + 1, argb(0.5f, rgb));
        Compat.uiFill(gfx, x1 - seg, y, x1, y + 1, argb(0.18f, rgb));
    }

    // ── Buttons & sliders ───────────────────────────────────────────────────

    /** Eases a hover amount toward its target (same response as the title menu). */
    public static float easeHover(float current, boolean hot, float dt) {
        float k = 1f - (float) Math.exp(-dt * 14f);
        return current + ((hot ? 1f : 0f) - current) * k;
    }

    /**
     * Title-screen glass button: fill and edge blend idle → hot with
     * {@code hover}, and the accent bar grows from the centre of the left edge.
     */
    public static void paintButton(Object gfx, int x, int y, int w, int h, boolean enabled,
            float hover, float alpha) {
        if (!enabled) {
            Compat.uiFill(gfx, x, y, x + w, y + h, scaleAlpha(BTN_FILL_OFF, alpha));
            Compat.uiOutline(gfx, x, y, w, h, scaleAlpha(BTN_EDGE_OFF, alpha));
            return;
        }
        Compat.uiFill(gfx, x, y, x + w, y + h,
                scaleAlpha(SuiteTheme.lerpArgb(BTN_FILL, BTN_FILL_HOT, hover), alpha));
        Compat.uiOutline(gfx, x, y, w, h,
                scaleAlpha(SuiteTheme.lerpArgb(SuiteTheme.accentHairline(), edgeHot(), hover), alpha));
        int bar = Math.round((h - 2) * hover);
        if (bar > 0) {
            int by = y + (h - bar) / 2;
            Compat.uiFill(gfx, x, by, x + 2, by + bar, scaleAlpha(SuiteTheme.accent(), alpha));
        }
    }

    /** Slider track (button glass) or handle (solid accent block). */
    public static void paintSlider(Object gfx, int x, int y, int w, int h, boolean handle,
            boolean enabled, boolean hot, float alpha) {
        if (!handle) {
            paintButton(gfx, x, y, w, h, enabled, hot ? 1f : 0f, alpha);
            return;
        }
        int rgb = (enabled ? (hot ? SuiteTheme.accentBright() : SuiteTheme.accent()) : 0xFF6A6F78) & 0xFFFFFF;
        Compat.uiFill(gfx, x + 1, y + 1, x + w - 1, y + h - 1, argb(0.85f * alpha, rgb));
        Compat.uiOutline(gfx, x, y, w, h, argb(alpha, 0x050608));
    }

    private static int edgeHot() {
        return (SuiteTheme.accent() & 0xFFFFFF) | 0xCC000000;
    }

    // ── Math ────────────────────────────────────────────────────────────────

    private static float clamp01(float v) {
        return v < 0f ? 0f : (v > 1f ? 1f : v);
    }

    private static float easeOutCubic(float t) {
        float u = 1f - t;
        return 1f - u * u * u;
    }

    private static int argb(float alpha, int rgb) {
        return (Math.round(clamp01(alpha) * 255f) << 24) | (rgb & 0xFFFFFF);
    }

    private static int withAlpha(int argb, float alpha) {
        return argb(alpha, argb);
    }

    private static int scaleAlpha(int argb, float factor) {
        return argb(((argb >>> 24) / 255f) * factor, argb);
    }
}
