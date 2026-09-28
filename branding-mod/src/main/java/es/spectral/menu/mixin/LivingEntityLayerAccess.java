package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.layers.RenderLayer;

/**
 * Exposes {@code LivingEntityRenderer#addLayer} to the avatar cosmetic
 * registration. Declared on the class that owns the method (an AvatarRenderer
 * mixin cannot reach it: Mixin resolves only against its own target class).
 * Shared by both lane mixin configs.
 */
@Mixin(LivingEntityRenderer.class)
public interface LivingEntityLayerAccess {

    @Invoker("addLayer")
    boolean espectral$addLayer(RenderLayer<?, ?> layer);
}
