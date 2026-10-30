package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;

/**
 * Slot bounds for {@code screen_tree}: {@code leftPos}/{@code topPos} are
 * protected (and differ per version in whether they exist as fields at all),
 * so the agent reads them through an accessor.
 */
@Mixin(AbstractContainerScreen.class)
public interface ContainerScreenAccess {

    @Accessor("leftPos")
    int espectral$leftPos();

    @Accessor("topPos")
    int espectral$topPos();
}
