package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import com.mojang.blaze3d.vertex.PoseStack;

import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.EntityRendererProvider;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.RenderLayerParent;
import net.minecraft.client.renderer.entity.player.AvatarRenderer;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.resources.Identifier;

import es.spectral.menu.cosmetics.render.CosmeticHand;
import es.spectral.menu.cosmetics.render.CosmeticLayer;

/**
 * Registers {@link CosmeticLayer} on the avatar renderer and draws the own
 * player's runic wraps on the first-person hands.
 *
 * <p>Third person is a render layer added in the {@code AvatarRenderer}
 * constructor (the layer reads {@code PlayerModel} parts for its head/body/arm
 * attachments). First person hooks the private {@code renderHand} TAIL: the
 * arm part still carries the exact pose vanilla just submitted. Shared by both
 * lane mixin configs — the descriptors are identical in 26.2 and 1.21.11.
 */
@Mixin(AvatarRenderer.class)
public abstract class CosmeticLayerMixin {

    @Inject(
            method = "<init>(Lnet/minecraft/client/renderer/entity/EntityRendererProvider$Context;Z)V",
            at = @At("RETURN")
    )
    private void espectral$addCosmeticLayer(EntityRendererProvider.Context context, boolean slim, CallbackInfo ci) {
        LivingEntityRenderer<?, ?, ?> self = (LivingEntityRenderer<?, ?, ?>) (Object) this;
        @SuppressWarnings("unchecked")
        RenderLayerParent<AvatarRenderState, PlayerModel> parent =
                (RenderLayerParent<AvatarRenderState, PlayerModel>) self;
        ((LivingEntityLayerAccess) self).espectral$addLayer(new CosmeticLayer(parent));
    }

    @Inject(
            method = "renderHand(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/resources/Identifier;Lnet/minecraft/client/model/geom/ModelPart;Z)V",
            at = @At("TAIL")
    )
    private void espectral$firstPersonWrap(PoseStack poseStack, SubmitNodeCollector collector, int packedLight,
            Identifier skinId, ModelPart armPart, boolean showSleeve, CallbackInfo ci) {
        CosmeticHand.render(poseStack, collector, packedLight, armPart);
    }
}
