package es.spectral.menu;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsScreen;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.player.PlayerModelType;

/** Screen-opening compatibility for 26.2 (setScreen moved to Gui). */
public final class Compat {

    /** Loader token for the minecraft:brand handshake ({@code espectral:<loader>[:<version>]}). */
    public static String brandLoader() {
        return "fabric";
    }

    public static void open(Minecraft minecraft, Screen screen) {
        minecraft.setScreenAndShow(screen);
    }

    public static Screen optionsScreen(Minecraft minecraft, Screen back) {
        return new OptionsScreen(back, minecraft.options, false);
    }

    public static boolean isScreenOpen(Minecraft minecraft) {
        return minecraft.gui != null && minecraft.gui.screen() != null;
    }

    public static Screen getScreen(Minecraft minecraft) {
        return minecraft.gui != null ? minecraft.gui.screen() : null;
    }

    public static void sendChat(Minecraft minecraft, String text) {
        if (minecraft.player != null && minecraft.player.connection != null) {
            if (text.startsWith("/")) {
                minecraft.player.connection.sendCommand(text.substring(1));
            } else {
                minecraft.player.connection.sendChat(text);
            }
        }
    }

    /**
     * Submits the Espectral text HUD overlays into the 26.2 extraction
     * pipeline: {@link HudEngine} left stack top-left, potion column
     * top-right. White text with shadow. Skips when there is no player/world
     * or the vanilla HUD is hidden (F1).
     */
    public static void drawHudOverlays(Minecraft minecraft,
            net.minecraft.client.gui.GuiGraphicsExtractor extractor) {
        if (minecraft == null || extractor == null) return;
        if (minecraft.player == null || minecraft.level == null) return;
        if (minecraft.gui != null && minecraft.gui.hud != null && minecraft.gui.hud.isHidden()) return;
        net.minecraft.client.gui.Font font = minecraft.font;
        if (font == null) return;
        int y = 4;
        for (String line : HudEngine.getInstance().leftLines(minecraft)) {
            extractor.text(font, line, 4, y, 0xFFFFFF, true);
            y += 10;
        }
        int width = extractor.guiWidth();
        int rightY = 4;
        for (String line : HudEngine.getInstance().rightLines(minecraft)) {
            extractor.text(font, line, width - font.width(line) - 4, rightY, 0xFFFFFF, true);
            rightY += 10;
        }
    }

    /**
     * Submits a ChatHeads player head at chat-local (x, y) into the 26.2
     * extraction pipeline. No-op on null input; callers resolve the skin
     * through {@link ChatHeads} (gated on the {@code chatheads} feature
     * flag), so a disabled feature never reaches this call with a skin.
     */
    public static void drawChatHead(net.minecraft.client.gui.GuiGraphicsExtractor extractor,
            net.minecraft.world.entity.player.PlayerSkin skin, int x, int y, int size) {
        if (extractor == null || skin == null) return;
        net.minecraft.client.gui.components.PlayerFaceExtractor.extractRenderState(extractor, skin, x, y, size);
    }

    /**
     * Resolves the voxelized player model for a third-person render state.
     * Returns {@code null} (caller keeps the vanilla model) when the
     * {@code skin3d} feature is off, the state carries no usable skin, or the
     * skin texture has not downloaded yet. Slim comes from the skin; the
     * renderer variant is only a fallback for unknown model types.
     */
    public static PlayerModel voxelForState(AvatarRenderState state, boolean rendererSlim) {
        if (!ClientConfig.getInstance().isFeatureEnabled("skin3d")) return null;
        if (state == null || state.skin == null || state.skin.body() == null) return null;
        boolean slim = state.skin.model() == PlayerModelType.SLIM
                || (state.skin.model() != PlayerModelType.WIDE && rendererSlim);
        return SkinModelCache.getOrBuild(Minecraft.getInstance(), state.skin.body().texturePath(), slim);
    }

    /**
     * Resolves the voxelized player model for the first-person hand path,
     * which only carries the skin id (slim comes from the renderer variant).
     * Returns {@code null} when the feature is off or the texture is missing.
     */
    public static PlayerModel voxelForHand(Identifier skinId, boolean slim) {
        if (!ClientConfig.getInstance().isFeatureEnabled("skin3d")) return null;
        if (skinId == null) return null;
        return SkinModelCache.getOrBuild(Minecraft.getInstance(), skinId, slim);
    }
    /**
     * Suite painter primitive: filled rectangle through the 26.2 extraction
     * pipeline. The graphics object is the lane's
     * {@code GuiGraphicsExtractor}; anything else is ignored.
     */
    public static void uiFill(Object gfx, int x1, int y1, int x2, int y2, int argb) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphicsExtractor extractor) {
            extractor.fill(x1, y1, x2, y2, argb);
        }
    }

    /** Suite painter primitive: vertical gradient rectangle. */
    public static void uiFillGradient(Object gfx, int x1, int y1, int x2, int y2,
            int argbTop, int argbBottom) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphicsExtractor extractor) {
            extractor.fillGradient(x1, y1, x2, y2, argbTop, argbBottom);
        }
    }

    /** Suite painter primitive: 1 px rectangle outline. */
    public static void uiOutline(Object gfx, int x, int y, int w, int h, int argb) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphicsExtractor extractor) {
            extractor.outline(x, y, w, h, argb);
        }
    }

    /** Suite painter primitive: left-aligned text. */
    public static void uiText(Object gfx, net.minecraft.client.gui.Font font,
            net.minecraft.network.chat.Component text, int x, int y, int argb, boolean shadow) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphicsExtractor extractor && font != null
                && text != null) {
            extractor.text(font, text, x, y, argb, shadow);
        }
    }

    /**
     * Suite painter primitive: text centred on {@code cx}. The extractor's
     * centred variant carries no shadow flag, so the shadow is an offset
     * darkened copy, matching the vanilla shadow tone.
     */
    public static void uiTextCentered(Object gfx, net.minecraft.client.gui.Font font,
            net.minecraft.network.chat.Component text, int cx, int y, int argb, boolean shadow) {
        if (!(gfx instanceof net.minecraft.client.gui.GuiGraphicsExtractor extractor)
                || font == null || text == null) {
            return;
        }
        if (shadow) {
            int shadowArgb = (argb & 0xFCFCFC) >> 2 | 0xFF000000;
            extractor.centeredText(font, text, cx + 1, y + 1, shadowArgb);
        }
        extractor.centeredText(font, text, cx, y, argb);
    }

    /**
     * Suite painter primitive: left-aligned text with a horizontal
     * per-character gradient (light left, deep right). Drawn one code point
     * at a time through the 26.2 extraction pipeline; anything else is
     * ignored. Plain {@code Component.getString()} text: gold labels carry
     * no inline formatting, so no style runs are lost.
     */
    public static void uiTextGradient(Object gfx, net.minecraft.client.gui.Font font,
            net.minecraft.network.chat.Component text, int x, int y,
            int argbLeft, int argbRight, boolean shadow) {
        if (!(gfx instanceof net.minecraft.client.gui.GuiGraphicsExtractor extractor)
                || font == null || text == null) {
            return;
        }
        String s = text.getString();
        int total = font.width(s);
        if (total <= 0) {
            return;
        }
        int cx = x;
        for (int i = 0; i < s.length();) {
            int cp = s.codePointAt(i);
            String ch = new String(Character.toChars(cp));
            int w = font.width(ch);
            float t = (float) (cx - x + w / 2) / (float) total;
            int color = es.spectral.menu.ui.SuiteTheme.lerpArgb(argbLeft, argbRight, t);
            extractor.text(font, net.minecraft.network.chat.Component.literal(ch),
                    cx, y, color, shadow);
            cx += w;
            i += Character.charCount(cp);
        }
    }

    /**
     * Suite painter primitive: gradient text centred on {@code cx}. Same
     * per-character interpolation as {@link #uiTextGradient}; the shadow, if
     * any, is drawn per character by the extractor.
     */
    public static void uiTextCenteredGradient(Object gfx, net.minecraft.client.gui.Font font,
            net.minecraft.network.chat.Component text, int cx, int y,
            int argbLeft, int argbRight, boolean shadow) {
        if (gfx == null || font == null || text == null) {
            return;
        }
        int total = font.width(text.getString());
        if (total <= 0) {
            return;
        }
        uiTextGradient(gfx, font, text, cx - total / 2, y, argbLeft, argbRight, shadow);
    }

    /**
     * Opens the canonical support URL in the system browser. Called only
     * after the SuiteConfirmScreen confirmation, never with parameters.
     */
    public static void openUri(String uri) {
        if (uri == null || uri.isBlank()) return;
        try {
            net.minecraft.util.Util.getPlatform().openUri(uri);
        } catch (Exception e) {
            org.slf4j.LoggerFactory.getLogger("espectral-menu")
                    .warn("Could not open support URL", e);
        }
    }
}
