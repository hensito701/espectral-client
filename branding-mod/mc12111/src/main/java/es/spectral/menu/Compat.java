package es.spectral.menu;

import java.nio.file.Files;
import java.nio.file.Path;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsScreen;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.player.PlayerModelType;

/** Screen-opening compatibility for 1.21.11. */
public final class Compat {

    /** Loader token for the minecraft:brand handshake ({@code espectral:<loader>[:<version>]}). */
    public static String brandLoader() {
        return "fabric";
    }

    public static void open(Minecraft minecraft, Screen screen) {
        minecraft.setScreen(screen);
    }

    public static Screen optionsScreen(Minecraft minecraft, Screen back) {
        return new OptionsScreen(back, minecraft.options);
    }

    public static boolean isScreenOpen(Minecraft minecraft) {
        return minecraft.screen != null;
    }

    public static Screen getScreen(Minecraft minecraft) {
        return minecraft.screen;
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
     * Draws the Espectral text HUD overlays: {@link HudEngine} left stack
     * top-left, potion column top-right. White text with shadow. Skips when
     * there is no player/world or the vanilla HUD is hidden (F1).
     */
    public static void drawHudOverlays(Minecraft minecraft,
            net.minecraft.client.gui.GuiGraphics graphics) {
        if (minecraft == null || graphics == null) return;
        if (minecraft.player == null || minecraft.level == null) return;
        if (minecraft.options.hideGui) return;
        net.minecraft.client.gui.Font font = minecraft.font;
        if (font == null) return;
        int guiW = graphics.guiWidth();
        int guiH = graphics.guiHeight();
        for (HudEngine.Block block : HudEngine.getInstance().blocks(minecraft)) {
            int w = blockWidth(font, block);
            int h = blockHeight(block);
            int[] pos = HudLayout.pos(block.id, guiW, guiH, w, h);
            int x = pos[0], y = pos[1];
            // 50% black backing box, then opaque white text.
            graphics.fill(x, y, x + w, y + h, 0x80000000);
            int rowTop = y + HudLayout.PAD;
            for (HudEngine.Row row : block.rows) {
                int rowH = HudEditLogic.rowHeight(row);
                int iconW = 0;
                if (row.icon != null) {
                    int iconSize = row.icon.size();
                    drawIcon(graphics, minecraft, row.icon,
                            x + HudLayout.PAD, rowTop + (rowH - iconSize) / 2);
                    iconW = iconSize + 3;
                }
                int textY = rowTop + (rowH - 9) / 2;
                graphics.drawString(font, row.text, x + HudLayout.PAD + iconW, textY, 0xFFFFFFFF, true);
                rowTop += rowH;
            }
        }
    }

    /**
     * Draws one overlay row icon at its native size (hearts 9px, items
     * 16px, effect sprites 18px).
     */
    public static void drawIcon(net.minecraft.client.gui.GuiGraphics gfx,
            Minecraft minecraft, HudIcon icon, int x, int y) {
        if (gfx == null || icon == null) return;
        switch (icon.kind) {
            case HEART: {
                if (icon.heart == null) break;
                boolean hardcore = minecraft != null && minecraft.level != null
                        && minecraft.level.getLevelData().isHardcore();
                // Gui.HeartType is package-private in 1.21.11, so build the
                // same full-heart sprite id getSprite(hardcore,false,false)
                // would return (hud/heart/<variant>[_hardcore]_full).
                String variant;
                switch (icon.heart) {
                    case POISIONED: variant = "poisoned_"; break;
                    case WITHERED: variant = "withered_"; break;
                    case ABSORBING: variant = "absorbing_"; break;
                    case FROZEN: variant = "frozen_"; break;
                    case NORMAL:
                    default: variant = ""; break;
                }
                Identifier s = Identifier.withDefaultNamespace(
                        "hud/heart/" + variant + (hardcore ? "hardcore_" : "") + "full");
                gfx.blitSprite(RenderPipelines.GUI_TEXTURED, s, x, y, 9, 9);
                break;
            }
            case ITEM:
                if (icon.item != null) gfx.renderItem(icon.item, x, y);
                break;
            case EFFECT: {
                Identifier s = net.minecraft.client.gui.Gui.getMobEffectSprite(icon.effect);
                gfx.blitSprite(RenderPipelines.GUI_TEXTURED, s, x, y, 18, 18);
                break;
            }
        }
    }

    /** Widest row width + padding — the overlay box width. */
    private static int blockWidth(net.minecraft.client.gui.Font font, HudEngine.Block block) {
        int w = 0;
        for (HudEngine.Row row : block.rows) {
            w = Math.max(w, HudEditLogic.rowWidth(font, row));
        }
        return w + HudLayout.PAD * 2;
    }

    /** Padding + summed row heights — the overlay box height. */
    private static int blockHeight(HudEngine.Block block) {
        int h = HudLayout.PAD * 2;
        for (HudEngine.Row row : block.rows) {
            h += HudEditLogic.rowHeight(row);
        }
        return h;
    }

    /** Opens the drag-to-move HUD overlay editor. */
    public static void openHudEditor(Minecraft minecraft, Screen parent) {
        open(minecraft, new es.spectral.menu.ui.HudEditScreen(parent));
    }

    /**
     * Draws a ChatHeads player head at chat-local (x, y). No-op on null
     * input; callers resolve the skin through {@link ChatHeads} (gated on
     * the {@code chatheads} feature flag), so a disabled feature never
     * reaches this call with a skin.
     */
    public static void drawChatHead(net.minecraft.client.gui.GuiGraphics graphics,
            net.minecraft.world.entity.player.PlayerSkin skin, int x, int y, int size) {
        if (graphics == null || skin == null) return;
        net.minecraft.client.gui.components.PlayerFaceRenderer.draw(graphics, skin, x, y, size);
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
     * Suite painter primitive: filled rectangle in the lane's own
     * {@code GuiGraphics} idiom. Anything else is ignored.
     */
    public static void uiFill(Object gfx, int x1, int y1, int x2, int y2, int argb) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphics graphics) {
            graphics.fill(x1, y1, x2, y2, argb);
        }
    }

    /** Suite painter primitive: vertical gradient rectangle. */
    public static void uiFillGradient(Object gfx, int x1, int y1, int x2, int y2,
            int argbTop, int argbBottom) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphics graphics) {
            graphics.fillGradient(x1, y1, x2, y2, argbTop, argbBottom);
        }
    }

    /** Suite painter primitive: 1 px rectangle outline, drawn as four fills. */
    public static void uiOutline(Object gfx, int x, int y, int w, int h, int argb) {
        if (!(gfx instanceof net.minecraft.client.gui.GuiGraphics graphics)) {
            return;
        }
        graphics.fill(x, y, x + w, y + 1, argb);
        graphics.fill(x, y + h - 1, x + w, y + h, argb);
        graphics.fill(x, y, x + 1, y + h, argb);
        graphics.fill(x + w - 1, y, x + w, y + h, argb);
    }

    /** Suite painter primitive: left-aligned text. */
    public static void uiText(Object gfx, net.minecraft.client.gui.Font font,
            net.minecraft.network.chat.Component text, int x, int y, int argb, boolean shadow) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphics graphics && font != null
                && text != null) {
            graphics.drawString(font, text, x, y, argb, shadow);
        }
    }

    /**
     * Suite painter primitive: text centred on {@code cx}. The shadow is an
     * offset darkened copy, matching the vanilla shadow tone.
     */
    public static void uiTextCentered(Object gfx, net.minecraft.client.gui.Font font,
            net.minecraft.network.chat.Component text, int cx, int y, int argb, boolean shadow) {
        if (!(gfx instanceof net.minecraft.client.gui.GuiGraphics graphics)
                || font == null || text == null) {
            return;
        }
        int left = cx - font.width(text) / 2;
        if (shadow) {
            int shadowArgb = (argb & 0xFCFCFC) >> 2 | 0xFF000000;
            graphics.drawString(font, text, left + 1, y + 1, shadowArgb, false);
        }
        graphics.drawString(font, text, left, y, argb, false);
    }
    /**
     * Suite painter primitive: left-aligned text with a horizontal
     * per-character gradient (light left, deep right). Drawn one code point
     * at a time in the lane's own {@code GuiGraphics} idiom; anything else
     * is ignored. Plain {@code Component.getString()} text: gold labels carry
     * no inline formatting, so no style runs are lost.
     */
    public static void uiTextGradient(Object gfx, net.minecraft.client.gui.Font font,
            net.minecraft.network.chat.Component text, int x, int y,
            int argbLeft, int argbRight, boolean shadow) {
        if (!(gfx instanceof net.minecraft.client.gui.GuiGraphics graphics)
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
            graphics.drawString(font, net.minecraft.network.chat.Component.literal(ch),
                    cx, y, color, shadow);
            cx += w;
            i += Character.charCount(cp);
        }
    }

    /**
     * Suite painter primitive: gradient text centred on {@code cx}. Same
     * per-character interpolation as {@link #uiTextGradient}; the shadow, if
     * any, is drawn per character by the graphics object.
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
     * Title-menu primitive: draw the full texture scaled to {@code w} x {@code h}.
     * Anything else is ignored.
     */
    public static void uiBlit(Object gfx, Identifier id, int x, int y, int w, int h) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphics graphics && id != null) {
            graphics.blit(id, x, y, w, h, 0f, 0f, 1f, 1f);
        }
    }

    /**
     * Draws a GUI-atlas sprite (assets/<ns>/textures/gui/sprites/<path>.png)
     * scaled to w×h. Alpha-safe — use this for the logo and icons, not uiBlit
     * (standalone mod textures don't auto-load through the blit path).
     */
    public static void uiSprite(Object gfx, Identifier spriteId, int x, int y, int w, int h) {
        if (gfx instanceof net.minecraft.client.gui.GuiGraphics graphics && spriteId != null) {
            graphics.blitSprite(net.minecraft.client.renderer.RenderPipelines.GUI_TEXTURED,
                    spriteId, x, y, w, h);
        }
    }

    /**
     * Title-menu primitive: horizontal-gradient text drawn at a uniform scale.
     * Translates to ({@code x}, {@code y}), scales, draws at the origin through
     * {@link #uiTextGradient}, then restores the pose.
     */
    public static void uiTextScaledGradient(Object gfx, net.minecraft.client.gui.Font font,
            String text, int x, int y, float scale, int argbLeft, int argbRight) {
        if (!(gfx instanceof net.minecraft.client.gui.GuiGraphics graphics)
                || font == null || text == null) {
            return;
        }
        graphics.pose().pushMatrix();
        graphics.pose().translate(x, y);
        graphics.pose().scale(scale, scale);
        uiTextGradient(graphics, font,
                net.minecraft.network.chat.Component.literal(text),
                0, 0, argbLeft, argbRight, false);
        graphics.pose().popMatrix();
    }

    /**
     * Loads a user-supplied PNG from disk as a {@code DynamicTexture} and
     * registers it under {@code espectral-menu:menu_bg}. Returns null on failure.
     */
    public static Identifier loadExternalTexture(Minecraft minecraft, Path path) {
        if (minecraft == null || path == null) {
            return null;
        }
        try (java.io.InputStream in = Files.newInputStream(path)) {
            com.mojang.blaze3d.platform.NativeImage image =
                    com.mojang.blaze3d.platform.NativeImage.read(in);
            net.minecraft.client.renderer.texture.DynamicTexture texture =
                    new net.minecraft.client.renderer.texture.DynamicTexture(
                            () -> "espectral-menu-bg", image);
            Identifier id = Identifier.fromNamespaceAndPath("espectral-menu", "menu_bg");
            minecraft.getTextureManager().register(id, texture);
            return id;
        } catch (Exception e) {
            return null;
        }
    }

    /** Pixel dimensions of a registered {@code DynamicTexture}; null-safe. */
    public static int[] textureSize(Minecraft minecraft, Identifier id) {
        if (minecraft == null || id == null) {
            return null;
        }
        try {
            net.minecraft.client.renderer.texture.AbstractTexture texture =
                    minecraft.getTextureManager().getTexture(id);
            if (texture instanceof net.minecraft.client.renderer.texture.DynamicTexture dynamic) {
                com.mojang.blaze3d.platform.NativeImage pixels = dynamic.getPixels();
                if (pixels == null) {
                    return null;
                }
                return new int[]{pixels.getWidth(), pixels.getHeight()};
            }
            return null;
        } catch (Exception e) {
            return null;
        }
    }

    /** Title-menu navigation targets. Unknown keys yield null. */
    public static Screen menuTarget(Minecraft minecraft, Screen back, String which) {
        if (minecraft == null || which == null) {
            return null;
        }
        switch (which) {
            case "selectWorld":
                return new net.minecraft.client.gui.screens.worldselection.SelectWorldScreen(back);
            case "joinMultiplayer":
                return new net.minecraft.client.gui.screens.multiplayer.JoinMultiplayerScreen(back);
            case "accessibility":
                return new net.minecraft.client.gui.screens.options.AccessibilityOptionsScreen(
                        back, minecraft.options);
            default:
                return null;
        }
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
