package es.spectral.menu.ui;

import net.minecraft.ChatFormatting;
import es.spectral.menu.Compat;
import es.spectral.menu.EspectralBrand;
import net.minecraft.client.gui.Font;
import net.minecraft.network.chat.Component;

/**
 * The optional dark-glass backdrop behind the Suite screens and the title
 * wordmark. Pure drawing through {@code Compat.ui*} with an opaque graphics
 * object, so this shared class never names a version-specific graphics type;
 * each lane's background hook calls it with its own extractor.
 */
public final class SuitePainter {

    private SuitePainter() {}

    /**
     * Title-screen backdrop: a full-width dark glass strip across the top
     * (behind the appended top-right action row) with a gold hairline, the
     * Espectral wordmark at the left and a dim version line under it. The
     * panorama, logo, splash and every vanilla widget are untouched.
     */
    public static void paintTitle(Object gfx, int width, int height, Font font) {
        if (gfx == null || font == null) return;
        int bandH = SuiteTheme.ACTION_Y + 20 + 8;
        if (height < bandH + 40) return;
        Compat.uiFill(gfx, 0, 0, width, bandH, SuiteTheme.PANEL);
        Compat.uiFill(gfx, 0, bandH, width, bandH + 1, SuiteTheme.GOLD_HAIRLINE);
        Compat.uiText(gfx, font,
                Component.translatable("espectral.suite.title").withStyle(ChatFormatting.GOLD),
                8, SuiteTheme.ACTION_Y + 1, SuiteTheme.GOLD, false);
        String version = EspectralBrand.modVersion();
        if (version != null && !version.isBlank()) {
            Compat.uiText(gfx, font, Component.literal("v" + version.trim()),
                    8, SuiteTheme.ACTION_Y + 12, SuiteTheme.TEXT_DIM, false);
        }
    }

    /**
     * Suite-screen backdrop: a full-screen dim plus the centred glass panel
     * with hairlines separating the header and footer zones. Row widgets draw
     * themselves; this only frames them.
     */
    public static void paintSuite(Object gfx, SuiteScreen screen, int width, int height, Font font) {
        if (gfx == null || font == null) return;
        paintPanel(gfx, width, height);
    }

    /** Confirm-dialog backdrop: dim plus the small centred glass panel. */
    public static void paintConfirm(Object gfx, SuiteConfirmScreen screen, int width, int height, Font font) {
        if (gfx == null || font == null || screen == null) return;
        Compat.uiFill(gfx, 0, 0, width, height, SuiteTheme.SCRIM);
        int cw = SuiteConfirmScreen.panelWidth(width);
        int ch = SuiteConfirmScreen.PANEL_H;
        int cx = (width - cw) / 2;
        int cy = (height - ch) / 2;
        Compat.uiFill(gfx, cx, cy, cx + cw, cy + ch, SuiteTheme.PANEL);
        Compat.uiOutline(gfx, cx, cy, cw, ch, SuiteTheme.PANEL_EDGE);
        Compat.uiTextCentered(gfx, font, screen.getTitleText(), width / 2, cy + 14,
                SuiteTheme.GOLD, false);
        Compat.uiTextCentered(gfx, font, screen.getMessageText(), width / 2, cy + 34,
                SuiteTheme.TEXT, false);
    }

    private static void paintPanel(Object gfx, int width, int height) {
        Compat.uiFill(gfx, 0, 0, width, height, SuiteTheme.SCRIM);
        int pw = SuiteTheme.panelWidth(width);
        int px = SuiteTheme.panelX(width);
        int top = SuiteTheme.PANEL_TOP;
        int bottom = height - SuiteTheme.PANEL_TOP;
        Compat.uiFill(gfx, px, top, px + pw, bottom, SuiteTheme.PANEL);
        Compat.uiOutline(gfx, px, top, pw, bottom - top, SuiteTheme.PANEL_EDGE);
        int headerLine = SuiteTheme.headerBottom() + 2;
        Compat.uiFill(gfx, px + 1, headerLine, px + pw - 1, headerLine + 1,
                SuiteTheme.GOLD_HAIRLINE);
        int footerLine = SuiteTheme.footerY(height) - 3;
        Compat.uiFill(gfx, px + 1, footerLine, px + pw - 1, footerLine + 1,
                SuiteTheme.GOLD_HAIRLINE);
    }
}
