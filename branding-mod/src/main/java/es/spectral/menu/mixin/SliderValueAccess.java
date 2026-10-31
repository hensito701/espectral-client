package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.gui.components.AbstractSliderButton;

/**
 * Slider value for {@code screen_tree} ({@code AbstractSliderButton.value} has
 * no public getter, and the label text alone loses precision).
 */
@Mixin(AbstractSliderButton.class)
public interface SliderValueAccess {

    @Accessor("value")
    double espectral$value();
}
