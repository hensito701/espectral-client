package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.entity.AbstractClientPlayer;
import net.minecraft.client.model.ModelPlayer;
import net.minecraft.client.renderer.entity.RenderPlayer;

import es.spectral.menu.Skin3d;

/**
 * 3D skin layers: right before the player's body renders (after vanilla set
 * the wear visibility), the flat wear parts are hidden when the voxel layer
 * will draw them. First-person arms go through {@code renderRightArm}, which
 * resets visibility itself, so they stay vanilla.
 */
@Mixin(RenderPlayer.class)
public abstract class RenderPlayerMixin {

    @Inject(method = "doRender(Lnet/minecraft/client/entity/AbstractClientPlayer;DDDFF)V",
            at = @At(value = "INVOKE", shift = At.Shift.BEFORE,
                    target = "Lnet/minecraft/client/renderer/entity/RendererLivingEntity;doRender(Lnet/minecraft/entity/EntityLivingBase;DDDFF)V"))
    private void espectral$hideFlatWear(AbstractClientPlayer player, double x, double y, double z, float yaw,
            float partialTicks, CallbackInfo ci) {
        if (!Skin3d.replacesWear(player)) return;
        ModelPlayer model = ((RenderPlayer) (Object) this).getMainModel();
        model.bipedHeadwear.showModel = false;
        model.bipedBodyWear.showModel = false;
        model.bipedLeftArmwear.showModel = false;
        model.bipedRightArmwear.showModel = false;
        model.bipedLeftLegwear.showModel = false;
        model.bipedRightLegwear.showModel = false;
    }
}
