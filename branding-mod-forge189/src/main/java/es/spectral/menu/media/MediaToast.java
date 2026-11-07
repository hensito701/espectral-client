package es.spectral.menu.media;

import java.util.Locale;

import es.spectral.menu.ui.Gfx;
import es.spectral.menu.ui.SuiteTheme;
import net.minecraft.client.gui.FontRenderer;

/**
 * The now-playing toast drawn over the in-game HUD on a track change. It
 * fades out with {@link MediaPlayer#toastTicksLeft()} and sits in the top-left
 * corner, clear of the vanilla chat and boss bar.
 */
public final class MediaToast {

    private static final int PAD = 4;
    private static final int X = 8;
    private static final int Y = 8;

    private MediaToast() {}

    /** Draws the toast when one is showing; a no-op otherwise. */
    public static void render() {
        MediaPlayer player = MediaPlayer.get();
        if (!player.toastVisible()) {
            return;
        }
        FontRenderer font = Gfx.font();
        if (font == null) {
            return;
        }
        String text = player.toastText();
        int alpha = Math.min(255, player.toastTicksLeft() * 12);
        int w = font.getStringWidth(text) + 2 * PAD + 10;
        int h = font.FONT_HEIGHT + 2 * PAD;
        Gfx.fill(X, Y, X + w, Y + h, (SuiteTheme.PANEL & 0x00FFFFFF) | (Math.min(0xE0, alpha) << 24));
        Gfx.fill(X, Y, X + 2, Y + h, SuiteTheme.accentHairline());
        Gfx.text(text, X + PAD + 6, Y + PAD, (SuiteTheme.TEXT & 0x00FFFFFF) | (alpha << 24), false);
    }

    /** {@code m:ss} for a duration in milliseconds (negative values read as zero). */
    public static String clock(long millis) {
        long totalSeconds = Math.max(0L, millis) / 1000L;
        return String.format(Locale.ROOT, "%d:%02d", totalSeconds / 60L, totalSeconds % 60L);
    }
}
