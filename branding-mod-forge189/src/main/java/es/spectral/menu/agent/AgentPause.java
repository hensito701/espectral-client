package es.spectral.menu.agent;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import net.minecraft.client.Minecraft;

import es.spectral.menu.Keys;
import es.spectral.menu.Lang;
import es.spectral.menu.ui.Gfx;

/**
 * The human's leash on the agent (agent launches only): F8 — a real
 * {@link Keys#AGENT_PAUSE} press, never an injected key (the {@code key} tool
 * refuses it) — toggles {@link #paused()}, after which every act-tier tool
 * answers {@code PAUSED}. While paused a banner is drawn at the top of the
 * screen.
 */
public final class AgentPause {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    private static volatile boolean paused;

    private AgentPause() {}

    public static boolean paused() {
        return paused;
    }

    /** Client tick: consume real F8 presses. */
    public static void onTick() {
        if (Keys.AGENT_PAUSE == null) return;
        while (Keys.AGENT_PAUSE.isPressed()) {
            paused = !paused;
            LOGGER.info("agent: {}", paused ? "PAUSED (F8)" : "ACTIVE (F8)");
        }
    }

    /** Banner while paused (drawn after the HUD and over screens). */
    public static void drawBanner(Minecraft mc, int screenWidth) {
        if (!paused) return;
        String text = Lang.tr("espectral.agent.paused");
        int w = mc.fontRendererObj.getStringWidth(text) + 12;
        int x = (screenWidth - w) / 2;
        Gfx.fill(x, 2, x + w, 16, 0xC0400000);
        Gfx.outline(x, 2, w, 14, 0xFFE0A24A);
        Gfx.textCentered(text, screenWidth / 2, 5, 0xFFFFE3A1, true);
    }
}
