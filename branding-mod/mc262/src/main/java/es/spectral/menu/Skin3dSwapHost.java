package es.spectral.menu;

import net.minecraft.client.model.EntityModel;

/**
 * Per-renderer state the 3D skin swap needs, implemented by
 * {@code es.spectral.menu.mixin.AvatarRendererMixin} and read by
 * {@code es.spectral.menu.mixin.LivingEntityRendererSubmitMixin}.
 *
 * <p>The swap spans two mixin classes because Mixin can only inject into
 * methods its own target declares: {@code submit} belongs to
 * {@code LivingEntityRenderer}, while {@code renderRightHand} /
 * {@code renderLeftHand} and the constructor belong to {@code AvatarRenderer}.
 * Both transforms land on the same renderer instance, so the avatar mixin —
 * which also captures the constructor {@code slim} flag — owns the state and
 * the submit mixin reaches it through this interface.</p>
 *
 * <p>Lives outside {@code es.spectral.menu.mixin} on purpose: Mixin owns that
 * package and refuses to transform any class in it that is not a registered
 * mixin, so a plain interface there throws {@code IllegalClassLoadError} the
 * first time a renderer loads — which aborts the resource reload and makes the
 * client reload everything a second time.</p>
 */
public interface Skin3dSwapHost {

    /** Vanilla model saved while a swap is active; {@code null} when none is. */
    EntityModel<?> espectral$voxelBackup();

    void espectral$voxelBackup(EntityModel<?> model);

    /** Constructor {@code slim} flag, the fallback for states without a model type. */
    boolean espectral$slimModel();
}
