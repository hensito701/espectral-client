package es.spectral.menu.agent;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.ConnectScreen;
import net.minecraft.client.gui.screens.DisconnectedScreen;
import net.minecraft.client.gui.screens.LevelLoadingScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.server.MinecraftServer;

/**
 * Computes the {@link AgentState} snapshot from the live client: phase, screen,
 * server, world, context and pause flag.
 *
 * <p>Phase rules (contract: {@code loading|title|joining|in_world|disconnected}):
 * <ul>
 *   <li>in a world/server → {@code in_world} (even with the pause menu or an
 *       inventory open: the connection is what the engine waits for);</li>
 *   <li>connecting ({@code ConnectScreen}) or loading ({@code LevelLoadingScreen})
 *       → {@code joining};</li>
 *   <li>{@code DisconnectedScreen} → {@code disconnected};</li>
 *   <li>otherwise → {@code title} (any menu screen, including the Suite) and
 *       {@code loading} while the very first screens are still settling.</li>
 * </ul>
 * Called on the render thread (or right after the render thread produced it);
 * every accessor is defensive so a state report can never throw.
 */
public final class AgentStates {

    private AgentStates() {}

    public static AgentState snapshot(Minecraft minecraft) {
        if (minecraft == null) {
            return new AgentState(AgentState.LOADING, null, null, null,
                    AgentPolicy.Context.MENU, AgentPause.paused(), false);
        }
        Screen screen = screenOf(minecraft);
        String screenName = screen != null ? screen.getClass().getSimpleName() : null;
        boolean connected = minecraft.getConnection() != null;
        boolean inWorld = connected || minecraft.level != null;


        String phase;
        if (inWorld) {
            phase = AgentState.IN_WORLD;
        } else if (screen instanceof ConnectScreen || screen instanceof LevelLoadingScreen) {
            phase = AgentState.JOINING;
        } else if (screen instanceof DisconnectedScreen) {
            phase = AgentState.DISCONNECTED;
        } else if (screen == null) {
            phase = AgentState.LOADING;
        } else {
            phase = AgentState.TITLE;
        }

        String server = null;
        String world = null;
        if (minecraft.hasSingleplayerServer()) {
            world = singleplayerWorldName(minecraft);
        } else {
            ServerData data = safeServer(minecraft);
            if (data != null && data.ip != null && !data.ip.isEmpty()) {
                server = data.ip;
            } else if (connected) {
                server = AgentPolicy.lastServer();
            } else if (screen instanceof DisconnectedScreen) {
                // Kicked: the client has already dropped the address, so the
                // latched one is the only honest answer — and the one that
                // keeps act tools refused while the reason is on screen.
                server = AgentPolicy.lastServer();
            }
        }
        AgentPolicy.Context context = AgentPolicy.classify(minecraft);
        // A joined-but-not-yet-in-world or a just-kicked client has no live
        // connection, so live classification lands on MENU. Fall back to the
        // server still selected in the client: an agent must never read "menu"
        // as permission to act against a server it was talking to. Fails closed
        // (any non-allowlisted host is `server`/`espectral`).
        if (context == AgentPolicy.Context.MENU && server != null) {
            context = AgentPolicy.classify(server, null);
        }
        // A kick leaves the last server latched; keep the refusal alive on the
        // disconnect screen even when the client dropped the address before the
        // first sample. Menus (title, Suite) stay `menu` so the agent can still
        // drive its own UI after leaving a server.
        if (phase.equals(AgentState.DISCONNECTED)) {
            String latched = AgentPolicy.lastServer();
            if (latched != null) {
                AgentPolicy.Context fromServer = AgentPolicy.classify(latched, null);
                if (!AgentTier.ACT.allows(fromServer)) context = fromServer;
            }
        }
        return new AgentState(phase, screenName, server, world, context,
                AgentPause.paused(), false);
    }

    private static Screen screenOf(Minecraft minecraft) {
        try {
            // Compat resolves gui.screen() (26.x) / minecraft.screen (1.21.11).
            return es.spectral.menu.Compat.getScreen(minecraft);
        } catch (Throwable t) {
            return null;
        }
    }

    private static ServerData safeServer(Minecraft minecraft) {
        try {
            return minecraft.getCurrentServer();
        } catch (Throwable t) {
            return null;
        }
    }

    /** Level name of the running integrated server ({@code saves/<folder>}). */
    private static String singleplayerWorldName(Minecraft minecraft) {
        try {
            MinecraftServer server = minecraft.getSingleplayerServer();
            if (server == null) return null;
            if (server.getWorldData() != null) {
                String name = server.getWorldData().getLevelName();
                if (name != null && !name.isEmpty()) return name;
            }
        } catch (Throwable ignored) {
            // fall through to the directory name
        }
        try {
            MinecraftServer server = minecraft.getSingleplayerServer();
            if (server != null && server.getServerDirectory() != null) {
                return server.getServerDirectory().getFileName().toString();
            }
        } catch (Throwable ignored) {
            // no world name available
        }
        return null;
    }
}
