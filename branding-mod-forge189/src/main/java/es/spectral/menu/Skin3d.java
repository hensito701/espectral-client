package es.spectral.menu;

import java.awt.image.BufferedImage;
import java.io.InputStream;
import java.util.LinkedHashMap;
import java.util.Map;

import javax.imageio.ImageIO;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import net.minecraft.client.Minecraft;
import net.minecraft.client.entity.AbstractClientPlayer;
import net.minecraft.client.model.ModelPlayer;
import net.minecraft.client.model.ModelRenderer;
import net.minecraft.client.renderer.GlStateManager;
import net.minecraft.client.renderer.ThreadDownloadImageData;
import net.minecraft.client.renderer.entity.RenderPlayer;
import net.minecraft.client.renderer.entity.layers.LayerRenderer;
import net.minecraft.client.renderer.texture.ITextureObject;
import net.minecraft.entity.player.EnumPlayerModelParts;
import net.minecraft.util.ResourceLocation;

import es.spectral.menu.mixin.feature.ThreadDownloadImageDataAccessor;

/**
 * 3D Skin Layers ({@code skin3d}, owned/live): in third person the skin's
 * second layer (hat, jacket, sleeves, pants) is drawn as a 1 px voxel shell
 * baked from the skin image instead of the flat vanilla overlay — the same
 * geometry as the Fabric mod ({@link SkinVoxelizer}, ported verbatim on top
 * of 1.8.9 {@link ModelRenderer} boxes, whose cube net is identical).
 *
 * <p>Per skin (location + slim) one {@link ModelPlayer} is baked and cached
 * (LRU 64). The skin image is read once per miss — the downloaded skin's
 * buffered image, or the default skin from resources — and only per-pixel
 * opacity is kept. Until a downloaded skin arrives, nothing is cached and the
 * player renders vanilla. The flat wear parts are hidden right before the
 * player's body renders ({@code RenderPlayerMixin}); first-person arms stay
 * vanilla. Off = vanilla-identical.</p>
 */
public final class Skin3d {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");
    private static final int MAX_ENTRIES = 64;
    private static final int MAX_IMAGE_DIM = 256;

    private static final Map<String, ModelPlayer> MODELS = new LinkedHashMap<String, ModelPlayer>(MAX_ENTRIES + 1, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, ModelPlayer> eldest) {
            return size() > MAX_ENTRIES;
        }
    };

    private Skin3d() {}

    public static boolean enabled() {
        return ClientConfig.getInstance().isFeatureEnabled("skin3d");
    }

    /** Adds the voxel layer to both player renderers (wide and slim). */
    public static void register() {
        for (RenderPlayer renderer : Minecraft.getMinecraft().getRenderManager().getSkinMap().values()) {
            renderer.addLayer(new Layer(renderer));
        }
    }

    public static synchronized void clear() {
        MODELS.clear();
    }

    /** Baked voxel model for this player's skin, or null (feature off / skin not ready). */
    public static synchronized ModelPlayer modelFor(AbstractClientPlayer player) {
        if (!enabled() || player == null) return null;
        ResourceLocation skin = player.getLocationSkin();
        boolean slim = "slim".equals(player.getSkinType());
        String key = skin + (slim ? "#slim" : "#wide");
        ModelPlayer cached = MODELS.get(key);
        if (cached != null) return cached;
        BufferedImage image = readSkin(skin);
        if (image == null) return null;
        int w = image.getWidth();
        int h = image.getHeight();
        if (w <= 0 || h <= 0 || w > MAX_IMAGE_DIM || h > MAX_IMAGE_DIM) return null;
        boolean[] occupied = new boolean[w * h];
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                occupied[y * w + x] = (image.getRGB(x, y) >>> 24) != 0;
            }
        }
        ModelPlayer built = build(occupied, w, h, slim);
        MODELS.put(key, built);
        LOGGER.info("3D Skin Layers: baked {} voxel model for {}", slim ? "slim" : "wide", skin);
        return built;
    }

    private static BufferedImage readSkin(ResourceLocation skin) {
        Minecraft mc = Minecraft.getMinecraft();
        ITextureObject texture = mc.getTextureManager().getTexture(skin);
        if (texture instanceof ThreadDownloadImageData) {
            return ((ThreadDownloadImageDataAccessor) texture).espectral$image();
        }
        if (texture == null) return null;
        try (InputStream in = mc.getResourceManager().getResource(skin).getInputStream()) {
            return ImageIO.read(in);
        } catch (Exception e) {
            return null;
        }
    }

    /** A fresh ModelPlayer whose six wear parts are voxel shells (vanilla 64x64 net). */
    private static ModelPlayer build(boolean[] occupied, int imgW, int imgH, boolean slim) {
        ModelPlayer model = new ModelPlayer(0.0f, slim);
        int armW = slim ? 3 : 4;
        model.bipedHeadwear = voxelPart(model, occupied, imgW, imgH, 32, 0, -4f, -8f, -4f, 8, 8, 8, 0.5f, model.bipedHeadwear);
        model.bipedBodyWear = voxelPart(model, occupied, imgW, imgH, 16, 32, -4f, 0f, -2f, 8, 12, 4, 0.25f, model.bipedBodyWear);
        model.bipedLeftArmwear = voxelPart(model, occupied, imgW, imgH, 48, 48, -1f, -2f, -2f, armW, 12, 4, 0.25f,
                model.bipedLeftArmwear);
        model.bipedRightArmwear = voxelPart(model, occupied, imgW, imgH, 40, 32, slim ? -2f : -3f, -2f, -2f, armW, 12, 4,
                0.25f, model.bipedRightArmwear);
        model.bipedLeftLegwear = voxelPart(model, occupied, imgW, imgH, 0, 48, -2f, 0f, -2f, 4, 12, 4, 0.25f,
                model.bipedLeftLegwear);
        model.bipedRightLegwear = voxelPart(model, occupied, imgW, imgH, 0, 32, -2f, 0f, -2f, 4, 12, 4, 0.25f,
                model.bipedRightLegwear);
        return model;
    }

    /**
     * Voxel shell replacing one wear part; regions missing from the image
     * (legacy 64x32 skins keep only the hat) keep the vanilla flat part.
     */
    private static ModelRenderer voxelPart(ModelPlayer model, boolean[] occupied, int imgW, int imgH, int texU, int texV,
            float minX, float minY, float minZ, int sizeX, int sizeY, int sizeZ, float inflate, ModelRenderer vanilla) {
        if (texU + 2 * (sizeX + sizeZ) > imgW || texV + sizeZ + sizeY > imgH) return vanilla;
        ModelRenderer part = new ModelRenderer(model, 0, 0);
        part.setTextureSize(64, 64);
        part.setRotationPoint(vanilla.rotationPointX, vanilla.rotationPointY, vanilla.rotationPointZ);
        SkinVoxelizer.voxelize((u, v, x, y, z) -> {
            part.setTextureOffset(u, v);
            part.addBox(x, y, z, 1, 1, 1);
        }, occupied, imgW, imgH, texU, texV, minX, minY, minZ, sizeX, sizeY, sizeZ, inflate);
        return part;
    }

    /** True when the player's flat wear should be hidden this frame (voxels will draw instead). */
    public static boolean replacesWear(AbstractClientPlayer player) {
        return modelFor(player) != null;
    }

    /** Draws the voxel wear parts the player has enabled, posed like the body. */
    private static final class Layer implements LayerRenderer<AbstractClientPlayer> {
        private final RenderPlayer renderer;

        Layer(RenderPlayer renderer) {
            this.renderer = renderer;
        }

        @Override
        public void doRenderLayer(AbstractClientPlayer player, float limbSwing, float limbSwingAmount, float partialTicks,
                float ageInTicks, float netHeadYaw, float headPitch, float scale) {
            ModelPlayer voxels = modelFor(player);
            if (voxels == null || player.isInvisible()) return;
            ModelPlayer main = renderer.getMainModel();
            voxels.setModelAttributes(main);
            voxels.setRotationAngles(limbSwing, limbSwingAmount, ageInTicks, netHeadYaw, headPitch, scale, player);
            renderer.bindTexture(player.getLocationSkin());
            GlStateManager.pushMatrix();
            if (player.isSneaking()) GlStateManager.translate(0f, 0.2f, 0f);
            GlStateManager.color(1f, 1f, 1f, 1f);
            GlStateManager.enableBlend();
            GlStateManager.tryBlendFuncSeparate(770, 771, 1, 0);
            if (player.isWearing(EnumPlayerModelParts.HAT)) voxels.bipedHeadwear.render(scale);
            if (player.isWearing(EnumPlayerModelParts.JACKET)) voxels.bipedBodyWear.render(scale);
            if (player.isWearing(EnumPlayerModelParts.LEFT_SLEEVE)) voxels.bipedLeftArmwear.render(scale);
            if (player.isWearing(EnumPlayerModelParts.RIGHT_SLEEVE)) voxels.bipedRightArmwear.render(scale);
            if (player.isWearing(EnumPlayerModelParts.LEFT_PANTS_LEG)) voxels.bipedLeftLegwear.render(scale);
            if (player.isWearing(EnumPlayerModelParts.RIGHT_PANTS_LEG)) voxels.bipedRightLegwear.render(scale);
            GlStateManager.disableBlend();
            GlStateManager.popMatrix();
        }

        @Override
        public boolean shouldCombineTextures() {
            return true;
        }
    }
}
