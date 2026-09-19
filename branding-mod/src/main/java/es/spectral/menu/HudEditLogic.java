package es.spectral.menu;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;

/**
 * Shared logic for the drag-to-move HUD overlay editor: block geometry,
 * hit-testing and drag state. Rendering and the version-specific mouse
 * signatures live in the per-version {@code HudEditScreen}; this class is the
 * version-agnostic core so both screens stay thin.
 *
 * <p>The editor shows every overlay block (enabled or not) with a sample
 * line so positions can be arranged before the feature is turned on. Live
 * content is used when a world is loaded; otherwise a placeholder line keeps
 * the block draggable on the title screen.
 */
public final class HudEditLogic {

    /** One draggable overlay block with its current on-screen rectangle. */
    public static final class EditBlock {
        public final String id;
        public final String label;
        public final List<String> lines;
        public int x;
        public int y;
        public int w;
        public int h;

        EditBlock(String id, String label, List<String> lines) {
            this.id = id;
            this.label = label;
            this.lines = lines;
        }

        public boolean contains(double mx, double my) {
            return mx >= x && mx < x + w && my >= y && my < y + h;
        }
    }

    /** Display labels per overlay id (Spanish, matching the suite UI). */
    private static String labelOf(String id) {
        switch (id) {
            case "fpsping": return "FPS / Ping";
            case "coords": return "Coordenadas";
            case "healthstatus": return "Salud";
            case "armorstatus": return "Armadura";
            case "potionstatus": return "Pociones";
            default: return id;
        }
    }

    /** Placeholder line when a block has no live content (no world / off). */
    private static String sampleOf(String id) {
        switch (id) {
            case "fpsping": return "FPS: 240 | Ping: 12 ms";
            case "coords": return "XYZ: 128.5 / 64.0 / -256.2 [Norte]";
            case "healthstatus": return "Salud: 20.0/20.0";
            case "armorstatus": return "Casco: 363/363";
            case "potionstatus": return "Velocidad II 0:42";
            default: return id;
        }
    }

    private final List<EditBlock> blocks = new ArrayList<>();
    private EditBlock dragging;
    private int dragOffX;
    private int dragOffY;
    private boolean dirty;

    /**
     * Rebuilds the block list with current positions. Live lines come from
     * {@link HudEngine#blocks} when a world is loaded; otherwise each overlay
     * gets its sample line so it stays draggable on the title screen.
     */
    public void rebuild(Minecraft minecraft, Font font, int guiW, int guiH) {
        blocks.clear();
        // Live content keyed by id, when a world is loaded.
        List<HudEngine.Block> live = HudEngine.getInstance().blocks(minecraft);
        for (String id : HudLayout.OVERLAY_IDS) {
            List<String> lines = null;
            for (HudEngine.Block b : live) {
                if (b.id.equals(id)) { lines = b.lines; break; }
            }
            if (lines == null || lines.isEmpty()) {
                lines = List.of(sampleOf(id));
            }
            EditBlock eb = new EditBlock(id, labelOf(id), lines);
            int w = 0;
            for (String line : lines) w = Math.max(w, font.width(line));
            eb.w = w + HudLayout.PAD * 2;
            eb.h = lines.size() * HudLayout.LINE_PITCH - (HudLayout.LINE_PITCH - 9) + HudLayout.PAD * 2;
            int[] pos = HudLayout.pos(id, guiW, guiH, eb.w, eb.h);
            eb.x = pos[0];
            eb.y = pos[1];
            blocks.add(eb);
        }
    }

    public List<EditBlock> blocks() {
        return blocks;
    }

    /** Starts a drag if the point is inside a block. Returns true on grab. */
    public boolean mouseDown(double mx, double my) {
        // Topmost wins: iterate in reverse draw order.
        for (int i = blocks.size() - 1; i >= 0; i--) {
            EditBlock b = blocks.get(i);
            if (b.contains(mx, my)) {
                dragging = b;
                dragOffX = (int) Math.round(mx - b.x);
                dragOffY = (int) Math.round(my - b.y);
                return true;
            }
        }
        return false;
    }

    /** Moves the dragged block to follow the cursor (clamped on release). */
    public void mouseMove(double mx, double my) {
        if (dragging == null) return;
        dragging.x = (int) Math.round(mx) - dragOffX;
        dragging.y = (int) Math.round(my) - dragOffY;
        dirty = true;
    }

    /** Ends the drag and persists the final position. */
    public void mouseUp(int guiW, int guiH) {
        if (dragging == null) return;
        HudLayout.setPos(dragging.id, dragging.x, dragging.y, guiW, guiH, dragging.w, dragging.h);
        // Re-read the clamped stored position so the block snaps inside bounds.
        int[] pos = HudLayout.pos(dragging.id, guiW, guiH, dragging.w, dragging.h);
        dragging.x = pos[0];
        dragging.y = pos[1];
        dragging = null;
    }

    /** Resets every overlay to its default anchor and rebuilds positions. */
    public void resetAll(Minecraft minecraft, Font font, int guiW, int guiH) {
        HudLayout.resetAll();
        rebuild(minecraft, font, guiW, guiH);
    }

    public boolean isDragging() {
        return dragging != null;
    }

    public EditBlock dragging() {
        return dragging;
    }
}
