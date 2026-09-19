package es.spectral.menu;

import net.minecraft.core.Holder;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.item.ItemStack;

/**
 * A version-agnostic icon attached to a HUD overlay row. The draw side
 * ({@code Compat.drawIcon}) maps each kind to the version's sprite/item call:
 *
 * <ul>
 *   <li>{@link Kind#HEART} — a {@link Heart} variant resolved to the vanilla
 *       {@code hud/heart/*} sprite (mirrors {@code HeartType.forPlayer}).</li>
 *   <li>{@link Kind#ITEM} — an {@link ItemStack} rendered as the item icon
 *       (armor pieces).</li>
 *   <li>{@link Kind#EFFECT} — a {@code Holder<MobEffect>} resolved to the
 *       vanilla mob-effect sprite (the inventory effect icon).</li>
 * </ul>
 *
 * <p>Icons carry their native pixel size so rows can size themselves: hearts
 * 9px, items 16px, effect sprites 18px.
 */
public final class HudIcon {

    public enum Kind { HEART, ITEM, EFFECT }

    /**
     * Heart variants mirroring {@code Gui/Hud.HeartType} (kept name-identical,
     * including vanilla's {@code POISIONED} typo, so {@code valueOf} maps
     * straight across on both versions).
     */
    public enum Heart { NORMAL, POISIONED, WITHERED, ABSORBING, FROZEN }

    public final Kind kind;
    public final Heart heart;          // HEART
    public final ItemStack item;       // ITEM
    public final Holder<MobEffect> effect; // EFFECT

    private HudIcon(Kind kind, Heart heart, ItemStack item, Holder<MobEffect> effect) {
        this.kind = kind;
        this.heart = heart;
        this.item = item;
        this.effect = effect;
    }

    public static HudIcon heart(Heart heart) {
        return new HudIcon(Kind.HEART, heart, null, null);
    }

    public static HudIcon item(ItemStack stack) {
        return new HudIcon(Kind.ITEM, null, stack, null);
    }

    public static HudIcon effect(Holder<MobEffect> effect) {
        return new HudIcon(Kind.EFFECT, null, null, effect);
    }

    /** Native pixel size of this icon (square). */
    public int size() {
        switch (kind) {
            case ITEM: return 16;
            case EFFECT: return 18;
            case HEART:
            default: return 9;
        }
    }
}
