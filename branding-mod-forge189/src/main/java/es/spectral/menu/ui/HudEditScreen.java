package es.spectral.menu.ui;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

import org.lwjgl.input.Keyboard;

import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.init.Items;
import net.minecraft.item.ItemStack;
import net.minecraft.potion.Potion;

import es.spectral.menu.FeatureRegistry;
import es.spectral.menu.HudEngine;
import es.spectral.menu.HudIcon;
import es.spectral.menu.HudLayout;
import es.spectral.menu.HudTheme;
import es.spectral.menu.Lang;
import es.spectral.menu.media.MediaHudCard;

/**
 * Drag-to-move editor for the HUD overlay blocks (same behaviour as the
 * Fabric lanes): every block is shown — live rows in a world, a sample row
 * otherwise or when the feature is off — so positions can be arranged before
 * turning a feature on. Drag to move (saved on release, clamped on screen),
 * right-click a block to reset it, "Reset" resets all, Done/Esc return.
 */
public class HudEditScreen extends GuiScreen {

    private static final class EditBlock {
        final String id;
        final List<HudEngine.Row> rows;
        int x;
        int y;
        int w;
        int h;

        EditBlock(String id, List<HudEngine.Row> rows) {
            this.id = id;
            this.rows = rows;
        }

        boolean contains(int mx, int my) {
            return mx >= x && mx < x + w && my >= y && my < y + h;
        }
    }

    private final GuiScreen parent;
    private final List<EditBlock> blocks = new ArrayList<EditBlock>();
    private EditBlock dragging;
    private int dragOffX;
    private int dragOffY;

    public HudEditScreen(GuiScreen parent) {
        this.parent = parent;
    }

    @Override
    public void initGui() {
        buttonList.clear();
        int bw = 90;
        int y = height - 26;
        buttonList.add(new SuiteButton(0, width / 2 - bw - 2, y, bw, 20, Lang.tr("espectral.hudedit.reset")));
        buttonList.add(new SuiteButton(1, width / 2 + 2, y, bw, 20, Lang.tr("espectral.hudedit.done")));
        rebuild();
    }

    private void rebuild() {
        blocks.clear();
        List<HudEngine.Block> live = HudEngine.blocks(mc);
        for (String id : HudLayout.OVERLAY_IDS) {
            if (MediaHudCard.FEATURE_ID.equals(id)) {
                // Text-less card: its box is the fixed card size, not row-measured.
                EditBlock card = new EditBlock(id, Collections.<HudEngine.Row>emptyList());
                card.w = MediaHudCard.WIDTH;
                card.h = MediaHudCard.HEIGHT;
                int[] cardPos = HudLayout.pos(id, width, height, card.w, card.h);
                card.x = cardPos[0];
                card.y = cardPos[1];
                blocks.add(card);
                continue;
            }
            List<HudEngine.Row> rows = null;
            for (HudEngine.Block b : live) {
                if (b.id.equals(id)) rows = b.rows;
            }
            if (rows == null || rows.isEmpty()) rows = Collections.singletonList(sample(id));
            EditBlock eb = new EditBlock(id, rows);
            int[] size = HudEngine.size(fontRendererObj, rows);
            eb.w = size[0];
            eb.h = size[1];
            int[] pos = HudLayout.pos(id, width, height, eb.w, eb.h);
            eb.x = pos[0];
            eb.y = pos[1];
            blocks.add(eb);
        }
    }

    /** Placeholder row sized like the real overlay. */
    private HudEngine.Row sample(String id) {
        switch (id) {
            case "fpsping":
                return new HudEngine.Row(Lang.tr("espectral.hud.fps", 240) + " | " + Lang.tr("espectral.hud.ping", 12), null);
            case "coords":
                return new HudEngine.Row(Lang.tr("espectral.hud.coords", "128.5", "64.0", "-256.2",
                        Lang.tr("espectral.hud.facing.north")), null);
            case "healthstatus":
                return new HudEngine.Row(Lang.tr("espectral.hud.health", "20.0", "20.0"), HudIcon.heart(HudIcon.Heart.NORMAL));
            case "armorstatus":
                return new HudEngine.Row(Lang.tr("espectral.hud.armor.head") + ": 363/363",
                        HudIcon.item(new ItemStack(Items.diamond_helmet)));
            case "potionstatus":
                return new HudEngine.Row(Lang.tr(Potion.moveSpeed.getName()) + " II 0:42", HudIcon.effect(Potion.moveSpeed));
            default:
                return new HudEngine.Row(id, null);
        }
    }

    @Override
    protected void actionPerformed(GuiButton button) {
        if (button.id == 0) {
            HudLayout.resetAll();
            rebuild();
        } else {
            mc.displayGuiScreen(parent);
        }
    }

    @Override
    protected void keyTyped(char typedChar, int keyCode) {
        if (keyCode == Keyboard.KEY_ESCAPE) mc.displayGuiScreen(parent);
    }

    @Override
    protected void mouseClicked(int mouseX, int mouseY, int mouseButton) throws IOException {
        for (int i = blocks.size() - 1; i >= 0; i--) {
            EditBlock b = blocks.get(i);
            if (!b.contains(mouseX, mouseY)) continue;
            if (mouseButton == 1) {
                HudLayout.resetPos(b.id);
                rebuild();
            } else if (mouseButton == 0) {
                dragging = b;
                dragOffX = mouseX - b.x;
                dragOffY = mouseY - b.y;
            }
            return;
        }
        super.mouseClicked(mouseX, mouseY, mouseButton);
    }

    @Override
    protected void mouseClickMove(int mouseX, int mouseY, int clickedMouseButton, long timeSinceLastClick) {
        if (dragging != null) {
            dragging.x = mouseX - dragOffX;
            dragging.y = mouseY - dragOffY;
        }
    }

    @Override
    protected void mouseReleased(int mouseX, int mouseY, int state) {
        if (dragging != null) {
            HudLayout.setPos(dragging.id, dragging.x, dragging.y, width, height, dragging.w, dragging.h);
            int[] pos = HudLayout.pos(dragging.id, width, height, dragging.w, dragging.h);
            dragging.x = pos[0];
            dragging.y = pos[1];
            dragging = null;
            return;
        }
        super.mouseReleased(mouseX, mouseY, state);
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        if (mc.theWorld == null) {
            MenuSkin.paintBackdrop(width, height);
        } else {
            Gfx.fill(0, 0, width, height, 0x60000000);
        }
        HudTheme theme = HudTheme.current();
        for (EditBlock b : blocks) {
            HudEngine.drawBlock(mc, theme, b.rows, b.x, b.y, b.w, b.h);
            if (MediaHudCard.FEATURE_ID.equals(b.id)) MediaHudCard.drawContents(b.x, b.y);
            boolean hot = b == dragging || (dragging == null && b.contains(mouseX, mouseY));
            Gfx.outline(b.x - 1, b.y - 1, b.w + 2, b.h + 2, hot ? theme.outlineHot : theme.outlineIdle);
        }
        // The default stack sits blocks 14 px apart, so labels go only on the
        // hovered/dragged block (above it, or below at the top edge).
        for (EditBlock b : blocks) {
            if (b != dragging && (dragging != null || !b.contains(mouseX, mouseY))) continue;
            String label = Lang.tr(FeatureRegistry.nameKey(b.id));
            int ly = b.y - 11 >= 0 ? b.y - 11 : b.y + b.h + 2;
            int lw = fontRendererObj.getStringWidth(label);
            Gfx.fill(b.x - 1, ly - 1, b.x + lw + 3, ly + 9, 0xC0000000);
            Gfx.text(label, b.x + 1, ly, theme.label, true);
        }
        Gfx.textCentered(Lang.tr("espectral.hudedit.title"), width / 2, height / 2 - 14, SuiteTheme.accentBright(), true);
        Gfx.textCentered(Lang.tr("espectral.hudedit.hint"), width / 2, height / 2 - 2, SuiteTheme.TEXT_DIM, true);
        super.drawScreen(mouseX, mouseY, partialTicks);
    }

    @Override
    public boolean doesGuiPauseGame() {
        return true;
    }
}
