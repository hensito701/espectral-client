package es.spectral.menu.agent;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiDownloadTerrain;
import net.minecraft.client.gui.GuiMainMenu;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.multiplayer.GuiConnecting;
import net.minecraft.client.multiplayer.ServerData;

/**
 * Builds the state snapshot from the live client. Phase rules follow the
 * modern mod ({@link es.spectral.menu.agent.AgentState}): in a world →
 * {@code in_world}; connecting/loading → {@code joining}; a disconnect screen
 * → {@code disconnected}; any other screen → {@code title}; nothing yet
 * → {@code loading}.
 *
 * <p>1.8.9 has no {@code DisconnectedScreen} class: a kick lands on
 * {@link GuiMainMenu} with a chat error, so a kick is reported as
 * {@code title} and the latched server address keeps the context honest.
 */
public final class AgentStates {

    /** Last server address seen while connected (kicks clear it). */
    private static volatile String lastServer;

    private AgentStates() {}

    static void latchServer(String address) {
        if (address != null && !address.isEmpty()) lastServer = address;
    }

    static String latchedServer() {
        return lastServer;
    }

    public static AgentState snapshot(Minecraft minecraft) {
        if (minecraft == null) {
            return new AgentState(AgentState.LOADING, null, null, null, "menu", false, false);
        }
        GuiScreen screen = null;
        String screenName = null;
        boolean connected = false;
        String world = null;
        String server = null;
        try {
            screen = minecraft.currentScreen;
            screenName = screen != null ? screen.getClass().getSimpleName() : null;
            connected = minecraft.getNetHandler() != null;
            if (minecraft.theWorld != null) {
                world = worldName(minecraft);
            }
            if (minecraft.isSingleplayer()) {
                server = null;
            } else {
                ServerData data = minecraft.getCurrentServerData();
                if (data != null && data.serverIP != null && !data.serverIP.isEmpty()) {
                    server = data.serverIP;
                    latchServer(server);
                } else {
                    server = lastServer;
                }
            }
        } catch (Throwable ignored) {
            // a state sample must never disturb the game
        }
        boolean inWorld = connected || minecraft.theWorld != null;

        String phase;
        if (inWorld) {
            phase = AgentState.IN_WORLD;
        } else if (screen instanceof GuiConnecting || screen instanceof GuiDownloadTerrain) {
            phase = AgentState.JOINING;
        } else if (screen == null) {
            phase = AgentState.LOADING;
        } else {
            phase = AgentState.TITLE;
        }

        // Context: 1.8.9 spike has no tier table; everything without a world is
        // "menu", a singleplayer world is "singleplayer", a server is "server".
        String context;
        if (minecraft.isSingleplayer() && minecraft.theWorld != null) {
            context = "singleplayer";
        } else if (connected || server != null) {
            context = "server";
        } else {
            context = "menu";
        }

        boolean paused = false;
        try {
            paused = minecraft.isGamePaused();
        } catch (Throwable ignored) {
            // not paused
        }
        boolean recording = false;
        return new AgentState(phase, screenName, server, world, context, paused, recording);
    }

    private static String worldName(Minecraft minecraft) {
        try {
            if (minecraft.isSingleplayer() && minecraft.getIntegratedServer() != null) {
                String name = minecraft.getIntegratedServer().getWorldName();
                if (name != null && !name.isEmpty()) return name;
                return minecraft.getIntegratedServer().getFolderName();
            }
        } catch (Throwable ignored) {
            // fall through to the client-side world info
        }
        try {
            if (minecraft.theWorld != null && minecraft.theWorld.getWorldInfo() != null) {
                return minecraft.theWorld.getWorldInfo().getWorldName();
            }
        } catch (Throwable ignored) {
            // no world name available
        }
        return null;
    }
}
