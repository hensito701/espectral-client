package es.spectral.menu;

import net.minecraft.client.Minecraft;

/**
 * Native hold-to-zoom (Contract A "zoom", owned/live feature).
 *
 * While the zoom key is held (and the feature is enabled), {@link #fovScale()}
 * eases the camera FOV multiplier toward {@link #TARGET_SCALE} with a
 * frame-rate-independent exponential lerp, and vanilla smooth-camera is
 * engaged for cinematic panning. On release everything eases back and the
 * previous smooth-camera setting is restored exactly.
 *
 * State machine driven from the client tick ({@code MinecraftMixin}); the
 * value is consumed on the render thread by the per-version FOV mixin
 * ({@code GameRendererFovMixin} / {@code CameraFovMixin}). Both hooks run on
 * the main client thread, so plain fields need no synchronization.
 */
public final class ZoomEngine {

    /** FOV multiplier while fully zoomed (vanilla runs at 1.0 = 70deg base). */
    public static final float TARGET_SCALE = 0.25f;

    /** Higher = snappier ease; ~14 reaches 95% of the way in ~210ms. */
    private static final float EASE_SPEED = 14.0f;
    private static final float EPSILON = 0.001f;
    private static final long MAX_FRAME_NANOS = 100_000_000L; // clamp pauses to 100ms

    private static final ZoomEngine INSTANCE = new ZoomEngine();

    public static ZoomEngine getInstance() {
        return INSTANCE;
    }

    private ZoomEngine() {}

    private volatile boolean engaged;
    private float scale = 1.0f;
    private long lastFrameNanos;
    private boolean smoothCameraBackup;
    private boolean smoothCameraOwned;

    /**
     * Polled every client tick via MinecraftMixin: refreshes the engagement
     * flag and owns the smooth-camera option while zooming.
     */
    public void onTick(Minecraft minecraft) {
        engaged = isEngaged(minecraft);
        if (engaged && !smoothCameraOwned) {
            smoothCameraBackup = minecraft.options.smoothCamera;
            minecraft.options.smoothCamera = true;
            smoothCameraOwned = true;
        } else if (!engaged && smoothCameraOwned) {
            minecraft.options.smoothCamera = smoothCameraBackup;
            smoothCameraOwned = false;
        }
    }

    /**
     * Current FOV multiplier for the render hook: 1.0 = untouched,
     * {@link #TARGET_SCALE} = fully zoomed. Call every frame; advances the
     * ease using real elapsed time so the animation is refresh-rate agnostic.
     */
    public float fovScale() {
        long now = System.nanoTime();
        float dt = lastFrameNanos == 0L ? 0.0f : Math.min((now - lastFrameNanos) / 1.0e9f, 0.1f);
        lastFrameNanos = now;

        float target = engaged ? TARGET_SCALE : 1.0f;
        if (Math.abs(target - scale) <= EPSILON) {
            scale = target;
        } else if (dt > 0.0f) {
            // Frame-rate independent exponential approach.
            scale += (target - scale) * (1.0f - (float) Math.exp(-EASE_SPEED * dt));
        }
        return scale;
    }

    private static boolean isEngaged(Minecraft minecraft) {
        return minecraft != null
                && minecraft.player != null
                && !Compat.isScreenOpen(minecraft)
                && ClientConfig.getInstance().isFeatureEnabled("zoom")
                && EspectralClient.ZOOM_KEY != null
                && EspectralClient.ZOOM_KEY.isDown();
    }
}
