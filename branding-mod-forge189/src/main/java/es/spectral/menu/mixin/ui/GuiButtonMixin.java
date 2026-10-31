package es.spectral.menu.mixin.ui;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiButton;

import es.spectral.menu.ui.Gfx;
import es.spectral.menu.ui.MenuSkin;
import es.spectral.menu.ui.SuiteTheme;

/**
 * Out-of-world vanilla buttons get the title screen's dark glass (eased
 * hover, accent edge and bar). {@code mouseDragged} still runs in its vanilla
 * place, so option sliders keep dragging and drawing their knob.
 */
@Mixin(GuiButton.class)
public abstract class GuiButtonMixin {

    @Shadow public int width;
    @Shadow public int height;
    @Shadow public int xPosition;
    @Shadow public int yPosition;
    @Shadow public String displayString;
    @Shadow public boolean enabled;
    @Shadow public boolean visible;
    @Shadow protected boolean hovered;
    @Shadow(remap = false) public int packedFGColour;

    @Shadow protected abstract void mouseDragged(Minecraft mc, int mouseX, int mouseY);

    @Unique private float espectral$hover;
    @Unique private long espectral$lastMs = -1;

    @Inject(method = "drawButton", at = @At("HEAD"), cancellable = true)
    private void espectral$skin(Minecraft mc, int mouseX, int mouseY, CallbackInfo ci) {
        if (!visible || !MenuSkin.skinWidgets()) return;
        hovered = mouseX >= xPosition && mouseY >= yPosition && mouseX < xPosition + width && mouseY < yPosition + height;
        long now = Minecraft.getSystemTime();
        float dt = espectral$lastMs < 0 ? 0f : Math.min(0.1f, (now - espectral$lastMs) / 1000f);
        espectral$lastMs = now;
        espectral$hover = MenuSkin.easeHover(espectral$hover, hovered && enabled, dt);
        MenuSkin.paintButton(xPosition, yPosition, width, height, enabled, espectral$hover, 1f);
        mouseDragged(mc, mouseX, mouseY);
        int color = packedFGColour != 0 ? packedFGColour | 0xFF000000
                : !enabled ? SuiteTheme.OFF
                : SuiteTheme.lerpArgb(SuiteTheme.TEXT, SuiteTheme.accentBright(), espectral$hover);
        Gfx.textCentered(displayString, xPosition + width / 2, yPosition + (height - 8) / 2, color, true);
        ci.cancel();
    }
}
