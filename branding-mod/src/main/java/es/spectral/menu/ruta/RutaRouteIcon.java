package es.spectral.menu.ruta;

import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.screens.multiplayer.ServerSelectionList;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.resources.sounds.SimpleSoundInstance;
import net.minecraft.network.chat.Component;
import net.minecraft.sounds.SoundEvents;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;

/**
 * The Ruta Espectral toggle on the multiplayer server list: a 20x20 button in a
 * column just right of the list, one per eligible online row, centred on that
 * row's vertical span.
 *
 * <p>Why a column and not a glyph in the row: the old glyph shared the row's
 * ping and flag band, so its hit area was a few pixels and it read as part of
 * the server's status. A fixed button beside the list is a full-size target
 * that does not move with the row's text.
 *
 * <p>Two mixins call in: {@code ServerListRutaColumnMixin} paints from the
 * list's render tail, and {@code ServerListRutaClickMixin} routes clicks through
 * {@link #clickColumn}. The geometry lives here once, so paint and hit-test
 * cannot drift apart. The list is typed as {@link ServerSelectionList}, which has
 * the same name on 1.21.11 and 26.2.
 *
 * <p>Clicking flips the stored per-server switch and marks the row
 * {@code INITIAL}, which makes vanilla re-ping it, so the number shown afterwards
 * is the one through the route.
 */
public final class RutaRouteIcon {

    /** Button edge in GUI pixels. */
    private static final int SIZE = 20;
    /**
     * Distance from the row's right edge to the button's left edge. It clears
     * vanilla's scrollbar, which sits just outside the row edge.
     */
    private static final int COLUMN_OFFSET = 18;
    /** Glyph inset from the button edge; the glyph is 12 px square inside. */
    private static final int INSET = 4;
    private static final int GLYPH = 12;

    private static final int FILL = 0xAA000000;
    private static final int FILL_HOVER = 0xCC000000;
    private static final int EDGE = 0xFF6A6A72;
    private static final int EDGE_HOVER = 0xFFB0B0B8;

    private static final int ON = 0xFFFFC24B;
    private static final int OFF = 0xFF8A8A92;
    private static final int BLOCKED = 0xFFFF5555;

    private RutaRouteIcon() {}

    /**
     * Paints the button column for one list render. Called from the list's
     * render tail, so it lands above vanilla's rows and scrollbar. Buttons are
     * clipped to the list's visible band, and the hovered button's tooltip is
     * drawn last, outside the clip.
     */
    public static void paintColumn(Object gfx, Font font, ServerSelectionList list,
            int mouseX, int mouseY) {
        if (gfx == null || font == null || list == null || !columnFits(list)) {
            return;
        }
        boolean available = Ruta.isAvailable();
        int hovered = buttonIndexAt(list, mouseX, mouseY);
        int x = colX(list);
        int top = list.getY();
        int bottom = top + list.getHeight();
        Component tip = null;

        Compat.uiScissor(gfx, list.getX(), top, list.getX() + list.getWidth(), bottom);
        int count = list.children().size();
        for (int i = 0; i < count; i++) {
            int y = buttonY(list, i);
            if (y + SIZE <= top || y >= bottom) {
                continue;
            }
            ServerData data = onlineData(list.children().get(i));
            String key = data == null ? null : Ruta.key(data.ip);
            if (key == null || !Ruta.isEligible(key)) {
                continue;
            }
            boolean routed = ClientConfig.getInstance().isRutaRouted(key);
            if (!routed && !available) {
                continue;
            }
            boolean hover = i == hovered;
            paintButton(gfx, x, y, routed, available, hover);
            if (hover) {
                tip = tooltip(routed, available);
            }
        }
        Compat.uiScissorEnd(gfx);
        if (tip != null) {
            Compat.uiTooltip(gfx, font, tip, mouseX, mouseY);
        }
    }

    /**
     * Routes a click on the button column. Returns {@code true} when the click
     * toggled a route, so the caller cancels vanilla's join or select for that
     * row. Only the left and right buttons toggle, and only a button that is
     * actually drawn takes the click; anything else falls through to vanilla.
     */
    public static boolean clickColumn(ServerSelectionList list, double mouseX, double mouseY,
            int button) {
        if (list == null || (button != 0 && button != 1)) {
            return false;
        }
        int i = buttonIndexAt(list, mouseX, mouseY);
        if (i < 0) {
            return false;
        }
        ServerData data = onlineData(list.children().get(i));
        String key = data == null ? null : Ruta.key(data.ip);
        if (key == null || !Ruta.isEligible(key)) {
            return false;
        }
        boolean routed = ClientConfig.getInstance().isRutaRouted(key);
        if (!routed && !Ruta.isAvailable()) {
            return false;
        }
        Ruta.setRouted(data.ip, !routed);
        // Vanilla re-pings a row whose state is INITIAL; this makes the shown
        // ping and channel reflect the new route immediately.
        data.setState(ServerData.State.INITIAL);
        Minecraft.getInstance().getSoundManager()
                .play(SimpleSoundInstance.forUI(SoundEvents.UI_BUTTON_CLICK, 1.0F));
        return true;
    }

    /** Left edge of the button column: just right of the row's right edge. */
    private static int colX(ServerSelectionList list) {
        return list.getRowRight() + COLUMN_OFFSET;
    }

    /** Whether the button fits inside the list; a narrow window drops the column. */
    private static boolean columnFits(ServerSelectionList list) {
        return colX(list) + SIZE <= list.getX() + list.getWidth();
    }

    /** Top edge of row {@code i}'s button, centred on the row's vertical span. */
    private static int buttonY(ServerSelectionList list, int i) {
        return (list.getRowTop(i) + list.getRowBottom(i)) / 2 - SIZE / 2;
    }

    /**
     * Index of the row whose button contains the point, or {@code -1}. Purely
     * geometric: eligibility is checked by the callers, so a hidden button is
     * still hit-tested as its slot and simply has no effect.
     */
    private static int buttonIndexAt(ServerSelectionList list, double mouseX, double mouseY) {
        if (!columnFits(list)) {
            return -1;
        }
        int x = colX(list);
        if (mouseX < x || mouseX >= x + SIZE) {
            return -1;
        }
        int top = list.getY();
        if (mouseY < top || mouseY >= top + list.getHeight()) {
            return -1;
        }
        int count = list.children().size();
        for (int i = 0; i < count; i++) {
            int y = buttonY(list, i);
            if (mouseY >= y && mouseY < y + SIZE) {
                return i;
            }
        }
        return -1;
    }

    /** The server data behind an online row, or {@code null} for any other row kind. */
    private static ServerData onlineData(Object entry) {
        return entry instanceof ServerSelectionList.OnlineServerEntry row
                ? row.getServerData()
                : null;
    }

    private static void paintButton(Object gfx, int x, int y, boolean routed, boolean available,
            boolean hover) {
        int accent = routed ? (available ? ON : BLOCKED) : OFF;
        Compat.uiFill(gfx, x, y, x + SIZE, y + SIZE, hover ? FILL_HOVER : FILL);
        // A routed button wears its state colour as the frame, so ON reads at a glance.
        Compat.uiOutline(gfx, x, y, SIZE, SIZE, routed ? accent : (hover ? EDGE_HOVER : EDGE));
        int gx = x + INSET;
        int gy = y + INSET;
        // Route mark: a start node bottom-left, a stepped path, an end node top-right.
        Compat.uiFill(gfx, gx, gy + GLYPH - 3, gx + 3, gy + GLYPH, accent);
        Compat.uiFill(gfx, gx + 3, gy + GLYPH - 2, gx + 6, gy + GLYPH - 1, accent);
        Compat.uiFill(gfx, gx + 5, gy + 1, gx + 6, gy + GLYPH - 1, accent);
        Compat.uiFill(gfx, gx + 6, gy + 1, gx + GLYPH - 3, gy + 2, accent);
        Compat.uiFill(gfx, gx + GLYPH - 3, gy, gx + GLYPH, gy + 3, accent);
    }

    /** Tooltip for the button's state, with the shared note underneath. */
    private static Component tooltip(boolean routed, boolean available) {
        String state = routed ? (available ? "on" : "blocked") : "off";
        return Component.translatable("espectral.ruta.tooltip." + state)
                .append(Component.literal("\n"))
                .append(Component.translatable("espectral.ruta.tooltip.note")
                        .withStyle(ChatFormatting.DARK_GRAY));
    }
}
