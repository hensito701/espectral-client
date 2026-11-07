package es.spectral.menu.geoip;

/**
 * Per-entry hit rectangles for the extras drawn on the multiplayer server row
 * (ping number and country flag).
 *
 * <p>One instance lives on each {@code OnlineServerEntry} (a {@code @Unique}
 * field in the lane mixin) and is reused every frame: {@link #reset()} clears
 * the frame's slots, {@link #place(int, int, int, int, int)} records the
 * rectangles the renderer actually drew, and {@link #hit(int, int)} answers
 * which one the mouse is over. No allocation happens per frame — the three
 * slots are fixed int arrays.
 *
 * <p>The Ruta Espectral toggle is not a row slot any more: it is a button
 * column beside the server list, so it has no entry here.
 */
public final class RowExtrasLayout {

    /** Right-aligned ping text ("38 ms"). */
    public static final int PING = 0;
    /** 16x11 country flag. */
    public static final int FLAG = 1;
    /** Free for later use (e.g. a sponge/whitelist badge). */
    public static final int SPARE = 2;

    private static final int SLOTS = 3;

    private final int[] slotX = new int[SLOTS];
    private final int[] slotY = new int[SLOTS];
    private final int[] slotW = new int[SLOTS];
    private final int[] slotH = new int[SLOTS];

    /** Clears every slot; call once at the top of the row's render pass. */
    public void reset() {
        for (int i = 0; i < SLOTS; i++) {
            slotW[i] = 0;
            slotH[i] = 0;
        }
    }

    /** Records the rectangle drawn for {@code slot} this frame. */
    public void place(int slot, int x, int y, int w, int h) {
        if (slot < 0 || slot >= SLOTS) {
            return;
        }
        slotX[slot] = x;
        slotY[slot] = y;
        slotW[slot] = w;
        slotH[slot] = h;
    }

    /** Whether {@code slot} was drawn this frame. */
    public boolean has(int slot) {
        return slot >= 0 && slot < SLOTS && slotW[slot] > 0 && slotH[slot] > 0;
    }

    /** Left edge of {@code slot} (meaningful only when {@link #has} is true). */
    public int x(int slot) {
        return slotX[slot];
    }

    /** Top edge of {@code slot}. */
    public int y(int slot) {
        return slotY[slot];
    }

    public int width(int slot) {
        return slotW[slot];
    }

    public int height(int slot) {
        return slotH[slot];
    }

    /**
     * Slot containing the given GUI-space point, or {@code -1}. Slots are
     * tested in draw order (right to left), so an overlap resolves to the
     * topmost one.
     */
    public int hit(int mouseX, int mouseY) {
        for (int slot = 0; slot < SLOTS; slot++) {
            if (!has(slot)) {
                continue;
            }
            if (mouseX >= slotX[slot] && mouseX < slotX[slot] + slotW[slot]
                    && mouseY >= slotY[slot] && mouseY < slotY[slot] + slotH[slot]) {
                return slot;
            }
        }
        return -1;
    }
}
