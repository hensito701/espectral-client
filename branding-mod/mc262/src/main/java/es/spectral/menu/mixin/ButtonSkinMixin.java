package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractButton;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.util.Util;

import es.spectral.menu.ui.MenuSkin;

/**
 * Out-of-world menus: vanilla's button sprite becomes the title screen's
 * dark-glass button ({@code MenuSkin#paintButton}) with an eased hover.
 * Hooks the shared default-sprite helper, so plain buttons and cycle
 * buttons are covered while buttons with their own sprites keep them; the
 * label is still drawn by vanilla.
 */
@Mixin(AbstractButton.class)
public abstract class ButtonSkinMixin {

    @Unique
    private float espectral$hover;
    @Unique
    private long espectral$hoverMs;

    @Inject(method = "extractDefaultSprite(Lnet/minecraft/client/gui/GuiGraphicsExtractor;)V", at = @At("HEAD"), cancellable = true)
    private void espectral$glassButton(GuiGraphicsExtractor gfx, CallbackInfo ci) {
        if (!MenuSkin.skinWidgets()) {
            return;
        }
        AbstractWidget widget = (AbstractWidget) (Object) this;
        long now = Util.getMillis();
        float dt = espectral$hoverMs == 0L ? 1f : Math.min(0.1f, (now - espectral$hoverMs) / 1000f);
        espectral$hoverMs = now;
        boolean enabled = widget.isActive();
        espectral$hover = MenuSkin.easeHover(espectral$hover, enabled && widget.isHoveredOrFocused(), dt);
        MenuSkin.paintButton(gfx, widget.getX(), widget.getY(), widget.getWidth(), widget.getHeight(),
                enabled, espectral$hover, widget.getAlpha());
        ci.cancel();
    }
}
