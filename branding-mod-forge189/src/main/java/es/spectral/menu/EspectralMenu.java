package es.spectral.menu;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.FontRenderer;
import net.minecraft.client.gui.GuiMainMenu;
import net.minecraftforge.client.event.GuiOpenEvent;
import net.minecraftforge.client.event.GuiScreenEvent;
import net.minecraftforge.common.MinecraftForge;
import net.minecraftforge.fml.common.Mod;
import net.minecraftforge.fml.common.event.FMLInitializationEvent;
import net.minecraftforge.fml.common.eventhandler.SubscribeEvent;
import net.minecraftforge.fml.relauncher.Side;

import es.spectral.menu.agent.AgentRelay;

/**
 * Espectral Menu for Minecraft 1.8.9 / Forge 11.15.1.2318.
 *
 * <p>Spike scope: title-screen branding (Forge event + window-title mixin),
 * the launcher agent bridge ({@code -Despectral.agent=1}) and the
 * {@code [espectral] presence: menu} marker the launcher watches.
 */
@Mod(modid = EspectralMenu.MODID, name = EspectralMenu.NAME, version = EspectralMenu.VERSION,
        acceptableRemoteVersions = "*", acceptedMinecraftVersions = "[1.8.9]",
        guiFactory = "")
public final class EspectralMenu {

    public static final String MODID = "espectralmenu";
    public static final String NAME = "Espectral Menu";
    /** Kept in step with ../package.json; the resource file is the runtime copy. */
    public static final String VERSION = "1.7.0";

    @Mod.EventHandler
    public void init(FMLInitializationEvent event) {
        if (event.getSide() != Side.CLIENT) return;
        org.apache.logging.log4j.LogManager.getLogger("espectral-client")
                .info("[espectral] menu: loaded (client " + Version.version()
                        + ", minecraft " + Version.minecraft() + ")");
        MinecraftForge.EVENT_BUS.register(new TitleScreen());
        MinecraftForge.EVENT_BUS.register(new AgentRelay.TickHook());
        AgentRelay.start();
    }

    /** Title-screen branding, drawn from Forge's post-draw event. */
    public static final class TitleScreen {

        @SubscribeEvent
        public void onGuiOpen(GuiOpenEvent event) {
            if (!(event.gui instanceof GuiMainMenu)) return;
            // Same marker the modern mod logs on its way back to the title
            // screen (branding-mod MinecraftMixin.java:69) — the launcher
            // watches it to flip Discord presence.
            org.apache.logging.log4j.LogManager.getLogger("espectral-client")
                    .info("[espectral] presence: menu");
        }

        @SubscribeEvent
        public void onDrawScreen(GuiScreenEvent.DrawScreenEvent.Post event) {
            if (!(event.gui instanceof GuiMainMenu)) return;
            draw();
        }

        /**
         * Both branding lines, top-left. Vanilla owns the bottom-left (MCP /
         * Forge version lines) and bottom-right (copyright) corners, so the
         * free corner is the only place that never overlaps.
         */
        private static void draw() {
            Minecraft minecraft = Minecraft.getMinecraft();
            FontRenderer font = minecraft == null ? null : minecraft.fontRendererObj;
            if (font == null) return;
            font.drawStringWithShadow("Espectral Client", 4, 4, 0x7C5CFF);
            font.drawStringWithShadow("v" + Version.version() + " \u2022 Forge 1.8.9",
                    4, 4 + font.FONT_HEIGHT + 2, 0xB9AEE8);
        }
    }
}
