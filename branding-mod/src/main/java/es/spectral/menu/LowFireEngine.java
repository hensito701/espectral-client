package es.spectral.menu;

/**
 * Low-fire overlay (Contract A "lowfire", owned/live feature).
 *
 * While enabled, the per-version screen-effects mixin shifts the vanilla
 * burning overlay downward in view space (a push/translate/pop around the
 * vanilla fire render), so the flames block less of the view. When disabled
 * the mixin performs no pose change and rendering is vanilla-identical.
 */
public final class LowFireEngine {

    private static final LowFireEngine INSTANCE = new LowFireEngine();

    public static LowFireEngine getInstance() {
        return INSTANCE;
    }

    private LowFireEngine() {}

    /**
     * Downward shift applied to the fire overlay in overlay view units (the
     * vanilla quad spans roughly -0.5..0.5 with -Y toward the bottom of the
     * screen, so this pushes the flames toward the bottom edge).
     */
    public static final float FIRE_Y_OFFSET = -0.25f;

    public boolean isEnabled() {
        return ClientConfig.getInstance().isFeatureEnabled("lowfire");
    }
}
