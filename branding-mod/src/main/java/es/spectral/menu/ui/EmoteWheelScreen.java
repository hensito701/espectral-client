package es.spectral.menu.ui;

import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.network.chat.Component;

import es.spectral.menu.Compat;
import es.spectral.menu.EmoteEngine;
import es.spectral.menu.EspectralClient;

/**
 * Radial emote wheel (default key {@code B}, category Espectral): five petal
 * slots arranged around a hub in the Suite dark-glass + gold palette
 * ({@link SuiteTheme} tokens). Hover highlights, left-click picks, Esc or
 * clicking the hub/background closes without starting anything. Re-pressing
 * the emote key closes the wheel too.
 *
 * <p>Drawn entirely through {@code Compat.ui*} primitives from
 * {@link #paint(Object, int, int, int, int, net.minecraft.client.gui.Font)}
 * so this shared class never names a lane-specific graphics type; each lane
 * hooks its screen-render path ({@code extractRenderState} on 26.2,
 * {@code render} on 1.21.11) to call it.</p>
 */
public class EmoteWheelScreen extends Screen {

    /** Wheel radius and hub radius in logical px. */
    private static final int RADIUS = 72;
    private static final int HUB_R = 14;
    private static final int PETAL_W = 72;
    private static final int PETAL_H = 16;

    public EmoteWheelScreen() {
        super(Component.translatable("espectral.emotes.wheel.title"));
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }

    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean doubleClick) {
        if (event.button() == com.mojang.blaze3d.platform.InputConstants.MOUSE_BUTTON_LEFT) {
            String pick = pick(event.x(), event.y());
            if (pick != null) {
                EmoteEngine.getInstance().start(pick);
            }
            this.onClose();
            return true;
        }
        return super.mouseClicked(event, doubleClick);
    }

    @Override
    public boolean keyPressed(KeyEvent event) {
        // Re-pressing the emote key with the wheel open closes it.
        if (EspectralClient.EMOTE_KEY != null && EspectralClient.EMOTE_KEY.matches(event)) {
            this.onClose();
            return true;
        }
        return super.keyPressed(event);
    }

    @Override
    public void onClose() {
        // Stamp the close so a queued emote-key click this tick cannot
        // instantly re-open the wheel (EmoteEngine suppression window).
        EmoteEngine.getInstance().noteWheelClosed();
        super.onClose();
    }

    /**
     * Hit test for the radial slots: returns the emote id under the pointer
     * or {@code null} for the hub/background (click = close, no selection).
     * Segment 0 starts at screen "up" and runs clockwise.
     */
    public String pick(double mouseX, double mouseY) {
        double dx = mouseX - this.width / 2.0;
        double dy = mouseY - this.height / 2.0;
        double dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < HUB_R || dist > RADIUS + 8) return null;
        // Angle 0 = up, increasing clockwise.
        double angle = Math.atan2(dx, -dy);
        if (angle < 0) angle += Math.PI * 2.0;
        int idx = (int) ((angle / (Math.PI * 2.0)) * EmoteEngine.EMOTES.length + 0.5)
                % EmoteEngine.EMOTES.length;
        return EmoteEngine.EMOTES[idx];
    }

    /**
     * Paints the scrim, five petal boxes arranged radially (segment 0 at
     * screen "up", clockwise), spokes, the gold hub, and the emote names.
     * {@code gfx} is opaque (26.2 extractor, 1.21.11 graphics).
     */
    public void paint(Object gfx, int width, int height, int mouseX, int mouseY,
            net.minecraft.client.gui.Font font) {
        int cx = width / 2;
        int cy = height / 2;
        Compat.uiFill(gfx, 0, 0, width, height, SuiteTheme.SCRIM);

        String hovered = pick(mouseX, mouseY);
        String[] ids = EmoteEngine.EMOTES;
        int n = ids.length;
        for (int i = 0; i < n; i++) {
            double angle = i * (Math.PI * 2.0 / n);
            int px = cx + (int) (RADIUS * Math.sin(angle));
            int py = cy - (int) (RADIUS * Math.cos(angle));
            boolean hot = ids[i].equals(hovered);
            Compat.uiFill(gfx, px - PETAL_W / 2, py - PETAL_H / 2,
                    px + PETAL_W / 2, py + PETAL_H / 2,
                    hot ? SuiteTheme.ROW_HOVER : SuiteTheme.ROW);
            Compat.uiOutline(gfx, px - PETAL_W / 2, py - PETAL_H / 2,
                    PETAL_W, PETAL_H,
                    hot ? SuiteTheme.accentBright() : SuiteTheme.accentHairline());
            // Spoke from hub edge to petal.
            drawLine(gfx,
                    cx + (int) (HUB_R * Math.sin(angle)),
                    cy - (int) (HUB_R * Math.cos(angle)),
                    cx + (int) ((RADIUS - PETAL_H / 2) * Math.sin(angle)),
                    cy - (int) ((RADIUS - PETAL_H / 2) * Math.cos(angle)),
                    SuiteTheme.accentHairline());
            Compat.uiTextCentered(gfx, font,
                    Component.translatable("espectral.emote." + ids[i]),
                    px, py - 4, hot ? SuiteTheme.accentBright() : SuiteTheme.TEXT, false);
        }

        // Hub + title/hint.
        Compat.uiFill(gfx, cx - HUB_R, cy - HUB_R, cx + HUB_R, cy + HUB_R, SuiteTheme.PANEL);
        Compat.uiOutline(gfx, cx - HUB_R, cy - HUB_R, HUB_R * 2, HUB_R * 2,
                SuiteTheme.panelEdge());
        Compat.uiTextCenteredGradient(gfx, font, this.title, cx,
                cy - RADIUS - PETAL_H - 14,
                SuiteTheme.accentGradientLeft(), SuiteTheme.accentGradientRight(), false);
        Compat.uiTextCentered(gfx, font,
                Component.translatable("espectral.emotes.wheel.hint"),
                cx, cy + RADIUS + PETAL_H + 6, SuiteTheme.TEXT_DIM, false);
    }

    /** Bresenham-ish hairline built from 1 px fills (no line primitive). */
    private static void drawLine(Object gfx, int x0, int y0, int x1, int y1, int argb) {
        int steps = Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0));
        for (int i = 0; i <= steps; i++) {
            int x = x0 + (x1 - x0) * i / Math.max(1, steps);
            int y = y0 + (y1 - y0) * i / Math.max(1, steps);
            Compat.uiFill(gfx, x, y, x + 1, y + 1, argb);
        }
    }
}
