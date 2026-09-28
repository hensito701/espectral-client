package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

import es.spectral.menu.cosmetics.render.CosmeticsTick;

/**
 * Drives the cosmetics state service and the per-wearer cloth simulation on
 * the client tick, and drops the sim state when leaving a world. Shared by both
 * lane mixin configs ({@code tick()V} and {@code disconnect(Screen;ZZ)V} have
 * the same descriptors in 26.2 and 1.21.11).
 */
@Mixin(Minecraft.class)
public abstract class CosmeticsTickMixin {

    @Inject(method = "tick()V", at = @At("TAIL"))
    private void espectral$cosmeticsTick(CallbackInfo ci) {
        CosmeticsTick.onClientTick((Minecraft) (Object) this);
    }

    @Inject(
            method = "disconnect(Lnet/minecraft/client/gui/screens/Screen;ZZ)V",
            at = @At("TAIL")
    )
    private void espectral$cosmeticsDisconnect(net.minecraft.client.gui.screens.Screen screen,
            boolean showDisconnectScreen, boolean teleporting, CallbackInfo ci) {
        CosmeticsTick.onDisconnect();
    }
}
