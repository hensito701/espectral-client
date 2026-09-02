package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

import es.spectral.menu.EspectralClient;
import es.spectral.menu.MacroEngine;

import es.spectral.menu.ZoomEngine;

/**
 * Hooks into Minecraft.tick() to handle the Right Shift GUI keybind, the
 * native zoom, and macro triggers.
 */
@Mixin(Minecraft.class)
public abstract class MinecraftMixin {

    @Inject(method = "tick()V", at = @At("TAIL"))
    private void espectral$onClientTick(CallbackInfo ci) {
        EspectralClient.ensureClientKeyRegistered();
        ZoomEngine.getInstance().onTick((Minecraft) (Object) this);
        MacroEngine.getInstance().onTick((Minecraft) (Object) this);
    }
}
