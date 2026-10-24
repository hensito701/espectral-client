package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.entity.EntityRendererProvider;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.RenderLayerParent;
import net.minecraft.client.renderer.entity.player.AvatarRenderer;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;

import es.spectral.menu.cosmetics.render.CosmeticLayer;

/**
 * Registers {@link CosmeticLayer} on the avatar renderer.
 *
 * <p>The layer is added in the {@code AvatarRenderer} constructor and reads
 * {@code PlayerModel} parts for its head/body attachments. Shared by both lane
 * mixin configs — the descriptor is identical in 26.2 and 1.21.11.
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
}
