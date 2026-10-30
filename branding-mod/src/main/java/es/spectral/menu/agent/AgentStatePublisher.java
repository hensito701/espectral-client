package es.spectral.menu.agent;

import net.minecraft.client.Minecraft;

/**
 * Publishes the current {@link AgentState} from the client tick, so the relay's
 * state thread can post it without touching the render thread.
 *
 * <p>Ticked every 500 ms what matters is <em>when it changes</em>; a 0.5 s
 * sampling lag is well inside the engine's expectations (it waits in 250 ms
 * steps) and costs one small object per sample, not per frame.
 */
public final class AgentStatePublisher {

    private static volatile AgentState latest;

    private AgentStatePublisher() {}

    public static void sample(Minecraft minecraft) {
        try {
            latest = AgentStates.snapshot(minecraft);
        } catch (Throwable t) {
            // A state sample must never disturb the game.
        }
    }

    /** Last sample (null before the first tick after startup). */
    public static AgentState current() {
        return latest;
    }
}
