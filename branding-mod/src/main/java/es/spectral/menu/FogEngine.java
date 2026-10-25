package es.spectral.menu;

import net.minecraft.client.renderer.fog.FogData;

/**
 * Native no-fog (Contract A "nofog", owned/live feature).
 *
 * <p>While enabled, {@link #applyAtmosphericFog(FogData)} pushes the
 * atmospheric fog (the Overworld/Nether/End distance haze, rain fog, sky and
 * cloud fade) beyond any view distance at the tail of
 * {@code AtmosphericFogEnvironment#setupFog}. It edits the per-frame
 * {@code FogData} — the same path clear water uses — so Sodium/Iris terrain
 * honours it; the old approach of flipping vanilla's global {@code fogEnabled}
 * switch only affected vanilla's own fog buffer and did nothing under Sodium.
 *
 * <p>Gameplay fogs (blindness, darkness, lava, powder snow, underwater — the
 * latter owned by clear water) are separate environments and stay vanilla, as
 * does the short fade at the render-distance edge that hides chunk loading.
 * {@code FogData} is rebuilt every frame, so there is nothing to restore.
 */
public final class FogEngine {

    /** Fog distances while no-fog is on: past the 32-chunk (512 block) maximum. */
    public static final float FAR = 1024.0f;

    private FogEngine() {}

    public static boolean isEnabled() {
        return ClientConfig.getInstance().isFeatureEnabled("nofog");
    }

    /** Called at the tail of {@code AtmosphericFogEnvironment#setupFog}; no-op unless enabled. */
    public static void applyAtmosphericFog(FogData fog) {
        if (fog == null || !isEnabled()) {
            return;
        }
        fog.environmentalStart = FAR;
        fog.environmentalEnd = FAR;
        fog.skyEnd = FAR;
        fog.cloudEnd = FAR;
    }
}
