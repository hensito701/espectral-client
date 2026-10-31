package es.spectral.menu.agent;

import java.util.Queue;
import java.util.concurrent.ConcurrentLinkedQueue;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import net.minecraft.client.Minecraft;

/**
 * Runs small actions on the client thread <em>outside</em> Minecraft's
 * scheduled-task drain.
 *
 * <p>1.8.9's {@code Minecraft.runGameLoop} executes scheduled tasks while
 * holding the {@code scheduledTasks} monitor, and Netty's packet handler takes
 * that same monitor in {@code PacketThreadUtil.checkThreadAndEnqueue}. An
 * action that waits for the network from inside a scheduled task — the
 * canonical case being {@code WorldClient.sendQuittingDisconnectingPacket},
 * which blocks in {@code NetworkManager.closeChannel} — therefore deadlocks
 * the client thread against Netty (verified with jstack: Client thread waiting
 * on a DefaultChannelPromise, Netty thread BLOCKED on the queue monitor).
 *
 * <p>So every agent tool is queued here and drained from {@code runTick}
 * (see {@code MinecraftMixin.espectral$agentTick}), which {@code runGameLoop}
 * calls after the scheduled-task drain and without holding its lock. Running
 * it any later — e.g. mid-frame from the renderer — tears down the world under
 * an in-progress {@code updateCameraAndRender} and NPEs.
 */
public final class ClientTasks {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    private static final Queue<Runnable> QUEUE = new ConcurrentLinkedQueue<Runnable>();

    private ClientTasks() {}

    public static void submit(Runnable task) {
        QUEUE.add(task);
    }

    /** Called once per rendered frame on the client thread. */
    public static void drain() {
        Runnable task;
        while ((task = QUEUE.poll()) != null) {
            try {
                task.run();
            } catch (Throwable t) {
                LOGGER.warn("agent: deferred client task failed", t);
            }
        }
    }

    /** True when the caller already runs on the client thread (and may act). */
    public static boolean onClientThread() {
        Minecraft minecraft = Minecraft.getMinecraft();
        return minecraft == null || minecraft.isCallingFromMinecraftThread();
    }
}
