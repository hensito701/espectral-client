package es.spectral.menu.geoip;

import java.util.Locale;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

import es.spectral.menu.Compat;
import net.minecraft.ChatFormatting;
import net.minecraft.client.gui.Font;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;

/**
 * Extras drawn on a multiplayer server row: the ping in milliseconds, the
 * country flag, and the reserved slot for the Ruta Espectral route icon.
 *
 * <p>Everything is drawn through {@link Compat}'s {@code ui*} primitives with an
 * opaque graphics object, so this shared class never names a version-specific
 * renderer ({@code GuiGraphics} on 1.21.11, {@code GuiGraphicsExtractor} on
 * 26.2+) and the lane mixins stay a few lines long.
 *
 * <h2>Layout (right to left, from the row's content box)</h2>
 * <pre>
 * statusIconX = contentRight - 15             // vanilla bars start here
 * statusX     = statusIconX - statusWidth - 5 // vanilla player-count text
 * pingRight   = statusX - 4                   // our ping text ends here
 * pingLeft    = pingRight - pingWidth         // drawn at y = contentY + 1
 * flagX       = pingLeft - 3 - 16             // 16x11 at y = contentY - 1
 * routeX      = (flag or ping) - 3 - routeWidth  // slot 2, y = contentY - 1
 * </pre>
 * The flag sits 1 px above the text baseline row, which centres its 11 px
 * height on the 9 px glyph line. Nothing is drawn when the numbers would
 * collide with the server icon (rows narrower than the screen).
 *
 * <h2>Strings</h2>
 * {@value #PING_KEY} is the "{@code %s ms}" template,
 * {@value #COUNTRY_KEY_PREFIX}<i>cc</i> the localized country name (lowercase
 * ISO-3166 alpha-2), {@value #ATTRIBUTION_KEY} the DB-IP credit its CC BY 4.0
 * licence requires and {@value #ATTRIBUTION_URL_KEY} the link line.
 */
public final class RowExtras {

    /** Ping text template; {@code %s} is the millisecond value. */
    public static final String PING_KEY = "espectral.server.ping";
    /** Country-name key prefix; the lowercase ISO-3166 alpha-2 code follows. */
    public static final String COUNTRY_KEY_PREFIX = "espectral.country.";
    /** CC BY 4.0 credit DB-IP requires wherever its data is shown. */
    public static final String ATTRIBUTION_KEY = "espectral.geoip.attribution";
    /** Attribution link line shown under the credit. */
    public static final String ATTRIBUTION_URL_KEY = "espectral.geoip.attribution.url";
    /** Attribution link target (DB-IP, CC BY 4.0). */
    public static final String ATTRIBUTION_URL = "https://db-ip.com";

    /** Bundled flag sprite size (famfamfam is 16x11). */
    public static final int FLAG_W = 16;
    public static final int FLAG_H = 11;
    /** Extras slot height: every slot is laid out at the flag's height. */
    public static final int SLOT_H = FLAG_H;

    /** Gap between the ping text and whatever hangs to its left. */
    private static final int GAP = 3;
    /** Gap between the vanilla player-count text and our ping number. */
    private static final int PING_GAP = 4;
    /** Row content starts after the 32 px server icon. */
    private static final int ICON_BAND = 32;
    /** Widest route icon the layout will place. */
    private static final int ROUTE_MAX_W = 24;

    private static final int GREEN = 0xFF55FF55;
    private static final int YELLOW = 0xFFFFFF55;
    private static final int RED = 0xFFFF5555;

    /**
     * ISO codes DB-IP can emit that famfamfam has no icon for, so no sprite is
     * bundled and the row shows none (drawing a missing sprite would paint the
     * purple/black checkerboard).
     */
    private static final Set<String> NO_FLAG = Set.of(
            "AQ", "BL", "BQ", "CW", "GG", "IM", "JE", "MF", "SS", "SX", "XK");

    /** server ip string → country code; {@code ""} means "looked up, none". */
    private static final ConcurrentHashMap<String, String> COUNTRY = new ConcurrentHashMap<>();
    private static final int COUNTRY_CACHE_MAX = 2048;

    /**
     * Hook for the Ruta Espectral route icon (slot
     * {@link RowExtrasLayout#ROUTE}). {@code null} — the default — draws
     * nothing and leaves the row exactly as shipped. The follow-up installs its
     * implementation here once; the row mixins already dispatch paint, tooltip
     * and click into it, so no further mixin is needed.
     */
    public static volatile RouteIcon ROUTE_ICON;

    /**
     * What the route icon needs from a row. {@code hostKey} is the normalized
     * {@code "<host>:<port>"} of the entry, so an implementation can key its own
     * per-server state on it.
     */
    public interface RouteIcon {

        /**
         * Width in GUI pixels for this server (clamped to 0..24), or 0 to hide
         * the slot for this row.
         */
        int width(ServerData data);

        /** Draws the icon inside the given rectangle (height is {@link #SLOT_H}). */
        void paint(Object gfx, ServerData data, int x, int y, int w, int h);

        /** Tooltip while the pointer is over the icon, or {@code null} for none. */
        Component tooltip(ServerData data);

        /**
         * Click on the icon. Return {@code true} when handled — the row then
         * neither joins nor swaps.
         */
        boolean click(ServerData data, double mouseX, double mouseY, int button);
    }

    private RowExtras() {}

    // ── Rendering ──────────────────────────────────────────────────────────

    /**
     * Draws the ping number and flag for one row, places the route icon and
     * registers every rectangle in {@code layout}. Called from the row mixin at
     * the tail of the vanilla render, so it paints above the row's own text.
     *
     * @param contentX     row content box left ({@code Entry.getContentX()})
     * @param contentY     row content box top ({@code Entry.getContentY()})
     * @param contentRight row content box right ({@code Entry.getContentRight()})
     */
    public static void render(Object gfx, Font font, RowExtrasLayout layout, ServerData data,
            int contentX, int contentY, int contentRight, int mouseX, int mouseY) {
        if (gfx == null || font == null || layout == null || data == null) {
            return;
        }
        layout.reset();

        // Where vanilla puts the player count, so we can stop right of it.
        // The incompatible state draws data.version in red; styling never
        // changes the measured width, so no copy is needed here.
        Component status = data.state() == ServerData.State.INCOMPATIBLE ? data.version : data.status;
        int statusWidth = status == null ? 0 : font.width(status);
        int statusX = contentRight - 15 - statusWidth - 5;
        int pingRight = statusX - PING_GAP;

        // ── Ping number (right-aligned on pingRight) ───────────────────────
        long ping = pingMillis(data);
        int anchorLeft = pingRight;
        if (ping >= 0L) {
            Component label = Component.translatable(PING_KEY, ping);
            int w = font.width(label);
            int pingLeft = pingRight - w;
            if (pingLeft > contentX + ICON_BAND) {
                Compat.uiText(gfx, font, label, pingLeft, contentY + 1, pingColor(ping), true);
                layout.place(RowExtrasLayout.PING, pingLeft, contentY, w, 9);
                anchorLeft = pingLeft;
            }
        }

        // ── Country flag (16x11, 3 px left of the ping text) ───────────────
        // Only once the row has a real number: while pinging/unreachable the
        // row would otherwise pair a flag with an empty status band.
        String country = ping >= 0L ? countryFor(data) : null;
        if (country != null) {
            Identifier sprite = flagSprite(country);
            int flagX = anchorLeft - GAP - FLAG_W;
            if (sprite != null && flagX > contentX + ICON_BAND) {
                Compat.uiSprite(gfx, sprite, flagX, contentY - 1, FLAG_W, FLAG_H, 0xFFFFFFFF);
                layout.place(RowExtrasLayout.FLAG, flagX, contentY - 1, FLAG_W, FLAG_H);
                anchorLeft = flagX;
            }
        }

        // ── Route icon (reserved slot 2, driven by ROUTE_ICON) ─────────────
        RouteIcon route = ROUTE_ICON;
        if (route != null) {
            int w = clamp(route.width(data), 0, ROUTE_MAX_W);
            int routeX = anchorLeft - GAP - w;
            if (w > 0 && routeX > contentX + ICON_BAND) {
                route.paint(gfx, data, routeX, contentY - 1, w, SLOT_H);
                layout.place(RowExtrasLayout.ROUTE, routeX, contentY - 1, w, SLOT_H);
            }
        }

        // ── Tooltips (explicit hit tests, like the vanilla row) ────────────
        int slot = layout.hit(mouseX, mouseY);
        if (slot == RowExtrasLayout.FLAG && country != null) {
            Compat.uiTooltip(gfx, font, flagTooltip(country), mouseX, mouseY);
        } else if (slot == RowExtrasLayout.ROUTE && route != null) {
            Component tip = route.tooltip(data);
            if (tip != null) {
                Compat.uiTooltip(gfx, font, tip, mouseX, mouseY);
            }
        }
    }

    /**
     * Dispatches a click to whatever occupies the extras slots. Returns
     * {@code true} when consumed — the caller then skips vanilla's join/swap.
     */
    public static boolean click(ServerData data, RowExtrasLayout layout,
            double mouseX, double mouseY, int button) {
        if (layout == null || data == null) {
            return false;
        }
        if (layout.hit((int) mouseX, (int) mouseY) != RowExtrasLayout.ROUTE) {
            return false;
        }
        RouteIcon route = ROUTE_ICON;
        return route != null && route.click(data, mouseX, mouseY, button);
    }

    // ── Pieces ─────────────────────────────────────────────────────────────

    /**
     * Ping in milliseconds, or {@code -1} while the state carries no meaningful
     * number (initial, pinging, unreachable).
     */
    private static long pingMillis(ServerData data) {
        ServerData.State state = data.state();
        if (state != ServerData.State.SUCCESSFUL && state != ServerData.State.INCOMPATIBLE) {
            return -1L;
        }
        return data.ping < 0L ? -1L : data.ping;
    }

    /** Green under 80 ms, yellow under 150 ms, red from there up. */
    public static int pingColor(long ping) {
        if (ping < 0L) {
            return RED;
        }
        if (ping < 80L) {
            return GREEN;
        }
        return ping < 150L ? YELLOW : RED;
    }

    /** Sprite id for a country's flag, or {@code null} when none is bundled. */
    public static Identifier flagSprite(String country) {
        if (country == null || country.length() != 2) {
            return null;
        }
        String upper = country.toUpperCase(Locale.ROOT);
        if (NO_FLAG.contains(upper)) {
            return null;
        }
        return Identifier.fromNamespaceAndPath("espectral-menu",
                "flags/" + country.toLowerCase(Locale.ROOT));
    }

    /** Localized country name plus the DB-IP credit the licence requires. */
    private static Component flagTooltip(String country) {
        return Component.translatable(COUNTRY_KEY_PREFIX + country.toLowerCase(Locale.ROOT), country)
                .append("\n")
                .append(Component.translatable(ATTRIBUTION_KEY).withStyle(ChatFormatting.DARK_GRAY))
                .append("\n")
                .append(Component.literal(ATTRIBUTION_URL).withStyle(ChatFormatting.DARK_GRAY));
    }

    /**
     * Country code for a server list entry, taken from the address vanilla
     * already resolved for its status ping — this method never performs a DNS
     * query and never blocks the render thread.
     */
    public static String countryFor(ServerData data) {
        if (data == null || data.isLan()) {
            return null;
        }
        GeoIp.start(); // idempotent; a volatile read once loaded
        if (!GeoIp.ready()) {
            // No table (never configured, still loading or permanently off):
            // nothing to show, and nothing to memoize.
            return null;
        }
        String serverIp = data.ip;
        if (serverIp == null || serverIp.isEmpty()) {
            return null;
        }
        String cached = COUNTRY.get(serverIp);
        if (cached != null) {
            return cached.isEmpty() ? null : cached;
        }
        java.net.InetAddress address = ServerAddresses.forServerIp(serverIp);
        if (address == null) {
            return null; // not resolved yet — retry next frame
        }
        String country = GeoIp.country(address);
        if (COUNTRY.size() > COUNTRY_CACHE_MAX) {
            COUNTRY.clear();
        }
        COUNTRY.put(serverIp, country == null ? "" : country);
        return country;
    }

    private static int clamp(int value, int min, int max) {
        return value < min ? min : Math.min(value, max);
    }
}
