package es.spectral.menu.agent;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiMainMenu;

/**
 * The in-client tool table. Same shape as the modern mod's
 * {@code es.spectral.menu.agent.AgentTools}: every tool runs on the client
 * thread ({@code Minecraft.addScheduledTask}), and the relay only does JSON.
 *
 * <p>Spike scope: {@code screenshot} (contract result shape) and
 * {@code disconnect}; anything else answers {@code UNSUPPORTED}.
 */
public final class AgentTools {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    private AgentTools() {}

    public static CompletableFuture<JsonObject> dispatch(Minecraft minecraft, String tool,
            JsonObject args, long timeoutMs) {
        final CompletableFuture<JsonObject> future = new CompletableFuture<JsonObject>();
        if (minecraft == null) {
            future.completeExceptionally(new AgentException("FAILED", "no client"));
            return future;
        }
        final String name = tool == null ? "" : tool.trim();
        // Runs on the render-frame hook, not inside Minecraft.addScheduledTask:
        // see ClientTasks for the deadlock that makes this mandatory.
        ClientTasks.submit(new Runnable() {
            @Override
            public void run() {
                try {
                    if ("screenshot".equals(name)) {
                        // Capture owns its own frame and answers the future itself.
                        Capture.request(new AgentArgs(args), future);
                        return;
                    }
                    future.complete(execute(name, new AgentArgs(args)));
                } catch (AgentException e) {
                    future.completeExceptionally(e);
                } catch (Throwable t) {
                    LOGGER.warn("agent: tool '{}' failed", name, t);
                    future.completeExceptionally(
                            new AgentException("FAILED", name + ": " + t));
                }
            }
        });
        return future;
    }

    private static JsonObject execute(String name, AgentArgs args) throws AgentException {
        if ("disconnect".equals(name)) {
            // Same entry point the in-game menu uses.
            return disconnect();
        }
        if ("screenshot".equals(name)) {
            throw new AgentException("FAILED", "screenshot must not run on the render thread");
        }
        throw new AgentException("UNSUPPORTED", "unknown tool '" + name + "'");
    }

    private static JsonObject disconnect() {
        Minecraft minecraft = Minecraft.getMinecraft();
        boolean inWorld = minecraft.theWorld != null;
        if (!inWorld) {
            JsonObject out = new JsonObject();
            out.addProperty("disconnected", false);
            out.addProperty("note", "not in a world");
            return out;
        }
        // 1.8.9 has no public disconnectFromWorld. This is the exact sequence
        // GuiIngameMenu's "Disconnect" button runs (sendQuittingDisconnectingPacket,
        // loadWorld(null), then show a screen) — the trailing displayGuiScreen is
        // not optional: without it the renderer keeps drawing a torn-down world
        // and NPEs in EntityRenderer.updateCameraAndRender.
        minecraft.theWorld.sendQuittingDisconnectingPacket();
        minecraft.loadWorld(null);
        minecraft.displayGuiScreen(new GuiMainMenu());
        JsonObject out = new JsonObject();
        out.addProperty("disconnected", true);
        return out;
    }

    /** Awaits a future for at most {@code timeoutMs} (relay helper). */
    public static JsonObject await(CompletableFuture<JsonObject> future, long timeoutMs)
            throws AgentException {
        try {
            return future.get(Math.max(1000L, timeoutMs), TimeUnit.MILLISECONDS);
        } catch (TimeoutException e) {
            throw new AgentException("BUSY", "the client did not run the tool in time");
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AgentException("FAILED", "interrupted");
        } catch (ExecutionException e) {
            Throwable cause = e.getCause();
            if (cause instanceof AgentException) throw (AgentException) cause;
            throw new AgentException("FAILED", String.valueOf(cause));
        }
    }
}
