package es.spectral.menu.ruta;

import net.minecraft.ChatFormatting;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.network.chat.Component;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;
import es.spectral.menu.geoip.RowExtras;

/**
 * The multiplayer-row toggle for Ruta Espectral: an 11 px glyph left of the
 * country flag. Dim when the server connects directly, accented when routed,
 * and hidden unless the player holds an MVP grant and the address is eligible.
 *
 * <p>Clicking it flips the stored per-server switch and marks the row
 * {@code INITIAL}, which is what makes vanilla re-ping it, so the number shown
 * afterwards is the one through the route.
 */
public final class RutaRouteIcon implements RowExtras.RouteIcon {

    /** Glyph width in GUI pixels. */
    private static final int WIDTH = 9;

    private static final int ON = 0xFFFFC24B;
    private static final int OFF = 0xFF6A6A72;
    private static final int BLOCKED = 0xFFFF5555;

    /** Installs the icon once; safe to call from the client initializer. */
    public static void install() {
        RowExtras.ROUTE_ICON = new RutaRouteIcon();
    }

    @Override
    public int width(ServerData data) {
        if (data == null) return 0;
        String key = Ruta.key(data.ip);
        if (key == null || !Ruta.isEligible(key)) return 0;
        if (!Ruta.isAvailable()) return ClientConfig.getInstance().isRutaRouted(key) ? WIDTH : 0;
        return WIDTH;
    }

    @Override
    public void paint(Object gfx, ServerData data, int x, int y, int w, int h) {
        if (gfx == null || data == null || w <= 0) return;
        String key = Ruta.key(data.ip);
        boolean routed = key != null && ClientConfig.getInstance().isRutaRouted(key);
        int accent = routed ? (Ruta.isAvailable() ? ON : BLOCKED) : OFF;
        int midY = y + h / 2;
        // A tiny "relay" mark: two stacked rails joined by a node.
        Compat.uiFill(gfx, x, midY - 3, x + w, midY - 2, accent);
        Compat.uiFill(gfx, x, midY + 2, x + w, midY + 3, accent);
        Compat.uiFill(gfx, x + w / 2 - 1, midY - 1, x + w / 2 + 1, midY + 2,
                routed ? accent : 0xFF8A8A92);
        if (routed) {
            Compat.uiFill(gfx, x, midY - 1, x + 1, midY + 2, accent);
            Compat.uiFill(gfx, x + w - 1, midY - 1, x + w, midY + 2, accent);
        }
    }

    @Override
    public Component tooltip(ServerData data) {
        if (data == null) return null;
        String key = Ruta.key(data.ip);
        boolean routed = key != null && ClientConfig.getInstance().isRutaRouted(key);
        String state = routed
                ? (Ruta.isAvailable() ? "on" : "blocked")
                : "off";
        return Component.translatable("espectral.ruta.tooltip." + state)
                .append(Component.literal("\n"))
                .append(Component.translatable("espectral.ruta.tooltip.note")
                        .withStyle(ChatFormatting.DARK_GRAY));
    }

    @Override
    public boolean click(ServerData data, double mouseX, double mouseY, int button) {
        if (data == null || button != 0 && button != 1) return false;
        String key = Ruta.key(data.ip);
        if (key == null || !Ruta.isEligible(key)) return false;
        boolean next = !ClientConfig.getInstance().isRutaRouted(key);
        Ruta.setRouted(data.ip, next);
        // Vanilla re-pings a row whose state is INITIAL; this makes the shown
        // ping/channel reflect the new route immediately.
        data.setState(ServerData.State.INITIAL);
        return true;
    }
}
