package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import com.mojang.blaze3d.vertex.PoseStack;

import es.spectral.menu.Compat;
import es.spectral.menu.Skin3dSwap;
import es.spectral.menu.Skin3dSwapHost;
import net.minecraft.client.model.EntityModel;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.EntityRendererProvider;
import net.minecraft.client.renderer.entity.player.AvatarRenderer;
import net.minecraft.resources.Identifier;

/**
 * 26.2: 3D Skin Layers — first-person hand half, plus the renderer state the
 * whole feature shares: the constructor {@code slim} flag and the vanilla
 * {@code model} saved while a swap is active ({@link Skin3dSwapHost}).
 *
 * <p>Third person is injected by {@link LivingEntityRendererSubmitMixin},
 * because {@code submit} is declared on {@code LivingEntityRenderer} and Mixin
 * only injects into methods its own target class declares.</p>
 *
 * <p>First person: the same swap wraps {@code AvatarRenderer#renderRightHand}
 * and {@code #renderLeftHand}; the private {@code renderHand} then poses the
 * voxel arm itself (resetPose, sleeve visibility from the showSleeve flag,
 * slight zRot), so no manual posing is needed there.</p>
 *
 * <p>Everything is gated on the {@code skin3d} feature inside the Compat
 * resolvers: when off (or the skin texture is not ready) the resolvers return
 * {@code null}, no swap happens, and rendering is vanilla-identical.</p>
 */
@Mixin(AvatarRenderer.class)
public abstract class AvatarRendererMixin implements Skin3dSwapHost {

    /** Vanilla model saved on swap; {@code null} when no swap is active. */
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

    @Override
    @Unique
    public EntityModel<?> espectral$voxelBackup() {
        return this.espectral$voxelBackup;
    }

    @Override
    @Unique
    public void espectral$voxelBackup(EntityModel<?> model) {
        this.espectral$voxelBackup = model;
    }

    @Override
    @Unique
    public boolean espectral$slimModel() {
        return this.espectral$slimModel;
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
        Skin3dSwap.in(this, voxel);
    }

    @Inject(
            method = "renderRightHand(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/resources/Identifier;Z)V",
            at = @At("TAIL")
    )
    private void espectral$rightHandTail(PoseStack poseStack, SubmitNodeCollector collector,
            int packedLight, Identifier skinId, boolean showSleeve, CallbackInfo ci) {
        Skin3dSwap.out(this);
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
        Skin3dSwap.in(this, voxel);
    }

    @Inject(
            method = "renderLeftHand(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/resources/Identifier;Z)V",
            at = @At("TAIL")
    )
    private void espectral$leftHandTail(PoseStack poseStack, SubmitNodeCollector collector,
            int packedLight, Identifier skinId, boolean showSleeve, CallbackInfo ci) {
        Skin3dSwap.out(this);
    }
}
