package es.spectral.menu.agent;

/**
 * The paused chip drawn top-left while agent mode is on. Deliberately tiny and
 * only while paused: an always-on badge would end up in every capture the
 * agent takes, and the contract asks for evidence, not watermarks.
 *
 * <p>Rendered through {@code Compat.ui*} so the shared class never names a
 * per-version graphics type.
 */
public final class AgentHud {

    private static final int PAD = 3;
    private static final int BG = 0xCC1A1210;
    private static final int FG = 0xFFFFD479;

    private AgentHud() {}

    /** Draws the chip when paused; a no-op otherwise. */
    public static void render(Object graphics, net.minecraft.client.gui.Font font) {
        if (!AgentRuntime.enabled() || font == null) return;
        String label = AgentPause.indicator();
        if (label == null) return;
        int width = font.width(label) + PAD * 2;
        int height = 9 + PAD * 2;
        es.spectral.menu.Compat.uiFill(graphics, 4, 4, 4 + width, 4 + height, BG);
        es.spectral.menu.Compat.uiText(graphics, font,
                net.minecraft.network.chat.Component.literal(label),
                4 + PAD, 4 + PAD, FG, true);
    }
}
