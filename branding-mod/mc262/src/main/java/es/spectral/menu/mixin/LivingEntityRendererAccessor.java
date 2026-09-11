package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.model.EntityModel;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;

/**
 * Read/write access to {@code LivingEntityRenderer#model} — the field
 * {@link AvatarRendererMixin} swaps around the 3D-skin voxel model.
 *
 * <p>Mixin resolves {@code @Shadow} fields against the mixin's own target
 * class only, never its superclasses, and 26.2 declares {@code model} here
 * (typed {@code M}, erasure {@code EntityModel}) while the injection points
 * live in {@code AvatarRenderer}. Shadowing it in {@code AvatarRendererMixin}
 * therefore fails every launch with "&#64;Shadow field model was not located in
 * the target class … No refMap loaded", which aborts the mixin's whole
 * transform (3D skin layers render vanilla) and forces the client into a
 * second resource reload (~1 s of boot time the menu marker never sees). The
 * accessor has to sit on the declaring class.</p>
 */
@Mixin(LivingEntityRenderer.class)
public interface LivingEntityRendererAccessor {

    @Accessor("model")
    EntityModel<?> espectral$getModel();

    @Accessor("model")
    void espectral$setModel(EntityModel<?> model);
}
