package es.spectral.menu.mixin.agent;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.gui.inventory.GuiContainer;

/** Container origin for the {@code screen_tree} slot coordinates. */
@Mixin(GuiContainer.class)
public interface GuiContainerAccessor {

    @Accessor("guiLeft")
    int espectral$guiLeft();

    @Accessor("guiTop")
    int espectral$guiTop();
}
