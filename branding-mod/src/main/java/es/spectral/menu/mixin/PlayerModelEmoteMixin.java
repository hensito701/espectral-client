package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;

import es.spectral.menu.EmoteEngine;

/**
 * Emote pose hook, shared across both lanes: applies the active local emote
 * to the avatar model at TAIL of {@code setupAnim(AvatarRenderState)} — the
 * same "pose after setupAnim" point the skin3d feature uses for its manual
 * {@code voxel.setupAnim(avatar)} calls.
 *
 * <p>This must live on the model, not on {@code LivingEntityRenderer#submit}:
 * the deferred submit pipeline re-runs {@code setupAnim} at draw time inside
 * {@code ModelFeatureRenderer#prepareModel} (verified by javap on both
 * lanes), so anything posing inside {@code submit} would be overwritten
 * before rasterisation. A TAIL hook here covers both the submit-time call
 * and the draw-time call. {@link EmoteEngine#pose} itself is gated on the
 * local player's entity id, so peer avatars are never posed.</p>
 */
@Mixin(PlayerModel.class)
public abstract class PlayerModelEmoteMixin {

    @Inject(
            method = "setupAnim(Lnet/minecraft/client/renderer/entity/state/AvatarRenderState;)V",
            at = @At("TAIL")
    )
    private void espectral$emotePose(AvatarRenderState state, CallbackInfo ci) {
        EmoteEngine.getInstance().pose((PlayerModel) (Object) this, state);
    }
}
