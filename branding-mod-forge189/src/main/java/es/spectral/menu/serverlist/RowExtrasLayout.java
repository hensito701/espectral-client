package es.spectral.menu.serverlist;

/**
 * Where one multiplayer row put its extra widgets.
 *
 * <p>One instance per list entry, reused every frame: {@link #reset()} then
 * {@link #place(int, int, int, int, int)} for each drawn slot, and
 * {@link #hit(int, int)} answers what the cursor is over. Kept free of any
 * Minecraft type so the geometry can be unit tested on plain ints, and sized
 * once (array of {@link #SLOT_COUNT}) so drawing a row allocates nothing.
 *
 * <p>Slot 2 ({@link #SLOT_ROUTE}) is reserved: nothing places it today, and the
 * later Ruta Espectral route icon only needs to call {@code place(SLOT_ROUTE, …)}
 * — the hit test and the tooltip plumbing already cover it.
 */
public final class RowExtrasLayout {

    /** Ping number "38 ms". */
    public static final int SLOT_PING = 0;
    /** 16×11 country flag. */
    public static final int SLOT_FLAG = 1;
    /** Reserved for the Ruta Espectral route icon. */
    public static final int SLOT_ROUTE = 2;
    /** Spare slot, unused by the design. */
    public static final int SLOT_EXTRA = 3;
    /** Size of the fixed slot table. */
    public static final int SLOT_COUNT = 4;

    private final int[] x = new int[SLOT_COUNT];
    private final int[] y = new int[SLOT_COUNT];
    private final int[] w = new int[SLOT_COUNT];
    private final int[] h = new int[SLOT_COUNT];
    private final boolean[] placed = new boolean[SLOT_COUNT];

    /** Forget every slot; call once at the start of each row draw. */
    public void reset() {
        for (int i = 0; i < SLOT_COUNT; i++) {
            placed[i] = false;
        }
    }

    /** Record the rectangle the given slot occupies this frame. */
    public void place(int slot, int sx, int sy, int sw, int sh) {
        if (slot < 0 || slot >= SLOT_COUNT) return;
        x[slot] = sx;
        y[slot] = sy;
        w[slot] = sw;
        h[slot] = sh;
        placed[slot] = sw > 0 && sh > 0;
    }

    public boolean isPlaced(int slot) {
        return slot >= 0 && slot < SLOT_COUNT && placed[slot];
    }

    public int slotX(int slot) {
        return slot >= 0 && slot < SLOT_COUNT ? x[slot] : 0;
    }

    public int slotY(int slot) {
        return slot >= 0 && slot < SLOT_COUNT ? y[slot] : 0;
    }

    public int slotW(int slot) {
        return slot >= 0 && slot < SLOT_COUNT ? w[slot] : 0;
    }

    public int slotH(int slot) {
        return slot >= 0 && slot < SLOT_COUNT ? h[slot] : 0;
    }

    /** The placed slot under the cursor, or {@code -1}. */
    public int hit(int mouseX, int mouseY) {
        for (int i = 0; i < SLOT_COUNT; i++) {
            if (!placed[i]) continue;
            if (mouseX >= x[i] && mouseX < x[i] + w[i] && mouseY >= y[i] && mouseY < y[i] + h[i]) {
                return i;
            }
        }
        return -1;
    }
}
