package es.spectral.menu.media;

import javax.sound.sampled.AudioFormat;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;
import net.minecraft.client.sounds.ChannelAccess;
import net.minecraft.sounds.SoundSource;

import es.spectral.menu.mixin.SoundEngineAccessor;
import es.spectral.menu.mixin.SoundManagerAccessor;

/**
 * Output through the game's already-initialised OpenAL context, via a
 * dedicated streaming channel of vanilla's own {@code ChannelAccess} pool.
 *
 * <p>Why this shape: every AL call must happen on the sound engine thread,
 * and the game already pumps streaming channels there
 * ({@code SoundEngine.tick} → {@code ChannelAccess.scheduleTick} →
 * {@code Channel.updateStream}). By acquiring our own handle from that same
 * pool we get the game's device, its resampling and its master volume for
 * free, with zero OpenAL calls from the client or decode threads.
 *
 * <p>Survives reloads: {@code SoundEngine.destroy()} (F3+T, device change,
 * emergency shutdown) clears the pool and destroys every handle — including
 * ours — so {@link #tick} notices a stopped handle and re-acquires one,
 * resuming at the current decode position (the ring and decoder are untouched).
 */
public final class MediaAudio {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-media");
    private static final MediaAudio INSTANCE = new MediaAudio();

    /** Fallback when the decoder could not report a format (nothing plays then anyway). */
    private static final AudioFormat FALLBACK_FORMAT = new AudioFormat(44100.0F, 16, 2, true, false);

    private static final int STATE_IDLE = 0;
    private static final int STATE_REQUESTED = 1;
    private static final int STATE_ACTIVE = 2;

    private MediaRing ring;
    private AudioFormat format = FALLBACK_FORMAT;
    private ChannelAccess.ChannelHandle handle;
    private int state = STATE_IDLE;
    private boolean paused;
    private float volume = 0.7F;
    private float appliedVolume = -1.0F;
    private boolean ducked;

    private MediaAudio() {}

    public static MediaAudio get() {
        return INSTANCE;
    }

    /** Starts (or restarts) streaming the current ring on the game's context. */
    public synchronized void startStreaming(MediaRing ring, MediaDecoder decoder, float volume) {
        this.ring = ring;
        this.format = decoder == null || decoder.format() == null
                ? FALLBACK_FORMAT : decoder.format();
        this.volume = volume;
        this.paused = false;
        this.state = STATE_REQUESTED;
        this.appliedVolume = -1.0F;
        acquire();
    }

    private void acquire() {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null || minecraft.getSoundManager() == null) {
            return;
        }
        try {
            net.minecraft.client.sounds.SoundEngine engine =
                    ((SoundManagerAccessor) minecraft.getSoundManager()).espectral$soundEngine();
            if (engine == null) {
                return;
            }
            ChannelAccess access = ((SoundEngineAccessor) engine).espectral$channelAccess();
            if (access == null) {
                return;
            }
            access.createHandle(com.mojang.blaze3d.audio.Library.Pool.STREAMING)
                    .thenAccept(created -> {
                        synchronized (MediaAudio.this) {
                            if (state != STATE_REQUESTED || created == null) {
                                if (created != null) {
                                    created.release();
                                }
                                return;
                            }
                            this.handle = created;
                            this.state = STATE_ACTIVE;
                            created.execute(channel -> {
                                channel.attachBufferStream(new MediaStream(this.ring, this.format));
                                channel.setRelative(true);
                                channel.disableAttenuation();
                                channel.setVolume(effectiveVolume());
                                channel.play();
                                if (paused) {
                                    channel.pause();
                                }
                            });
                        }
                    });
        } catch (Throwable e) {
            LOGGER.warn("Could not acquire a streaming channel: {}", e.toString());
            this.state = STATE_IDLE;
        }
    }

    /** Master volume from the game's own options, multiplied by the media slider. */
    private float effectiveVolume() {
        Minecraft minecraft = Minecraft.getInstance();
        float master = 1.0F;
        if (minecraft != null && minecraft.options != null) {
            master = minecraft.options.getFinalSoundSourceVolume(SoundSource.MASTER);
        }
        return Math.max(0.0F, Math.min(1.0F, volume * master));
    }

    public synchronized void applyVolume(float value) {
        this.volume = value;
        pushVolume();
    }

    private void pushVolume() {
        ChannelAccess.ChannelHandle current = this.handle;
        if (current == null || state != STATE_ACTIVE) {
            return;
        }
        float want = effectiveVolume();
        if (Math.abs(want - appliedVolume) < 0.005F) {
            return;
        }
        appliedVolume = want;
        current.execute(channel -> channel.setVolume(want));
    }

    public synchronized void setPaused(boolean value) {
        this.paused = value;
        ChannelAccess.ChannelHandle current = this.handle;
        if (current == null || state != STATE_ACTIVE) {
            return;
        }
        current.execute(channel -> {
            if (value) {
                channel.pause();
            } else {
                channel.unpause();
            }
        });
    }

    public synchronized void stopStreaming() {
        ChannelAccess.ChannelHandle current = this.handle;
        this.handle = null;
        this.state = STATE_IDLE;
        this.appliedVolume = -1.0F;
        if (current != null) {
            try {
                current.release();
            } catch (Throwable e) {
                LOGGER.debug("Channel release failed: {}", e.toString());
            }
        }
    }

    /**
     * Vanilla background music is silenced while a track plays (setting, on by
     * default) and restored on pause/stop. This is the sound engine's per-source
     * gain map, an extra multiplier on top of the user's option volume (vanilla's
     * MusicManager uses it for fades), so the neutral restore value is 1.0 —
     * {@code options.txt} is never touched.
     */
    public void duckVanillaMusic(boolean duck) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null || minecraft.getSoundManager() == null) {
            return;
        }
        if (duck == ducked) {
            return;
        }
        ducked = duck;
        // gainBySource is an EXTRA multiplier on top of the user's own MUSIC
        // slider (vanilla multiplies both), so "restore" means 1.0; restoring
        // the slider value would square the user's music volume.
        minecraft.getSoundManager().updateCategoryVolume(SoundSource.MUSIC, duck ? 0.0F : 1.0F);
    }

    /** Client tick: detects a destroyed/reloaded sound engine and recovers. */
    public synchronized void tick(Minecraft minecraft) {
        if (state == STATE_IDLE || minecraft == null || minecraft.getSoundManager() == null) {
            return;
        }
        if (state == STATE_REQUESTED) {
            return;
        }
        ChannelAccess.ChannelHandle current = this.handle;
        if (current == null || current.isStopped()) {
            // F3+T / device change / shutdown destroyed the pool: rebuild.
            if (current != null) {
                try {
                    current.release();
                } catch (Throwable ignored) {
                    // already gone
                }
            }
            this.handle = null;
            this.state = STATE_REQUESTED;
            this.appliedVolume = -1.0F;
            acquire();
            return;
        }
        pushVolume();
        if (ducked != (MediaPlayer.get().isPlaying() && !MediaPlayer.get().isPaused()
                && MediaPlayer.get().duckMusic())) {
            duckVanillaMusic(MediaPlayer.get().isPlaying() && !MediaPlayer.get().isPaused()
                    && MediaPlayer.get().duckMusic());
        }
    }
}
