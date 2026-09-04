package es.spectral.menu;

import net.minecraft.client.renderer.fog.FogData;

/**
 * Clear-water rendering (Contract A "clearwater", owned/live feature).
 *
 * While enabled, the per-version screen-effects mixin cancels the vanilla
 * underwater overlay, and {@link #applyWaterFog(FogData)} pushes the
 * {@code WaterFogEnvironment} distances far out every frame. {@code FogData}
 * is rebuilt per frame by vanilla, so there is no persistent state to capture
 * or restore: when disabled this method is never called and water rendering
 * is vanilla-identical.
 */
public final class ClearWaterEngine {

    private static final ClearWaterEngine INSTANCE = new ClearWaterEngine();

    public static ClearWaterEngine getInstance() {
        return INSTANCE;
    }

    private ClearWaterEngine() {}

    /** Fog starts here while clearwater is on (vanilla water starts ~0). */
    public static final float WATER_FOG_START = 64.0f;
    /** Fog ends here while clearwater is on (beyond max view distance). */
    public static final float WATER_FOG_END = 1024.0f;

    public boolean isEnabled() {
        return ClientConfig.getInstance().isFeatureEnabled("clearwater");
    }

    /**
     * Pushes the water fog distances far. Called at the tail of
     * {@code WaterFogEnvironment#setupFog}; a no-op unless enabled.
     */
    public void applyWaterFog(FogData fogData) {
        if (fogData == null || !isEnabled()) {
            return;
        }
        fogData.environmentalStart = WATER_FOG_START;
        fogData.environmentalEnd = WATER_FOG_END;
    }
}
