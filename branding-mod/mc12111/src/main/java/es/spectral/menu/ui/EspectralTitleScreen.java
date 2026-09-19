package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.network.chat.Component;

import es.spectral.menu.TitleMenu;

/**
 * Espectral replacement for the vanilla title screen (1.21.11 lane).
 * Owns the Screen lifecycle and delegates layout, drawing and clicks to the
 * shared {@link TitleMenu} controller.
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
    public void render(GuiGraphics gfx, int mouseX, int mouseY, float delta) {
        menu.render(gfx, this.font, mouseX, mouseY, delta);
        super.render(gfx, mouseX, mouseY, delta);
    }

    @Override
    public void renderBackground(GuiGraphics gfx, int mouseX, int mouseY, float delta) {
    }

    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean dbl) {
        if (menu.mouseClicked(event.x(), event.y(), event.button())) {
            return true;
        }
        return super.mouseClicked(event, dbl);
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }
}
