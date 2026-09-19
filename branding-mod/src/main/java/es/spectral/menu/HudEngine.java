package es.spectral.menu;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.item.ItemStack;

/**
 * Text HUD overlays (Contract A owned/live features): potionstatus, coords,
 * healthstatus, armorstatus, fpsping.
 *
 * This class is pure data: it builds the overlay lines from live client state
 * and performs no rendering itself, so it compiles unchanged against both
 * supported versions. The per-version drawing lives in {@code Compat}
 * ({@code drawHudOverlays}), fed by the per-version HUD mixin
 * ({@code GuiHudMixin} on 1.21.11, {@code HudOverlayMixin} on 26.2).
 *
 * Layout contract: potion status is first-class, drawn as a top-right column;
 * fpsping/coords/health/armor stack top-left. White text with shadow is applied
 * by the drawing side. Returns empty lists when there is no player/world, so
 * the default-off path never touches rendering.
 */
public final class HudEngine {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    private static final HudEngine INSTANCE = new HudEngine();

    public static HudEngine getInstance() {
        return INSTANCE;
    }

    private HudEngine() {}

    /** Last observed lowfire/clearwater states; drives transition logging. */
    private boolean lastLowFire;
    private boolean lastClearWater;

    public static boolean isEnabled(String id) {
        return ClientConfig.getInstance().isFeatureEnabled(id);
    }

    /**
     * Polled every client tick via MinecraftMixin: logs the lowfire/clearwater
     * enable/disable transitions (the render hooks themselves are stateless, so
     * this is the only tick-side work the HUD needs).
     */
    public void onTick(Minecraft minecraft) {
        if (minecraft == null) return;
        boolean lowFire = isEnabled("lowfire");
        if (lowFire != lastLowFire) {
            lastLowFire = lowFire;
            LOGGER.info("LowFire {}", lowFire ? "on: fire overlay lowered" : "off: vanilla fire overlay restored");
        }
        boolean clearWater = isEnabled("clearwater");
        if (clearWater != lastClearWater) {
            lastClearWater = clearWater;
            LOGGER.info("ClearWater {}", clearWater ? "on: underwater overlay + water fog cleared"
                    : "off: vanilla water rendering restored");
        }
    }

    /**
     * One text row inside an overlay block, optionally with a leading icon
     * (heart / armor item / potion effect sprite). {@code icon} is null for
     * plain text rows.
     */
    public static final class Row {
        public final String text;
        public final HudIcon icon;

        public Row(String text, HudIcon icon) {
            this.text = text;
            this.icon = icon;
        }
    }

    /**
     * One movable overlay block: a feature id plus its rows. The draw side
     * measures the widest row (icon + text) for the box width and stacks rows
     * at their own heights.
     */
    public static final class Block {
        public final String id;
        public final List<Row> rows;

        public Block(String id, List<Row> rows) {
            this.id = id;
            this.rows = rows;
        }
    }

    /**
     * Enabled overlay blocks in draw order: fpsping, coords, healthstatus,
     * armorstatus, then potionstatus. Each block carries only its own rows so
     * it can be positioned independently. Empty blocks (e.g. armor with no
     * gear, potions with none active) are omitted so they never reserve space.
     * Returns empty when there is no player/world.
     */
    public List<Block> blocks(Minecraft minecraft) {
        if (minecraft == null || minecraft.player == null || minecraft.level == null) {
            return Collections.emptyList();
        }
        List<Block> out = new ArrayList<>(5);
        if (isEnabled("fpsping")) {
            out.add(new Block("fpsping", List.of(new Row(fpsPingLine(minecraft), null))));
        }
        if (isEnabled("coords")) {
            out.add(new Block("coords", List.of(new Row(String.format(Locale.ROOT, "XYZ: %.1f / %.1f / %.1f [%s]",
                    minecraft.player.getX(), minecraft.player.getY(), minecraft.player.getZ(),
                    facingName(minecraft)), null))));
        }
        if (isEnabled("healthstatus")) {
            float health = minecraft.player.getHealth();
            float max = minecraft.player.getMaxHealth();
            float absorption = minecraft.player.getAbsorptionAmount();
            String line = absorption > 0.05f
                    ? String.format(Locale.ROOT, "Salud: %.1f/%.1f (+%.1f)", health, max, absorption)
                    : String.format(Locale.ROOT, "Salud: %.1f/%.1f", health, max);
            out.add(new Block("healthstatus", List.of(new Row(line, HudIcon.heart(heartFor(minecraft))))));
        }
        if (isEnabled("armorstatus")) {
            List<Row> armor = armorRows(minecraft);
            if (!armor.isEmpty()) {
                out.add(new Block("armorstatus", armor));
            }
        }
        if (isEnabled("potionstatus")) {
            List<Row> potions = potionRows(minecraft);
            if (!potions.isEmpty()) {
                out.add(new Block("potionstatus", potions));
            }
        }
        return out;
    }

    /**
     * Heart variant for the health overlay, mirroring vanilla
     * {@code HeartType.forPlayer}: poison > wither > absorption > frozen >
     * normal. Absorption shows gold hearts in vanilla; we surface it as the
     * ABSORBING variant so the icon matches the (+%.1f) text.
     */
    private static HudIcon.Heart heartFor(Minecraft minecraft) {
        var player = minecraft.player;
        if (player.hasEffect(net.minecraft.world.effect.MobEffects.POISON)) {
            return HudIcon.Heart.POISIONED;
        }
        if (player.hasEffect(net.minecraft.world.effect.MobEffects.WITHER)) {
            return HudIcon.Heart.WITHERED;
        }
        if (player.getAbsorptionAmount() > 0.05f) {
            return HudIcon.Heart.ABSORBING;
        }
        if (player.isFullyFrozen()) {
            return HudIcon.Heart.FROZEN;
        }
        return HudIcon.Heart.NORMAL;
    }

    /**
     * One row per active potion effect: effect sprite + name, amplifier level
     * and remaining duration. Empty when no effects are active.
     */
    private static List<Row> potionRows(Minecraft minecraft) {
        List<Row> rows = new ArrayList<>();
        for (MobEffectInstance instance : minecraft.player.getActiveEffects()) {
            String name = instance.getEffect().value().getDisplayName().getString();
            String level = toRoman(instance.getAmplifier() + 1);
            rows.add(new Row(name + " " + level + " " + formatDuration(instance),
                    HudIcon.effect(instance.getEffect())));
        }
        return rows;
    }

    private static String fpsPingLine(Minecraft minecraft) {
        String line = "FPS: " + minecraft.getFps();
        if (minecraft.getConnection() != null && minecraft.player != null) {
            PlayerInfo info = minecraft.getConnection().getPlayerInfo(minecraft.player.getUUID());
            if (info != null) {
                line += " | Ping: " + info.getLatency() + " ms";
            }
        }
        return line;
    }

    private static String facingName(Minecraft minecraft) {
        String raw = minecraft.player.getDirection().getName();
        switch (raw) {
            case "north": return "Norte";
            case "south": return "Sur";
            case "west": return "Oeste";
            case "east": return "Este";
            case "up": return "Arriba";
            case "down": return "Abajo";
            default: return raw;
        }
    }

    /**
     * One row per worn armor piece: the item's icon + label and remaining
     * durability. Non-damageable pieces show "--". Empty when nothing is worn.
     */
    private static List<Row> armorRows(Minecraft minecraft) {
        List<Row> rows = new ArrayList<>(4);
        addArmorPiece(rows, minecraft, EquipmentSlot.HEAD, "Casco");
        addArmorPiece(rows, minecraft, EquipmentSlot.CHEST, "Peto");
        addArmorPiece(rows, minecraft, EquipmentSlot.LEGS, "Grebas");
        addArmorPiece(rows, minecraft, EquipmentSlot.FEET, "Botas");
        return rows;
    }

    private static void addArmorPiece(List<Row> rows, Minecraft minecraft, EquipmentSlot slot, String label) {
        ItemStack stack = minecraft.player.getItemBySlot(slot);
        if (stack == null || stack.isEmpty()) {
            return;
        }
        HudIcon icon = HudIcon.item(stack);
        if (!stack.isDamageableItem()) {
            rows.add(new Row(label + ": --", icon));
            return;
        }
        int max = stack.getMaxDamage();
        int remaining = Math.max(0, max - stack.getDamageValue());
        rows.add(new Row(label + ": " + remaining + "/" + max, icon));
    }

    private static String formatDuration(MobEffectInstance instance) {
        if (instance.isInfiniteDuration()) {
            return "∞";
        }
        int totalSeconds = Math.max(0, instance.getDuration() / 20);
        return String.format(Locale.ROOT, "%d:%02d", totalSeconds / 60, totalSeconds % 60);
    }

    private static String toRoman(int number) {
        switch (number) {
            case 1: return "I";
            case 2: return "II";
            case 3: return "III";
            case 4: return "IV";
            case 5: return "V";
            case 6: return "VI";
            case 7: return "VII";
            case 8: return "VIII";
            case 9: return "IX";
            case 10: return "X";
            default: return String.valueOf(number);
        }
    }
}
