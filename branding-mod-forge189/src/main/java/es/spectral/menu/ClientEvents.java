package es.spectral.menu;

import net.minecraft.block.material.Material;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiIngameMenu;
import net.minecraft.client.gui.GuiMainMenu;
import net.minecraft.client.renderer.GlStateManager;
import net.minecraftforge.client.event.EntityViewRenderEvent;
import net.minecraftforge.client.event.GuiOpenEvent;
import net.minecraftforge.client.event.GuiScreenEvent;
import net.minecraftforge.client.event.RenderBlockOverlayEvent;
import net.minecraftforge.client.event.RenderGameOverlayEvent;
import net.minecraftforge.fml.common.eventhandler.SubscribeEvent;
import net.minecraftforge.fml.common.gameevent.TickEvent;
import net.minecraftforge.fml.common.network.FMLNetworkEvent;
import org.lwjgl.opengl.GL11;

import es.spectral.menu.agent.AgentInput;
import es.spectral.menu.agent.AgentJoin;
import es.spectral.menu.agent.AgentPause;
import es.spectral.menu.bench.BenchEngine;
import es.spectral.menu.ui.EspectralTitleScreen;
import es.spectral.menu.ui.SuiteConfirmScreen;
import es.spectral.menu.ui.SuiteScreen;

/**
 * Every Forge event the menu listens to (registered on both the Forge and
 * FML buses by {@link EspectralMenu}):
 * <ul>
 *   <li>client tick: live config reload (every second), zoom, macros, the
 *       Right Shift Suite shortcut, menu-art release, agent pause/input
 *       leases and the agent world join;</li>
 *   <li>title screen swap ({@code GuiMainMenu} → {@link EspectralTitleScreen})
 *       and the pause-menu "Espectral Client…" / "♥ Support" row;</li>
 *   <li>HUD overlays after vanilla's HUD;</li>
 *   <li>no fog (distance fog pushed past view distance, gameplay fogs
 *       untouched) and clear water (far water fog, no underwater overlay).</li>
 * </ul>
 */
public final class ClientEvents {

    /** Fog distance while no-fog / clear water is on: past the 32-chunk maximum. */
    private static final float FAR = 1024.0f;
    private static final float WATER_FOG_START = 64.0f;
    private static final int ID_PAUSE_CLIENT = 7101;
    private static final int ID_PAUSE_SUPPORT = 7102;

    private int ticks;

    @SubscribeEvent
    public void onClientTick(TickEvent.ClientTickEvent event) {
        if (event.phase != TickEvent.Phase.END) return;
        Minecraft mc = Minecraft.getMinecraft();
        if (++ticks % 20 == 0) {
            ClientConfig.getInstance().maybeReload();
        }
        ZoomEngine.onTick(mc);
        MacroEngine.onTick(mc);
        if (Keys.CLIENT != null && Keys.CLIENT.isPressed() && mc.currentScreen == null) {
            mc.displayGuiScreen(new SuiteScreen(null));
        }
        MenuBackground.releaseIfHeld(mc);
        AgentPause.onTick();
        AgentInput.onTick();
        AgentJoin.onTick(mc);
        BenchEngine.onTick(mc);
        GameDefaults.onTick(mc);
    }

    @SubscribeEvent
    public void onRenderTick(TickEvent.RenderTickEvent event) {
        if (event.phase == TickEvent.Phase.END) BenchEngine.onFrameEnd();
    }

    @SubscribeEvent
    public void onGuiOpen(GuiOpenEvent event) {
        if (event.gui instanceof GuiMainMenu) {
            event.gui = new EspectralTitleScreen();
        }
    }

    @SubscribeEvent
    public void onInitGui(GuiScreenEvent.InitGuiEvent.Post event) {
        if (!(event.gui instanceof GuiIngameMenu)) return;
        int w = event.gui.width;
        int y = event.gui.height / 4 + 144 - 16;
        event.buttonList.add(new GuiButton(ID_PAUSE_CLIENT, w / 2 - 100, y, 98, 20, Lang.tr("espectral.pause.client")));
        event.buttonList.add(new GuiButton(ID_PAUSE_SUPPORT, w / 2 + 2, y, 98, 20, Lang.tr("espectral.pause.support")));
    }

    @SubscribeEvent
    public void onAction(GuiScreenEvent.ActionPerformedEvent.Pre event) {
        if (!(event.gui instanceof GuiIngameMenu)) return;
        Minecraft mc = Minecraft.getMinecraft();
        if (event.button.id == ID_PAUSE_CLIENT) {
            mc.displayGuiScreen(new SuiteScreen(event.gui));
            event.setCanceled(true);
        } else if (event.button.id == ID_PAUSE_SUPPORT) {
            mc.displayGuiScreen(SuiteConfirmScreen.support(event.gui));
            event.setCanceled(true);
        }
    }

    @SubscribeEvent
    public void onOverlay(RenderGameOverlayEvent.Post event) {
        if (event.type == RenderGameOverlayEvent.ElementType.ALL) {
            HudEngine.draw(Minecraft.getMinecraft(), event.resolution);
            AgentPause.drawBanner(Minecraft.getMinecraft(), event.resolution.getScaledWidth());
        }
    }

    @SubscribeEvent
    public void onDrawScreen(GuiScreenEvent.DrawScreenEvent.Post event) {
        AgentPause.drawBanner(Minecraft.getMinecraft(), event.gui.width);
    }

    /** Distance fog (overworld, nether, end): fired after vanilla set it up. */
    @SubscribeEvent
    public void onRenderFog(EntityViewRenderEvent.RenderFogEvent event) {
        if (ClientConfig.getInstance().isFeatureEnabled("nofog")) {
            // start < end is required: GL's linear fog divides by (end - start).
            GlStateManager.setFogStart(FAR);
            GlStateManager.setFogEnd(FAR * 2);
        }
    }

    /** Underwater fog: linear and far instead of the dense exponential haze. */
    @SubscribeEvent
    public void onFogDensity(EntityViewRenderEvent.FogDensity event) {
        if (event.block.getMaterial() == Material.water
                && ClientConfig.getInstance().isFeatureEnabled("clearwater")) {
            GlStateManager.setFog(GL11.GL_LINEAR);
            GlStateManager.setFogStart(WATER_FOG_START);
            GlStateManager.setFogEnd(FAR);
            event.density = 0f;
            event.setCanceled(true);
        }
    }

    @SubscribeEvent
    public void onBlockOverlay(RenderBlockOverlayEvent event) {
        if (event.overlayType == RenderBlockOverlayEvent.OverlayType.WATER
                && ClientConfig.getInstance().isFeatureEnabled("clearwater")) {
            event.setCanceled(true);
        }
    }

    @SubscribeEvent
    public void onDisconnect(FMLNetworkEvent.ClientDisconnectionFromServerEvent event) {
        ChatHeads.clear();
    }
}
