package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiButton;

/**
 * Dark-glass button used by every Espectral screen: the title-screen glass
 * (eased hover, accent edge, accent bar from the left edge), centred label,
 * optional tooltip. A real {@link GuiButton} so vanilla input, narration-free
 * 1.8.9 focus and the agent {@code screen_tree}/{@code click} tools see it.
 */
public class SuiteButton extends GuiButton {

    /** Hover text shown by the owning screen ({@link #drawTooltips}); null = none. */
    public String tooltip;
    /** Selected look (active category tab): accent text, no hover bar. */
    public boolean selected;
    private float hover;
    private long lastDrawMs = -1;

    public SuiteButton(int id, int x, int y, int w, int h, String label) {
        super(id, x, y, w, h, label);
    }

    @Override
    public void drawButton(Minecraft mc, int mouseX, int mouseY) {
        if (!this.visible) return;
        this.hovered = mouseX >= this.xPosition && mouseY >= this.yPosition
                && mouseX < this.xPosition + this.width && mouseY < this.yPosition + this.height;
        long now = Minecraft.getSystemTime();
        float dt = lastDrawMs < 0 ? 0f : Math.min(0.1f, (now - lastDrawMs) / 1000f);
        lastDrawMs = now;
        hover = MenuSkin.easeHover(hover, this.hovered && this.enabled, dt);
        MenuSkin.paintButton(xPosition, yPosition, width, height, this.enabled || selected,
                selected ? 0.35f : hover, 1f);
        int color = !this.enabled && !selected ? SuiteTheme.OFF
                : selected ? SuiteTheme.accentBright()
                : SuiteTheme.lerpArgb(SuiteTheme.TEXT, SuiteTheme.accentBright(), hover);
        String label = Gfx.font().trimStringToWidth(this.displayString, Math.max(0, this.width - 6));
        Gfx.textCentered(label, xPosition + width / 2, yPosition + (height - 8) / 2, color, true);
    }

    /** Draws the tooltip of whichever visible SuiteButton is hovered. */
    public static void drawTooltips(java.util.List<GuiButton> buttons, int mx, int my, int w, int h) {
        for (GuiButton b : buttons) {
            if (b instanceof SuiteButton && b.visible && b.isMouseOver() && ((SuiteButton) b).tooltip != null) {
                Gfx.tooltip(((SuiteButton) b).tooltip, mx, my, w, h);
                return;
            }
        }
    }
}
