package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.network.chat.Component;

import es.spectral.menu.TitleMenu;

/**
 * 26.2 Espectral main menu. Owns the Screen lifecycle and delegates
 * {@code init}/{@code render}/{@code mouseClicked} to the shared
 * {@link TitleMenu} controller, so all layout, art direction and click
 * handling live in one place and compile against both supported versions.
 *
 * <p>26.2 renders screens through {@code extractRenderState}; the vanilla
 * panorama/dirt background is suppressed by leaving
 * {@code extractBackground} empty because {@link TitleMenu#render} draws our
 * own backdrop.
 */
public class EspectralTitleScreen extends Screen {

    private final TitleMenu menu = new TitleMenu(Minecraft.getInstance(), this);

    public EspectralTitleScreen() {
        super(Component.translatable("espectral.menu.title"));
    }

    @Override
    protected void init() {
        super.init();
        menu.init(this.width, this.height);
    }

    @Override
    public void extractRenderState(GuiGraphicsExtractor gfx, int mouseX, int mouseY, float delta) {
        menu.render(gfx, this.font, mouseX, mouseY, delta);
        super.extractRenderState(gfx, mouseX, mouseY, delta);
    }

    @Override
    public void extractBackground(GuiGraphicsExtractor gfx, int mouseX, int mouseY, float delta) {
        // Suppress the vanilla panorama/dirt; menu.render draws our background.
    }

    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean dbl) {
        if (menu.mouseClicked(event.x(), event.y(), event.button())) return true;
        return super.mouseClicked(event, dbl);
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }
}
