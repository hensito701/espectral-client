package es.spectral.menu;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;

import es.spectral.menu.ui.SuiteTheme;

/**
 * Shared controller for the Espectral custom main menu. Not a {@link Screen} —
 * the per-version {@code EspectralTitleScreen} owns the Screen lifecycle and
 * delegates {@code init}/{@code render}/{@code mouseClicked} here, so all
 * layout, art direction and click handling live in one place and compile
 * against both supported versions.
 *
 * <p>Layout (per the approved design): a full-bleed background (user PNG with
 * a slow Ken Burns pan, else a branded gradient), a vertical button stack on
 * the left, and the circular logo + "ESPECTRAL" wordmark on the right. A small
 * icon row sits top-left (quit, accessibility, screenshot) and a support heart
 * top-right.
 *
 * <p>All drawing goes through {@code Compat.ui*} primitives taking the
 * version's graphics object as {@code Object}, so this class never names a
 * version-specific type.
 */
public final class TitleMenu {

    /** Bundled logo, drawn through the GUI sprite atlas (alpha-safe). */
    private static final Identifier LOGO =
            Identifier.fromNamespaceAndPath("espectral-menu", "logo");

    /** Where a user-supplied background PNG is looked for, in priority order. */
    private static List<Path> backgroundCandidates(Minecraft minecraft) {
        List<Path> out = new ArrayList<>(3);
        String override = System.getProperty("espectral.menu.background");
        if (override != null && !override.isBlank()) {
            out.add(Path.of(override));
        }
        if (minecraft != null && minecraft.gameDirectory != null) {
            out.add(minecraft.gameDirectory.toPath().resolve("config").resolve("espectral-menu-background.png"));
            out.add(minecraft.gameDirectory.toPath().resolve("espectral-menu-background.png"));
        }
        return out;
    }

    private final Minecraft minecraft;
    private final Screen host;

    // Background texture state (loaded lazily, cached).
    private boolean bgTried;
    private Identifier bgTexture;
    private int bgW;
    private int bgH;
    private long bgStartMs = -1;

    // Layout rects (recomputed on init/resize).
    private final List<Btn> buttons = new ArrayList<>();
    private final List<Btn> icons = new ArrayList<>();
    private int width;
    private int height;

    public TitleMenu(Minecraft minecraft, Screen host) {
        this.minecraft = minecraft;
        this.host = host;
    }

    /** One clickable region: a label + an action. */
    private static final class Btn {
        final String key;      // lang key
        final Runnable action;
        int x, y, w, h;
        Btn(String key, Runnable action) { this.key = key; this.action = action; }
        boolean hit(double mx, double my) { return mx >= x && mx < x + w && my >= y && my < y + h; }
    }

    // ── Layout ──────────────────────────────────────────────────────────────

    /** Recompute layout for the current screen size. Called from init/resize. */
    public void init(int width, int height) {
        this.width = width;
        this.height = height;
        buttons.clear();
        icons.clear();

        // Left vertical stack, vertically centered.
        int bw = Math.min(150, Math.max(110, width / 5));
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

        // Top-left icon row: quit, accessibility, screenshot.
        int is = 18;
        int iy = 10;
        int ix = 10;
        addIcon("✕", ix, iy, is, () -> minecraft.stop());
        addIcon("♿", ix + is + 6, iy, is, () ->
                Compat.open(minecraft, newScreen("accessibility")));
        addIcon("📷", ix + 2 * (is + 6), iy, is, () -> {}); // screenshot: no-op placeholder

        // Top-right support heart.
        addIcon("♥", width - 10 - is, iy, is, () ->
                Compat.open(minecraft, es.spectral.menu.ui.SuiteConfirmScreen.support(host)));
    }

    private void addButton(String key, int x, int y, int w, int h, Runnable action) {
        Btn b = new Btn(key, action);
        b.x = x; b.y = y; b.w = w; b.h = h;
        buttons.add(b);
    }

    private void addIcon(String glyph, int x, int y, int s, Runnable action) {
        Btn b = new Btn(glyph, action);
        b.x = x; b.y = y; b.w = s; b.h = s;
        icons.add(b);
    }

    /** Version-divergent screen construction, resolved through Compat. */
    private Screen newScreen(String which) {
        return Compat.menuTarget(minecraft, host, which);
    }

    // ── Background ──────────────────────────────────────────────────────────

    /** Lazily resolve + load the background texture. Null → gradient fallback. */
    private Identifier background() {
        if (bgTried) return bgTexture;
        bgTried = true;
        for (Path p : backgroundCandidates(minecraft)) {
            try {
                if (Files.isRegularFile(p)) {
                    Identifier id = Compat.loadExternalTexture(minecraft, p);
                    int[] dim = Compat.textureSize(minecraft, id);
                    if (id != null && dim != null && dim[0] > 0 && dim[1] > 0) {
                        bgTexture = id;
                        bgW = dim[0];
                        bgH = dim[1];
                        return bgTexture;
                    }
                }
            } catch (Exception ignored) {
                // try next candidate
            }
        }
        return null;
    }

    private void drawBackground(Object gfx) {
        Identifier bg = background();
        if (bg == null) {
            // Branded fallback: deep charcoal with a gold vignette glow.
            Compat.uiFill(gfx, 0, 0, width, height, 0xFF0B0D10);
            Compat.uiFillGradient(gfx, 0, 0, width, height, 0x00000000, 0x66000000);
            return;
        }
        // Ken Burns: a slowly drifting crop window over the source image.
        if (bgStartMs < 0) bgStartMs = System.currentTimeMillis();
        float t = (System.currentTimeMillis() - bgStartMs) / 1000.0f;
        // Cover-fit: scale source to fill the screen, then pan within the
        // overflow. ~5% zoom breathing over a 24s cycle.
        float zoom = 1.08f + 0.04f * (float) Math.sin(t * (Math.PI * 2 / 24.0));
        float scale = Math.max(width / (float) bgW, height / (float) bgH) * zoom;
        float drawW = bgW * scale;
        float drawH = bgH * scale;
        float overX = Math.max(0, drawW - width);
        float overY = Math.max(0, drawH - height);
        float px = (float) (0.5 + 0.5 * Math.sin(t * (Math.PI * 2 / 30.0)));
        float py = (float) (0.5 + 0.5 * Math.cos(t * (Math.PI * 2 / 36.0)));
        int dx = (int) (-overX * px);
        int dy = (int) (-overY * py);
        Compat.uiBlit(gfx, bg, dx, dy, (int) drawW, (int) drawH);
        // Readability scrim behind the left stack.
        Compat.uiFillGradient(gfx, 0, 0, width / 2, height, 0xB0080A0C, 0x00000000);
    }

    // ── Render ──────────────────────────────────────────────────────────────

    /**
     * Draw the whole menu. {@code gfx} is the version's graphics object
     * (GuiGraphicsExtractor on 26.2, GuiGraphics on 1.21.11).
     */
    public void render(Object gfx, Font font, int mouseX, int mouseY, float delta) {
        drawBackground(gfx);

        // Wordmark + logo, right side.
        drawBrand(gfx, font);

        // Buttons.
        for (Btn b : buttons) {
            boolean hot = b.hit(mouseX, mouseY);
            int fill = hot ? SuiteTheme.ROW_HOVER : 0x66090B0E;
            int edge = hot ? SuiteTheme.GOLD : SuiteTheme.GOLD_HAIRLINE;
            Compat.uiFill(gfx, b.x, b.y, b.x + b.w, b.y + b.h, fill);
            Compat.uiOutline(gfx, b.x, b.y, b.w, b.h, edge);
            int textColor = hot ? SuiteTheme.GOLD_BRIGHT : SuiteTheme.TEXT;
            Compat.uiText(gfx, font, Component.translatable(b.key), b.x + 12, b.y + (b.h - 9) / 2 + 1, textColor, false);
        }

        // Icon row (small, dim until hovered).
        for (Btn b : icons) {
            boolean hot = b.hit(mouseX, mouseY);
            int color = hot ? SuiteTheme.GOLD_BRIGHT : SuiteTheme.TEXT_DIM;
            Compat.uiTextCentered(gfx, font, Component.literal(b.key), b.x + b.w / 2, b.y + (b.h - 9) / 2 + 1, color, false);
        }

        // Footer: version + loader, bottom-right, dim.
        String footer = "v" + EspectralBrand.modVersion() + " · " + Compat.brandLoader();
        Compat.uiText(gfx, font, Component.literal(footer),
                width - font.width(footer) - 8, height - 14, SuiteTheme.TEXT_DIM, false);
    }

    /** Circular logo + "ESPECTRAL" gradient wordmark, right-aligned. */
    private void drawBrand(Object gfx, Font font) {
        int logoSize = Math.min(96, height / 4);
        int cx = width - Math.max(24, width / 10) - logoSize / 2;
        int cy = height / 3;

        // Logo through the GUI sprite atlas so transparency renders.
        Compat.uiSprite(gfx, LOGO, cx - logoSize / 2, cy - logoSize / 2, logoSize, logoSize);

        // Wordmark under the logo, gold gradient, scaled up.
        String word = "ESPECTRAL";
        float scale = Math.min(3.0f, Math.max(1.6f, width / 420.0f));
        int wpx = (int) (font.width(word) * scale);
        Compat.uiTextScaledGradient(gfx, font, word,
                cx - wpx / 2, cy + logoSize / 2 + 10, scale,
                SuiteTheme.GOLD_GRADIENT_LEFT, SuiteTheme.GOLD_GRADIENT_RIGHT);
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
}
