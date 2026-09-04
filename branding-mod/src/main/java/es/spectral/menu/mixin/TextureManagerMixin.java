package es.spectral.menu.mixin;

import java.util.concurrent.CompletableFuture;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import es.spectral.menu.SkinModelCache;
import net.minecraft.client.renderer.texture.TextureManager;

/**
 * Drops baked 3D Skin Layers voxel models on resource reload (F3+T, pack
 * changes). Skin {@code DynamicTexture} pixel storage can be replaced across a
 * reload, so per-skin baked geometry must not outlive it; entries rebuild
 * lazily on the next render. The {@code reload} signature is identical in both
 * supported versions, so this shared mixin serves both mixins configs.
 */
@Mixin(TextureManager.class)
public abstract class TextureManagerMixin {

    @Inject(method = "reload", at = @At("HEAD"))
    private void espectral$clearSkinCache(CallbackInfoReturnable<CompletableFuture<Void>> cir) {
        SkinModelCache.clear();
    }
}
