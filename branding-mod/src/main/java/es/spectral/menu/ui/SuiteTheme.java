package es.spectral.menu.ui;

import net.minecraft.ChatFormatting;

import es.spectral.menu.ClientConfig;

/**
 * Visual tokens and layout metrics for the Suite screens (dark glass +
 * accent, restrained). Rows are square with 1&nbsp;px hairlines and no
 * textures.
 *
 * <p>Layout must work at the 854x480 window (GUI scale 2, i.e. 427x240
 * logical) and scale up gracefully: margin 8, panel width
 * {@code min(width - 16, 420)}, panel {@code y = 6 .. height - 6}.
 *
 * <p>Accent colours are no longer constants: {@link #accent()} and friends
 * resolve the palette selected in {@code personalization.theme} of
 * {@code espectral-client.json} (Workstream 4 — one palette drives the Suite,
 * the title menu and the launcher). {@code oro} reproduces exactly the
 * original gold palette. Layout metrics stay theme-independent constants.
 */
public final class SuiteTheme {

    private SuiteTheme() {}

    /**
     * One accent palette per theme id (contract:
     * {@code oro|carmesi|amatista|esmeralda|glaciar}). {@code bright} is the
     * hover/highlight shade, {@code gradLeft}/{@code gradRight} the wordmark
     * gradient stops and {@code chat} the nearest vanilla formatting for
     * styled components.
     */
    private enum Palette {
        ORO(0xFFD9A93B, 0xFFFFD873, 0xFFFFE3A1, 0xFFC47B1E, ChatFormatting.GOLD),
        CARMESI(0xFFE5484D, 0xFFFF8A92, 0xFFFFB3B8, 0xFFB01E3C, ChatFormatting.RED),
        AMATISTA(0xFFBB86FC, 0xFFD9B3FF, 0xFFF0DDFF, 0xFF7C3AED, ChatFormatting.LIGHT_PURPLE),
        ESMERALDA(0xFF10B981, 0xFF6EE7B7, 0xFFA7F3D0, 0xFF059669, ChatFormatting.GREEN),
        GLACIAR(0xFF38BDF8, 0xFF7DD3FC, 0xFFC9EDFF, 0xFF0284C7, ChatFormatting.AQUA);

        final int accent;
        final int bright;
        final int gradLeft;
        final int gradRight;
        final ChatFormatting chat;

        Palette(int accent, int bright, int gradLeft, int gradRight, ChatFormatting chat) {
            this.accent = accent;
            this.bright = bright;
            this.gradLeft = gradLeft;
            this.gradRight = gradRight;
            this.chat = chat;
        }
    }

    /** Palette for the configured theme id; {@code ORO} on unknown/absent. */
    private static Palette palette() {
        return switch (ClientConfig.getInstance().theme()) {
            case "carmesi" -> Palette.CARMESI;
            case "amatista" -> Palette.AMATISTA;
            case "esmeralda" -> Palette.ESMERALDA;
            case "glaciar" -> Palette.GLACIAR;
            default -> Palette.ORO;
        };
    }

    /** Primary accent (was {@code GOLD}). */
    public static int accent() {
        return palette().accent;
    }

    /** Bright accent for hover text/icons (was {@code GOLD_BRIGHT}). */
    public static int accentBright() {
        return palette().bright;
    }

    /** 25%-alpha accent hairline (was {@code GOLD_HAIRLINE}). */
    public static int accentHairline() {
        return (palette().accent & 0xFFFFFF) | 0x40000000;
    }

    /** 40%-alpha accent panel edge (was {@code PANEL_EDGE}). */
    public static int panelEdge() {
        return (palette().accent & 0xFFFFFF) | 0x66000000;
    }

    /** Wordmark gradient: light left edge (was {@code GOLD_GRADIENT_LEFT}). */
    public static int accentGradientLeft() {
        return palette().gradLeft;
    }

    /** Wordmark gradient: deep right edge (was {@code GOLD_GRADIENT_RIGHT}). */
    public static int accentGradientRight() {
        return palette().gradRight;
    }

    /** Nearest vanilla chat formatting for the accent (was hardcoded GOLD). */
    public static ChatFormatting accentFormat() {
        return palette().chat;
    }

    // Glass palette (ARGB) — theme-independent.
    public static final int PANEL = 0xF0121418;
    public static final int ROW = 0x8C181B21;
    public static final int ROW_HOVER = 0x9C20242C;
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
     * Pure arithmetic for the painter's per-character gradient.
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
