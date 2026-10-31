package es.spectral.menu.agent;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiDisconnected;
import net.minecraft.client.gui.GuiDownloadTerrain;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.gui.GuiScreenWorking;
import net.minecraft.client.multiplayer.GuiConnecting;
import net.minecraft.client.multiplayer.ServerData;

/**
 * The state snapshot from the live client (same rules as the Fabric mod):
 * in a world → {@code in_world}; connecting / terrain download / integrated
 * server start → {@code joining}; the disconnect screen → {@code disconnected};
 * any other screen → {@code title}; nothing yet → {@code loading}. The
 * context comes from {@link AgentPolicy#classify}, and on the disconnect
 * screen the latched server keeps a kicked client's context honest (so act
 * tools stay refused on public servers). {@code paused} is the F8 agent pause.
 */
public final class AgentStates {

    private AgentStates() {}

    public static AgentState snapshot(Minecraft mc) {
        if (mc == null) {
            return new AgentState(AgentState.LOADING, null, null, null, "menu", AgentPause.paused(), false);
        }
        GuiScreen screen = mc.currentScreen;
        String screenName = screen != null ? screen.getClass().getSimpleName() : null;
        boolean inWorld = mc.theWorld != null && mc.thePlayer != null;
        String world = inWorld ? worldName(mc) : null;
        String server = null;
        if (!mc.isSingleplayer()) {
            ServerData data = mc.getCurrentServerData();
            if (data != null && data.serverIP != null && !data.serverIP.isEmpty()) server = data.serverIP;
        }

        String phase;
        if (inWorld) {
            phase = AgentState.IN_WORLD;
        } else if (screen instanceof GuiConnecting || screen instanceof GuiDownloadTerrain
                || screen instanceof GuiScreenWorking) {
            phase = AgentState.JOINING;
        } else if (screen instanceof GuiDisconnected) {
            phase = AgentState.DISCONNECTED;
        } else if (screen == null) {
            phase = AgentState.LOADING;
        } else {
            phase = AgentState.TITLE;
        }

        AgentPolicy.Context context;
        try {
            context = AgentPolicy.classify(mc);
            if (context == AgentPolicy.Context.MENU && screen instanceof GuiDisconnected
                    && AgentPolicy.lastServer() != null) {
                context = AgentPolicy.classify(AgentPolicy.lastServer(), null);
                server = AgentPolicy.lastServer();
            }
        } catch (Throwable t) {
            context = AgentPolicy.Context.ESPECTRAL;
        }
        return new AgentState(phase, screenName, server, world, context.wireName(), AgentPause.paused(), false);
    }

    private static String worldName(Minecraft mc) {
        try {
            if (mc.isSingleplayer() && mc.getIntegratedServer() != null) {
                String name = mc.getIntegratedServer().getWorldName();
                return name != null && !name.isEmpty() ? name : mc.getIntegratedServer().getFolderName();
            }
            if (mc.theWorld.getWorldInfo() != null) return mc.theWorld.getWorldInfo().getWorldName();
        } catch (Throwable ignored) {
            // no world name available
        }
        return null;
    }
}
