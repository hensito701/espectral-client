package es.spectral.menu;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.renderer.GlStateManager;
import net.minecraft.client.renderer.RenderHelper;
import net.minecraft.item.ItemStack;
import net.minecraft.potion.Potion;
import net.minecraft.util.ResourceLocation;

/**
 * Leading icon of a HUD overlay row (same kinds as the Fabric lanes): a heart
 * from {@code gui/icons.png}, an item (armor piece) or a potion effect icon
 * from the inventory sheet. Icons carry their native size so rows size
 * themselves: hearts 9 px, items 16 px, effects 18 px.
 */
public final class HudIcon {

    private static final ResourceLocation INVENTORY = new ResourceLocation("textures/gui/container/inventory.png");

    public enum Kind { HEART, ITEM, EFFECT }

    /** Heart variants and their {@code icons.png} column (full heart, row 0). */
    public enum Heart {
        NORMAL(52), POISONED(88), WITHERED(124), ABSORBING(160);

        final int u;

        Heart(int u) {
            this.u = u;
        }
    }

    public final Kind kind;
    public final Heart heart;
    public final ItemStack item;
    public final Potion effect;

    private HudIcon(Kind kind, Heart heart, ItemStack item, Potion effect) {
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

    public static HudIcon effect(Potion potion) {
        return new HudIcon(Kind.EFFECT, null, null, potion);
    }

    /** Native pixel size (square). */
    public int size() {
        switch (kind) {
            case ITEM: return 16;
            case EFFECT: return 18;
            default: return 9;
        }
    }

    /** Draws the icon with its top-left at (x, y). */
    public void draw(Minecraft mc, int x, int y) {
        GlStateManager.color(1f, 1f, 1f, 1f);
        GlStateManager.enableBlend();
        switch (kind) {
            case HEART:
                mc.getTextureManager().bindTexture(Gui.icons);
                Gui.drawModalRectWithCustomSizedTexture(x, y, 16, 0, 9, 9, 256, 256);
                Gui.drawModalRectWithCustomSizedTexture(x, y, heart.u, 0, 9, 9, 256, 256);
                break;
            case ITEM:
                RenderHelper.enableGUIStandardItemLighting();
                mc.getRenderItem().renderItemAndEffectIntoGUI(item, x, y);
                RenderHelper.disableStandardItemLighting();
                break;
            case EFFECT:
                if (effect.hasStatusIcon()) {
                    int index = effect.getStatusIconIndex();
                    mc.getTextureManager().bindTexture(INVENTORY);
                    Gui.drawModalRectWithCustomSizedTexture(x, y, index % 8 * 18, 198 + index / 8 * 18, 18, 18, 256, 256);
                }
                break;
            default:
                break;
        }
        GlStateManager.disableBlend();
    }
}
