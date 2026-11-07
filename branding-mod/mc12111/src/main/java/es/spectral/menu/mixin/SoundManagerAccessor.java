package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.sounds.SoundEngine;
import net.minecraft.client.sounds.SoundManager;

/**
 * Read-only window onto the sound engine behind {@code SoundManager} (the
 * field is private and has no getter). The Media Player needs it to reach the
 * streaming channel pool; see {@link SoundEngineAccessor}.
 */
@Mixin(SoundManager.class)
public interface SoundManagerAccessor {

    @Accessor("soundEngine")
    SoundEngine espectral$soundEngine();
}
