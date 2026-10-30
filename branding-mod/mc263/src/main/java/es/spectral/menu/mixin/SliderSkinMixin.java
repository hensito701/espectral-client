package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import com.mojang.renderpearl.api.pipeline.RenderPipeline;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractSliderButton;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.resources.Identifier;

import es.spectral.menu.ui.MenuSkin;

/**
 * Out-of-world menus: the slider track and handle sprites (the method's
 * only two {@code blitSprite} calls) become a glass track and a solid accent
 * handle; vanilla still positions both and draws the label.
 */
@Mixin(AbstractSliderButton.class)
public abstract class SliderSkinMixin {

    @Redirect(
            method = "extractWidgetRenderState(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIF)V",
            at = @At(value = "INVOKE",
                    target = "Lnet/minecraft/client/gui/GuiGraphicsExtractor;blitSprite(Lcom/mojang/renderpearl/api/pipeline/RenderPipeline;Lnet/minecraft/resources/Identifier;IIIII)V")
    )
    private void espectral$glassSlider(GuiGraphicsExtractor gfx, RenderPipeline pipeline, Identifier sprite,
            int x, int y, int width, int height, int color) {
        if (!MenuSkin.skinWidgets()) {
            gfx.blitSprite(pipeline, sprite, x, y, width, height, color);
            return;
        }
        AbstractWidget widget = (AbstractWidget) (Object) this;
        boolean handle = sprite.getPath().startsWith("widget/slider_handle");
        MenuSkin.paintSlider(gfx, x, y, width, height, handle, widget.isActive(),
                widget.isHoveredOrFocused(), (color >>> 24) / 255f);
    }
}
