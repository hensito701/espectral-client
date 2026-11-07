package es.spectral.menu.ruta;

import net.minecraft.client.Minecraft;
import net.minecraft.client.audio.PositionedSoundRecord;
import net.minecraft.client.gui.GuiMultiplayer;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.util.ResourceLocation;

import es.spectral.menu.Lang;
import es.spectral.menu.mixin.feature.GuiMultiplayerRutaAccessor;
import es.spectral.menu.serverlist.RowExtras;
import es.spectral.menu.ui.Gfx;
import es.spectral.menu.ui.SuiteTheme;

/**
 * Ruta Espectral per-server toggle for the 1.8.9 multiplayer list. It is a 20×20
 * button in a column to the right of the server rows, not an icon inside the row.
 *
 * <p>Why a column: vanilla dispatches a click to a row only inside the row's own
 * x-range, and the button sits outside it, so a row never receives the click.
 * {@code GuiMultiplayerRouteColumnMixin} takes the click on the screen instead, and
 * {@code ServerListEntryNormalMixin} paints the button from {@code drawEntry} so it
 * stays on the geometry of the row it belongs to.
 */
public final class RutaIcon {

    /** Button edge length in GUI pixels. Matches the Fabric column. */
    public static final int SIZE = 20;
    /** Gap between the list's right edge and the button column. */
    private static final int COLUMN_GAP = 22;
    /** Top inset that centres the button in the 32 px row band. */
    private static final int BAND_INSET = 6;

    private RutaIcon() {}

    /**
     * Draws the button for one row and returns the tooltip while the pointer is over
     * it, or {@code null}. Draws nothing for a row that cannot route.
     */
    public static String paintColumn(ServerData server, int rowX, int rowY, int listWidth,
            int mouseX, int mouseY) {
        if (!offersButton(server)) return null;
        String ip = server.serverIP;
        int x = columnX(rowX, listWidth);
        int y = buttonY(rowY);
        boolean routed = Ruta.isRouted(ip);
        boolean broken = routed && Ruta.recentFailure(ip) != null;
        int accent = SuiteTheme.accent();
        int border = broken ? 0xFFE5484D : routed ? accent : 0xFF6B7280;
        int body = routed ? 0xC0181B21 : 0xA014171C;
        int ink = broken ? 0xFFFF8A92 : routed ? accent : 0xFF9AA0A8;

        Gfx.fill(x, y, x + SIZE, y + SIZE, border);
        Gfx.fill(x + 1, y + 1, x + SIZE - 1, y + SIZE - 1, body);
        // Route mark (same as Fabric): start node bottom-left, a stepped path,
        // end node top-right, inside a 12 px square.
        int gx = x + 4;
        int gy = y + 4;
        Gfx.fill(gx, gy + 9, gx + 3, gy + 12, ink);
        Gfx.fill(gx + 3, gy + 10, gx + 6, gy + 11, ink);
        Gfx.fill(gx + 5, gy + 1, gx + 6, gy + 11, ink);
        Gfx.fill(gx + 6, gy + 1, gx + 9, gy + 2, ink);
        Gfx.fill(gx + 9, gy, gx + 12, gy + 3, ink);
        if (broken) {
            Gfx.fill(x + 16, y + 1, x + 19, y + 4, 0xFFE5484D);
        }

        if (!onButton(rowX, rowY, listWidth, mouseX, mouseY)) return null;
        return tooltip(ip);
    }

    /** True when the pointer is over the button of the row whose band starts at {@code rowY}. */
    public static boolean onButton(int rowX, int rowY, int listWidth, int mouseX, int mouseY) {
        int x = columnX(rowX, listWidth);
        int y = buttonY(rowY);
        return mouseX >= x && mouseX < x + SIZE && mouseY >= y && mouseY < y + SIZE;
    }

    /**
     * Handles a click on the button of {@code server}. Returns {@code true} when the
     * click was consumed, so the caller must not join the server.
     */
    public static boolean clickColumn(ServerData server, int button) {
        if (button != 0 && button != 1) return false;
        if (!offersButton(server)) return false;
        String ip = server.serverIP;
        Ruta.setRouted(ip, !Ruta.isRouted(ip));
        Ruta.clearFailure(ip);
        Minecraft.getMinecraft().getSoundHandler()
                .playSound(PositionedSoundRecord.create(new ResourceLocation("gui.button.press")));
        reping();
        return true;
    }

    /**
     * Only online 1.8.9 servers the Ruta protocol can route show the button. It
     * appears while the grant allows routing, or while the server is already routed so
     * the user can switch it off.
     */
    private static boolean offersButton(ServerData server) {
        if (server == null || server.isOnLAN() || server.version != RowExtras.PROTOCOL) return false;
        if (!RutaProtocol.isEligible(server.serverIP)) return false;
        return Ruta.isAvailable() || Ruta.isRouted(server.serverIP);
    }

    private static int columnX(int rowX, int listWidth) {
        return rowX + listWidth + COLUMN_GAP;
    }

    private static int buttonY(int rowY) {
        return rowY + BAND_INSET;
    }

    private static String tooltip(String serverIp) {
        if (!Ruta.isAvailable()) {
            // The grant expired (or belongs to another account) after the row was
            // switched on: say why instead of pretending it is off.
            return RutaMessages.status(Ruta.decide(serverIp).code) + "\n"
                    + Lang.tr("espectral.ruta.tooltip.blocked");
        }
        String failure = Ruta.recentFailure(serverIp);
        if (failure != null) {
            return RutaMessages.status(failure) + "\n" + Lang.tr("espectral.ruta.tooltip.blocked");
        }
        String state = Lang.tr(Ruta.isRouted(serverIp)
                ? "espectral.ruta.tooltip.on" : "espectral.ruta.tooltip.off");
        // GuiMultiplayer splits the hover text on '\n' but never wraps, so the
        // long note is wrapped here to the same 220 px the Fabric tooltip uses.
        StringBuilder out = new StringBuilder(state);
        for (String line : Gfx.font().listFormattedStringToWidth(Lang.tr("espectral.ruta.tooltip.note"), 220)) {
            out.append('\n').append("\u00a78").append(line);
        }
        return out.toString();
    }

    /** Re-applies the server list so the row picks up the new toggle state. */
    private static void reping() {
        Minecraft mc = Minecraft.getMinecraft();
        if (mc.currentScreen instanceof GuiMultiplayer) {
            ((GuiMultiplayerRutaAccessor) mc.currentScreen).espectral$refreshServerList();
        }
    }
}
