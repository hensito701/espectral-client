package es.spectral.menu.ruta;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiMultiplayer;

import es.spectral.menu.Lang;
import es.spectral.menu.mixin.feature.GuiMultiplayerRutaAccessor;
import es.spectral.menu.serverlist.RowExtras;
import es.spectral.menu.ui.Gfx;
import es.spectral.menu.ui.SuiteTheme;

/**
 * The Ruta Espectral icon in the multiplayer row (slot
 * {@link es.spectral.menu.serverlist.RowExtrasLayout#SLOT_ROUTE}).
 *
 * <p>Installed once from {@code EspectralMenu.init}; from then on the row mixin
 * already dispatches paint, tooltip and click here, so the feature needs no
 * mixin of its own. The icon is drawn only where it can act: an eligible server
 * (never {@code *.uhce.top}/{@code *.espectral.es}, LAN, loopback or private
 * addresses) and a grant that carries {@code mvp} for the signed-in account.
 *
 * <p>Click toggles the per-server setting and re-pings the list, so the number,
 * the flag and the status text immediately reflect the route. There is no
 * silent direct fallback: with the toggle on, a failed route fails the
 * connection with the localized reason.
 */
public final class RutaIcon implements RowExtras.RouteIcon {

    /** Icon width in GUI pixels. */
    public static final int WIDTH = 16;
    /** Icon height (matches the flag band). */
    public static final int HEIGHT = RowExtras.SLOT_H;

    private static final RutaIcon INSTANCE = new RutaIcon();

    private RutaIcon() {}

    /** Installs the icon into the row renderer (idempotent). */
    public static void install() {
        RowExtras.ROUTE_ICON = INSTANCE;
    }

    @Override
    public int width(String serverIp) {
        if (!RutaProtocol.isEligible(serverIp)) return 0;
        if (!Ruta.isAvailable() && !Ruta.isRouted(serverIp)) return 0;
        return WIDTH;
    }

    @Override
    public void paint(String serverIp, int x, int y, int w, int h) {
        boolean routed = Ruta.isRouted(serverIp);
        boolean broken = routed && Ruta.recentFailure(serverIp) != null;
        String failure = Ruta.recentFailure(serverIp);

        int border = broken ? 0xFFE5484D
                : routed ? accent()
                : 0xFF6B7280;
        int body = routed ? 0xC0181B21 : 0xA014171C;
        Gfx.fill(x, y, x + w, y + h, body);
        Gfx.outline(x, y, w, h, border);

        // The glyph: two stacked arrows (out through the relay, back in).
        int ink = broken ? 0xFFFF8A92 : routed ? accent() : 0xFF9AA0A8;
        int midY = y + h / 2;
        Gfx.fill(x + 3, midY - 3, x + 12, midY - 2, ink);
        Gfx.fill(x + 10, midY - 4, x + 12, midY - 1, ink);
        Gfx.fill(x + 4, midY + 2, x + 13, midY + 3, ink);
        Gfx.fill(x + 4, midY + 1, x + 6, midY + 4, ink);
        if (failure != null) {
            // A dot in the corner: this row's route is failing right now.
            Gfx.fill(x + w - 4, y + 1, x + w - 1, y + 4, 0xFFE5484D);
        }
    }

    @Override
    public String tooltip(String serverIp) {
        if (!RutaProtocol.isEligible(serverIp)) return null;
        if (!Ruta.isAvailable()) {
            // The grant expired (or belongs to another account) after the row
            // was switched on: say why instead of pretending it is off.
            return RutaMessages.status(Ruta.decide(serverIp).code) + "\n"
                    + Lang.tr("espectral.ruta.tooltip.blocked");
        }
        String failure = Ruta.recentFailure(serverIp);
        if (failure != null) {
            return RutaMessages.status(failure) + "\n" + Lang.tr("espectral.ruta.tooltip.blocked");
        }
        return Lang.tr(Ruta.isRouted(serverIp)
                ? "espectral.ruta.tooltip.on" : "espectral.ruta.tooltip.off");
    }

    @Override
    public boolean click(String serverIp, int mouseX, int mouseY, int button) {
        if (button != 0 && button != 1) return false;
        if (!RutaProtocol.isEligible(serverIp)) return false;
        if (!Ruta.isAvailable() && !Ruta.isRouted(serverIp)) return false;
        boolean next = !Ruta.isRouted(serverIp);
        Ruta.setRouted(serverIp, next);
        Ruta.clearFailure(serverIp);
        reping();
        return true;
    }

    /** Re-runs the list ping so the row reflects the new route immediately. */
    private static void reping() {
        Minecraft mc = Minecraft.getMinecraft();
        if (mc == null || !(mc.currentScreen instanceof GuiMultiplayer)) return;
        ((GuiMultiplayerRutaAccessor) mc.currentScreen).espectral$refreshServerList();
    }

    private static int accent() {
        return SuiteTheme.accent() | 0xFF000000;
    }
}
