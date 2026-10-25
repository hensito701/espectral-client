package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Camera;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.fog.FogData;
import net.minecraft.client.renderer.fog.environment.AtmosphericFogEnvironment;

import es.spectral.menu.FogEngine;

/**
 * Pushes atmospheric fog far while nofog is enabled ({@link FogEngine}). The
 * {@code AtmosphericFogEnvironment#setupFog} signature is identical in both
 * supported versions (checked with javap), so this shared mixin serves both
 * mixins configs, like {@code WaterFogMixin}.
 */
@Mixin(AtmosphericFogEnvironment.class)
public abstract class AtmosphericFogMixin {

    @Inject(
            method = "setupFog(Lnet/minecraft/client/renderer/fog/FogData;Lnet/minecraft/client/Camera;Lnet/minecraft/client/multiplayer/ClientLevel;FLnet/minecraft/client/DeltaTracker;)V",
            at = @At("RETURN")
    )
    private void espectral$pushAtmosphericFogFar(FogData fogData, Camera camera, ClientLevel level,
            float renderDistance, DeltaTracker deltaTracker, CallbackInfo ci) {
        FogEngine.applyAtmosphericFog(fogData);
    }
}
