package es.spectral.menu.ui;

import net.minecraft.client.gui.Font;

/**
 * A Suite screen that draws the shared dark-glass sheet behind its widgets.
 *
 * <p>The shared screens stay free of any lane-specific graphics type: each
 * lane's background mixin detects this interface and forwards its own
 * graphics object here. 26.2 hooks {@code Screen#extractBackground}, 1.21.11
 * hooks {@code Screen#renderBackground} — both run before the widget list is
 * drawn, so the sheet is always the backdrop and never covers a control.
 */
public interface SuiteSheet {

    /**
     * Paints the sheet (scrim, panel, frame hairlines) and this screen's
     * static labels. {@code gfx} is opaque: the 26.2 extractor or the
     * 1.21.11 {@code GuiGraphics}.
     */
    void paintSheet(Object gfx, int width, int height, Font font);
}
