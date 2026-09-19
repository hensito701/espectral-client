package es.spectral.menu;

import java.util.List;

/**
 * Positions for the draggable HUD overlay blocks (fpsping, coords,
 * healthstatus, armorstatus, potionstatus).
 *
 * <p>Each overlay is one movable block. A block's stored position is the
 * pixel coordinate of its top-left corner, kept in the feature's config
 * object ({@code features.<id>.x/y}); when absent the block sits at its
 * default anchor. {@code potionstatus} defaults to a right edge anchor so it
 * hugs the top-right like vanilla status effects; the rest anchor top-left.
 *
 * <p>This class is pure layout math + config access — no rendering — so it
 * compiles unchanged against both supported versions. The per-version draw
 * and the drag-edit screen feed it through {@code Compat}.
 */
public final class HudLayout {

    /** Overlay ids in draw order (top-left stack first, then the right column). */
    public static final List<String> OVERLAY_IDS =
            List.of("fpsping", "coords", "healthstatus", "armorstatus", "potionstatus");

    /** Box padding (px) around the text lines inside an overlay block. */
    public static final int PAD = 3;
    /** Line pitch (px) between text rows inside a block. */
    public static final int LINE_PITCH = 10;

    private HudLayout() {}

    /** True when this feature id is a draggable HUD overlay block. */
    public static boolean isOverlay(String id) {
        return OVERLAY_IDS.contains(id);
    }

    /** Default top-left anchor for an overlay that has no stored position. */
    public static int[] defaultPos(String id, int guiWidth, int guiHeight, int blockWidth, int blockHeight) {
        if ("potionstatus".equals(id)) {
            // Right edge, top — mirrors the vanilla status-effect column.
            return new int[] { Math.max(0, guiWidth - blockWidth - 4), 4 };
        }
        return new int[] { 4, 4 + stackOffset(id) };
    }

    /**
     * Vertical offset for the default top-left stack so the blocks don't
     * overlap: each earlier enabled block shifts the next one down. Order is
     * fixed (fpsping, coords, healthstatus, armorstatus) regardless of which
     * are enabled — keeps anchors stable as features toggle.
     */
    private static int stackOffset(String id) {
        switch (id) {
            case "coords": return 14;
            case "healthstatus": return 28;
            case "armorstatus": return 42;
            default: return 0; // fpsping
        }
    }

    /**
     * Resolved top-left pixel position for an overlay: the stored drag
     * position when present, else the default anchor.
     */
    public static int[] pos(String id, int guiWidth, int guiHeight, int blockWidth, int blockHeight) {
        int[] stored = ClientConfig.getInstance().getFeaturePos(id);
        if (stored != null) {
            return clamp(id, stored[0], stored[1], guiWidth, guiHeight, blockWidth, blockHeight);
        }
        return defaultPos(id, guiWidth, guiHeight, blockWidth, blockHeight);
    }

    /** Clamps a top-left position so the block stays fully on screen. */
    public static int[] clamp(String id, int x, int y, int guiWidth, int guiHeight, int blockWidth, int blockHeight) {
        int maxX = Math.max(0, guiWidth - blockWidth);
        int maxY = Math.max(0, guiHeight - blockHeight);
        return new int[] {
            Math.max(0, Math.min(x, maxX)),
            Math.max(0, Math.min(y, maxY)),
        };
    }

    /** Persists a dragged position (clamped). */
    public static void setPos(String id, int x, int y, int guiWidth, int guiHeight, int blockWidth, int blockHeight) {
        int[] c = clamp(id, x, y, guiWidth, guiHeight, blockWidth, blockHeight);
        ClientConfig.getInstance().setFeaturePos(id, c[0], c[1]);
    }

    /** Returns an overlay to its default anchor. */
    public static void resetPos(String id) {
        ClientConfig.getInstance().clearFeaturePos(id);
    }

    /** Resets every overlay to its default anchor. */
    public static void resetAll() {
        for (String id : OVERLAY_IDS) {
            ClientConfig.getInstance().clearFeaturePos(id);
        }
    }
}
