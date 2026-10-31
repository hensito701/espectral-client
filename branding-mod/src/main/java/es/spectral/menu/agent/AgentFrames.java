package es.spectral.menu.agent;

import java.util.ArrayList;
import java.util.List;

/**
 * Runs a task after N more frames have been rendered — for tool replies that
 * must reflect what the next frames show (a shader pack switch, a HUD toggle)
 * without blocking the render thread while waiting. Render thread only.
 */
public final class AgentFrames {

    private static final List<Waiter> WAITERS = new ArrayList<>();

    private record Waiter(int[] left, Runnable task) {}

    private AgentFrames() {}

    /** Schedules {@code task} after {@code frames} more end-of-frame hooks. */
    public static void after(int frames, Runnable task) {
        WAITERS.add(new Waiter(new int[] {Math.max(1, frames)}, task));
    }

    /** End-of-frame hook (see AgentFrameMixin). */
    public static void onFrameEnd() {
        if (WAITERS.isEmpty()) return;
        List<Runnable> due = null;
        for (int i = WAITERS.size() - 1; i >= 0; i--) {
            Waiter waiter = WAITERS.get(i);
            if (--waiter.left()[0] <= 0) {
                if (due == null) due = new ArrayList<>();
                due.add(waiter.task());
                WAITERS.remove(i);
            }
        }
        if (due != null) {
            for (Runnable task : due) task.run();
        }
    }
}
