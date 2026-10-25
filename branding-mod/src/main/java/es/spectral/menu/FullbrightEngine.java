package es.spectral.menu;

/**
 * Native fullbright (Contract A "fullbright", owned/live feature).
 *
 * <p>While enabled, the per-version lightmap mixin feeds the lightmap a full
 * night-vision factor — the same path the Night Vision effect uses — so caves
 * and nights render fully lit. The gamma option is never touched: vanilla
 * clamps it to 1.0 ("Bright"), which only lifts mid-tones and leaves dark
 * areas dark, and players who already sat at 1.0 saw no change at all.
 *
 * <p>The lightmap is recomputed every tick from scratch, so there is no state
 * to capture or restore: disabling the feature simply stops the override.
 * Gameplay darkening (the Darkness effect pulse, boss-fight world darkening)
 * is applied separately by vanilla and stays.
 */
public final class FullbrightEngine {

    /** Night-vision factor fed to the lightmap while fullbright is on. */
    public static final float NIGHT_VISION = 1.0f;

    private FullbrightEngine() {}

    public static boolean isEnabled() {
        return ClientConfig.getInstance().isFeatureEnabled("fullbright");
    }
}
