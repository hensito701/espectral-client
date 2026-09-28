package es.spectral.menu.cosmetics;

import java.util.Locale;
import java.util.Set;
import java.util.UUID;

import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FontDescription;
import net.minecraft.network.chat.MutableComponent;
import net.minecraft.network.chat.Style;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.player.Player;

import es.spectral.menu.ClientConfig;

/**
 * Public cosmetics/badge API shared between MOD-CORE and the render/emotes
 * slices. All lookups are O(1) reads over {@link CosmeticsService}'s verified
 * state — no I/O, no allocation of consequence, safe on the render thread.
 *
 * <p>Trust model: {@code loadoutFor} / {@code isSupporter} only honour verified
 * grants (pinned Ed25519 key, subject bound to the uuid). Presence ({@code ⚛})
 * comes from the launcher bridge answering for the uuid; a missing or invalid
 * grant strips {@code ★} and every cosmetic but never the {@code ⚛}.
 *
 * <p>Badge glyphs: {@code ⚛} renders through the mod's bitmap font
 * {@code espectral-menu:badge} (vanilla would fall back to 16 px Unifont);
 * {@code ★} uses the default font. U+FE0E is never emitted.
 */
public final class Cosmetics {

    /** Registry feature id gating cosmetic rendering. */
    public static final String FEATURE_COSMETICS = "cosmetics";
    /** Registry feature id gating ⚛/★ badge decoration. */
    public static final String FEATURE_BADGES = "badges";

    /** Font carrying the ⚛ badge glyph. */
    private static final FontDescription BADGE_FONT = new FontDescription.Resource(
            Identifier.fromNamespaceAndPath("espectral-menu", "badge"));

    private Cosmetics() {}

    // ---------------------------------------------------------------
    // Entitlements / state
    // ---------------------------------------------------------------

    /**
     * Verified loadout for {@code uuid}, or {@code null} when nothing may
     * render: the cosmetics feature is off, the player is hidden by
     * {@code hidden_players}, or they carry no verified grant. Own player uses
     * the launcher-written loadout (filtered the same way). Callers still skip
     * invisible/spectator wearers themselves.
     */
    public static Loadout loadoutFor(UUID uuid) {
        if (uuid == null || !feature(FEATURE_COSMETICS)) return null;
        if (isSelf(uuid)) {
            // Self paths only apply when espectral-cosmetics.json's uuid is
            // this local player — a stale/mismatched file must not render
            // someone else's grant/loadout on us.
            CosmeticsService.Own own = CosmeticsService.own();
            if (!ownMatches(uuid, own)) return null;
            return own.grant() != null ? own.loadout() : null;
        }
        if (hidden(uuid)) return null;
        GrantVerifier.Grant grant = CosmeticsService.peerGrant(uuid);
        if (grant == null) return null;
        Loadout loadout = CosmeticsService.peerLoadout(uuid);
        return loadout != null && !loadout.isEmpty() ? loadout : null;
    }

    /**
     * ⚛ presence: the uuid is backed by live Espectral presence. Own player
     * counts only while sharing is on; peers need a bridge-confirmed entry.
     */
    public static boolean hasBadge(UUID uuid) {
        if (uuid == null || !feature(FEATURE_BADGES)) return false;
        if (isSelf(uuid)) {
            CosmeticsService.Own own = CosmeticsService.own();
            return ownMatches(uuid, own) && own.share();
        }
        if (hidden(uuid)) return false;
        return CosmeticsService.peerPresent(uuid);
    }

    /**
     * ★ supporter mark: presence plus a verified grant carrying
     * {@code "supporter"}.
     */
    public static boolean isSupporter(UUID uuid) {
        if (!hasBadge(uuid)) return false;
        GrantVerifier.Grant grant = isSelf(uuid)
                ? CosmeticsService.own().grant()
                : CosmeticsService.peerGrant(uuid);
        return grant != null && grant.hasFlag("supporter");
    }

    /** Own verified grant carries {@code "mvp"} (the emotes gate). */
    public static boolean ownMvp() {
        Minecraft minecraft = Minecraft.getInstance();
        CosmeticsService.Own own = CosmeticsService.own();
        return minecraft != null && minecraft.player != null
                && ownMatches(minecraft.player.getUUID(), own)
                && own.grant() != null && own.grant().hasFlag("mvp");
    }

    /**
     * The {@code emotes} flag from {@code espectral-cosmetics.json} (absent
     * defaults to on), ANDed with the cosmetics feature so the Suite master
     * and per-feature toggle still suppress it.
     */
    public static boolean emotesEnabled() {
        return feature(FEATURE_COSMETICS) && CosmeticsService.own().emotes();
    }

    /** Player-chosen reduced-motion toggle (personalization, free feature). */
    public static boolean reducedMotion() {
        return ClientConfig.getInstance().reducedMotion();
    }

    // ---------------------------------------------------------------
    // Render-time helpers
    // ---------------------------------------------------------------

    /**
     * Client tick entry point: snapshots the vanilla-visible uuid set for the
     * bridge. Wire from a Minecraft.tick mixin; does no I/O itself.
     */
    public static void onClientTick(Minecraft minecraft) {
        CosmeticsService.onClientTick(minecraft);
    }

    /**
     * Prepends the ⚛/★ badge to an entity nametag component, preserving the
     * server-sent component untouched (a leading sibling is added, the name is
     * never recoloured or rebuilt). Returns {@code base} unchanged when no
     * badge applies. Never emits U+FE0E.
     */
    public static Component decorateNameTag(Component base, UUID uuid) {
        if (base == null || uuid == null || !feature(FEATURE_BADGES)) return base;
        if (!hasBadge(uuid)) return base;
        return Component.empty()
                .append(badgeGlyphs(uuid))
                .append(base);
    }

    /**
     * Title/pause button label: "⚛ " in the badge font followed by the
     * translated label in the active theme accent. Used instead of the former
     * "★ Espectral Client…" literals so the mark renders through the mod's
     * bitmap font at Mojangles metrics.
     */
    public static MutableComponent suiteLabel(String translationKey) {
        return Component.empty()
                .withStyle(es.spectral.menu.ui.SuiteTheme.accentFormat())
                .append(Component.literal("⚛ ")
                        .setStyle(Style.EMPTY.withFont(BADGE_FONT)))
                .append(Component.translatable(translationKey));
    }

    /** Gold ⚛ (badge font) + ★ (default font) glyph run for a badged uuid. */
    private static MutableComponent badgeGlyphs(UUID uuid) {
        MutableComponent out = Component.literal("⚛")
                .setStyle(Style.EMPTY.withFont(BADGE_FONT).withColor(ChatFormatting.GOLD));
        if (isSupporter(uuid)) {
            out.append(Component.literal("★")
                    .setStyle(Style.EMPTY.withColor(ChatFormatting.GOLD)));
        }
        out.append(Component.literal(" "));
        return out;
    }

    // ---------------------------------------------------------------
    // Internals
    // ---------------------------------------------------------------

    private static boolean feature(String id) {
        return ClientConfig.getInstance().isFeatureEnabled(id);
    }

    private static boolean isSelf(UUID uuid) {
        Minecraft minecraft = Minecraft.getInstance();
        return minecraft != null && minecraft.player != null
                && uuid.equals(minecraft.player.getUUID());
    }

    /**
     * True when the state file's {@code uuid} field actually names the
     * account being rendered / the local player — the file is
     * launcher-written but a stale or foreign file is never trusted for
     * self paths.
     */
    private static boolean ownMatches(UUID uuid, CosmeticsService.Own own) {
        return own.uuid() != null && own.uuid().equals(uuid);
    }

    /**
     * The viewer hid this player (case-insensitive {@code hidden_players}
     * match against the vanilla tab-list name).
     */
    private static boolean hidden(UUID uuid) {
        Set<String> hidden = CosmeticsService.own().hiddenPlayers();
        if (hidden.isEmpty()) return false;
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null || minecraft.getConnection() == null) return false;
        PlayerInfo info = minecraft.getConnection().getPlayerInfo(uuid);
        if (info == null || info.getProfile() == null) return false;
        String name = info.getProfile().name();
        return name != null && hidden.contains(name.toLowerCase(Locale.ROOT));
    }

    /**
     * Player-entity guard used by the nametag mixin: only {@link Player}
     * wearers carry badges.
     */
    public static Component decorateEntityNameTag(Component base, Object entity) {
        if (entity instanceof Player player) {
            return decorateNameTag(base, player.getUUID());
        }
        return base;
    }
}
