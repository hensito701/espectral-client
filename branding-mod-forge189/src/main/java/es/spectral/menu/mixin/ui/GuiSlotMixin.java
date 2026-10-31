package es.spectral.menu.mixin.ui;

import org.lwjgl.opengl.GL11;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiSlot;
import net.minecraft.client.gui.ScaledResolution;
import net.minecraft.client.renderer.Tessellator;

import es.spectral.menu.ui.MenuSkin;

/**
 * Out-of-world lists (worlds, servers, resource packs…): a dark glass band
 * between accent hairlines instead of the dirt list texture, and the
 * header/footer masks repaint the menu art (scissored) instead of dirt.
 */
@Mixin(GuiSlot.class)
public abstract class GuiSlotMixin {

    @Shadow public int width;
    @Shadow public int height;
    @Shadow public int top;
    @Shadow public int bottom;
    @Shadow public int left;
    @Shadow public int right;

    @Inject(method = "drawContainerBackground", at = @At("HEAD"), cancellable = true, remap = false)
    private void espectral$glass(Tessellator tessellator, CallbackInfo ci) {
        if (!MenuSkin.skinWidgets()) return;
        MenuSkin.paintListGlass(left, top, right, bottom);
        ci.cancel();
    }

    @Inject(method = "overlayBackground", at = @At("HEAD"), cancellable = true)
    private void espectral$mask(int startY, int endY, int startAlpha, int endAlpha, CallbackInfo ci) {
        if (!MenuSkin.skinWidgets()) return;
        Minecraft mc = Minecraft.getMinecraft();
        ScaledResolution res = new ScaledResolution(mc);
        int scale = res.getScaleFactor();
        GL11.glEnable(GL11.GL_SCISSOR_TEST);
        GL11.glScissor(0, mc.displayHeight - endY * scale, mc.displayWidth, (endY - startY) * scale);
        MenuSkin.paintBackdrop(res.getScaledWidth(), res.getScaledHeight());
        GL11.glDisable(GL11.GL_SCISSOR_TEST);
        MenuSkin.hairline(left, startY == 0 ? endY - 1 : startY, right);
        ci.cancel();
    }
}
