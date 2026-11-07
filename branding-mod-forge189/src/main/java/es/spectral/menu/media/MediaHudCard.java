package es.spectral.menu.media;

import net.minecraft.client.gui.FontRenderer;
import net.minecraft.client.gui.ScaledResolution;
import net.minecraft.util.ResourceLocation;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.HudLayout;
import es.spectral.menu.HudTheme;
import es.spectral.menu.Lang;
import es.spectral.menu.ui.Gfx;

/**
 * The movable "Now playing" card: the Media Player's current track as a HUD
 * block. It is an ordinary overlay ({@link HudLayout} position, {@link HudTheme}
 * styling), so it drags in the HUD editor exactly like the text blocks.
 *
 * <p>Same layout as the Fabric card (40 px tall): the Espectral logo as cover
 * art, title with previous / play-pause / next marks, artist, then a progress
 * bar with elapsed and total time. The marks are display-only; K / , / . drive
 * playback.
 *
 * <p>Draws only while a track is loaded and both the Media Player and this card
 * are enabled. {@link #draw} places the themed frame and contents in the live
 * HUD; {@link #drawContents} draws everything inside the frame, so the editor
 * can paint its own frame and still preview the card.
 */
public final class MediaHudCard {

    /** Feature id: a separate toggle, enabled alongside the Media Player. */
    public static final String FEATURE_ID = "mediahud";
    /** Fixed box size (px), shared by the live draw and the editor. */
    public static final int WIDTH = 184;
    public static final int HEIGHT = 40;

    private static final ResourceLocation LOGO = new ResourceLocation("espectralmenu", "textures/gui/logo.png");
    private static final int PAD = 4;
    private static final int ART = 32;
    private static final int TEXT_X = PAD + ART + 6;
    private static final int GLYPH = 7;
    private static final int GLYPH_GAP = 4;
    private static final int CONTROLS_W = 3 * GLYPH + 2 * GLYPH_GAP;
    private static final int BAR_Y = 27;
    private static final int BAR_H = 2;
    private static final int ART_BG = 0x55000000;
    private static final String ELLIPSIS = "...";

    // Per-track text cache (render thread only).
    private static MediaTrack cachedTrack;
    private static boolean cachedPaused;
    private static String cachedLang;
    private static String cachedTitle = "";
    private static String cachedSub = "";
    private static long cachedElapsedS = -1L;
    private static long cachedDurationMs = -1L;
    private static String cachedElapsed = "0:00";
    private static String cachedTotal = "0:00";

    private MediaHudCard() {}

    /** Draws the live card this frame; a no-op unless enabled with a loaded track. */
    public static void draw(ScaledResolution res) {
        ClientConfig config = ClientConfig.getInstance();
        if (!config.isFeatureEnabled(MediaPlayer.FEATURE_ID) || !config.isFeatureEnabled(FEATURE_ID)) {
            return;
        }
        if (MediaPlayer.get().current() == null) {
            return;
        }
        int[] pos = HudLayout.pos(FEATURE_ID, res.getScaledWidth(), res.getScaledHeight(), WIDTH, HEIGHT);
        int x = pos[0];
        int y = pos[1];
        HudTheme theme = HudTheme.current();
        if (theme.hasFill()) {
            Gfx.fill(x, y, x + WIDTH, y + HEIGHT, theme.fill);
        }
        if (theme.hasFrame()) {
            Gfx.outline(x, y, WIDTH, HEIGHT, theme.frame);
        }
        drawContents(x, y);
    }

    /**
     * Everything inside the card's frame at (x, y). The HUD editor calls this
     * for its preview, so with nothing loaded it shows a sample line instead.
     */
    public static void drawContents(int x, int y) {
        FontRenderer font = Gfx.font();
        if (font == null) {
            return;
        }
        MediaPlayer player = MediaPlayer.get();
        MediaTrack track = player.current();
        boolean paused = track == null || player.isPaused();
        HudTheme theme = HudTheme.current();
        int textX = x + TEXT_X;
        int right = x + WIDTH - PAD - 1;
        int controlsX = right - CONTROLS_W;

        refreshText(font, track, paused, controlsX - 4 - textX, right - textX);
        long elapsed = track == null ? 0L : player.elapsedMs();
        long duration = track == null ? 0L : track.durationMs();
        refreshClock(elapsed, duration);

        Gfx.fill(x + PAD, y + PAD, x + PAD + ART, y + PAD + ART, ART_BG);
        Gfx.sprite(LOGO, x + PAD + 2, y + PAD + 2, ART - 4, ART - 4, 0xFFFFFFFF);

        Gfx.text(cachedTitle, textX, y + 5, theme.text, theme.shadow);
        if (!cachedSub.isEmpty()) {
            Gfx.text(cachedSub, textX, y + 15, theme.label, false);
        }

        int gy = y + 5;
        prevGlyph(controlsX, gy, theme.text);
        if (paused) {
            playGlyph(controlsX + GLYPH + GLYPH_GAP, gy, theme.text);
        } else {
            pauseGlyph(controlsX + GLYPH + GLYPH_GAP, gy, theme.text);
        }
        nextGlyph(controlsX + 2 * (GLYPH + GLYPH_GAP), gy, theme.text);

        int barY = y + BAR_Y;
        Gfx.fill(textX, barY, right, barY + BAR_H, theme.outlineIdle);
        double frac = duration > 0 ? Math.min(1.0, Math.max(0.0, (double) elapsed / duration)) : 0.0;
        int filled = (int) Math.round((right - textX) * frac);
        if (filled > 0) {
            Gfx.fill(textX, barY, textX + filled, barY + BAR_H, theme.outlineHot);
        }
        Gfx.text(cachedElapsed, textX, barY + 4, theme.label, false);
        Gfx.textRight(cachedTotal, right, barY + 4, theme.label, false);
    }

    private static void refreshText(FontRenderer font, MediaTrack track, boolean paused, int titleW, int subW) {
        String lang = net.minecraft.client.Minecraft.getMinecraft().gameSettings.language;
        if (track == cachedTrack && paused == cachedPaused && lang != null && lang.equals(cachedLang)) {
            return;
        }
        cachedLang = lang;
        cachedTrack = track;
        cachedPaused = paused;
        if (track == null) {
            cachedTitle = fit(font, Lang.tr("espectral.media.card.sample"), titleW);
            cachedSub = "";
            return;
        }
        String title = track.title().isEmpty() ? track.displayName() : track.title();
        String sub = !track.artist().isEmpty() ? track.artist()
                : Lang.tr(paused ? "espectral.media.card.paused" : "espectral.media.card.playing");
        cachedTitle = fit(font, title, titleW);
        cachedSub = fit(font, sub, subW);
    }

    private static void refreshClock(long elapsedMs, long durationMs) {
        long elapsedS = elapsedMs / 1000L;
        if (elapsedS != cachedElapsedS) {
            cachedElapsedS = elapsedS;
            cachedElapsed = MediaToast.clock(elapsedMs);
        }
        if (durationMs != cachedDurationMs) {
            cachedDurationMs = durationMs;
            cachedTotal = MediaToast.clock(durationMs);
        }
    }

    /** Right-pointing triangle, 7 px tall. */
    private static void playGlyph(int x, int y, int argb) {
        for (int i = 0; i < 4; i++) {
            Gfx.fill(x + 1 + i, y + i, x + 2 + i, y + GLYPH - i, argb);
        }
    }

    /** Two bars. */
    private static void pauseGlyph(int x, int y, int argb) {
        Gfx.fill(x + 1, y, x + 3, y + GLYPH, argb);
        Gfx.fill(x + 4, y, x + 6, y + GLYPH, argb);
    }

    /** A bar and a left-pointing triangle. */
    private static void prevGlyph(int x, int y, int argb) {
        Gfx.fill(x, y, x + 1, y + GLYPH, argb);
        for (int i = 0; i < 4; i++) {
            Gfx.fill(x + 2 + i, y + 3 - i, x + 3 + i, y + 4 + i, argb);
        }
    }

    /** A right-pointing triangle and a bar. */
    private static void nextGlyph(int x, int y, int argb) {
        for (int i = 0; i < 4; i++) {
            Gfx.fill(x + 1 + i, y + i, x + 2 + i, y + GLYPH - i, argb);
        }
        Gfx.fill(x + 6, y, x + 7, y + GLYPH, argb);
    }

    /** Trims {@code text} with an ASCII ellipsis so it fits {@code maxWidth} px. */
    private static String fit(FontRenderer font, String text, int maxWidth) {
        if (maxWidth <= 0) {
            return "";
        }
        if (font.getStringWidth(text) <= maxWidth) {
            return text;
        }
        int limit = maxWidth - font.getStringWidth(ELLIPSIS);
        int end = text.length();
        while (end > 0 && font.getStringWidth(text.substring(0, end)) > limit) {
            end--;
        }
        return end > 0 ? text.substring(0, end) + ELLIPSIS : "";
    }
}
