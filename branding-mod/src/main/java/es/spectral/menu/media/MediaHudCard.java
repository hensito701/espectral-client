package es.spectral.menu.media;

import net.minecraft.client.Minecraft;
import net.minecraft.resources.Identifier;
import net.minecraft.client.gui.Font;
import net.minecraft.network.chat.Component;

import es.spectral.menu.Compat;
import es.spectral.menu.HudEngine;
import es.spectral.menu.HudLayout;
import es.spectral.menu.HudTheme;

/**
 * The movable "Now playing" card: the Media Player's current track as a HUD
 * block. It is an ordinary overlay ({@link HudLayout} position, {@link HudTheme}
 * styling), so it drags in the HUD editor exactly like the text blocks.
 *
 * <p>Layout (40 px tall): the Espectral logo as cover art on the left; title
 * with previous / play-pause / next marks on the first row; artist on the
 * second; a progress bar with elapsed and total time under it. The marks are
 * display-only (the mouse is grabbed in game); K / , / . drive playback.
 *
 * <p>Draws only while a track is loaded and both the Media Player and this card
 * are enabled. {@link #draw} places the themed frame and contents in the live
 * HUD; {@link #drawContents} draws everything inside the frame, so the editor
 * can paint its own frame and still preview the card.
 *
 * <p>Pure drawing through {@code Compat.ui*} with an opaque graphics object, so
 * the class is lane-neutral (1.21.11 {@code GuiGraphics} / 26.x extractor).
 * Trimmed strings are cached per track/state, so a steady frame allocates only
 * the two clock strings, once per second.
 */
public final class MediaHudCard {

    /** Feature id: a separate toggle, enabled alongside the Media Player. */
    public static final String FEATURE_ID = "mediahud";
    /** Fixed box size (px), shared by the live draw and the editor. */
    public static final int WIDTH = 184;
    public static final int HEIGHT = 40;

    private static final Identifier LOGO = Identifier.fromNamespaceAndPath("espectral-menu", "logo");
    private static final int PAD = 4;
    private static final int ART = 32;
    /** Text column start, relative to the card's left edge. */
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
    public static void draw(Object gfx, Minecraft minecraft, int guiW, int guiH) {
        if (minecraft == null || minecraft.font == null) {
            return;
        }
        if (!HudEngine.isEnabled(MediaPlayer.FEATURE_ID) || !HudEngine.isEnabled(FEATURE_ID)) {
            return;
        }
        if (MediaPlayer.get().current() == null) {
            return;
        }
        int[] pos = HudLayout.pos(FEATURE_ID, guiW, guiH, WIDTH, HEIGHT);
        int x = pos[0];
        int y = pos[1];
        HudTheme theme = HudTheme.current();
        if (theme.hasFill()) {
            Compat.uiFill(gfx, x, y, x + WIDTH, y + HEIGHT, theme.fill);
        }
        if (theme.hasFrame()) {
            Compat.uiOutline(gfx, x, y, WIDTH, HEIGHT, theme.frame);
        }
        drawContents(gfx, minecraft, x, y);
    }

    /**
     * Everything inside the card's frame at (x, y). The HUD editor calls this
     * for its preview, so with nothing loaded it shows a sample line instead.
     */
    public static void drawContents(Object gfx, Minecraft minecraft, int x, int y) {
        if (minecraft == null || minecraft.font == null) {
            return;
        }
        Font font = minecraft.font;
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

        // Cover art: the Espectral logo on a darker square.
        Compat.uiFill(gfx, x + PAD, y + PAD, x + PAD + ART, y + PAD + ART, ART_BG);
        Compat.uiSprite(gfx, LOGO, x + PAD + 2, y + PAD + 2, ART - 4, ART - 4, 0xFFFFFFFF);

        Compat.uiText(gfx, font, Component.literal(cachedTitle), textX, y + 5, theme.text, theme.shadow);
        if (!cachedSub.isEmpty()) {
            Compat.uiText(gfx, font, Component.literal(cachedSub), textX, y + 15, theme.label, false);
        }

        int gy = y + 5;
        prevGlyph(gfx, controlsX, gy, theme.text);
        if (paused) {
            playGlyph(gfx, controlsX + GLYPH + GLYPH_GAP, gy, theme.text);
        } else {
            pauseGlyph(gfx, controlsX + GLYPH + GLYPH_GAP, gy, theme.text);
        }
        nextGlyph(gfx, controlsX + 2 * (GLYPH + GLYPH_GAP), gy, theme.text);

        int barY = y + BAR_Y;
        Compat.uiFill(gfx, textX, barY, right, barY + BAR_H, theme.outlineIdle);
        double frac = duration > 0 ? Math.min(1.0, Math.max(0.0, (double) elapsed / duration)) : 0.0;
        int filled = (int) Math.round((right - textX) * frac);
        if (filled > 0) {
            Compat.uiFill(gfx, textX, barY, textX + filled, barY + BAR_H, theme.outlineHot);
        }
        Compat.uiText(gfx, font, Component.literal(cachedElapsed), textX, barY + 4, theme.label, false);
        Compat.uiText(gfx, font, Component.literal(cachedTotal),
                right - font.width(cachedTotal), barY + 4, theme.label, false);
    }

    private static void refreshText(Font font, MediaTrack track, boolean paused, int titleW, int subW) {
        String lang = Minecraft.getInstance().getLanguageManager().getSelected();
        if (track == cachedTrack && paused == cachedPaused && lang.equals(cachedLang)) {
            return;
        }
        cachedTrack = track;
        cachedPaused = paused;
        cachedLang = lang;
        if (track == null) {
            cachedTitle = fit(font, Component.translatable("espectral.media.card.sample").getString(), titleW);
            cachedSub = "";
            return;
        }
        String title = track.title().isEmpty() ? track.displayName() : track.title();
        String sub = !track.artist().isEmpty() ? track.artist()
                : Component.translatable(paused ? "espectral.media.card.paused" : "espectral.media.card.playing")
                        .getString();
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

    /** ▶: a right-pointing triangle, 7 px tall. */
    private static void playGlyph(Object gfx, int x, int y, int argb) {
        for (int i = 0; i < 4; i++) {
            Compat.uiFill(gfx, x + 1 + i, y + i, x + 2 + i, y + GLYPH - i, argb);
        }
    }

    /** ⏸: two bars. */
    private static void pauseGlyph(Object gfx, int x, int y, int argb) {
        Compat.uiFill(gfx, x + 1, y, x + 3, y + GLYPH, argb);
        Compat.uiFill(gfx, x + 4, y, x + 6, y + GLYPH, argb);
    }

    /** ⏮: a bar and a left-pointing triangle. */
    private static void prevGlyph(Object gfx, int x, int y, int argb) {
        Compat.uiFill(gfx, x, y, x + 1, y + GLYPH, argb);
        for (int i = 0; i < 4; i++) {
            Compat.uiFill(gfx, x + 2 + i, y + 3 - i, x + 3 + i, y + 4 + i, argb);
        }
    }

    /** ⏭: a right-pointing triangle and a bar. */
    private static void nextGlyph(Object gfx, int x, int y, int argb) {
        for (int i = 0; i < 4; i++) {
            Compat.uiFill(gfx, x + 1 + i, y + i, x + 2 + i, y + GLYPH - i, argb);
        }
        Compat.uiFill(gfx, x + 6, y, x + 7, y + GLYPH, argb);
    }

    /** Trims {@code text} with an ASCII ellipsis so it fits {@code maxWidth} px. */
    private static String fit(Font font, String text, int maxWidth) {
        if (maxWidth <= 0) {
            return "";
        }
        if (font.width(text) <= maxWidth) {
            return text;
        }
        int limit = maxWidth - font.width(ELLIPSIS);
        int end = text.length();
        while (end > 0 && font.width(text.substring(0, end)) > limit) {
            end--;
        }
        return end > 0 ? text.substring(0, end) + ELLIPSIS : "";
    }
}
