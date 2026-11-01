package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.MouseHandler;
import net.minecraft.client.input.MouseButtonInfo;

import es.spectral.menu.ui.MenuBall;

/**
 * Feeds raw mouse buttons to the menu ball before vanilla dispatches them, so
 * the ball wins over any button or list row underneath it. Every lane
 * declares {@code onButton(long, MouseButtonInfo, int)} (private on 1.21.11
 * and 26.2, public on 26.3); a press that grabs the ball and its release are
 * cancelled, everything else runs untouched.
 */
@Mixin(MouseHandler.class)
public abstract class MenuBallInputMixin {

    @Shadow
    @Final
    private Minecraft minecraft;

    @Inject(
            method = "onButton(JLnet/minecraft/client/input/MouseButtonInfo;I)V",
            at = @At("HEAD"),
            cancellable = true
    )
    private void espectral$menuBall(long window, MouseButtonInfo info, int action, CallbackInfo ci) {
        if (MenuBall.get().onMouseButton(minecraft, info.button(), action)) {
            ci.cancel();
        }
    }
}
