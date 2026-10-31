package es.spectral.menu.agent;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;

/**
 * Per-tick agent housekeeping, called from the client tick when agent mode is
 * on: expire held-key leases, sample the F8 pause binding, release everything
 * on a screen/world transition and lazily start the relay.
 *
 * <p>Every method returns on a single volatile read when agent mode is off, so
 * a normal game pays nothing for this class.
 */
public final class AgentTick {

    /** Sample cadence: the engine polls state every 250 ms when waiting. */
    private static final int SAMPLE_EVERY_TICKS = 5;

    private static String lastScreen;
    private static boolean lastInWorld;
    private static boolean relayStarted;
    private static int sampleCountdown;

    private AgentTick() {}

    public static void onClientTick(Minecraft minecraft) {
        if (!AgentRuntime.enabled() || minecraft == null) return;
        if (!relayStarted) {
            relayStarted = true;
            AgentRelay.start();
            AgentPolicy.startResolver();
            AgentPolicy.noteServerFromArguments();
        }
        AgentPause.onTick(minecraft);
        AgentInput.tick(minecraft, System.currentTimeMillis());
        AgentReplay.onClientTick(minecraft);
        // Sampling the state here (not in the relay thread) keeps the client
        // data on the client thread; the relay only transports the snapshot.
        if (--sampleCountdown <= 0) {
            sampleCountdown = SAMPLE_EVERY_TICKS;
            AgentStatePublisher.sample(minecraft);
        }

        Screen screen = screenOf(minecraft);
        String screenName = screen != null ? screen.getClass().getName() : null;
        boolean inWorld = minecraft.level != null;
        // Opening/closing a screen and entering/leaving a world both drop held
        // keys (a stuck W after a world change is the classic failure).
        if (!equalsNullable(screenName, lastScreen) || inWorld != lastInWorld) {
            AgentInput.releaseAll(minecraft);
            lastScreen = screenName;
            lastInWorld = inWorld;
        }
    }

    private static boolean equalsNullable(String a, String b) {
        return a == null ? b == null : a.equals(b);
    }

    private static Screen screenOf(Minecraft minecraft) {
        try {
            return es.spectral.menu.Compat.getScreen(minecraft);
        } catch (Throwable t) {
            return null;
        }
    }
}
