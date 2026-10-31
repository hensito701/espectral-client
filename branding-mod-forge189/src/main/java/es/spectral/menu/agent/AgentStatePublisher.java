package es.spectral.menu.agent;

import net.minecraft.client.Minecraft;

/**
 * Publishes the current {@link AgentState} from the client tick, so the relay's
 * state thread never touches client data structures. Same 500 ms sampling
 * contract as the modern mod: what matters is when the phase changes, and the
 * engine waits in 250 ms steps.
 */
public final class AgentStatePublisher {

    private static volatile AgentState latest;

    private AgentStatePublisher() {}

    public static void sample(Minecraft minecraft) {
        try {
            latest = AgentStates.snapshot(minecraft);
        } catch (Throwable t) {
            // a state sample must never disturb the game
        }
    }

    /** Last sample (null before the first tick after startup). */
    public static AgentState current() {
        return latest;
    }
}
