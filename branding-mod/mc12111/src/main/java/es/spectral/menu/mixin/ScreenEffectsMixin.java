package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import com.mojang.blaze3d.vertex.PoseStack;

import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.ScreenEffectRenderer;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;

import es.spectral.menu.ClearWaterEngine;
import es.spectral.menu.LowFireEngine;

/**
 * 1.21.11: vanilla renders the burning overlay in
 * {@code ScreenEffectRenderer#renderFire} and the underwater overlay in
 * {@code ScreenEffectRenderer#renderWater} (both private static; Mixin can
 * still hook them). LowFire wraps the fire render in a push/translate/pop so
 * the flames sit lower; ClearWater cancels the water overlay. Disabled paths
 * do nothing, keeping rendering vanilla-identical.
 */
@Mixin(ScreenEffectRenderer.class)
public abstract class ScreenEffectsMixin {

    @Inject(
            method = "renderFire(Lcom/mojang/blaze3d/vertex/PoseStack;"
                    + "Lnet/minecraft/client/renderer/MultiBufferSource;"
                    + "Lnet/minecraft/client/renderer/texture/TextureAtlasSprite;)V",
            at = @At("HEAD")
    )
    private static void espectral$lowerFireHead(PoseStack poseStack, MultiBufferSource bufferSource,
            TextureAtlasSprite sprite, CallbackInfo ci) {
        if (LowFireEngine.getInstance().isEnabled()) {
            poseStack.pushPose();
            poseStack.translate(0.0f, LowFireEngine.FIRE_Y_OFFSET, 0.0f);
        }
    }

    @Inject(
            method = "renderFire(Lcom/mojang/blaze3d/vertex/PoseStack;"
                    + "Lnet/minecraft/client/renderer/MultiBufferSource;"
                    + "Lnet/minecraft/client/renderer/texture/TextureAtlasSprite;)V",
            at = @At("TAIL")
    )
    private static void espectral$lowerFireTail(PoseStack poseStack, MultiBufferSource bufferSource,
            TextureAtlasSprite sprite, CallbackInfo ci) {
        if (LowFireEngine.getInstance().isEnabled()) {
            poseStack.popPose();
        }
    }

    @Inject(
            method = "renderWater(Lnet/minecraft/client/Minecraft;"
                    + "Lcom/mojang/blaze3d/vertex/PoseStack;"
                    + "Lnet/minecraft/client/renderer/MultiBufferSource;)V",
            at = @At("HEAD"),
            cancellable = true
    )
    private static void espectral$clearWaterHead(Minecraft minecraft, PoseStack poseStack,
            MultiBufferSource bufferSource, CallbackInfo ci) {
        if (ClearWaterEngine.getInstance().isEnabled()) {
            ci.cancel();
        }
    }
}
