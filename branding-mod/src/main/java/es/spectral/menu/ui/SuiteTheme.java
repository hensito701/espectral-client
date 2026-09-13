package es.spectral.menu.ui;

/**
 * Frozen visual tokens and layout metrics for the Suite screens (dark glass +
 * gold, restrained). Rows are square with 1&nbsp;px hairlines and no textures.
 *
 * <p>Layout must work at the 854x480 window (GUI scale 2, i.e. 427x240
 * logical) and scale up gracefully: margin 8, panel width
 * {@code min(width - 16, 420)}, panel {@code y = 6 .. height - 6}.
 */
public final class SuiteTheme {

    private SuiteTheme() {}

    // Glass + gold palette (ARGB).
    public static final int PANEL = 0xF0121418;
    public static final int PANEL_EDGE = 0x66D9A93B;
    public static final int ROW = 0x8C181B21;
    public static final int ROW_HOVER = 0x9C20242C;
    public static final int GOLD = 0xFFD9A93B;
    public static final int GOLD_BRIGHT = 0xFFFFD873;
    public static final int GOLD_HAIRLINE = 0x40D9A93B;
    /** Horizontal gold gradient: light left edge, deep right edge. */
    public static final int GOLD_GRADIENT_LEFT = 0xFFFFE3A1;
    public static final int GOLD_GRADIENT_RIGHT = 0xFFC47B1E;
    public static final int TEXT = 0xFFE8E6E1;
    public static final int TEXT_DIM = 0xFF9AA0A8;
    public static final int OK = 0xFF7FD58B;
    public static final int OFF = 0xFF8A8F98;
    public static final int WARN = 0xFFE0A24A;
    /** Full-screen dim drawn behind modal panels. */
    public static final int SCRIM = 0xC0080A0C;

    // Layout metrics (logical px). Every screen recomputes geometry from
    // width/height in init(); nothing is cached between calls.
    public static final int MARGIN = 8;
    public static final int PANEL_TOP = 6;
    public static final int MAX_PANEL_W = 420;
    public static final int PAD = 6;
    public static final int HEADER_H = 16;
    public static final int SEARCH_H = 18;
    public static final int CAT_H = 14;
    public static final int ROW_H = 22;
    public static final int TOGGLE_W = 86;
    public static final int MASTER_W = 118;
    public static final int FOOTER_H = 20;
    public static final int PAGER_H = 20;
    public static final int GAP = 4;

    /** Panel width for a screen of the given logical width. */
    public static int panelWidth(int screenWidth) {
        return Math.min(screenWidth - 2 * MARGIN, MAX_PANEL_W);
    }

    /** Left edge of the centred panel. */
    public static int panelX(int screenWidth) {
        return (screenWidth - panelWidth(screenWidth)) / 2;
    }

    /**
     * Linear interpolation between two opaque ARGB colors ({@code t} 0..1).
     * Pure arithmetic for the painter's per-character gold gradient.
     */
    public static int lerpArgb(int left, int right, float t) {
        float clamped = Math.min(1.0F, Math.max(0.0F, t));
        int a = Math.round(((left >>> 24) & 0xFF) * (1.0F - clamped)
                + ((right >>> 24) & 0xFF) * clamped);
        int r = Math.round(((left >>> 16) & 0xFF) * (1.0F - clamped)
                + ((right >>> 16) & 0xFF) * clamped);
        int g = Math.round(((left >>> 8) & 0xFF) * (1.0F - clamped)
                + ((right >>> 8) & 0xFF) * clamped);
        int b = Math.round((left & 0xFF) * (1.0F - clamped)
                + (right & 0xFF) * clamped);
        return (a << 24) | (r << 16) | (g << 8) | b;
    }

    // Title action row: one right-aligned row at the top-right corner
    // (y = 6, 6 px margin), widths 120 + 100 with a 4 px gap. (The Esc
    // screen buttons live in the vanilla button column instead.)
    public static final int ACTION_Y = 6;
    public static final int ACTION_MARGIN = 6;
    public static final int ACTION_CLIENT_W = 120;
    public static final int ACTION_SUPPORT_W = 100;
    public static final int ACTION_ROW_W = ACTION_CLIENT_W + GAP + ACTION_SUPPORT_W;

    /** Left edge of the right-aligned title action row. */
    public static int actionRowX(int screenWidth) {
        return Math.max(ACTION_MARGIN, screenWidth - ACTION_MARGIN - ACTION_ROW_W);
    }
    /** Bottom edge of the Suite header zone (title + master chip). */
    public static int headerBottom() {
        return PANEL_TOP + PAD + HEADER_H;
    }

    /** Top edge of the footer button row. */
    public static int footerY(int screenHeight) {
        return screenHeight - PANEL_TOP - PAD - FOOTER_H;
    }
}
