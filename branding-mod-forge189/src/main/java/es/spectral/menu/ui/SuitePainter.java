package es.spectral.menu.ui;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.client.gui.FontRenderer;

/**
 * Dark-glass backdrops of the Suite screens (same tokens as the Fabric
 * lanes): full-screen dim + centred glass panel with accent hairlines between
 * the header and footer zones, and the small confirm-dialog panel. Gold
 * titles render as a light-to-deep gradient; body text stays flat.
 */
public final class SuitePainter {

    private SuitePainter() {}

    private static final int LINE_H = 10;

    /** Suite / sub-sheet backdrop. */
    public static void paintPanel(int width, int height) {
        Gfx.fill(0, 0, width, height, SuiteTheme.SCRIM);
        int pw = SuiteTheme.panelWidth(width);
        int px = SuiteTheme.panelX(width);
        int top = SuiteTheme.PANEL_TOP;
        int bottom = height - SuiteTheme.PANEL_TOP;
        Gfx.fill(px, top, px + pw, bottom, SuiteTheme.PANEL);
        Gfx.outline(px, top, pw, bottom - top, SuiteTheme.panelEdge());
        int headerLine = SuiteTheme.headerBottom() + 2;
        Gfx.fill(px + 1, headerLine, px + pw - 1, headerLine + 1, SuiteTheme.accentHairline());
        int footerLine = SuiteTheme.footerY(height) - 3;
        Gfx.fill(px + 1, footerLine, px + pw - 1, footerLine + 1, SuiteTheme.accentHairline());
    }

    /** Confirm-dialog backdrop: dim, small panel, gradient title, wrapped message. */
    public static void paintConfirm(int width, int height, int panelW, int panelH, String title, List<String> lines) {
        Gfx.fill(0, 0, width, height, SuiteTheme.SCRIM);
        int cx = (width - panelW) / 2;
        int cy = (height - panelH) / 2;
        Gfx.fill(cx, cy, cx + panelW, cy + panelH, SuiteTheme.PANEL);
        Gfx.outline(cx, cy, panelW, panelH, SuiteTheme.panelEdge());
        Gfx.textCenteredGradient(title, width / 2, cy + 14, SuiteTheme.accentGradientLeft(),
                SuiteTheme.accentGradientRight(), false);
        int my = cy + 34;
        for (String line : lines) {
            Gfx.textCentered(line, width / 2, my, SuiteTheme.TEXT, false);
            my += LINE_H;
        }
    }

    /** Greedy word wrap; an over-long single word keeps its own line. */
    public static List<String> wrap(String text, FontRenderer font, int maxW) {
        List<String> out = new ArrayList<String>();
        if (text == null || text.isEmpty() || font == null || maxW <= 0) {
            out.add(text == null ? "" : text);
            return out;
        }
        StringBuilder line = new StringBuilder();
        for (String word : text.split(" ")) {
            if (word.isEmpty()) continue;
            String candidate = line.length() == 0 ? word : line + " " + word;
            if (font.getStringWidth(candidate) <= maxW || line.length() == 0) {
                line.setLength(0);
                line.append(candidate);
            } else {
                out.add(line.toString());
                line.setLength(0);
                line.append(word);
            }
        }
        if (line.length() > 0) out.add(line.toString());
        if (out.isEmpty()) out.add("");
        return out;
    }
}
