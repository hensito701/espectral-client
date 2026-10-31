package es.spectral.menu.ui;

import java.io.File;
import java.io.IOException;

import org.apache.logging.log4j.LogManager;
import org.lwjgl.Sys;
import org.lwjgl.input.Keyboard;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiLanguage;
import net.minecraft.client.gui.GuiMultiplayer;
import net.minecraft.client.gui.GuiOptions;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.gui.GuiSelectWorld;
import net.minecraft.util.ResourceLocation;
import net.minecraft.util.Util;
import net.minecraftforge.fml.client.GuiModList;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Lang;
import es.spectral.menu.Version;
import es.spectral.menu.agent.AgentLabeled;

/**
 * The Espectral main menu for 1.8.9 (replaces {@code GuiMainMenu}): the same
 * art direction as the Fabric lanes' {@code TitleMenu} — full-bleed menu art
 * with a slow drift and light mouse parallax, a left readability scrim,
 * drifting accent motes, a vertical glass button stack on the left, and the
 * circular logo with a breathing halo + "ESPECTRAL" gradient wordmark +
 * tagline on the right; an icon row top-left (quit, language, screenshots)
 * and the support heart top-right.
 *
 * <p>Forge adds a "Mods" entry (the FML mod list) to the stack. The first
 * title screen of a session plays a short entrance; later returns render
 * settled. Buttons are real {@link GuiButton}s (mouse, Up/Down/Tab + Enter
 * and the agent tools work on them).
 */
public class EspectralTitleScreen extends GuiScreen implements AgentLabeled {

    private static final ResourceLocation LOGO = new ResourceLocation("espectralmenu", "textures/gui/logo.png");
    private static final ResourceLocation GLOW = new ResourceLocation("espectralmenu", "textures/gui/glow.png");
    private static final ResourceLocation SCRIM = new ResourceLocation("espectralmenu", "textures/gui/scrim.png");
    private static final int MOTE_WHITE = 0xFFF4EEDF;
    private static final int MOTES = 28;

    private static final int ID_SINGLE = 1;
    private static final int ID_MULTI = 2;
    private static final int ID_OPTIONS = 3;
    private static final int ID_MODS = 4;
    private static final int ID_SUITE = 5;
    private static final int ID_QUIT = 10;
    private static final int ID_LANGUAGE = 11;
    private static final int ID_SCREENSHOTS = 12;
    private static final int ID_SUPPORT = 13;

    /** Session start of the entrance animation; -1 until the first title frame. */
    private static long introStartMs = -1;

    private int focused = -1;
    private int lastMouseX = Integer.MIN_VALUE;
    private int lastMouseY = Integer.MIN_VALUE;
    private long lastFrameMs = -1;
    private float parX;
    private float parY;
    private float intro;

    public EspectralTitleScreen() {
        LogManager.getLogger("espectral-client").info("[espectral] presence: menu");
    }

    @Override
    public void initGui() {
        buttonList.clear();
        es.spectral.menu.MenuBackground.request(mc);
        int bw = Math.min(160, Math.max(116, width / 5));
        int bh = 22;
        int gap = 6;
        int bx = Math.max(18, width / 12);
        int count = 5;
        int total = count * bh + (count - 1) * gap;
        int by = (height - total) / 2 + height / 10;
        String[] keys = {"espectral.menu.singleplayer", "espectral.menu.multiplayer", "espectral.menu.options",
                "espectral.menu.mods", "espectral.menu.suite"};
        int[] ids = {ID_SINGLE, ID_MULTI, ID_OPTIONS, ID_MODS, ID_SUITE};
        for (int i = 0; i < count; i++) {
            buttonList.add(new TitleButton(ids[i], i, bx, by + i * (bh + gap), bw, bh, Lang.tr(keys[i])));
        }
        if (focused >= count) focused = -1;
        int is = 18;
        buttonList.add(new IconButton(ID_QUIT, 8, 8, is, "\u2715", Lang.tr("espectral.menu.quit")));
        buttonList.add(new IconButton(ID_LANGUAGE, 8 + is + 4, 8, is, "\u2691", Lang.tr("espectral.menu.language")));
        buttonList.add(new IconButton(ID_SCREENSHOTS, 8 + 2 * (is + 4), 8, is, "\u25A3",
                Lang.tr("espectral.menu.screenshots")));
        buttonList.add(new IconButton(ID_SUPPORT, width - 8 - is, 8, is, "\u2665", Lang.tr("espectral.menu.support")));
    }

    @Override
    protected void actionPerformed(GuiButton button) throws IOException {
        switch (button.id) {
            case ID_SINGLE: mc.displayGuiScreen(new GuiSelectWorld(this)); break;
            case ID_MULTI: mc.displayGuiScreen(new GuiMultiplayer(this)); break;
            case ID_OPTIONS: mc.displayGuiScreen(new GuiOptions(this, mc.gameSettings)); break;
            case ID_MODS: mc.displayGuiScreen(new GuiModList(this)); break;
            case ID_SUITE: mc.displayGuiScreen(new SuiteScreen(this)); break;
            case ID_QUIT: mc.shutdown(); break;
            case ID_LANGUAGE: mc.displayGuiScreen(new GuiLanguage(this, mc.gameSettings, mc.getLanguageManager())); break;
            case ID_SCREENSHOTS: openFolder(new File(mc.mcDataDir, "screenshots")); break;
            case ID_SUPPORT: mc.displayGuiScreen(SuiteConfirmScreen.support(this)); break;
            default: break;
        }
    }

    /** Opens {@code dir} in the OS file browser (vanilla's resource-pack folder logic). */
    static void openFolder(File dir) {
        dir.mkdirs();
        String path = dir.getAbsolutePath();
        try {
            if (Util.getOSType() == Util.EnumOS.OSX) {
                Runtime.getRuntime().exec(new String[] {"/usr/bin/open", path});
                return;
            }
            if (Util.getOSType() == Util.EnumOS.WINDOWS) {
                Runtime.getRuntime().exec(new String[] {"explorer", path});
                return;
            }
            Sys.openURL("file://" + path);
        } catch (Exception e) {
            LogManager.getLogger("espectral-menu").warn("Could not open folder {}: {}", path, e.toString());
        }
    }

    @Override
    protected void keyTyped(char typedChar, int keyCode) {
        // Esc never leaves the main menu; arrows/Tab move the focus, Enter/Space press.
        int n = 5;
        switch (keyCode) {
            case Keyboard.KEY_DOWN:
                focused = focused < 0 ? 0 : (focused + 1) % n;
                break;
            case Keyboard.KEY_UP:
                focused = focused < 0 ? n - 1 : (focused + n - 1) % n;
                break;
            case Keyboard.KEY_TAB:
                boolean back = isShiftKeyDown();
                focused = focused < 0 ? (back ? n - 1 : 0) : (focused + (back ? n - 1 : 1)) % n;
                break;
            case Keyboard.KEY_RETURN:
            case Keyboard.KEY_NUMPADENTER:
            case Keyboard.KEY_SPACE:
                if (focused >= 0) {
                    for (GuiButton b : buttonList) {
                        if (b instanceof TitleButton && ((TitleButton) b).index == focused) {
                            b.playPressSound(mc.getSoundHandler());
                            try {
                                actionPerformed(b);
                            } catch (IOException ignored) {
                                // GuiScreen's signature; none of the actions throw
                            }
                            return;
                        }
                    }
                }
                break;
            default:
                break;
        }
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        boolean reduced = ClientConfig.getInstance().reducedMotion();
        long now = Minecraft.getSystemTime();
        if (introStartMs < 0) introStartMs = now;
        float dt = lastFrameMs < 0 ? 0f : Math.min(0.1f, (now - lastFrameMs) / 1000f);
        lastFrameMs = now;
        intro = reduced ? Float.MAX_VALUE : (now - introStartMs) / 1000f;

        if (mouseX != lastMouseX || mouseY != lastMouseY) {
            if (lastMouseX != Integer.MIN_VALUE) focused = -1;
            lastMouseX = mouseX;
            lastMouseY = mouseY;
        }
        if (!reduced && width > 0 && height > 0) {
            float tx = Gfx.clamp01(mouseX / (float) width) * 2f - 1f;
            float ty = Gfx.clamp01(mouseY / (float) height) * 2f - 1f;
            float k = 1f - (float) Math.exp(-dt * 2.5f);
            parX += (tx - parX) * k;
            parY += (ty - parY) * k;
        } else {
            parX = 0f;
            parY = 0f;
        }

        MenuSkin.paintArt(width, height, now, reduced, parX, parY);
        Gfx.sprite(SCRIM, 0, 0, Math.max(1, (int) (width * 0.62f)), height, 0xFFFFFFFF);
        Gfx.gradientV(0, (int) (height * 0.55f), width, height, 0x00000000, 0xA6000000);
        Gfx.gradientV(0, 0, width, 36, 0x73000000, 0x00000000);
        if (!reduced) drawMotes(intro, fade(intro, 0.6f, 1.6f));
        drawBrand(intro, reduced);

        for (GuiButton b : buttonList) {
            if (b instanceof TitleButton) ((TitleButton) b).dt = dt;
            if (b instanceof IconButton) ((IconButton) b).dt = dt;
        }
        super.drawScreen(mouseX, mouseY, partialTicks);
        for (GuiButton b : buttonList) {
            if (b instanceof IconButton && b.isMouseOver() && fade(intro, 0.5f, 0.6f) > 0.5f) {
                Gfx.tooltip(((IconButton) b).tooltip, mouseX, mouseY, width, height);
            }
        }
        float footA = fade(intro, 0.9f, 0.6f);
        if (footA > 0.02f) {
            String footer = "Minecraft " + Version.minecraft() + " \u00b7 Forge \u00b7 Espectral " + Version.version();
            Gfx.textRight(footer, width - 8, height - 14, Gfx.withAlpha(SuiteTheme.TEXT_DIM, footA), false);
        }
    }

    private void drawMotes(float t, float alpha) {
        if (alpha <= 0.01f || width <= 0 || height <= 0) return;
        float span = height + 24f;
        for (int i = 0; i < MOTES; i++) {
            float r1 = rnd(i * 3 + 1);
            float r2 = rnd(i * 3 + 2);
            float r3 = rnd(i * 3 + 3);
            float speed = 5f + 9f * r2;
            float life = span / speed;
            float phase = ((t + r3 * life) % life) / life;
            float y = height + 12f - phase * span;
            float x = r1 * width + 7f * (float) Math.sin(t * (0.35f + 0.4f * r3) + i * 1.3f);
            float twinkle = 0.65f + 0.35f * (float) Math.sin(t * (1.4f + r1) + i * 1.7f);
            float a = alpha * (float) Math.sin(phase * Math.PI) * (0.22f + 0.38f * r3) * twinkle;
            if (a <= 0.02f) continue;
            float size = r2 > 0.82f ? 2f : 1f;
            Gfx.fill(x, y, x + size, y + size, Gfx.withAlpha(r1 > 0.3f ? SuiteTheme.accentBright() : MOTE_WHITE, a));
        }
    }

    private void drawBrand(float t, boolean reduced) {
        float a = fade(t, 0.15f, 1.0f);
        if (a <= 0.01f) return;
        int logoSize = Math.max(48, Math.min(104, (int) (height / 3.6f)));
        float wordScale = Math.min(3.0f, Math.max(1.6f, width / 420.0f));
        int wordH = (int) (9 * wordScale);
        int blockH = logoSize + 12 + wordH + 9 + 1 + 7 + 9;
        int cx = width - Math.max(32, (int) (width * 0.19f));
        int top = Math.max(40, (int) (height * 0.44f) - blockH / 2);
        int cy = top + logoSize / 2;
        String word = "ESPECTRAL";
        int wpx = (int) (fontRendererObj.getStringWidth(word) * wordScale);
        int accentRgb = SuiteTheme.accent() & 0xFFFFFF;

        int poolW = (int) (Math.max(wpx, logoSize) * 1.25f);
        int poolH = (int) (blockH * 0.95f);
        int poolCy = top + blockH / 2;
        Gfx.sprite(GLOW, cx - poolW, poolCy - poolH, poolW * 2, poolH * 2, Gfx.argb(0.62f * a, 0x040507));

        float rise = (1f - Gfx.easeOutCubic(a)) * 10f;
        float bob = reduced ? 0f : 2.2f * (float) Math.sin(t * (Math.PI * 2 / 4.8));
        Gfx.push(-parX * 5f, -parY * 3.5f + bob + rise);
        float pulse = reduced ? 0.5f : 0.5f + 0.5f * (float) Math.sin(t * (Math.PI * 2 / 5.5));
        int halo = (int) (logoSize * (0.95f + 0.07f * pulse));
        Gfx.sprite(GLOW, cx - halo, cy - halo, halo * 2, halo * 2, Gfx.argb((0.38f + 0.22f * pulse) * a, accentRgb));
        Gfx.sprite(LOGO, cx - logoSize / 2, cy - logoSize / 2, logoSize, logoSize, Gfx.argb(a, 0xFFFFFF));
        Gfx.pop();

        int wy = cy + logoSize / 2 + 12;
        if (a > 0.02f) {
            Gfx.textScaledGradient(word, cx - wpx / 2, wy, wordScale,
                    Gfx.withAlpha(SuiteTheme.accentGradientLeft(), a), Gfx.withAlpha(SuiteTheme.accentGradientRight(), a));
        }
        int dy = wy + wordH + 9;
        int half = Math.max(24, wpx / 3);
        int seg = half / 3;
        Gfx.fill(cx - half, dy, cx - half + seg, dy + 1, Gfx.argb(0.18f * a, accentRgb));
        Gfx.fill(cx - half + seg, dy, cx + half - seg, dy + 1, Gfx.argb(0.55f * a, accentRgb));
        Gfx.fill(cx + half - seg, dy, cx + half, dy + 1, Gfx.argb(0.18f * a, accentRgb));
        float tagA = fade(t, 0.7f, 0.8f);
        if (tagA > 0.02f) {
            String custom = ClientConfig.getInstance().tagline();
            String tag = custom != null ? custom : Lang.tr("espectral.menu.tagline");
            int tcx = Math.min(cx, width - 8 - fontRendererObj.getStringWidth(tag) / 2);
            Gfx.textCentered(tag, tcx, dy + 7, Gfx.withAlpha(0xFFD8D2C4, tagA), true);
        }
    }

    @Override
    public boolean doesGuiPauseGame() {
        return false;
    }

    /** 0 before {@code start} s, 1 after {@code start + dur} s, linear between. */
    private static float fade(float t, float start, float dur) {
        return Gfx.clamp01((t - start) / dur);
    }

    /** Stable hash of {@code n} to [0, 1). */
    private static float rnd(int n) {
        int h = n * 0x9E3779B1;
        h ^= h >>> 15;
        h *= 0x85EBCA77;
        h ^= h >>> 13;
        return (h & 0xFFFFFF) / 16777216f;
    }

    /** Left-stack glass button with the staggered slide-in and the hover chevron. */
    private final class TitleButton extends GuiButton {
        final int index;
        float hover;
        float dt;

        TitleButton(int id, int index, int x, int y, int w, int h, String label) {
            super(id, x, y, w, h, label);
            this.index = index;
        }

        @Override
        public void drawButton(Minecraft mc, int mouseX, int mouseY) {
            if (!visible) return;
            hovered = mouseX >= xPosition && mouseY >= yPosition && mouseX < xPosition + width
                    && mouseY < yPosition + height;
            boolean hot = focused >= 0 ? focused == index : hovered;
            hover = MenuSkin.easeHover(hover, hot, dt);
            float appear = Gfx.easeOutCubic(fade(intro, 0.35f + 0.08f * index, 0.5f));
            if (appear <= 0.01f) return;
            Gfx.push((1f - appear) * -16f, 0f);
            MenuSkin.paintButton(xPosition, yPosition, width, height, true, hover, appear);
            if (appear > 0.03f) {
                int textColor = Gfx.scaleAlpha(SuiteTheme.lerpArgb(SuiteTheme.TEXT, SuiteTheme.accentBright(), hover), appear);
                int ty = yPosition + (height - 8) / 2;
                Gfx.text(displayString, xPosition + 12 + Math.round(3f * hover), ty, textColor, true);
                if (hover * appear > 0.03f) {
                    Gfx.text("\u203A", xPosition + width - 14 + Math.round(3f * (1f - hover)), ty,
                            Gfx.withAlpha(SuiteTheme.accentBright(), hover * appear), false);
                }
            }
            Gfx.pop();
        }
    }

    /** Small glyph button of the icon row (tooltip drawn by the screen). */
    private final class IconButton extends GuiButton {
        final String tooltip;
        float hover;
        float dt;

        IconButton(int id, int x, int y, int size, String glyph, String tooltip) {
            super(id, x, y, size, size, glyph);
            this.tooltip = tooltip;
        }

        @Override
        public void drawButton(Minecraft mc, int mouseX, int mouseY) {
            if (!visible) return;
            hovered = mouseX >= xPosition && mouseY >= yPosition && mouseX < xPosition + width
                    && mouseY < yPosition + height;
            float alpha = fade(intro, 0.5f, 0.6f);
            if (alpha <= 0.02f) return;
            hover = MenuSkin.easeHover(hover, hovered, dt);
            if (hover > 0.02f) {
                Gfx.fill(xPosition, yPosition, xPosition + width, yPosition + height, Gfx.argb(0.45f * hover * alpha, 0));
                Gfx.outline(xPosition, yPosition, width, height,
                        Gfx.argb(0.5f * hover * alpha, SuiteTheme.accent() & 0xFFFFFF));
            }
            int color = Gfx.scaleAlpha(SuiteTheme.lerpArgb(0xFFC9CDD3, SuiteTheme.accentBright(), hover), alpha);
            Gfx.textCentered(displayString, xPosition + width / 2, yPosition + (height - 8) / 2, color, true);
        }
    }

    /** Icons are glyphs: the agent sees their tooltip instead. */
    @Override
    public String agentLabel(GuiButton b) {
        return b instanceof IconButton ? ((IconButton) b).tooltip : null;
    }
}
