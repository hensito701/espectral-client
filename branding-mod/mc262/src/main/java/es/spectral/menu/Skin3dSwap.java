package es.spectral.menu;

import es.spectral.menu.mixin.LivingEntityRendererAccessor;
import net.minecraft.client.model.EntityModel;
import net.minecraft.client.model.player.PlayerModel;

/**
 * The swap itself: {@code LivingEntityRenderer#model} is exchanged for the
 * cached voxel model while a player is submitted and restored afterwards.
 *
 * <p>Field access goes through {@link LivingEntityRendererAccessor} — Mixin
 * resolves {@code @Shadow} fields against the mixin's own target class only,
 * and the field is declared on {@code LivingEntityRenderer}, not on
 * {@code AvatarRenderer}. Per-instance state lives in {@link Skin3dSwapHost}.
 * Both mixins share these helpers so the hand path and the submit path can
 * never drift apart.</p>
 */
public final class Skin3dSwap {

    private Skin3dSwap() {}

    /** True when the renderer carries both mixins and can therefore host a swap. */
    public static boolean canSwap(Object renderer) {
        return renderer instanceof Skin3dSwapHost && renderer instanceof LivingEntityRendererAccessor;
    }

    /** Points the renderer at {@code voxel}, remembering the vanilla model. */
    public static void in(Object renderer, PlayerModel voxel) {
        Skin3dSwapHost host = (Skin3dSwapHost) renderer;
        LivingEntityRendererAccessor accessor = (LivingEntityRendererAccessor) renderer;
        host.espectral$voxelBackup(accessor.espectral$getModel());
        accessor.espectral$setModel(voxel);
    }

    /** Restores the vanilla model after a swap; no-op when none is active. */
    public static void out(Object renderer) {
        Skin3dSwapHost host = (Skin3dSwapHost) renderer;
        EntityModel<?> backup = host.espectral$voxelBackup();
        if (backup != null) {
            ((LivingEntityRendererAccessor) renderer).espectral$setModel(backup);
            host.espectral$voxelBackup(null);
        }
    }
}
