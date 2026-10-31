package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiScreen;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.MenuBackground;

/**
 * The Espectral main-menu look carried into every vanilla menu opened outside
 * a world (Multiplayer, Singleplayer, Options and their sub-screens): the
 * menu art with the title screen's slow Ken Burns drift instead of the
 * panorama/dirt, dark-glass buttons with the accent bar, and a glass band
 * between accent hairlines for lists. Painting only — the mixins on
 * {@code GuiScreen}, {@code GuiButton} and {@code GuiSlot} route vanilla's
 * draw calls here, so layouts and input stay vanilla. In a world everything
 * stays vanilla.
 */
public final class MenuSkin {

    /** Branded charcoal under the art (and the whole fallback). */
    public static final int BASE = 0xFF0B0D10;
    public static final int BTN_FILL = 0x8C090B0E;
    public static final int BTN_FILL_HOT = 0xC8151A21;
    private static final int BTN_FILL_OFF = 0x59090B0E;
    private static final int BTN_EDGE_OFF = 0x1FFFFFFF;
    private static final int LIST_GLASS = 0x9E07090C;

    private MenuSkin() {}

    /** True on the menus before/after a world (no world loaded). */
    public static boolean outOfWorld() {
        Minecraft mc = Minecraft.getMinecraft();
        return mc != null && mc.theWorld == null;
    }

    /** True when vanilla widgets on the current screen get the Espectral skin. */
    public static boolean skinWidgets() {
        Minecraft mc = Minecraft.getMinecraft();
        if (mc == null || mc.theWorld != null) return false;
        GuiScreen screen = mc.currentScreen;
        return screen != null && !screen.getClass().getName().startsWith("es.spectral.");
    }

    /**
     * Cover-fit menu art with the slow drift (about ±3% zoom over 28 s,
     * 40 s / 52 s Lissajous pan) plus an optional parallax offset in [-1, 1];
     * the branded charcoal gradient while no pack is ready.
     */
    public static void paintArt(int width, int height, long now, boolean reduced, float parX, float parY) {
        MenuBackground.request(Minecraft.getMinecraft());
        Gfx.fill(0, 0, width, height, BASE);
        if (!MenuBackground.ready()) {
            Gfx.gradientV(0, 0, width, height, 0x00000000, 0x66000000);
            return;
        }
        float bw = MenuBackground.width();
        float bh = MenuBackground.height();
        float t = reduced ? 0f : (now - MenuBackground.readyAtMs()) / 1000f;
        float zoom = 1.08f + 0.03f * (float) Math.sin(t * (Math.PI * 2 / 28.0));
        float scale = Math.max(width / bw, height / bh) * zoom;
        float uw = Math.min(1f, width / scale / bw);
        float vh = Math.min(1f, height / scale / bh);
        float px = Gfx.clamp01(0.5f + 0.32f * (float) Math.sin(t * (Math.PI * 2 / 40.0)) + 0.14f * parX);
        float py = Gfx.clamp01(0.5f + 0.32f * (float) Math.cos(t * (Math.PI * 2 / 52.0)) + 0.14f * parY);
        float u0 = (1f - uw) * px;
        float v0 = (1f - vh) * py;
        MenuBackground.bind();
        Gfx.blitBound(0, 0, width, height, u0, v0, u0 + uw, v0 + vh, 0xFFFFFFFF);
        float reveal = reduced ? 1f
                : Gfx.easeOutCubic(Gfx.clamp01(((now - MenuBackground.readyAtMs()) / 1000f) / 0.9f));
        if (reveal < 1f) {
            Gfx.fill(0, 0, width, height, Gfx.withAlpha(BASE, 1f - reveal));
        }
    }

    /** Replaces the panorama / dirt on out-of-world menus. */
    public static void paintBackdrop(int width, int height) {
        paintArt(width, height, Minecraft.getSystemTime(), ClientConfig.getInstance().reducedMotion(), 0f, 0f);
        Gfx.gradientV(0, 0, width, 40, 0x8C000000, 0x00000000);
        Gfx.gradientV(0, Math.max(0, height - 72), width, height, 0x00000000, 0xA6000000);
    }

    public static void paintListGlass(int x0, int y0, int x1, int y1) {
        Gfx.fill(x0, y0, x1, y1, LIST_GLASS);
    }

    /** Accent hairline fading out at both ends (list top/bottom edges). */
    public static void hairline(int x0, int y, int x1) {
        int rgb = SuiteTheme.accent() & 0xFFFFFF;
        int seg = Math.max(1, (x1 - x0) / 5);
        Gfx.fill(x0, y, x0 + seg, y + 1, Gfx.argb(0.18f, rgb));
        Gfx.fill(x0 + seg, y, x1 - seg, y + 1, Gfx.argb(0.5f, rgb));
        Gfx.fill(x1 - seg, y, x1, y + 1, Gfx.argb(0.18f, rgb));
    }

    /** Eases a hover amount toward its target (same response as the title menu). */
    public static float easeHover(float current, boolean hot, float dt) {
        float k = 1f - (float) Math.exp(-dt * 14f);
        return current + ((hot ? 1f : 0f) - current) * k;
    }

    /**
     * Title-screen glass button: fill and edge blend idle → hot with
     * {@code hover}, the accent bar grows from the centre of the left edge.
     */
    public static void paintButton(int x, int y, int w, int h, boolean enabled, float hover, float alpha) {
        if (!enabled) {
            Gfx.fill(x, y, x + w, y + h, Gfx.scaleAlpha(BTN_FILL_OFF, alpha));
            Gfx.outline(x, y, w, h, Gfx.scaleAlpha(BTN_EDGE_OFF, alpha));
            return;
        }
        Gfx.fill(x, y, x + w, y + h, Gfx.scaleAlpha(SuiteTheme.lerpArgb(BTN_FILL, BTN_FILL_HOT, hover), alpha));
        Gfx.outline(x, y, w, h,
                Gfx.scaleAlpha(SuiteTheme.lerpArgb(SuiteTheme.accentHairline(), edgeHot(), hover), alpha));
        int bar = Math.round((h - 2) * hover);
        if (bar > 0) {
            int by = y + (h - bar) / 2;
            Gfx.fill(x, by, x + 2, by + bar, Gfx.scaleAlpha(SuiteTheme.accent(), alpha));
        }
    }

    public static int edgeHot() {
        return (SuiteTheme.accent() & 0xFFFFFF) | 0xCC000000;
    }
}
