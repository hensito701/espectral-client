package es.spectral.menu;

import org.apache.logging.log4j.LogManager;

import net.minecraftforge.common.MinecraftForge;
import net.minecraftforge.fml.common.Mod;
import net.minecraftforge.fml.common.event.FMLInitializationEvent;
import net.minecraftforge.fml.common.event.FMLPreInitializationEvent;

import es.spectral.menu.agent.AgentRelay;

/**
 * Espectral Menu for Minecraft 1.8.9 (Forge 11.15.1.2318): the Espectral
 * title screen and menu skin, the Suite (feature toggles, macros, HUD
 * editor), the owned features (fullbright, no fog, low fire, clear water,
 * 3D skin layers, HUD overlays, chat/tab heads, zoom, macros) and the
 * launcher agent bridge. Client-only; the version comes from mcmod.info
 * (the launcher's package.json version, expanded at build time).
 */
@Mod(modid = EspectralMenu.MODID, name = "Espectral Menu", useMetadata = true, clientSideOnly = true,
        acceptableRemoteVersions = "*", acceptedMinecraftVersions = "[1.8.9]")
public final class EspectralMenu {

    public static final String MODID = "espectralmenu";

    @Mod.EventHandler
    public void preInit(FMLPreInitializationEvent event) {
        ClientConfig.setConfigDir(event.getModConfigurationDirectory());
        ClientConfig.getInstance().load();
        es.spectral.menu.ruta.RutaGrants.setConfigDir(event.getModConfigurationDirectory());
        es.spectral.menu.ruta.RutaGrants.refresh();
        Keys.register();
        es.spectral.menu.bench.BenchEngine.maybeArm(event.getModConfigurationDirectory().getParentFile());
    }

    @Mod.EventHandler
    public void init(FMLInitializationEvent event) {
        LogManager.getLogger("espectral-client").info("[espectral] menu: loaded (client {}, minecraft {})",
                Version.version(), Version.minecraft());
        ClientEvents events = new ClientEvents();
        MinecraftForge.EVENT_BUS.register(events);
        Skin3d.register();
        MinecraftForge.EVENT_BUS.register(new AgentRelay.TickHook());
        AgentRelay.start();
    }
}
