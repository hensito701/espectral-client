package es.spectral.menu;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import es.spectral.menu.mixin.FogRendererAccessor;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.fog.FogRenderer;

/**
 * Native no-fog (Contract A "nofog", owned/live feature).
 *
 * While the feature is enabled, vanilla's own fog switch
 * ({@link FogRenderer} {@code fogEnabled}, the same flag the vanilla F3+F
 * debug {@code toggleFog()} flips) is held off every client tick. The user's
 * prior state is captured once per session on the first override and restored
 * exactly on the enabled-&gt;disabled transition.
 *
 * When the feature is off and was never overridden this session, vanilla
 * state is never touched: the default-off path is behavior-identical to an
 * unmodded client. Driven from the client tick ({@code MinecraftMixin}); the
 * tick and rendering live on the main client thread, so plain fields need no
 * synchronization.
 */
public final class FogEngine {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    private static final FogEngine INSTANCE = new FogEngine();

    public static FogEngine getInstance() {
        return INSTANCE;
    }

    private FogEngine() {}

    /** Vanilla fog state before our first override; null until then. */
    private Boolean capturedFogEnabled;

    /**
     * Polled every client tick via MinecraftMixin: state-ensures fog off
     * while enabled, restores the captured state once on disable.
     */
    public void onTick(Minecraft minecraft) {
        if (minecraft == null) return;
        if (!ClientConfig.getInstance().isFeatureEnabled("nofog")) {
            if (capturedFogEnabled != null) {
                boolean restored = capturedFogEnabled;
                capturedFogEnabled = null;
                FogRendererAccessor.espectral$setFogEnabled(restored);
                LOGGER.info("NoFog off: restored vanilla fog to {}", restored ? "on" : "off");
            }
            return;
        }
        if (capturedFogEnabled == null) {
            capturedFogEnabled = FogRendererAccessor.espectral$getFogEnabled();
            LOGGER.info("NoFog on: captured prior vanilla fog {}",
                    capturedFogEnabled ? "on" : "off");
        }
        if (FogRendererAccessor.espectral$getFogEnabled()) {
            FogRendererAccessor.espectral$setFogEnabled(false);
        }
    }
}
