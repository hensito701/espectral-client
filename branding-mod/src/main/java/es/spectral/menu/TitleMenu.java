package es.spectral.menu;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import net.minecraft.SharedConstants;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.util.Util;
import com.mojang.blaze3d.platform.InputConstants;

import es.spectral.menu.ui.MenuSkin;
import es.spectral.menu.ui.SuiteTheme;

/**
 * Shared controller for the Espectral custom main menu. Not a {@link Screen} —
 * the per-version {@code EspectralTitleScreen} owns the Screen lifecycle and
 * delegates {@code init}/{@code render}/{@code mouseClicked}/{@code keyPressed}
 * here, so all layout, art direction and input live in one place and compile
 * against both supported versions.
 *
 * <p>Layout: a full-bleed background (the user PNG from {@link MenuBackground}
 * with a slow sub-pixel Ken Burns drift and a light mouse parallax, else a
 * branded charcoal), a left-hand readability scrim, drifting gold motes, a
 * vertical button stack on the left, and the circular logo (with a breathing
 * gold halo) + "ESPECTRAL" wordmark + tagline on the right. A small icon row
 * sits top-left (quit, accessibility, screenshots folder) and a support heart
 * top-right; icons carry tooltips.
 *
 * <p>The first title screen of a session plays a short entrance (background
 * reveal, staggered buttons); later returns render settled immediately.
 * Buttons respond to the mouse and to Up/Down/Tab + Enter/Space.
 *
 * <p>All drawing goes through {@code Compat.ui*} primitives taking the
 * version's graphics object as {@code Object}, so this class never names a
 * version-specific type.
 */
public final class TitleMenu {

    private static final Identifier LOGO = Identifier.fromNamespaceAndPath("espectral-menu", "logo");
    /** White radial falloff, tinted at draw time (gold halo, dark contact shadow). */
    private static final Identifier GLOW = Identifier.fromNamespaceAndPath("espectral-menu", "glow");
    /** Horizontal near-black → transparent readability gradient. */
    private static final Identifier SCRIM = Identifier.fromNamespaceAndPath("espectral-menu", "scrim");

    private static final int BTN_FILL = MenuSkin.BTN_FILL;
    private static final int BTN_FILL_HOT = MenuSkin.BTN_FILL_HOT;
    private static final int MOTE_WHITE = 0xFFF4EEDF;
    private static final int MOTES = 28;
    private static final Component CHEVRON = Component.literal("›");
    private static final Component DEFAULT_TAGLINE = Component.translatable("espectral.menu.tagline");

    /** Hot-state button edge: 80%-alpha accent (theme-driven, was a constant). */
    private static int btnEdgeHot() {
        return (SuiteTheme.accent() & 0xFFFFFF) | 0xCC000000;
    }

    /** Session start of the entrance animation; -1 until the first title frame. */
    private static long introStartMs = -1;

    private final Minecraft minecraft;
    private final Screen host;

    // Layout rects (recomputed on init/resize).
    private final List<Btn> buttons = new ArrayList<>();
    private final List<Btn> icons = new ArrayList<>();
    private int width;
    private int height;
    private Component footer;
    private String taglineSource;
    private Component taglineText;

    // Interaction + animation state (per screen instance).
    private int focused = -1;
    private int lastMouseX = Integer.MIN_VALUE;
    private int lastMouseY = Integer.MIN_VALUE;
    private long lastFrameMs = -1;
    private float parX;
    private float parY;

    public TitleMenu(Minecraft minecraft, Screen host) {
        this.minecraft = minecraft;
        this.host = host;
    }

    /** One clickable region: a label (lang key or glyph) + an action. */
    private static final class Btn {
        final Component text;
        final Component tooltip;
        final Runnable action;
        int x, y, w, h;
        float hover;

        Btn(Component text, Component tooltip, Runnable action) {
            this.text = text;
            this.tooltip = tooltip;
            this.action = action;
        }

        boolean hit(double mx, double my) {
            return mx >= x && mx < x + w && my >= y && my < y + h;
        }
    }

    // ── Layout ──────────────────────────────────────────────────────────────

    /** Recompute layout for the current screen size. Called from init/resize. */
    public void init(int width, int height) {
        this.width = width;
        this.height = height;
        // Off-thread decode; the first call of the session starts it, the
        // rest are no-ops. The branded fallback shows until it lands.
        MenuBackground.request(minecraft);
        buttons.clear();
        icons.clear();
        footer = Component.literal("Minecraft " + SharedConstants.getCurrentVersion().name()
                + " · Espectral " + EspectralBrand.modVersion());

        // Left vertical stack, a little below centre.
        int bw = Math.min(160, Math.max(116, width / 5));
        int bh = 22;
        int gap = 6;
        int bx = Math.max(18, width / 12);
        int total = 4 * bh + 3 * gap;
        int by = (height - total) / 2 + height / 10;

        addButton("espectral.menu.singleplayer", bx, by, bw, bh, () ->
                Compat.open(minecraft, newScreen("selectWorld")));
        addButton("espectral.menu.multiplayer", bx, by + (bh + gap), bw, bh, () ->
                Compat.open(minecraft, newScreen("joinMultiplayer")));
        addButton("espectral.menu.options", bx, by + 2 * (bh + gap), bw, bh, () ->
                Compat.open(minecraft, Compat.optionsScreen(minecraft, host)));
        addButton("espectral.menu.suite", bx, by + 3 * (bh + gap), bw, bh, () ->
                Compat.open(minecraft, new es.spectral.menu.ui.SuiteScreen(host)));
        if (focused >= buttons.size()) {
            focused = -1;
        }

        // Top-left icon row: quit, accessibility, screenshots folder.
        int is = 18;
        int iy = 8;
        int ix = 8;
        addIcon("✕", "espectral.menu.quit", ix, iy, is, minecraft::stop);
        addIcon("♿", "espectral.menu.accessibility", ix + is + 4, iy, is, () ->
                Compat.open(minecraft, newScreen("accessibility")));
        addIcon("📷", "espectral.menu.screenshots", ix + 2 * (is + 4), iy, is, this::openScreenshots);

        // Top-right support heart.
        addIcon("♥", "espectral.menu.support", width - 8 - is, iy, is, () ->
                Compat.open(minecraft, es.spectral.menu.ui.SuiteConfirmScreen.support(host)));
    }

    private void addButton(String key, int x, int y, int w, int h, Runnable action) {
        Btn b = new Btn(Component.translatable(key), null, action);
        b.x = x; b.y = y; b.w = w; b.h = h;
        buttons.add(b);
    }

    private void addIcon(String glyph, String tooltipKey, int x, int y, int s, Runnable action) {
        Btn b = new Btn(Component.literal(glyph), Component.translatable(tooltipKey), action);
        b.x = x; b.y = y; b.w = s; b.h = s;
        icons.add(b);
    }

    /** Version-divergent screen construction, resolved through Compat. */
    private Screen newScreen(String which) {
        return Compat.menuTarget(minecraft, host, which);
    }

    /** Opens {@code <gameDir>/screenshots} (vanilla's F2 target) in the OS file browser. */
    private void openScreenshots() {
        if (minecraft.gameDirectory == null) {
            return;
        }
        Path dir = minecraft.gameDirectory.toPath().resolve("screenshots");
        try {
            Files.createDirectories(dir);
            Compat.openPath(dir);
        } catch (Exception e) {
            org.slf4j.LoggerFactory.getLogger("espectral-menu")
                    .warn("Could not open screenshots folder {}", dir, e);
        }
    }

    // ── Render ──────────────────────────────────────────────────────────────

    /**
     * Draw the whole menu. {@code gfx} is the version's graphics object
     * (GuiGraphicsExtractor on 26.2, GuiGraphics on 1.21.11).
     */
    public void render(Object gfx, Font font, int mouseX, int mouseY, float delta) {
        // Reduced motion (personalization.reduced_motion): no entrance, no
        // parallax, no drifting motes, no Ken Burns — everything settles.
        boolean reduced = ClientConfig.getInstance().reducedMotion();
        long now = Util.getMillis();
        if (introStartMs < 0) {
            introStartMs = now;
        }
        float dt = lastFrameMs < 0 ? 0f : Math.min(0.1f, (now - lastFrameMs) / 1000f);
        lastFrameMs = now;
        float intro = reduced ? Float.MAX_VALUE : (now - introStartMs) / 1000f;

        // A real mouse move hands focus back from the keyboard to the pointer.
        if (mouseX != lastMouseX || mouseY != lastMouseY) {
            if (lastMouseX != Integer.MIN_VALUE) {
                focused = -1;
            }
            lastMouseX = mouseX;
            lastMouseY = mouseY;
        }

        // Eased parallax target in [-1, 1] from the pointer position.
        if (!reduced && width > 0 && height > 0) {
            float tx = clamp01(mouseX / (float) width) * 2f - 1f;
            float ty = clamp01(mouseY / (float) height) * 2f - 1f;
            float k = 1f - (float) Math.exp(-dt * 2.5f);
            parX += (tx - parX) * k;
            parY += (ty - parY) * k;
        } else {
            parX = 0f;
            parY = 0f;
        }

        drawBackground(gfx, now, reduced);
        if (!reduced) {
            drawMotes(gfx, intro, fade(intro, 0.6f, 1.6f));
        }
        drawBrand(gfx, font, intro, reduced);
        drawButtons(gfx, font, mouseX, mouseY, dt, intro);
        drawIcons(gfx, font, mouseX, mouseY, dt, fade(intro, 0.5f, 0.6f));

        // Footer: game + suite version, bottom-right, dim.
        float footA = fade(intro, 0.9f, 0.6f);
        if (footA > 0.02f && footer != null) {
            Compat.uiText(gfx, font, footer,
                    width - font.width(footer) - 8, height - 14,
                    withAlpha(SuiteTheme.TEXT_DIM, footA), false);
        }
    }

    // ── Background ──────────────────────────────────────────────────────────

    private void drawBackground(Object gfx, long now, boolean reduced) {
        // Shared with every out-of-world menu (MenuSkin), so the drift carries
        // on seamlessly into Multiplayer/Options; parallax is title-only.
        MenuSkin.paintArt(gfx, width, height, now, reduced, parX, parY);

        // Readability: soft left scrim behind the buttons, a bottom vignette
        // for the footer, and a thin top shade for the icon row.
        Compat.uiSprite(gfx, SCRIM, 0, 0, Math.max(1, (int) (width * 0.62f)), height, 0xFFFFFFFF);
        Compat.uiFillGradient(gfx, 0, (int) (height * 0.55f), width, height, 0x00000000, 0xA6000000);
        Compat.uiFillGradient(gfx, 0, 0, width, 36, 0x73000000, 0x00000000);
    }

    /**
     * Drifting gold "spectral" motes: deterministic per index, rising slowly
     * with a gentle sway and twinkle. Drawn at sub-pixel positions.
     */
    private void drawMotes(Object gfx, float t, float alpha) {
        if (alpha <= 0.01f || width <= 0 || height <= 0) {
            return;
        }
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
            if (a <= 0.02f) {
                continue;
            }
            int size = r2 > 0.82f ? 2 : 1;
            Compat.uiPush(gfx, x, y);
            Compat.uiFill(gfx, 0, 0, size, size, withAlpha(r1 > 0.3f ? SuiteTheme.accentBright() : MOTE_WHITE, a));
            Compat.uiPop(gfx);
        }
    }

    // ── Brand ───────────────────────────────────────────────────────────────

    /** Circular logo with halo + "ESPECTRAL" gradient wordmark + tagline, right side. */
    private void drawBrand(Object gfx, Font font, float t, boolean reduced) {
        float a = fade(t, 0.15f, 1.0f);
        if (a <= 0.01f) {
            return;
        }
        int logoSize = Math.max(48, Math.min(104, (int) (height / 3.6f)));
        float wordScale = Math.min(3.0f, Math.max(1.6f, width / 420.0f));
        int wordH = (int) (9 * wordScale);
        int blockH = logoSize + 12 + wordH + 9 + 1 + 7 + 9;
        int cx = width - Math.max(32, (int) (width * 0.19f));
        int top = Math.max(40, (int) (height * 0.44f) - blockH / 2);
        int cy = top + logoSize / 2;
        String word = "ESPECTRAL";
        int wpx = (int) (font.width(word) * wordScale);
        int accentRgb = SuiteTheme.accent() & 0xFFFFFF;

        // Soft dark pool behind the whole block so the accent reads over any photo.
        int poolW = (int) (Math.max(wpx, logoSize) * 1.25f);
        int poolH = (int) (blockH * 0.95f);
        int poolCy = top + blockH / 2;
        Compat.uiSprite(gfx, GLOW, cx - poolW, poolCy - poolH, poolW * 2, poolH * 2,
                argb(0.62f * a, 0x040507));

        // Logo group floats (4.8 s bob) and counter-parallaxes the background;
        // both freeze under reduced motion.
        float rise = (1f - easeOutCubic(a)) * 10f;
        float bob = reduced ? 0f : 2.2f * (float) Math.sin(t * (Math.PI * 2 / 4.8));
        Compat.uiPush(gfx, -parX * 5f, -parY * 3.5f + bob + rise);
        float pulse = reduced ? 0.5f : 0.5f + 0.5f * (float) Math.sin(t * (Math.PI * 2 / 5.5));
        int halo = (int) (logoSize * (0.95f + 0.07f * pulse));
        Compat.uiSprite(gfx, GLOW, cx - halo, cy - halo, halo * 2, halo * 2,
                argb((0.38f + 0.22f * pulse) * a, accentRgb));
        Compat.uiSprite(gfx, LOGO, cx - logoSize / 2, cy - logoSize / 2, logoSize, logoSize,
                argb(a, 0xFFFFFF));
        Compat.uiPop(gfx);

        // Wordmark: accent gradient, scaled, with a drop shadow.
        int wy = cy + logoSize / 2 + 12;
        if (a > 0.02f) {
            Compat.uiTextScaledGradient(gfx, font, word, cx - wpx / 2, wy, wordScale,
                    withAlpha(SuiteTheme.accentGradientLeft(), a),
                    withAlpha(SuiteTheme.accentGradientRight(), a));
        }

        // Hairline divider fading out at both ends, then the tagline.
        int dy = wy + wordH + 9;
        int half = Math.max(24, wpx / 3);
        int seg = half / 3;
        Compat.uiFill(gfx, cx - half, dy, cx - half + seg, dy + 1, argb(0.18f * a, accentRgb));
        Compat.uiFill(gfx, cx - half + seg, dy, cx + half - seg, dy + 1, argb(0.55f * a, accentRgb));
        Compat.uiFill(gfx, cx + half - seg, dy, cx + half, dy + 1, argb(0.18f * a, accentRgb));
        float tagA = fade(t, 0.7f, 0.8f);
        if (tagA > 0.02f) {
            // Custom lines can be wider than the wordmark: keep them on screen.
            Component tag = tagline();
            int tcx = Math.min(cx, width - 8 - font.width(tag) / 2);
            Compat.uiTextCentered(gfx, font, tag,
                    tcx, dy + 7, withAlpha(0xFFD8D2C4, tagA), true);
        }
    }

    /** Instance's custom tagline (launcher → Appearance), else the localized default. */
    private Component tagline() {
        String custom = ClientConfig.getInstance().tagline();
        if (custom == null) {
            return DEFAULT_TAGLINE;
        }
        if (!custom.equals(taglineSource)) {
            taglineSource = custom;
            taglineText = Component.literal(custom);
        }
        return taglineText;
    }

    // ── Buttons & icons ─────────────────────────────────────────────────────

    private void drawButtons(Object gfx, Font font, int mouseX, int mouseY, float dt, float intro) {
        float k = 1f - (float) Math.exp(-dt * 14f);
        for (int i = 0; i < buttons.size(); i++) {
            Btn b = buttons.get(i);
            boolean hot = focused >= 0 ? focused == i : b.hit(mouseX, mouseY);
            b.hover += ((hot ? 1f : 0f) - b.hover) * k;
            float h = b.hover;
            float appear = easeOutCubic(fade(intro, 0.35f + 0.08f * i, 0.5f));
            if (appear <= 0.01f) {
                continue;
            }

            Compat.uiPush(gfx, (1f - appear) * -16f, 0f);
            Compat.uiFill(gfx, b.x, b.y, b.x + b.w, b.y + b.h,
                    scaleAlpha(SuiteTheme.lerpArgb(BTN_FILL, BTN_FILL_HOT, h), appear));
            Compat.uiOutline(gfx, b.x, b.y, b.w, b.h,
                    scaleAlpha(SuiteTheme.lerpArgb(SuiteTheme.accentHairline(), btnEdgeHot(), h), appear));
            // Gold accent bar grows from the centre of the left edge.
            int bar = Math.round((b.h - 2) * h);
            if (bar > 0) {
                int by = b.y + (b.h - bar) / 2;
                Compat.uiFill(gfx, b.x, by, b.x + 2, by + bar, scaleAlpha(SuiteTheme.accent(), appear));
            }
            if (appear > 0.03f) {
                int textColor = scaleAlpha(SuiteTheme.lerpArgb(SuiteTheme.TEXT, SuiteTheme.accentBright(), h), appear);
                int ty = b.y + (b.h - 9) / 2 + 1;
                Compat.uiText(gfx, font, b.text,
                        b.x + 12 + Math.round(3f * h), ty, textColor, true);
                if (h * appear > 0.03f) {
                    Compat.uiText(gfx, font, CHEVRON, b.x + b.w - 14 + Math.round(3f * (1f - h)), ty,
                            withAlpha(SuiteTheme.accentBright(), h * appear), false);
                }
            }
            Compat.uiPop(gfx);
        }
    }

    private void drawIcons(Object gfx, Font font, int mouseX, int mouseY, float dt, float alpha) {
        if (alpha <= 0.02f) {
            return;
        }
        float k = 1f - (float) Math.exp(-dt * 14f);
        for (Btn b : icons) {
            boolean hot = b.hit(mouseX, mouseY);
            b.hover += ((hot ? 1f : 0f) - b.hover) * k;
            if (b.hover > 0.02f) {
                Compat.uiFill(gfx, b.x, b.y, b.x + b.w, b.y + b.h, argb(0.45f * b.hover * alpha, 0x000000));
                Compat.uiOutline(gfx, b.x, b.y, b.w, b.h, argb(0.5f * b.hover * alpha, SuiteTheme.accent() & 0xFFFFFF));
            }
            int color = scaleAlpha(SuiteTheme.lerpArgb(0xFFC9CDD3, SuiteTheme.accentBright(), b.hover), alpha);
            Compat.uiTextCentered(gfx, font, b.text,
                    b.x + b.w / 2, b.y + (b.h - 9) / 2 + 1, color, true);
            if (hot && b.tooltip != null) {
                Compat.uiTooltip(gfx, font, b.tooltip, mouseX, mouseY);
            }
        }
    }

    // ── Input ───────────────────────────────────────────────────────────────

    /** Returns true when the click landed on a button or icon. */
    public boolean mouseClicked(double mx, double my, int button) {
        if (button != 0) return false;
        for (Btn b : buttons) {
            if (b.hit(mx, my)) { b.action.run(); return true; }
        }
        for (Btn b : icons) {
            if (b.hit(mx, my)) { b.action.run(); return true; }
        }
        return false;
    }

    /**
     * Keyboard navigation over the button stack: Up/Down (and Tab /
     * Shift+Tab) move the focus, Enter/Space activate it. Returns true when
     * the key was consumed.
     */
    public boolean keyPressed(int key, int modifiers) {
        int n = buttons.size();
        if (n == 0) return false;
        switch (key) {
            case InputConstants.KEY_DOWN:
                focused = focused < 0 ? 0 : (focused + 1) % n;
                return true;
            case InputConstants.KEY_UP:
                focused = focused < 0 ? n - 1 : (focused + n - 1) % n;
                return true;
            case InputConstants.KEY_TAB:
                boolean back = (modifiers & InputConstants.MOD_SHIFT) != 0;
                focused = focused < 0 ? (back ? n - 1 : 0) : (focused + (back ? n - 1 : 1)) % n;
                return true;
            case InputConstants.KEY_RETURN:
            case InputConstants.KEY_NUMPADENTER:
            case InputConstants.KEY_SPACE:
                if (focused >= 0 && focused < n) {
                    buttons.get(focused).action.run();
                    return true;
                }
                return false;
            default:
                return false;
        }
    }

    // ── Math ────────────────────────────────────────────────────────────────

    /** 0 before {@code start} s, 1 after {@code start + dur} s, linear between. */
    private static float fade(float t, float start, float dur) {
        return clamp01((t - start) / dur);
    }

    private static float clamp01(float v) {
        return v < 0f ? 0f : (v > 1f ? 1f : v);
    }

    private static float easeOutCubic(float t) {
        float u = 1f - t;
        return 1f - u * u * u;
    }

    /** RGB with the given opacity (0..1). */
    private static int argb(float alpha, int rgb) {
        return (Math.round(clamp01(alpha) * 255f) << 24) | (rgb & 0xFFFFFF);
    }

    /** Replaces the colour's alpha with {@code alpha} (0..1). */
    private static int withAlpha(int argb, float alpha) {
        return argb(alpha, argb);
    }

    /** Multiplies the colour's existing alpha by {@code factor} (0..1). */
    private static int scaleAlpha(int argb, float factor) {
        return argb(((argb >>> 24) / 255f) * factor, argb);
    }

    /** Stable hash of {@code n} to [0, 1). */
    private static float rnd(int n) {
        int h = n * 0x9E3779B1;
        h ^= h >>> 15;
        h *= 0x85EBCA77;
        h ^= h >>> 13;
        return (h & 0xFFFFFF) / 16777216f;
    }
}
