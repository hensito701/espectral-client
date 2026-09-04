package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import com.mojang.blaze3d.vertex.PoseStack;

import es.spectral.menu.Compat;
import net.minecraft.client.model.EntityModel;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.EntityRendererProvider;
import net.minecraft.client.renderer.entity.player.AvatarRenderer;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.resources.Identifier;

/**
 * 26.2: 3D Skin Layers — swaps the per-skin voxel model in around
 * {@code LivingEntityRenderer#submit} and the first-person hand path.
 *
 * <p>Third person: on {@code submit} HEAD the renderer {@code model} field is
 * pointed at the cached voxel model ({@link Compat#voxelForState}), which is
 * then {@code resetPose()} + {@code setupAnim(state)} — vanilla already posed
 * the original during extraction, so without this the voxel instance would
 * render in T-pose. Per-player {@code showHat}/{@code showJacket}/{@code
 * showLeftSleeve}/... flags keep working because {@code PlayerModel#setupAnim}
 * flows them into the voxel parts' visibility. TAIL restores the original.</p>
 *
 * <p>First person: the same swap wraps {@code AvatarRenderer#renderRightHand}
 * and {@code #renderLeftHand}; the private {@code renderHand} then poses the
 * voxel arm itself (resetPose, sleeve visibility from the showSleeve flag,
 * slight zRot), so no manual posing is needed there.</p>
 *
 * <p>Everything is gated on the {@code skin3d} feature inside the Compat
 * resolvers: when off (or the skin texture is not ready) the resolvers return
 * {@code null}, no swap happens, and rendering is vanilla-identical.</p>
 *
 * <p>Only the {@code CameraRenderState} package differs from 1.21.11
 * ({@code renderer.state.level} here vs {@code renderer.state} there).</p>
 */
@Mixin(AvatarRenderer.class)
public abstract class AvatarRendererMixin {

    /** Vanilla model saved on swap; {@code null} when no swap is active. */
    @Shadow
    protected EntityModel<?> model;

    @Unique
    private EntityModel<?> espectral$voxelBackup;

    @Unique
    private boolean espectral$slimModel;

    @Inject(
            method = "<init>(Lnet/minecraft/client/renderer/entity/EntityRendererProvider$Context;Z)V",
            at = @At("RETURN")
    )
    private void espectral$captureSlim(EntityRendererProvider.Context context, boolean slim, CallbackInfo ci) {
        this.espectral$slimModel = slim;
    }

    @Unique
    private void espectral$swapIn(PlayerModel voxel) {
        this.espectral$voxelBackup = this.model;
        this.model = voxel;
    }

    @Unique
    private void espectral$swapOut() {
        if (this.espectral$voxelBackup != null) {
            this.model = this.espectral$voxelBackup;
            this.espectral$voxelBackup = null;
        }
    }

    @Inject(
            method = "submit(Lnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/client/renderer/state/level/CameraRenderState;)V",
            at = @At("HEAD")
    )
    private void espectral$submitHead(LivingEntityRenderState state, PoseStack poseStack,
            SubmitNodeCollector collector, CameraRenderState camera, CallbackInfo ci) {
        this.espectral$voxelBackup = null;
        if (!(state instanceof AvatarRenderState avatar)) return;
        PlayerModel voxel = Compat.voxelForState(avatar, this.espectral$slimModel);
        if (voxel == null) return;
        espectral$swapIn(voxel);
        voxel.resetPose();
        voxel.setupAnim(avatar);
    }

    @Inject(
            method = "submit(Lnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/client/renderer/state/level/CameraRenderState;)V",
            at = @At("TAIL")
    )
    private void espectral$submitTail(LivingEntityRenderState state, PoseStack poseStack,
            SubmitNodeCollector collector, CameraRenderState camera, CallbackInfo ci) {
        espectral$swapOut();
    }

    @Inject(
            method = "renderRightHand(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/resources/Identifier;Z)V",
            at = @At("HEAD")
    )
    private void espectral$rightHandHead(PoseStack poseStack, SubmitNodeCollector collector,
            int packedLight, Identifier skinId, boolean showSleeve, CallbackInfo ci) {
        this.espectral$voxelBackup = null;
        PlayerModel voxel = Compat.voxelForHand(skinId, this.espectral$slimModel);
        if (voxel == null) return;
        espectral$swapIn(voxel);
    }

    @Inject(
            method = "renderRightHand(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/resources/Identifier;Z)V",
            at = @At("TAIL")
    )
    private void espectral$rightHandTail(PoseStack poseStack, SubmitNodeCollector collector,
            int packedLight, Identifier skinId, boolean showSleeve, CallbackInfo ci) {
        espectral$swapOut();
    }

    @Inject(
            method = "renderLeftHand(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/resources/Identifier;Z)V",
            at = @At("HEAD")
    )
    private void espectral$leftHandHead(PoseStack poseStack, SubmitNodeCollector collector,
            int packedLight, Identifier skinId, boolean showSleeve, CallbackInfo ci) {
        this.espectral$voxelBackup = null;
        PlayerModel voxel = Compat.voxelForHand(skinId, this.espectral$slimModel);
        if (voxel == null) return;
        espectral$swapIn(voxel);
    }

    @Inject(
            method = "renderLeftHand(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/resources/Identifier;Z)V",
            at = @At("TAIL")
    )
    private void espectral$leftHandTail(PoseStack poseStack, SubmitNodeCollector collector,
            int packedLight, Identifier skinId, boolean showSleeve, CallbackInfo ci) {
        espectral$swapOut();
    }
}
