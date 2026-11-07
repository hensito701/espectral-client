package es.spectral.menu.serverlist;

import java.io.IOException;
import java.io.InputStream;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

import net.minecraft.client.Minecraft;
import net.minecraft.util.EnumChatFormatting;
import net.minecraft.util.ResourceLocation;

import es.spectral.menu.Lang;
import es.spectral.menu.geoip.GeoIp;
import es.spectral.menu.ui.Gfx;

/**
 * Ping number and country flag for one multiplayer server row (1.8.9).
 *
 * <p>Vanilla draws the 10×8 status bars at {@code rowX + listWidth - 15} and
 * right-aligns the player count ({@code "1/100"}) so its right edge is
 * {@code rowX + listWidth - 17}; the ping number only exists as hover text.
 * This class works right-to-left from the count, so nothing steps on vanilla
 * and the layout mirrors the Fabric lanes:
 *
 * <pre>
 *   [bars 10×8] | 2 | 1/100 | 4 | 38 ms | 3 | flag 16×11
 * </pre>
 *
 * <p>Ping text is right-aligned at {@code countLeft - 4} on the name baseline
 * ({@code rowY + 1}); the flag sits at {@code rowY}, its 11 px vertically
 * centred on the 9 px line. Nothing is drawn that would overlap the 32 px server
 * icon, so narrow rows simply get fewer extras.
 *
 * <p>A row that has not been pinged yet, whose ping failed, or that is out of
 * date (vanilla already prints "Client/Server out of date!" there) draws no
 * extras. A host the table does not know gets no flag, and a flag sprite the
 * jar does not ship (famfamfam has no AQ/BL/BQ/CW/GG/IM/JE/MF/SS/SX/XK) is
 * skipped after one probe.
 *
 * <p>Geometry lives in {@link RowExtrasLayout} — one instance per list entry,
 * nothing allocated per frame.
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

    /** Bundled flag sprite size (famfamfam is 16×11). */
    public static final int FLAG_W = 16;
    public static final int FLAG_H = 11;
    /** Slot height: every extras slot is laid out at the flag's height. */
    public static final int SLOT_H = FLAG_H;

    /** Gap between the ping text and whatever hangs to its left. */
    public static final int GAP = 3;
    /** Gap between the vanilla player count and our ping number. */
    public static final int PING_GAP = 4;
    /** Row content starts after the 32 px server icon. */
    public static final int ICON_BAND = 32;

    /** Nether/End-free 1.8.9 network protocol; any other version is "out of date". */
    public static final int PROTOCOL = 47;

    public static final int COLOR_GOOD = 0xFF55FF55;
    public static final int COLOR_WARN = 0xFFFFFF55;
    public static final int COLOR_BAD = 0xFFFF5555;

    /** Codes whose sprite resolves; probed once and reused. */
    private static final ConcurrentHashMap<String, ResourceLocation> FLAGS =
            new ConcurrentHashMap<String, ResourceLocation>();
    /** Codes probed once and found absent. */
    private static final Set<String> MISSING_FLAGS = ConcurrentHashMap.newKeySet();

    private RowExtras() {}

    // ── Rendering ──────────────────────────────────────────────────────────

    /**
     * Draws the ping number and flag for one row and registers every rectangle in
     * {@code layout}. Called from the row mixin at
     * the tail of the vanilla render, so it paints above the row's own text.
     *
     * @param populationInfo vanilla's {@code ServerData.populationInfo}
     *        ({@code "1/100"}), whose left edge the ping text stops before
     * @return the tooltip to show, or {@code null}
     */
    public static String draw(RowExtrasLayout layout, long pingMillis, String populationInfo,
            String serverIp, int rowX, int rowY, int listWidth, int mouseX, int mouseY) {
        layout.reset();
        String country = null;

        int countWidth = populationInfo == null ? 0 : Gfx.font().getStringWidth(populationInfo);
        int pingRight = rowX + listWidth - 17 - countWidth - PING_GAP;
        int anchorLeft = pingRight;

        // ── Ping number (right-aligned on pingRight) ───────────────────────
        if (pingMillis >= 0L) {
            String text = pingText(pingMillis);
            int width = Gfx.font().getStringWidth(text);
            int pingLeft = pingRight - width;
            if (pingLeft > rowX + ICON_BAND) {
                Gfx.text(text, pingLeft, rowY + 1, pingColor(pingMillis), true);
                layout.place(RowExtrasLayout.SLOT_PING, pingLeft, rowY, width, SLOT_H);
                anchorLeft = pingLeft;
            }
        }

        // ── Country flag (16×11, 3 px left of the ping text) ───────────────
        // Only once the row has a real number: while pinging or unreachable the
        // row would otherwise pair a flag with an empty status band.
        if (pingMillis >= 0L) {
            country = GeoIp.getInstance().country(serverIp);
        }
        if (country != null) {
            ResourceLocation sprite = flagTexture(country);
            int flagX = anchorLeft - GAP - FLAG_W;
            if (sprite != null && flagX > rowX + ICON_BAND) {
                Gfx.sprite(sprite, flagX, rowY, FLAG_W, FLAG_H, 0xFFFFFFFF);
                layout.place(RowExtrasLayout.SLOT_FLAG, flagX, rowY, FLAG_W, FLAG_H);
            }
        }

        // ── Tooltips (explicit hit tests, like the vanilla row) ────────────
        int slot = layout.hit(mouseX, mouseY);
        if (slot == RowExtrasLayout.SLOT_FLAG && country != null) {
            return countryTooltip(country);
        }
        return null;
    }

    // ── Pieces ─────────────────────────────────────────────────────────────

    /** {@code "38 ms"} in the game's language. */
    public static String pingText(long millis) {
        String text = Lang.tr(PING_KEY, Long.toString(millis));
        return PING_KEY.equals(text) ? millis + " ms" : text;
    }

    /** Green under 80 ms, yellow under 150 ms, red from there up. */
    public static int pingColor(long millis) {
        if (millis < 0L) return COLOR_BAD;
        if (millis < 80L) return COLOR_GOOD;
        return millis < 150L ? COLOR_WARN : COLOR_BAD;
    }

    /** Localized country name, then the DB-IP credit and link (grey). */
    public static String countryTooltip(String countryCode) {
        String key = COUNTRY_KEY_PREFIX + countryCode.toLowerCase(Locale.ROOT);
        String name = Lang.tr(key);
        if (key.equals(name)) name = countryCode;
        String url = Lang.tr(ATTRIBUTION_URL_KEY);
        if (ATTRIBUTION_URL_KEY.equals(url)) url = ATTRIBUTION_URL;
        return name
                + "\n" + EnumChatFormatting.DARK_GRAY + Lang.tr(ATTRIBUTION_KEY)
                + "\n" + EnumChatFormatting.DARK_GRAY + url;
    }

    /**
     * The flag sprite for {@code countryCode}, or {@code null} when the jar has
     * none. A code is probed once: the answer is cached either way.
     */
    public static ResourceLocation flagTexture(String countryCode) {
        if (countryCode == null || countryCode.length() != 2) return null;
        String code = countryCode.toLowerCase(Locale.ROOT);
        ResourceLocation cached = FLAGS.get(code);
        if (cached != null) return cached;
        if (MISSING_FLAGS.contains(code)) return null;
        ResourceLocation texture = new ResourceLocation("espectralmenu", "textures/gui/flags/" + code + ".png");
        try {
            InputStream in = Minecraft.getMinecraft().getResourceManager().getResource(texture).getInputStream();
            in.close();
        } catch (IOException absent) {
            MISSING_FLAGS.add(code);
            return null;
        }
        FLAGS.put(code, texture);
        return texture;
    }
}
