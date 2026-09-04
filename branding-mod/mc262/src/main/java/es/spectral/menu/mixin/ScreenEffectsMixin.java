package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import com.mojang.blaze3d.vertex.PoseStack;

import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.ScreenEffectRenderer;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;

import es.spectral.menu.ClearWaterEngine;
import es.spectral.menu.LowFireEngine;

/**
 * 26.2: vanilla submits the burning overlay in
 * {@code ScreenEffectRenderer#submitFire} and the underwater overlay in
 * {@code ScreenEffectRenderer#submitWater} (both private static; Mixin can
 * still hook them). LowFire wraps the fire submit in a push/translate/pop so
 * the flames sit lower; ClearWater cancels the water submit. Disabled paths
 * do nothing, keeping rendering vanilla-identical.
 */
@Mixin(ScreenEffectRenderer.class)
public abstract class ScreenEffectsMixin {

    @Inject(
            method = "submitFire(Lcom/mojang/blaze3d/vertex/PoseStack;"
                    + "Lnet/minecraft/client/renderer/SubmitNodeCollector;"
                    + "Lnet/minecraft/client/renderer/texture/TextureAtlasSprite;)V",
            at = @At("HEAD")
    )
    private static void espectral$lowerFireHead(PoseStack poseStack, SubmitNodeCollector collector,
            TextureAtlasSprite sprite, CallbackInfo ci) {
        if (LowFireEngine.getInstance().isEnabled()) {
            poseStack.pushPose();
            poseStack.translate(0.0f, LowFireEngine.FIRE_Y_OFFSET, 0.0f);
        }
    }

    @Inject(
            method = "submitFire(Lcom/mojang/blaze3d/vertex/PoseStack;"
                    + "Lnet/minecraft/client/renderer/SubmitNodeCollector;"
                    + "Lnet/minecraft/client/renderer/texture/TextureAtlasSprite;)V",
            at = @At("TAIL")
    )
    private static void espectral$lowerFireTail(PoseStack poseStack, SubmitNodeCollector collector,
            TextureAtlasSprite sprite, CallbackInfo ci) {
        if (LowFireEngine.getInstance().isEnabled()) {
            poseStack.popPose();
        }
    }

    @Inject(
            method = "submitWater(Lnet/minecraft/client/Minecraft;"
                    + "Lcom/mojang/blaze3d/vertex/PoseStack;"
                    + "Lnet/minecraft/client/renderer/SubmitNodeCollector;)V",
            at = @At("HEAD"),
            cancellable = true
    )
    private static void espectral$clearWaterHead(Minecraft minecraft, PoseStack poseStack,
            SubmitNodeCollector collector, CallbackInfo ci) {
        if (ClearWaterEngine.getInstance().isEnabled()) {
            ci.cancel();
        }
    }
}
