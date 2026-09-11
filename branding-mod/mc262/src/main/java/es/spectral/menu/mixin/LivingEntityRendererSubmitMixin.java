package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import com.mojang.blaze3d.vertex.PoseStack;

import es.spectral.menu.Compat;
import es.spectral.menu.Skin3dSwap;
import es.spectral.menu.Skin3dSwapHost;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.minecraft.client.renderer.state.level.CameraRenderState;

/**
 * 26.2: 3D Skin Layers, third-person half — swaps the per-skin voxel model in
 * around {@code LivingEntityRenderer#submit}.
 *
 * <p>{@code submit} is declared here, not on {@code AvatarRenderer}, and Mixin
 * only injects into methods its own target class declares — so this injection
 * cannot live in {@link AvatarRendererMixin}, which owns the state and the
 * first-person hand path ({@link Skin3dSwapHost}). Non-avatar renderers bail
 * out immediately: the transform is inert for every other living entity, and
 * the voxel model is only ever built for avatar render states.</p>
 */
@Mixin(LivingEntityRenderer.class)
public abstract class LivingEntityRendererSubmitMixin {

    @Inject(
            method = "submit(Lnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/client/renderer/state/level/CameraRenderState;)V",
            at = @At("HEAD")
    )
    private void espectral$submitHead(LivingEntityRenderState state, PoseStack poseStack,
            SubmitNodeCollector collector, CameraRenderState camera, CallbackInfo ci) {
        if (!Skin3dSwap.canSwap(this)) return;
        Skin3dSwapHost host = (Skin3dSwapHost) (Object) this;
        host.espectral$voxelBackup(null);
        if (!(state instanceof AvatarRenderState avatar)) return;
        PlayerModel voxel = Compat.voxelForState(avatar, host.espectral$slimModel());
        if (voxel == null) return;
        Skin3dSwap.in(this, voxel);
        voxel.resetPose();
        voxel.setupAnim(avatar);
    }

    @Inject(
            method = "submit(Lnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/client/renderer/state/level/CameraRenderState;)V",
            at = @At("TAIL")
    )
    private void espectral$submitTail(LivingEntityRenderState state, PoseStack poseStack,
            SubmitNodeCollector collector, CameraRenderState camera, CallbackInfo ci) {
        if (!Skin3dSwap.canSwap(this)) return;
        Skin3dSwap.out(this);
    }
}
