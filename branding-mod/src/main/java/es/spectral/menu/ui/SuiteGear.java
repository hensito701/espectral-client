package es.spectral.menu.ui;

import net.minecraft.resources.Identifier;

import es.spectral.menu.Compat;

/**
 * Paints the Suite's gear sprite over every settings button.
 *
 * <p>The buttons are vanilla {@code Button}s with an empty label, so a lane's
 * screen-paint mixin tails the base {@code render}/{@code extractRenderState}
 * and calls {@link #paintGearOverlays} while a {@link SuiteScreen} is up. The
 * sprite is drawn on a new GUI stratum, above the widget layer, so the button's
 * own frame and hover state stay visible underneath.
 *
 * <p>Lane-neutral by construction: it only talks to {@link SuiteScreen}, the
 * shared {@link SuiteTheme} metrics and {@code Compat.uiSprite}.
 */
public final class SuiteGear {

    /** {@code assets/espectral-menu/textures/gui/sprites/gear.png}. */
    private static final Identifier GEAR =
            Identifier.fromNamespaceAndPath("espectral-menu", "gear");

    /** Sprite edge inside the config button (1 px padding around 17 px). */
    private static final int SIZE = 12;

    private SuiteGear() {}

    /** Draws the cog on each visible settings button of {@code screen}. */
    public static void paintGearOverlays(Object gfx, SuiteScreen screen) {
        if (screen == null) {
            return;
        }
        Compat.uiNextLayer(gfx);
        for (int i = 0; i < screen.gearButtons().size(); i++) {
            net.minecraft.client.gui.components.Button button = screen.gearButtons().get(i);
            if (button == null || !button.visible) {
                continue;
            }
            int x = button.getX() + (button.getWidth() - SIZE) / 2;
            int y = button.getY() + (button.getHeight() - SIZE) / 2;
            int argb = button.active ? 0xFFFFFFFF : 0xFF6B7078;
            if (button.isHoveredOrFocused()) {
                argb = SuiteTheme.accentBright();
            }
            Compat.uiSprite(gfx, GEAR, x, y, SIZE, SIZE, argb);
        }
    }

    /** Convenience for a mixin that only has a screen instance in hand. */
    public static void paintFor(Object gfx, Object screen) {
        if (!(screen instanceof SuiteScreen suite)) {
            return;
        }
        paintGearOverlays(gfx, suite);
    }
}
