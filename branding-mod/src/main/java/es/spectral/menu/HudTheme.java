package es.spectral.menu;

/**
 * HUD overlay themes (Workstream 4 free personalization).
 *
 * <p>One version-neutral palette for the box the overlay blocks draw around
 * their rows: backing fill, optional border frame, text and the editor's
 * label/outline accents. Both lanes' {@code Compat.drawHudOverlays} and both
 * {@code HudEditScreen}s consume the same constants so a theme switch is a
 * config write, not per-version code.
 *
 * <p>{@link #current()} resolves {@code personalization.hud_theme} through
 * {@link ClientConfig#hudTheme()} every call — a launcher PATCH hot-reloads
 * the config and the HUD restyles without a restart. Unknown/missing ids
 * fall back to {@link #CRISTAL} on the ClientConfig side already.
 */
public enum HudTheme {

    /** Today's look, unchanged: 50% black box, opaque white shadowed text. */
    CRISTAL(0x80000000, 0x00000000, 0xFFFFFFFF, 0xFFAAAAAA, 0x80FFFFFF, 0xFFFFD700, false),

    /** Warm gold: dark smoke fill with a faint gold hairline, ivory text. */
    ORO(0xB3140F08, 0x59D9A93B, 0xFFEFE6D4, 0xFFD9A93B, 0x80D9A93B, 0xFFFFD873, false),

    /** Bare minimum: no box, plain text with only the vanilla drop shadow. */
    MINIMO(0x00000000, 0x00000000, 0xFFFFFFFF, 0xFF8A8F98, 0x40FFFFFF, 0xFFFFD700, false),

    /** Retro pixel terminal: opaque black box with a hard white frame and
     * phosphor-green text (no soft shadow — the frame does the separation). */
    PIXEL(0xCC000000, 0xFFFFFFFF, 0xFF8CFF6A, 0xFF8CFF6A, 0xFF8CFF6A, 0xFFFFFFFF, true);

    /** ARGB fill of the whole block rectangle; alpha 0 disables it. */
    public final int fill;
    /** ARGB 1-px frame around the block; alpha 0 disables it. */
    public final int frame;
    /** ARGB row text color (extractor needs opaque alpha to draw). */
    public final int text;
    /** ARGB feature label above each editor box. */
    public final int label;
    /** ARGB editor outline, idle. */
    public final int outlineIdle;
    /** ARGB editor outline while hovered/dragging. */
    public final int outlineHot;
    /** When true the text keeps its vanilla drop shadow. */
    public final boolean shadow;

    HudTheme(int fill, int frame, int text, int label, int outlineIdle, int outlineHot, boolean shadow) {
        this.fill = fill;
        this.frame = frame;
        this.text = text;
        this.label = label;
        this.outlineIdle = outlineIdle;
        this.outlineHot = outlineHot;
        this.shadow = shadow;
    }

    /** Theme selected in the client config; CRISTAL when unset/invalid. */
    public static HudTheme current() {
        try {
            String id = ClientConfig.getInstance().hudTheme();
            for (HudTheme t : values()) {
                if (t.id().equals(id)) {
                    return t;
                }
            }
        } catch (Throwable ignored) {
            // Config not loaded yet — keep the default look.
        }
        return CRISTAL;
    }

    /** Lowercase config id (name() lowercased; English 'i' is safe). */
    public String id() {
        return name().toLowerCase(java.util.Locale.ROOT);
    }

    /** True when the fill should be painted (non-zero alpha). */
    public boolean hasFill() {
        return (fill >>> 24) != 0;
    }

    /** True when the 1-px frame should be painted (non-zero alpha). */
    public boolean hasFrame() {
        return (frame >>> 24) != 0;
    }
}
