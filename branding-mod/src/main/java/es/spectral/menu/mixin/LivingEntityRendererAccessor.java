package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Mutable;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.model.EntityModel;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;

/**
 * Accesses the model owned by {@link LivingEntityRenderer}. AvatarRenderer
 * inherits this field, so shadowing it from an AvatarRenderer mixin fails at
 * runtime even though javac accepts the source.
 */
@Mixin(LivingEntityRenderer.class)
public interface LivingEntityRendererAccessor {

    @Accessor("model")
    EntityModel<?> espectral$getModel();

    @Mutable
    @Accessor("model")
    void espectral$setModel(EntityModel<?> model);
}
