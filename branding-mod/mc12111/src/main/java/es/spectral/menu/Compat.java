package es.spectral.menu;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsScreen;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.player.PlayerModelType;

/** Screen-opening compatibility for 1.21.11. */
public final class Compat {

    private Compat() {}

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
        int y = 4;
        for (String line : HudEngine.getInstance().leftLines(minecraft)) {
            graphics.drawString(font, line, 4, y, 0xFFFFFF, true);
            y += 10;
        }
        int width = graphics.guiWidth();
        int rightY = 4;
        for (String line : HudEngine.getInstance().rightLines(minecraft)) {
            graphics.drawString(font, line, width - font.width(line) - 4, rightY, 0xFFFFFF, true);
            rightY += 10;
        }
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
}
