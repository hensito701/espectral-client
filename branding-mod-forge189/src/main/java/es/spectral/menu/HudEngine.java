package es.spectral.menu;

import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.FontRenderer;
import net.minecraft.client.gui.ScaledResolution;
import net.minecraft.client.network.NetworkPlayerInfo;
import net.minecraft.client.renderer.GlStateManager;
import net.minecraft.client.resources.I18n;
import net.minecraft.entity.player.EntityPlayer;
import net.minecraft.item.ItemStack;
import net.minecraft.potion.Potion;
import net.minecraft.potion.PotionEffect;

import es.spectral.menu.ui.Gfx;

/**
 * Text HUD overlays (owned/live features potionstatus, coords, healthstatus,
 * armorstatus, fpsping): each enabled feature is one movable block
 * positioned by {@link HudLayout} and styled by {@link HudTheme}, drawn from
 * Forge's post-HUD overlay event. Empty blocks (no armor, no effects) are
 * omitted; nothing is built without a player.
 */
public final class HudEngine {

    private HudEngine() {}

    /** One row: text plus an optional leading icon. */
    public static final class Row {
        public final String text;
        public final HudIcon icon;

        public Row(String text, HudIcon icon) {
            this.text = text;
            this.icon = icon;
        }
    }

    /** One movable overlay block. */
    public static final class Block {
        public final String id;
        public final List<Row> rows;

        public Block(String id, List<Row> rows) {
            this.id = id;
            this.rows = rows;
        }
    }

    private static boolean on(String id) {
        return ClientConfig.getInstance().isFeatureEnabled(id);
    }

    /** Enabled, non-empty blocks in draw order; empty without a player. */
    public static List<Block> blocks(Minecraft mc) {
        if (mc == null || mc.thePlayer == null || mc.theWorld == null) return Collections.emptyList();
        EntityPlayer player = mc.thePlayer;
        List<Block> out = new ArrayList<Block>(5);
        if (on("fpsping")) {
            out.add(new Block("fpsping", Collections.singletonList(new Row(fpsPing(mc), null))));
        }
        if (on("coords")) {
            out.add(new Block("coords", Collections.singletonList(new Row(I18n.format("espectral.hud.coords",
                    fmt(player.posX), fmt(player.getEntityBoundingBox().minY), fmt(player.posZ), facing(player)), null))));
        }
        if (on("healthstatus")) {
            out.add(new Block("healthstatus", Collections.singletonList(healthRow(player))));
        }
        if (on("armorstatus")) {
            List<Row> armor = armorRows(player);
            if (!armor.isEmpty()) out.add(new Block("armorstatus", armor));
        }
        if (on("potionstatus")) {
            List<Row> potions = potionRows(player);
            if (!potions.isEmpty()) out.add(new Block("potionstatus", potions));
        }
        return out;
    }

    static Row healthRow(EntityPlayer player) {
        float absorption = player.getAbsorptionAmount();
        String line = absorption > 0.05f
                ? I18n.format("espectral.hud.health.absorption", fmt(player.getHealth()), fmt(player.getMaxHealth()), fmt(absorption))
                : I18n.format("espectral.hud.health", fmt(player.getHealth()), fmt(player.getMaxHealth()));
        HudIcon.Heart heart = player.isPotionActive(Potion.poison) ? HudIcon.Heart.POISONED
                : player.isPotionActive(Potion.wither) ? HudIcon.Heart.WITHERED
                : absorption > 0.05f ? HudIcon.Heart.ABSORBING : HudIcon.Heart.NORMAL;
        return new Row(line, HudIcon.heart(heart));
    }

    private static String fmt(double v) {
        return String.format(Locale.ROOT, "%.1f", v);
    }

    private static String fpsPing(Minecraft mc) {
        String line = I18n.format("espectral.hud.fps", Minecraft.getDebugFPS());
        if (mc.getNetHandler() != null) {
            NetworkPlayerInfo info = mc.getNetHandler().getPlayerInfo(mc.thePlayer.getUniqueID());
            if (info != null) line += " | " + I18n.format("espectral.hud.ping", info.getResponseTime());
        }
        return line;
    }

    private static String facing(EntityPlayer player) {
        return I18n.format("espectral.hud.facing." + player.getHorizontalFacing().getName());
    }

    /** One row per worn armor piece: icon + label + remaining durability. */
    private static List<Row> armorRows(EntityPlayer player) {
        List<Row> rows = new ArrayList<Row>(4);
        String[] labels = {"head", "chest", "legs", "feet"};
        for (int i = 0; i < 4; i++) {
            ItemStack stack = player.getCurrentArmor(3 - i);
            if (stack == null) continue;
            String label = I18n.format("espectral.hud.armor." + labels[i]);
            if (!stack.isItemStackDamageable()) {
                rows.add(new Row(label + ": --", HudIcon.item(stack)));
                continue;
            }
            int max = stack.getMaxDamage();
            rows.add(new Row(label + ": " + Math.max(0, max - stack.getItemDamage()) + "/" + max, HudIcon.item(stack)));
        }
        return rows;
    }

    private static List<Row> potionRows(EntityPlayer player) {
        List<Row> rows = new ArrayList<Row>();
        Collection<PotionEffect> effects = player.getActivePotionEffects();
        for (PotionEffect effect : effects) {
            Potion potion = Potion.potionTypes[effect.getPotionID()];
            if (potion == null) continue;
            int seconds = Math.max(0, effect.getDuration() / 20);
            String text = I18n.format(potion.getName()) + " " + roman(effect.getAmplifier() + 1) + " "
                    + String.format(Locale.ROOT, "%d:%02d", seconds / 60, seconds % 60);
            rows.add(new Row(text, HudIcon.effect(potion)));
        }
        return rows;
    }

    static String roman(int n) {
        String[] r = {"I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"};
        return n >= 1 && n <= 10 ? r[n - 1] : String.valueOf(n);
    }

    public static int rowHeight(Row row) {
        return row.icon != null ? Math.max(HudLayout.LINE_PITCH, row.icon.size() + 2) : HudLayout.LINE_PITCH;
    }

    public static int rowWidth(FontRenderer font, Row row) {
        return font.getStringWidth(row.text) + (row.icon != null ? row.icon.size() + 3 : 0);
    }

    /** Block size {width, height} including padding. */
    public static int[] size(FontRenderer font, List<Row> rows) {
        int w = 0;
        int h = HudLayout.PAD * 2;
        for (Row row : rows) {
            w = Math.max(w, rowWidth(font, row));
            h += rowHeight(row);
        }
        return new int[] {w + HudLayout.PAD * 2, h};
    }

    /** Draws every enabled block (post-HUD overlay). */
    public static void draw(Minecraft mc, ScaledResolution res) {
        // The HUD editor draws the blocks itself (with sample rows).
        if (mc.gameSettings.showDebugInfo || mc.gameSettings.hideGUI
                || mc.currentScreen instanceof es.spectral.menu.ui.HudEditScreen) {
            return;
        }
        List<Block> blocks = blocks(mc);
        if (blocks.isEmpty()) return;
        FontRenderer font = mc.fontRendererObj;
        HudTheme theme = HudTheme.current();
        for (Block block : blocks) {
            int[] size = size(font, block.rows);
            int[] pos = HudLayout.pos(block.id, res.getScaledWidth(), res.getScaledHeight(), size[0], size[1]);
            drawBlock(mc, theme, block.rows, pos[0], pos[1], size[0], size[1]);
        }
        GlStateManager.color(1f, 1f, 1f, 1f);
    }

    /** One themed block (also used by the HUD editor with sample rows). */
    public static void drawBlock(Minecraft mc, HudTheme theme, List<Row> rows, int x, int y, int w, int h) {
        if (theme.hasFill()) Gfx.fill(x, y, x + w, y + h, theme.fill);
        if (theme.hasFrame()) Gfx.outline(x, y, w, h, theme.frame);
        int ry = y + HudLayout.PAD;
        for (Row row : rows) {
            int rh = rowHeight(row);
            int tx = x + HudLayout.PAD;
            if (row.icon != null) {
                row.icon.draw(mc, tx, ry + (rh - row.icon.size()) / 2);
                tx += row.icon.size() + 3;
            }
            Gfx.text(row.text, tx, ry + (rh - 8) / 2, theme.text, theme.shadow);
            ry += rh;
        }
    }
}
