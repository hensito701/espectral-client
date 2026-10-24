package es.spectral.menu;

import java.util.Collection;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.network.chat.MessageSignature;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.entity.player.PlayerSkin;

/**
 * Owned/live ChatHeads state (Contract A "chatheads", default off).
 *
 * <p>Vanilla {@code GuiMessage$Line} carries no sender reference, so the
 * sender is threaded through two small maps:
 *
 * <ol>
 *   <li>{@code SENDERS}: {@link MessageSignature} -&gt; sender {@link UUID},
 *       captured at intake in {@code ChatListener#showMessageToPlayer}. The
 *       exact signature instance is threaded through to {@code GuiMessage},
 *       so record equality is never relied upon. This is the preferred,
 *       exact path: it is used whenever the message was signed.</li>
 *   <li>{@code LINES}: identity map from the split
 *       {@link FormattedCharSequence} instance of a message's first line to
 *       the sender UUID, populated when {@code ChatComponent} queues display
 *       lines. Only first lines ({@code endOfEntry}) are mapped, so
 *       continuation lines never draw heads.</li>
 * </ol>
 *
 * <p><strong>System-formatted chat.</strong> Plenty of servers — the
 * Espectral ones included — run a chat plugin that broadcasts player chat as
 * a <em>system</em> message ({@code [System] [CHAT] <[ADMIN] henMV> asd}):
 * there is no {@code PlayerChatMessage}, hence no signature to key on, and
 * the signed path alone would leave every line headless. For those lines
 * {@link #senderForText(String)} resolves a sender by matching an
 * <em>exact</em> profile name taken from the current tab list
 * ({@code ClientPacketListener#getOnlinePlayers()}) against the line's
 * sender prefix (the text before the first {@code >}, {@code :}, {@code »} or
 * {@code |}) — see {@link ChatPrefix} for the position, whole-word and
 * broadcast-rejection rules. This is deliberately
 * <em>not</em> nickname guessing ({@code ChatListener#guessChatUUID}
 * substring-matches free text and is nickname-fragile): a candidate must be
 * a live tab-list name, occur whole-word, and sit in one of the two sender
 * positions of the prefix, or the line stays vanilla. Lines with no separator (broadcasts such as
 * {@code ¡Bienvenido a ESPECTRAL, henMV!}) never resolve, and the exact
 * signed mapping always wins over the resolver.
 *
 * <p>Every render lookup is gated on the {@code chatheads} feature flag, so a
 * disabled feature is behavior-identical to vanilla (the maps are invisible
 * side state). All entry points are safe to call from any thread; chat work
 * happens on the client thread in practice, and the render path never
 * allocates: it only does an identity-map lookup.
 */
public final class ChatHeads {

    /** Feature id (reserved in {@link FeatureRegistry} and {@link ClientConfig}). */
    public static final String FEATURE_ID = "chatheads";

    /** Head footprint in chat space. */
    public static final int HEAD_SIZE = 8;

    /** Gap between the head and the shifted text. */
    public static final int GUTTER = 2;

    /** Horizontal text shift applied to headed lines (head + gutter). */
    public static final int SHIFT = HEAD_SIZE + GUTTER;

    /** Sender-map cap; vanilla keeps at most 100 chat messages. */
    private static final int SENDER_CAP = 256;

    /** Line-map guard; refreshed wholesale on every trim rebuild. */
    private static final int LINE_CAP = 4096;

    private static final Map<MessageSignature, UUID> SENDERS =
            Collections.synchronizedMap(new LinkedHashMap<MessageSignature, UUID>(64, 0.75f, true) {
                @Override
                protected boolean removeEldestEntry(Map.Entry<MessageSignature, UUID> eldest) {
                    return size() > SENDER_CAP;
                }
            });

    private static final Map<FormattedCharSequence, UUID> LINES =
            Collections.synchronizedMap(new IdentityHashMap<>());

    /** Tab-list snapshot backing {@link #senderForText(String)}; null = stale. */
    private static volatile TabSnapshot tabSnapshot;

    private ChatHeads() {}

    /**
     * Intake hook: records which sender a signed player message belongs to.
     * Null signatures (unsigned messages) are skipped here and fall through to
     * {@link #senderForText(String)} at display time.
     * While the feature is inactive (including suite master off) nothing is
     * recorded, so a disabled feature accumulates no side state.
     */
    public static void onPlayerMessage(MessageSignature signature, UUID senderId) {
        if (signature == null || senderId == null) return;
        if (!ClientConfig.getInstance().isFeatureEnabled(FEATURE_ID)) return;
        SENDERS.put(signature, senderId);
    }

    /** Sender previously captured for a signature, or null when unknown/unsigned. */
    public static UUID senderFor(MessageSignature signature) {
        if (signature == null) return null;
        return SENDERS.get(signature);
    }

    /**
     * Resolves the sender of a message that carries no usable signature —
     * typically a chat line a server plugin sent as a system message.
     *
     * <p>Only call on message intake (once per chat line), never per frame:
     * this walks the tab list to (re)build a snapshot of exact profile names
     * and ids, and then runs the prefix parser.
     *
     * @param text the message's plain text ({@code Component#getString()})
     * @return the matching player's uuid, or null when nothing matched
     */
    public static UUID senderForText(String text) {
        if (text == null || text.isEmpty()) return null;
        if (!ClientConfig.getInstance().isFeatureEnabled(FEATURE_ID)) return null;
        TabSnapshot snapshot = snapshot();
        if (snapshot == null) return null;
        int index = ChatPrefix.findSender(text, snapshot.names);
        return index < 0 ? null : snapshot.ids[index];
    }

    /**
     * Maps a message's first-line content instance to its sender. Only the
     * first line is ever mapped, so continuation lines never draw heads.
     * No-op while the feature is inactive, matching {@link #onPlayerMessage}.
     */
    public static void mapFirstLine(UUID senderId, FormattedCharSequence firstLine) {
        if (senderId == null || firstLine == null) return;
        if (!ClientConfig.getInstance().isFeatureEnabled(FEATURE_ID)) return;
        if (LINES.size() >= LINE_CAP) LINES.clear();
        LINES.put(firstLine, senderId);
    }

    /** Drops line mappings; the sender map survives (signatures stay valid). */
    public static void clearLineSenders() {
        LINES.clear();
    }

    /** Drops all cached sender state (chat cleared / disconnected). */
    public static void clear() {
        SENDERS.clear();
        LINES.clear();
        tabSnapshot = null;
    }

    /**
     * Resolves the skin to draw for a queued line-content instance, or null
     * when the feature is off, the line is unmapped (system/unsigned/unknown),
     * the player info is gone, or no connection exists.
     */
    public static PlayerSkin skinForContent(FormattedCharSequence content) {
        if (content == null) return null;
        if (!ClientConfig.getInstance().isFeatureEnabled(FEATURE_ID)) return null;
        UUID senderId = LINES.get(content);
        if (senderId == null) return null;
        return resolveSkin(senderId);
    }

    /**
     * Current tab-list names/ids, rebuilt only when the roster changed
     * (connection identity, size, or any {@code PlayerInfo} instance swapped).
     * Cheap to call — the common path is a few field reads plus an
     * element-by-element identity compare that allocates nothing.
     */
    private static TabSnapshot snapshot() {
        ClientPacketListener connection;
        Collection<PlayerInfo> players;
        try {
            Minecraft minecraft = Minecraft.getInstance();
            if (minecraft == null) return null;
            connection = minecraft.getConnection();
            if (connection == null) return null;
            players = connection.getOnlinePlayers();
        } catch (RuntimeException e) {
            tabSnapshot = null;
            return null;
        }
        if (players == null) return null;
        TabSnapshot cached = tabSnapshot;
        if (cached != null && cached.matches(connection, players)) return cached;
        TabSnapshot rebuilt;
        try {
            rebuilt = TabSnapshot.of(connection, players);
        } catch (RuntimeException e) {
            // Roster mutated mid-walk: keep the old snapshot (or none) and let
            // the next line retry. Chat must never crash on this.
            return null;
        }
        tabSnapshot = rebuilt;
        return rebuilt;
    }

    private static PlayerSkin resolveSkin(UUID senderId) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null) return null;
        ClientPacketListener connection;
        try {
            connection = minecraft.getConnection();
        } catch (RuntimeException e) {
            return null;
        }
        if (connection == null) return null;
        PlayerInfo info;
        try {
            info = connection.getPlayerInfo(senderId);
        } catch (RuntimeException e) {
            return null;
        }
        if (info == null) return null;
        try {
            return info.getSkin();
        } catch (RuntimeException e) {
            return null;
        }
    }

    /**
     * Immutable copy of the tab list's exact profile names and ids, plus the
     * roster snapshot it was built from so staleness can be detected without
     * rebuilding. {@code PlayerInfo} keeps identity equality, so comparing the
     * live roster element-by-element against the captured array is exact and
     * allocates nothing.
     */
    private static final class TabSnapshot {

        private final ClientPacketListener connection;
        private final PlayerInfo[] roster;
        private final String[] names;
        private final UUID[] ids;

        private TabSnapshot(ClientPacketListener connection, PlayerInfo[] roster, String[] names,
                UUID[] ids) {
            this.connection = connection;
            this.roster = roster;
            this.names = names;
            this.ids = ids;
        }

        private static TabSnapshot of(ClientPacketListener connection,
                Collection<PlayerInfo> players) {
            PlayerInfo[] roster = players.toArray(new PlayerInfo[0]);
            String[] names = new String[roster.length];
            UUID[] ids = new UUID[roster.length];
            int count = 0;
            for (PlayerInfo info : roster) {
                if (info == null) continue;
                String name;
                UUID id;
                try {
                    name = info.getProfile().name();
                    id = info.getProfile().id();
                } catch (RuntimeException e) {
                    continue;
                }
                if (name == null || name.isEmpty() || id == null) continue;
                names[count] = name;
                ids[count] = id;
                count++;
            }
            if (count != names.length) {
                // A profile was unusable; shrink so no null names remain.
                String[] trimmedNames = new String[count];
                UUID[] trimmedIds = new UUID[count];
                System.arraycopy(names, 0, trimmedNames, 0, count);
                System.arraycopy(ids, 0, trimmedIds, 0, count);
                names = trimmedNames;
                ids = trimmedIds;
            }
            return new TabSnapshot(connection, roster, names, ids);
        }

        private boolean matches(ClientPacketListener connection, Collection<PlayerInfo> players) {
            if (this.connection != connection) return false;
            if (this.roster.length != players.size()) return false;
            int i = 0;
            for (PlayerInfo info : players) {
                if (info != this.roster[i]) return false;
                i++;
            }
            return true;
        }
    }
}
