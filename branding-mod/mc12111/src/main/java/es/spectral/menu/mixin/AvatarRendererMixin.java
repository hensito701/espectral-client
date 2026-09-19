package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
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
import net.minecraft.resources.Identifier;

/**
 * 1.21.11: captures slim-arm construction state and swaps the cached voxel
 * model around AvatarRenderer's first-person hand methods. Third-person
 * submission is inherited from LivingEntityRenderer and is handled by
 * {@link LivingEntityRendererMixin} on the class that owns that method.
 */
@Mixin(AvatarRenderer.class)
public abstract class AvatarRendererMixin {

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

    @Unique
    private void espectral$swapIn(PlayerModel voxel) {
        LivingEntityRendererAccessor renderer = (LivingEntityRendererAccessor) (Object) this;
        this.espectral$voxelBackup = renderer.espectral$getModel();
        renderer.espectral$setModel(voxel);
    }

    @Unique
    private void espectral$swapOut() {
        if (this.espectral$voxelBackup != null) {
            ((LivingEntityRendererAccessor) (Object) this)
                    .espectral$setModel(this.espectral$voxelBackup);
            this.espectral$voxelBackup = null;
        }
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
