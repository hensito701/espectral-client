package es.spectral.menu.agent;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import net.minecraft.client.Minecraft;
import net.minecraft.world.storage.ISaveFormat;
import net.minecraft.world.storage.WorldInfo;

import es.spectral.menu.ui.EspectralTitleScreen;

/**
 * Singleplayer join for pre-1.20 clients: the launcher has no quick play to
 * pass, so it sets {@code -Despectral.join.world=<save folder>} and this opens
 * that save once, as soon as the Espectral title screen is up. A missing save
 * is logged and left alone (never created).
 */
public final class AgentJoin {

    public static final String PROPERTY = "espectral.join.world";
    private static final Logger LOGGER = LogManager.getLogger("espectral-client");
    private static boolean done;

    private AgentJoin() {}

    /** Client tick (outside the scheduled-task drain, so starting a server is safe). */
    public static void onTick(Minecraft mc) {
        if (done || !(mc.currentScreen instanceof EspectralTitleScreen)) return;
        done = true;
        String folder = System.getProperty(PROPERTY);
        if (folder == null || folder.trim().isEmpty()) return;
        folder = folder.trim();
        ISaveFormat saves = mc.getSaveLoader();
        WorldInfo info = saves.canLoadWorld(folder) ? saves.getWorldInfo(folder) : null;
        if (info == null) {
            LOGGER.warn("[espectral] agent: join world '{}' not found in saves/", folder);
            return;
        }
        LOGGER.info("[espectral] agent: joining world '{}'", folder);
        net.minecraftforge.fml.client.FMLClientHandler.instance()
                .tryLoadExistingWorld(new net.minecraft.client.gui.GuiSelectWorld(mc.currentScreen), folder, info.getWorldName());
    }
}
