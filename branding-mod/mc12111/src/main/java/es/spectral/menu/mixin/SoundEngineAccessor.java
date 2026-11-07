package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.sounds.ChannelAccess;
import net.minecraft.client.sounds.SoundEngine;

/**
 * Read-only window onto the sound engine's streaming pool, used by the Media
 * Player to acquire its own channel (see {@code es.spectral.menu.media.MediaAudio}).
 *
 * <p>The field is private and there is no public getter; going through the game
 * pool — instead of creating OpenAL sources ourselves — is what keeps every AL
 * call on the sound engine thread and the game's chosen device.
 */
@Mixin(SoundEngine.class)
public interface SoundEngineAccessor {

    @Accessor("channelAccess")
    ChannelAccess espectral$channelAccess();
}
