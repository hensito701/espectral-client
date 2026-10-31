package es.spectral.menu;

import net.minecraft.client.Minecraft;

/**
 * Hold-to-zoom ({@code zoom}, default Z): while the key is held the world FOV
 * eases toward {@link #TARGET_SCALE} with a frame-rate independent
 * exponential lerp and vanilla smooth camera is engaged; on release it eases
 * back and the previous smooth-camera setting is restored exactly. Same
 * constants as the Fabric mod. The tick decides engagement, the FOV hook
 * ({@code EntityRendererFeatureMixin}) reads {@link #fovScale()} every frame.
 */
public final class ZoomEngine {

    public static final float TARGET_SCALE = 0.25f;
    private static final float EASE_SPEED = 14.0f;
    private static final float EPSILON = 0.001f;

    private static boolean engaged;
    private static float scale = 1.0f;
    private static long lastFrameNanos;
    private static boolean smoothBackup;
    private static boolean smoothOwned;

    private ZoomEngine() {}

    /** Client tick: engagement + smooth-camera ownership. */
    public static void onTick(Minecraft mc) {
        engaged = mc.thePlayer != null && mc.currentScreen == null
                && ClientConfig.getInstance().isFeatureEnabled("zoom")
                && Keys.ZOOM != null && Keys.ZOOM.isKeyDown();
        if (engaged && !smoothOwned) {
            smoothBackup = mc.gameSettings.smoothCamera;
            mc.gameSettings.smoothCamera = true;
            smoothOwned = true;
        } else if (!engaged && smoothOwned) {
            mc.gameSettings.smoothCamera = smoothBackup;
            smoothOwned = false;
        }
    }

    /** FOV multiplier for this frame (1 = untouched); advances the ease on real time. */
    public static float fovScale() {
        long now = System.nanoTime();
        float dt = lastFrameNanos == 0L ? 0f : Math.min((now - lastFrameNanos) / 1.0e9f, 0.1f);
        lastFrameNanos = now;
        float target = engaged ? TARGET_SCALE : 1.0f;
        if (Math.abs(target - scale) <= EPSILON) {
            scale = target;
        } else if (dt > 0f) {
            scale += (target - scale) * (1.0f - (float) Math.exp(-EASE_SPEED * dt));
        }
        return scale;
    }
}
