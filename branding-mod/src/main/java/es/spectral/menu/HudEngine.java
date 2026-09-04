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
     * Top-left stack: fpsping, coords, healthstatus, armorstatus (only the
     * enabled ones, in that order). Empty when there is no player/world.
     */
    public List<String> leftLines(Minecraft minecraft) {
        if (minecraft == null || minecraft.player == null || minecraft.level == null) {
            return Collections.emptyList();
        }
        List<String> lines = new ArrayList<>(8);
        if (isEnabled("fpsping")) {
            lines.add(fpsPingLine(minecraft));
        }
        if (isEnabled("coords")) {
            lines.add(String.format(Locale.ROOT, "XYZ: %.1f / %.1f / %.1f [%s]",
                    minecraft.player.getX(), minecraft.player.getY(), minecraft.player.getZ(),
                    facingName(minecraft)));
        }
        if (isEnabled("healthstatus")) {
            float health = minecraft.player.getHealth();
            float max = minecraft.player.getMaxHealth();
            float absorption = minecraft.player.getAbsorptionAmount();
            if (absorption > 0.05f) {
                lines.add(String.format(Locale.ROOT, "Salud: %.1f/%.1f (+%.1f)", health, max, absorption));
            } else {
                lines.add(String.format(Locale.ROOT, "Salud: %.1f/%.1f", health, max));
            }
        }
        if (isEnabled("armorstatus")) {
            lines.addAll(armorLines(minecraft));
        }
        return lines;
    }

    /**
     * Top-right column: one line per active potion effect, each with name,
     * amplifier level and remaining duration. Empty when the feature is off,
     * when there is no player/world, or when no effects are active.
     */
    public List<String> rightLines(Minecraft minecraft) {
        if (minecraft == null || minecraft.player == null || minecraft.level == null) {
            return Collections.emptyList();
        }
        if (!isEnabled("potionstatus")) {
            return Collections.emptyList();
        }
        List<String> lines = new ArrayList<>();
        for (MobEffectInstance instance : minecraft.player.getActiveEffects()) {
            String name = instance.getEffect().value().getDisplayName().getString();
            String level = toRoman(instance.getAmplifier() + 1);
            lines.add(name + " " + level + " " + formatDuration(instance));
        }
        return lines;
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

    private static List<String> armorLines(Minecraft minecraft) {
        List<String> lines = new ArrayList<>(4);
        addArmorPiece(lines, minecraft, EquipmentSlot.HEAD, "Casco");
        addArmorPiece(lines, minecraft, EquipmentSlot.CHEST, "Peto");
        addArmorPiece(lines, minecraft, EquipmentSlot.LEGS, "Grebas");
        addArmorPiece(lines, minecraft, EquipmentSlot.FEET, "Botas");
        return lines;
    }

    private static void addArmorPiece(List<String> lines, Minecraft minecraft, EquipmentSlot slot, String label) {
        ItemStack stack = minecraft.player.getItemBySlot(slot);
        if (stack == null || stack.isEmpty()) {
            return;
        }
        if (!stack.isDamageableItem()) {
            lines.add(label + ": --");
            return;
        }
        int max = stack.getMaxDamage();
        int remaining = Math.max(0, max - stack.getDamageValue());
        lines.add(label + ": " + remaining + "/" + max);
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
