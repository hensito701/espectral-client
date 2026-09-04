package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.renderer.fog.FogRenderer;

/**
 * Read/write access to vanilla's fog switch ({@code fogEnabled}, the flag the
 * vanilla F3+F debug {@code toggleFog()} flips). Lets {@code FogEngine} hold
 * distance fog off while the owned nofog feature is enabled and restore the
 * exact prior state on disable. The field exists with this name in both
 * supported versions, so this shared accessor serves both mixins configs.
 */
@Mixin(FogRenderer.class)
public interface FogRendererAccessor {

    @Accessor("fogEnabled")
    static boolean espectral$getFogEnabled() {
        throw new AssertionError();
    }

    @Accessor("fogEnabled")
    static void espectral$setFogEnabled(boolean value) {
        throw new AssertionError();
    }
}
