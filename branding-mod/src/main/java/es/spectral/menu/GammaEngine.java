package es.spectral.menu;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;

/**
 * Native fullbright (Contract A "fullbright", owned/live feature).
 *
 * While the feature is enabled, the vanilla gamma option is forced to
 * {@link #FULLBRIGHT_GAMMA} every client tick — without calling
 * {@code Options#save()}, so no disk write happens per tick. The user's prior
 * value is captured once per session on the first override and restored
 * exactly on the enabled-&gt;disabled transition (persisted once via
 * {@code Options#save()}).
 *
 * When the feature is off and was never overridden this session, vanilla
 * state is never touched: the default-off path is behavior-identical to an
 * unmodded client. Driven from the client tick ({@code MinecraftMixin}); the
 * tick and the options live on the main client thread, so plain fields need
 * no synchronization.
 */
public final class GammaEngine {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    /** Gamma forced while fullbright is on (vanilla slider caps at 1.0). */
    public static final double FULLBRIGHT_GAMMA = 15.0;

    private static final GammaEngine INSTANCE = new GammaEngine();

    public static GammaEngine getInstance() {
        return INSTANCE;
    }

    private GammaEngine() {}

    /** User's pre-override gamma; null until the first override of the session. */
    private Double capturedGamma;

    /**
     * Polled every client tick via MinecraftMixin: forces fullbright gamma
     * while enabled, restores the captured value once on disable.
     */
    public void onTick(Minecraft minecraft) {
        if (minecraft == null || minecraft.options == null) return;
        if (!ClientConfig.getInstance().isFeatureEnabled("fullbright")) {
            if (capturedGamma != null) {
                double restored = capturedGamma;
                capturedGamma = null;
                minecraft.options.gamma().set(restored);
                minecraft.options.save();
                LOGGER.info("Fullbright off: restored gamma to {}", restored);
            }
            return;
        }
        if (capturedGamma == null) {
            capturedGamma = minecraft.options.gamma().get();
            LOGGER.info("Fullbright on: captured prior gamma {}", capturedGamma);
        }
        minecraft.options.gamma().set(FULLBRIGHT_GAMMA);
    }
}
