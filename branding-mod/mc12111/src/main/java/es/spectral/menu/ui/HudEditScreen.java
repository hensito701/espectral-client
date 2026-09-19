package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.network.chat.Component;

import es.spectral.menu.Compat;
import es.spectral.menu.HudEditLogic;
import es.spectral.menu.HudEngine;
import es.spectral.menu.HudLayout;

/**
 * 1.21.11 drag-to-move editor for the HUD overlay blocks (fpsping, coords,
 * healthstatus, armorstatus, potionstatus).
 *
 * <p>Shows every overlay as a draggable box (live content in a world, a
 * sample line otherwise) over a dimmed backdrop. Dragging updates the stored
 * position via {@link HudLayout}; a Reset button returns all blocks to their
 * default anchors. Esc / Done returns to the parent screen.
 *
 * <p>1.21.11 renders screens through {@code render(GuiGraphics,…)} and reports
 * mouse input as raw coordinates — the shared logic lives in
 * {@link HudEditLogic} so this class only adapts the version-specific API.
 */
public class HudEditScreen extends Screen {

    private final Screen parent;
    private final HudEditLogic logic = new HudEditLogic();

    public HudEditScreen(Screen parent) {
        super(Component.translatable("espectral.hudedit.title"));
        this.parent = parent;
    }

    @Override
    protected void init() {
        super.init();
        logic.rebuild(this.minecraft, this.font, this.width, this.height);
        int btnW = 90;
        int y = this.height - 26;
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.hudedit.reset"),
                b -> logic.resetAll(this.minecraft, this.font, this.width, this.height))
                .bounds(this.width / 2 - btnW - 4, y, btnW, 20).build());
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.hudedit.done"),
                b -> onClose())
                .bounds(this.width / 2 + 4, y, btnW, 20).build());
    }

    @Override
    public void render(GuiGraphics graphics, int mouseX, int mouseY, float delta) {
        // Dim backdrop so the boxes read clearly over the world/title.
        graphics.fill(0, 0, this.width, this.height, 0x60000000);
        for (HudEditLogic.EditBlock b : logic.blocks()) {
            boolean hot = b == logic.dragging() || b.contains(mouseX, mouseY);
            // Box: 50% black fill + accent outline while hovered/dragged.
            graphics.fill(b.x, b.y, b.x + b.w, b.y + b.h, 0x80000000);
            int outline = hot ? 0xFFFFD700 : 0x80FFFFFF;
            graphics.fill(b.x, b.y, b.x + b.w, b.y + 1, outline);
            graphics.fill(b.x, b.y + b.h - 1, b.x + b.w, b.y + b.h, outline);
            int rowTop = b.y + HudLayout.PAD;
            for (HudEngine.Row row : b.rows) {
                int rowH = HudEditLogic.rowHeight(row);
                int iconW = 0;
                if (row.icon != null) {
                    int iconSize = row.icon.size();
                    Compat.drawIcon(graphics, this.minecraft, row.icon,
                            b.x + HudLayout.PAD, rowTop + (rowH - iconSize) / 2);
                    iconW = iconSize + 3;
                }
                int textY = rowTop + (rowH - 9) / 2;
                graphics.drawString(this.font, row.text, b.x + HudLayout.PAD + iconW, textY, 0xFFFFFFFF, true);
                rowTop += rowH;
            }
            // Feature label above the box.
            graphics.drawString(this.font, b.label, b.x, b.y - 10, 0xFFAAAAAA, false);
        }
        // Hint line.
        graphics.drawCenteredString(this.font,
                Component.translatable("espectral.hudedit.hint"),
                this.width / 2, this.height - 44, 0xFFCCCCCC);
        super.render(graphics, mouseX, mouseY, delta);
    }

    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean doubleClick) {
        if (event.button() == 0 && logic.mouseDown(event.x(), event.y())) {
            return true;
        }
        if (event.button() == 1 && logic.resetAt(event.x(), event.y(), this.minecraft, this.font, this.width, this.height)) {
            return true;
        }
        return super.mouseClicked(event, doubleClick);
    }

    @Override
    public boolean mouseDragged(MouseButtonEvent event, double dx, double dy) {
        if (logic.isDragging()) {
            logic.mouseMove(event.x(), event.y());
            return true;
        }
        return super.mouseDragged(event, dx, dy);
    }

    @Override
    public boolean mouseReleased(MouseButtonEvent event) {
        if (logic.isDragging()) {
            logic.mouseUp(this.width, this.height);
            return true;
        }
        return super.mouseReleased(event);
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }

    @Override
    public void onClose() {
        Minecraft.getInstance().setScreen(parent);
    }
}
