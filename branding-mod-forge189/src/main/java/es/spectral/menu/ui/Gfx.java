package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.FontRenderer;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.renderer.GlStateManager;
import net.minecraft.client.renderer.Tessellator;
import net.minecraft.client.renderer.WorldRenderer;
import net.minecraft.client.renderer.vertex.DefaultVertexFormats;
import net.minecraft.util.ResourceLocation;
import org.lwjgl.opengl.GL11;

/**
 * 1.8.9 drawing kit: the counterpart of the Fabric lanes' {@code Compat.ui*}
 * primitives (fills, gradients, outlines, tinted sprites, gradient/scaled
 * text, translate push/pop, tooltips), so the screens port their layout code
 * call for call. Everything is immediate-mode GL through
 * {@link GlStateManager}/{@link Tessellator}; colours are ARGB ints.
 */
public final class Gfx {

    private Gfx() {}

    public static FontRenderer font() {
        return Minecraft.getMinecraft().fontRendererObj;
    }

    /** Filled rectangle [x0,x1) x [y0,y1). */
    public static void fill(int x0, int y0, int x1, int y1, int argb) {
        if ((argb >>> 24) == 0 || x1 <= x0 || y1 <= y0) return;
        Gui.drawRect(x0, y0, x1, y1, argb);
    }

    /** Fractional-position fill (sub-pixel motes). */
    public static void fill(float x0, float y0, float x1, float y1, int argb) {
        quadGradient(x0, y0, x1, y1, argb, argb, argb, argb);
    }

    /** 1 px frame around (x, y, w, h). */
    public static void outline(int x, int y, int w, int h, int argb) {
        fill(x, y, x + w, y + 1, argb);
        fill(x, y + h - 1, x + w, y + h, argb);
        fill(x, y + 1, x + 1, y + h - 1, argb);
        fill(x + w - 1, y + 1, x + w, y + h - 1, argb);
    }

    /** Vertical gradient (top colour to bottom colour). */
    public static void gradientV(int x0, int y0, int x1, int y1, int top, int bottom) {
        quadGradient(x0, y0, x1, y1, top, top, bottom, bottom);
    }

    /** Horizontal gradient (left colour to right colour). */
    public static void gradientH(int x0, int y0, int x1, int y1, int left, int right) {
        quadGradient(x0, y0, x1, y1, left, right, right, left);
    }

    /** Corner colours in order top-left, top-right, bottom-right, bottom-left. */
    private static void quadGradient(float x0, float y0, float x1, float y1, int tl, int tr, int br, int bl) {
        GlStateManager.disableTexture2D();
        GlStateManager.enableBlend();
        GlStateManager.disableAlpha();
        GlStateManager.tryBlendFuncSeparate(770, 771, 1, 0);
        GlStateManager.shadeModel(GL11.GL_SMOOTH);
        Tessellator tess = Tessellator.getInstance();
        WorldRenderer wr = tess.getWorldRenderer();
        wr.begin(GL11.GL_QUADS, DefaultVertexFormats.POSITION_COLOR);
        vertex(wr, x1, y0, tr);
        vertex(wr, x0, y0, tl);
        vertex(wr, x0, y1, bl);
        vertex(wr, x1, y1, br);
        tess.draw();
        GlStateManager.shadeModel(GL11.GL_FLAT);
        GlStateManager.disableBlend();
        GlStateManager.enableAlpha();
        GlStateManager.enableTexture2D();
    }

    private static void vertex(WorldRenderer wr, float x, float y, int argb) {
        wr.pos(x, y, 0).color((argb >> 16) & 0xFF, (argb >> 8) & 0xFF, argb & 0xFF, argb >>> 24).endVertex();
    }

    /** Whole texture stretched into (x, y, w, h), tinted ARGB (alpha = opacity). */
    public static void sprite(ResourceLocation tex, float x, float y, float w, float h, int tint) {
        blit(tex, x, y, w, h, 0f, 0f, 1f, 1f, tint);
    }

    /** UV sub-window (0..1) of a texture into (x, y, w, h), tinted. */
    public static void blit(ResourceLocation tex, float x, float y, float w, float h,
            float u0, float v0, float u1, float v1, int tint) {
        if ((tint >>> 24) == 0) return;
        Minecraft.getMinecraft().getTextureManager().bindTexture(tex);
        blitBound(x, y, w, h, u0, v0, u1, v1, tint);
    }

    /** Same as {@link #blit} for an already bound texture (dynamic textures). */
    public static void blitBound(float x, float y, float w, float h, float u0, float v0, float u1, float v1, int tint) {
        GlStateManager.enableBlend();
        GlStateManager.tryBlendFuncSeparate(770, 771, 1, 0);
        GlStateManager.disableAlpha();
        GlStateManager.color(((tint >> 16) & 0xFF) / 255f, ((tint >> 8) & 0xFF) / 255f, (tint & 0xFF) / 255f,
                (tint >>> 24) / 255f);
        Tessellator tess = Tessellator.getInstance();
        WorldRenderer wr = tess.getWorldRenderer();
        wr.begin(GL11.GL_QUADS, DefaultVertexFormats.POSITION_TEX);
        wr.pos(x, y + h, 0).tex(u0, v1).endVertex();
        wr.pos(x + w, y + h, 0).tex(u1, v1).endVertex();
        wr.pos(x + w, y, 0).tex(u1, v0).endVertex();
        wr.pos(x, y, 0).tex(u0, v0).endVertex();
        tess.draw();
        GlStateManager.color(1f, 1f, 1f, 1f);
        GlStateManager.enableAlpha();
        GlStateManager.disableBlend();
    }

    /** Plain text; nothing is drawn below ~2% alpha (1.8.9 treats alpha<4 as opaque). */
    public static void text(String s, int x, int y, int argb, boolean shadow) {
        if (s == null || (argb >>> 24) < 5) return;
        GlStateManager.enableBlend();
        font().drawString(s, x, y, argb, shadow);
        GlStateManager.disableBlend();
    }

    public static void textCentered(String s, int cx, int y, int argb, boolean shadow) {
        text(s, cx - font().getStringWidth(s) / 2, y, argb, shadow);
    }

    public static void textRight(String s, int right, int y, int argb, boolean shadow) {
        text(s, right - font().getStringWidth(s), y, argb, shadow);
    }

    /** Text with a per-character horizontal gradient (the gold wordmark look). */
    public static void textGradient(String s, int x, int y, int left, int right, boolean shadow) {
        if (s == null || s.isEmpty()) return;
        FontRenderer font = font();
        int total = Math.max(1, font.getStringWidth(s));
        int cx = x;
        for (int i = 0; i < s.length(); i++) {
            String ch = String.valueOf(s.charAt(i));
            int w = font.getStringWidth(ch);
            float t = (cx - x + w / 2f) / total;
            text(ch, cx, y, SuiteTheme.lerpArgb(left, right, t), shadow);
            cx += w;
        }
    }

    public static void textCenteredGradient(String s, int cx, int y, int left, int right, boolean shadow) {
        textGradient(s, cx - font().getStringWidth(s) / 2, y, left, right, shadow);
    }

    /** Scaled gradient text with a drop shadow (title wordmark). */
    public static void textScaledGradient(String s, int x, int y, float scale, int left, int right) {
        GlStateManager.pushMatrix();
        GlStateManager.translate(x, y, 0);
        GlStateManager.scale(scale, scale, 1f);
        textGradient(s, 0, 0, left, right, true);
        GlStateManager.popMatrix();
    }

    /** Pushes a translated matrix (pair with {@link #pop()}). */
    public static void push(float dx, float dy) {
        GlStateManager.pushMatrix();
        GlStateManager.translate(dx, dy, 0);
    }

    public static void pop() {
        GlStateManager.popMatrix();
    }

    /** Dark tooltip box next to the cursor, kept on screen. */
    public static void tooltip(String s, int mx, int my, int screenW, int screenH) {
        if (s == null || s.isEmpty()) return;
        FontRenderer font = font();
        int w = font.getStringWidth(s) + 8;
        int h = 14;
        int x = Math.min(mx + 10, screenW - w - 2);
        int y = Math.max(2, Math.min(my - 16, screenH - h - 2));
        GlStateManager.pushMatrix();
        GlStateManager.translate(0, 0, 400);
        fill(x, y, x + w, y + h, 0xF0101216);
        outline(x, y, w, h, SuiteTheme.panelEdge());
        text(s, x + 4, y + 3, SuiteTheme.TEXT, false);
        GlStateManager.popMatrix();
    }

    /** RGB with the given opacity (0..1). */
    public static int argb(float alpha, int rgb) {
        return (Math.round(clamp01(alpha) * 255f) << 24) | (rgb & 0xFFFFFF);
    }

    /** Replaces the colour's alpha with {@code alpha} (0..1). */
    public static int withAlpha(int argb, float alpha) {
        return argb(alpha, argb);
    }

    /** Multiplies the colour's existing alpha by {@code factor} (0..1). */
    public static int scaleAlpha(int argb, float factor) {
        return argb(((argb >>> 24) / 255f) * factor, argb);
    }

    public static float clamp01(float v) {
        return v < 0f ? 0f : (v > 1f ? 1f : v);
    }

    public static float easeOutCubic(float t) {
        float u = 1f - t;
        return 1f - u * u * u;
    }
}
