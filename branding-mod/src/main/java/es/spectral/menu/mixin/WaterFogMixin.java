package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Camera;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.fog.FogData;
import net.minecraft.client.renderer.fog.environment.WaterFogEnvironment;

import es.spectral.menu.ClearWaterEngine;

/**
 * Pushes water fog far while clearwater is enabled. The
 * {@code WaterFogEnvironment#setupFog} signature is identical in both
 * supported versions, so this shared mixin serves both mixins configs.
 * {@code FogData} is rebuilt per frame, so gating inside
 * {@link ClearWaterEngine#applyWaterFog} keeps the disabled path
 * vanilla-identical with no capture/restore needed.
 */
@Mixin(WaterFogEnvironment.class)
public abstract class WaterFogMixin {

    @Inject(
            method = "setupFog(Lnet/minecraft/client/renderer/fog/FogData;Lnet/minecraft/client/Camera;Lnet/minecraft/client/multiplayer/ClientLevel;FLnet/minecraft/client/DeltaTracker;)V",
            at = @At("RETURN")
    )
    private void espectral$pushWaterFogFar(FogData fogData, Camera camera, ClientLevel level,
            float renderDistance, DeltaTracker deltaTracker, CallbackInfo ci) {
        ClearWaterEngine.getInstance().applyWaterFog(fogData);
    }
}
