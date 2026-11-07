package es.spectral.menu.media;

import java.util.Locale;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;

import es.spectral.menu.Compat;
import es.spectral.menu.ui.SuiteTheme;

/**
 * The non-intrusive "now playing" toast: one dim glass line in the top-left
 * corner that fades out by itself (3 s) and only draws while a track change
 * happened with the setting on.
 *
 * <p>Pure drawing through {@code Compat.ui*} with an opaque graphics object, so
 * the class is lane-neutral (1.21.11 {@code GuiGraphics} / 26.x extractor).
 */
public final class MediaToast {

    private static final int PAD = 4;

    private MediaToast() {}

    /** Draws the toast for this frame when one is live; no-op otherwise. */
    public static void render(Object gfx, Minecraft minecraft) {
        MediaPlayer player = MediaPlayer.get();
        if (!player.toastVisible() || minecraft == null || minecraft.font == null) {
            return;
        }
        Font font = minecraft.font;
        String text = player.toastText();
        int w = font.width(text) + 2 * PAD + 10;
        int h = font.lineHeight + 2 * PAD;
        int x = SuiteTheme.MARGIN;
        int y = SuiteTheme.ACTION_Y + SuiteTheme.FOOTER_H + 6;
        int alpha = Math.min(255, player.toastTicksLeft() * 12);
        int fill = (SuiteTheme.PANEL & 0x00FFFFFF) | (Math.min(0xE0, alpha) << 24);
        int textColor = (SuiteTheme.TEXT & 0x00FFFFFF) | (alpha << 24);
        Compat.uiFill(gfx, x, y, x + w, y + h, fill);
        Compat.uiFill(gfx, x, y, x + 2, y + h, SuiteTheme.accentHairline());
        Compat.uiText(gfx, font, net.minecraft.network.chat.Component.literal(text),
                x + PAD + 6, y + PAD, textColor, false);
    }

    /** {@code m:ss} — shared with the sub-screen's elapsed/total line. */
    public static String clock(long millis) {
        long totalSeconds = Math.max(0L, millis) / 1000L;
        return String.format(Locale.ROOT, "%d:%02d", totalSeconds / 60L, totalSeconds % 60L);
    }
}
