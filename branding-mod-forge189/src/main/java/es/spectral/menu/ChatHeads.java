package es.spectral.menu;

import java.util.Collection;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.Map;
import java.util.UUID;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.ChatLine;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.network.NetHandlerPlayClient;
import net.minecraft.client.network.NetworkPlayerInfo;
import net.minecraft.client.renderer.GlStateManager;

/**
 * Player heads next to chat messages (owned/live feature {@code chatheads},
 * default off). 1.8.9 chat carries no sender, so — like the Fabric mod's
 * path for system-formatted chat — the sender is resolved once per message
 * by matching an exact tab-list name in the line's sender prefix
 * ({@link ChatPrefix}: position, whole-word and broadcast rules), and the
 * first display line of that message is mapped to the sender's UUID. Only
 * that first line draws a head; continuation lines and unresolved lines
 * stay vanilla. With the feature off nothing is recorded or drawn.
 */
public final class ChatHeads {

    public static final String FEATURE_ID = "chatheads";
    public static final int HEAD_SIZE = 8;
    public static final int GUTTER = 2;
    public static final int SHIFT = HEAD_SIZE + GUTTER;
    private static final int LINE_CAP = 4096;

    private static final Map<ChatLine, UUID> LINES = Collections.synchronizedMap(new IdentityHashMap<ChatLine, UUID>());

    private ChatHeads() {}

    public static boolean enabled() {
        return ClientConfig.getInstance().isFeatureEnabled(FEATURE_ID);
    }

    /** Sender of a message from its plain text, or null (intake only, never per frame). */
    public static UUID senderForText(String text) {
        if (text == null || text.isEmpty() || !enabled()) return null;
        NetHandlerPlayClient net = Minecraft.getMinecraft().getNetHandler();
        if (net == null) return null;
        Collection<NetworkPlayerInfo> players = net.getPlayerInfoMap();
        String[] names = new String[players.size()];
        UUID[] ids = new UUID[players.size()];
        int n = 0;
        for (NetworkPlayerInfo info : players) {
            if (info == null || info.getGameProfile() == null) continue;
            String name = info.getGameProfile().getName();
            UUID id = info.getGameProfile().getId();
            if (name == null || name.isEmpty() || id == null) continue;
            names[n] = name;
            ids[n] = id;
            n++;
        }
        if (n < names.length) {
            String[] trimmed = new String[n];
            System.arraycopy(names, 0, trimmed, 0, n);
            names = trimmed;
        }
        int index = ChatPrefix.findSender(text, names);
        return index < 0 ? null : ids[index];
    }

    /** Maps a message's first display line to its sender. */
    public static void mapFirstLine(ChatLine line, UUID sender) {
        if (line == null || sender == null) return;
        if (LINES.size() >= LINE_CAP) LINES.clear();
        LINES.put(line, sender);
    }

    /** Drops every mapping (chat cleared, disconnected). */
    public static void clear() {
        LINES.clear();
    }

    /** Sender's tab-list info for a drawn line, or null (feature off / unmapped / gone). */
    public static NetworkPlayerInfo infoFor(ChatLine line) {
        if (line == null || !enabled()) return null;
        UUID id = LINES.get(line);
        if (id == null) return null;
        NetHandlerPlayClient net = Minecraft.getMinecraft().getNetHandler();
        return net == null ? null : net.getPlayerInfo(id);
    }

    /** Face + hat layer of the skin at (x, y), 8×8, with the line's fade alpha. */
    public static void drawHead(NetworkPlayerInfo info, int x, int y, int alpha) {
        Minecraft.getMinecraft().getTextureManager().bindTexture(info.getLocationSkin());
        GlStateManager.enableBlend();
        GlStateManager.color(1f, 1f, 1f, alpha / 255f);
        Gui.drawScaledCustomSizeModalRect(x, y, 8, 8, 8, 8, HEAD_SIZE, HEAD_SIZE, 64, 64);
        Gui.drawScaledCustomSizeModalRect(x, y, 40, 8, 8, 8, HEAD_SIZE, HEAD_SIZE, 64, 64);
        GlStateManager.color(1f, 1f, 1f, 1f);
    }
}
