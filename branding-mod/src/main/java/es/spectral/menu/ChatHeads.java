package es.spectral.menu;

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
 * <p>Vanilla {@code GuiMessage$Line} carries no sender reference (and must
 * never be resolved by nickname matching — {@code ChatListener#guessChatUUID}
 * is nickname-fragile), so the sender is threaded through two small maps:
 *
 * <ol>
 *   <li>{@code SENDERS}: {@link MessageSignature} -&gt; sender {@link UUID},
 *       captured at intake in {@code ChatListener#showMessageToPlayer}. The
 *       exact signature instance is threaded through to {@code GuiMessage},
 *       so record equality is never relied upon. Unsigned messages carry a
 *       null signature and are skipped (known limitation: no head).</li>
 *   <li>{@code LINES}: identity map from the split
 *       {@link FormattedCharSequence} instance of a message's first line to
 *       the sender UUID, populated when {@code ChatComponent} queues display
 *       lines. Only first lines ({@code endOfEntry}) of signed,
 *       sender-known player messages are mapped, so system/deleted lines
 *       simply get no head.</li>
 * </ol>
 *
 * <p>Every render lookup is gated on the {@code chatheads} feature flag, so a
 * disabled feature is behavior-identical to vanilla (the maps are invisible
 * side state). All entry points are safe to call from any thread; chat work
 * happens on the client thread in practice.
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

    private ChatHeads() {}

    /**
     * Intake hook: records which sender a signed player message belongs to.
     * Null signatures (unsigned messages) are skipped — those lines get no head.
     */
    public static void onPlayerMessage(MessageSignature signature, UUID senderId) {
        if (signature == null || senderId == null) return;
        SENDERS.put(signature, senderId);
    }

    /** Sender previously captured for a signature, or null when unknown/unsigned. */
    public static UUID senderFor(MessageSignature signature) {
        if (signature == null) return null;
        return SENDERS.get(signature);
    }

    /**
     * Maps a message's first-line content instance to its sender. Only the
     * first line is ever mapped, so continuation lines never draw heads.
     */
    public static void mapFirstLine(UUID senderId, FormattedCharSequence firstLine) {
        if (senderId == null || firstLine == null) return;
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
}
