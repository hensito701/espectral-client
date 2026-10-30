package es.spectral.menu.agent;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;
import net.minecraft.client.KeyMapping;

import es.spectral.menu.EspectralClient;

/**
 * F8 pause: the human's kill switch for agent input.
 *
 * <p>It is user-only by construction. The binding ({@link EspectralClient#AGENT_PAUSE_KEY})
 * is a real {@code KeyMapping}, and pause state is derived from
 * {@code KeyMapping.isDown()} edge transitions computed on the client tick from
 * the OS keyboard state — never from anything the agent's {@code key} tool
 * sends. Injected keys go through {@code KeyboardHandler}, which does not
 * update {@code KeyMapping.isDown()}; the tool handler additionally refuses to
 * route the pause binding at all. An agent therefore cannot unpause itself.
 *
 * <p>While paused every {@link AgentTier#ACT} tool fails with {@code PAUSED};
 * observe and capture tools keep working (they are the human's override for
 * "let me see what it is doing").
 */
public final class AgentPause {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    private static volatile boolean paused;
    private static boolean lastDown;
    /** True once the binding reported a real press; used for the HUD chip. */
    private static volatile long lastToggleAt;

    private AgentPause() {}

    public static boolean paused() {
        return paused;
    }

    /**
     * Samples the F8 binding on the client tick and flips on a fresh press.
     * Cheap when agent mode is off ({@link AgentRuntime#enabled()} gate lives
     * in the caller).
     */
    public static void onTick(Minecraft minecraft) {
        KeyMapping key = EspectralClient.AGENT_PAUSE_KEY;
        if (key == null) return;
        boolean down = false;
        try {
            down = key.isDown();
        } catch (Throwable t) {
            return;
        }
        if (down && !lastDown) {
            paused = !paused;
            lastToggleAt = System.currentTimeMillis();
            LOGGER.info("agent: {}", paused ? "PAUSED (F8)" : "ACTIVE (F8)");
        }
        lastDown = down;
    }

    /** Milliseconds of the last human toggle (0 when never pressed). */
    public static long lastToggleAt() {
        return lastToggleAt;
    }

    /**
     * HUD chip text, or null when nothing should be drawn. Only the paused
     * state is shown: an always-on "AGENT ACTIVE" badge would end up in
     * captures the agent takes.
     */
    public static String indicator() {
        return paused ? "AGENT PAUSED (F8)" : null;
    }
}
