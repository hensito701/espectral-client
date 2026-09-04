package es.spectral.menu;

import java.util.LinkedHashMap;
import java.util.Map;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.mojang.blaze3d.platform.NativeImage;

import net.minecraft.client.Minecraft;
import net.minecraft.client.model.geom.PartPose;
import net.minecraft.client.model.geom.builders.CubeDeformation;
import net.minecraft.client.model.geom.builders.CubeListBuilder;
import net.minecraft.client.model.geom.builders.LayerDefinition;
import net.minecraft.client.model.geom.builders.MeshDefinition;
import net.minecraft.client.model.geom.builders.PartDefinition;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.resources.Identifier;

/**
 * Per-skin voxelized {@link PlayerModel} cache ("3D Skin Layers", owned/live).
 *
 * <p>Keyed by (skin {@link Identifier}, slim flag). Each entry reuses the
 * vanilla {@link PlayerModel#createMesh} layout, but the six overlay parts
 * (hat, jacket, left/right sleeves, left/right pants) have their flat vanilla
 * boxes replaced with per-pixel voxel shells baked from the skin image
 * ({@link SkinVoxelizer}). Base parts (head, body, arms, legs) are untouched,
 * so proportions, pivots and first-person arm offsets stay vanilla.</p>
 *
 * <p>Skin pixels are read ONCE per cache miss: opacity is copied eagerly into a
 * throwaway boolean array via {@code TextureManager#getTexture} +
 * {@code DynamicTexture#getPixels()}; no texture, image or manager reference
 * is ever retained. Until the skin texture has downloaded, lookup returns
 * {@code null} (without caching the miss) so the caller keeps rendering
 * vanilla and retries on later frames.</p>
 *
 * <p>LRU-capped at 64 entries; {@link #clear()} is called explicitly on
 * disconnect ({@code Minecraft#disconnect} hook) and on resource reload
 * ({@code TextureManager#reload} hook). All methods are synchronized: lookups
 * happen on the render thread, clears on the client/reload threads.</p>
 *
 * <p>When the {@code skin3d} feature is off this class is never consulted: the
 * default-off path is behavior-identical to an unmodded client.</p>
 */
public final class SkinModelCache {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    /** Maximum cached voxel models; bounds GPU/CPU memory on busy servers. */
    private static final int MAX_ENTRIES = 64;

    /** Sanity cap for skin image dimensions (vanilla skins are 64x64/64x32). */
    private static final int MAX_IMAGE_DIM = 256;

    private record CacheKey(Identifier id, boolean slim) {}

    private static final Map<CacheKey, PlayerModel> MODELS = new LinkedHashMap<>(MAX_ENTRIES + 1, 0.75F, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<CacheKey, PlayerModel> eldest) {
            return size() > MAX_ENTRIES;
        }
    };

    private SkinModelCache() {}

    /** Drops every baked voxel model (disconnect / resource reload). */
    public static synchronized void clear() {
        if (!MODELS.isEmpty()) {
            MODELS.clear();
            LOGGER.info("3D Skin Layers: cleared voxel model cache");
        }
    }

    /**
     * Returns the voxelized model for a skin, baking and caching it on miss.
     * Returns {@code null} when the skin texture is not (yet) a readable
     * {@link DynamicTexture}; callers must fall back to the vanilla model.
     */
    public static synchronized PlayerModel getOrBuild(Minecraft minecraft, Identifier skinId, boolean slim) {
        if (minecraft == null || skinId == null) return null;
        CacheKey key = new CacheKey(skinId, slim);
        PlayerModel cached = MODELS.get(key);
        if (cached != null) return cached;

        NativeImage pixels = readSkinPixels(minecraft, skinId);
        if (pixels == null) return null;

        final boolean[] occupancy;
        final int width;
        final int height;
        try {
            width = pixels.getWidth();
            height = pixels.getHeight();
            if (width <= 0 || height <= 0 || width > MAX_IMAGE_DIM || height > MAX_IMAGE_DIM) return null;
            occupancy = snapshot(pixels, width, height);
        } catch (RuntimeException e) {
            // Closed/released image (e.g. mid-reload): stay vanilla, retry later.
            return null;
        }
        // pixels/occupancy locals are dropped here: nothing native is retained.

        final PlayerModel built;
        try {
            built = build(occupancy, width, height, slim);
        } catch (RuntimeException e) {
            LOGGER.warn("3D Skin Layers: failed to bake voxel model for {}", skinId, e);
            return null;
        }
        MODELS.put(key, built);
        LOGGER.info("3D Skin Layers: baked {} voxel model for {}", slim ? "slim" : "wide", skinId);
        return built;
    }

    /**
     * Reads the live skin image without retaining it. The caller must copy out
     * whatever it needs synchronously; the returned reference must not escape.
     */
    private static NativeImage readSkinPixels(Minecraft minecraft, Identifier skinId) {
        TextureManager textureManager = minecraft.getTextureManager();
        if (textureManager == null) return null;
        final AbstractTexture texture;
        try {
            texture = textureManager.getTexture(skinId);
        } catch (RuntimeException e) {
            return null;
        }
        if (!(texture instanceof DynamicTexture dynamic)) return null;
        try {
            return dynamic.getPixels();
        } catch (RuntimeException e) {
            return null;
        }
    }

    /** Copies per-pixel opacity (alpha != 0) into a plain boolean array. */
    private static boolean[] snapshot(NativeImage pixels, int width, int height) {
        boolean[] occupancy = new boolean[width * height];
        for (int y = 0; y < height; y++) {
            for (int x = 0; x < width; x++) {
                occupancy[y * width + x] = (pixels.getPixel(x, y) >>> 24) != 0;
            }
        }
        return occupancy;
    }

    /**
     * Bakes a fresh {@link PlayerModel} whose overlay parts are voxel shells.
     * Overlay texture regions missing from the image (legacy 64x32 skins keep
     * only the hat region) are left as their vanilla flat boxes, exactly
     * matching vanilla rendering for those slots.
     */
    private static PlayerModel build(boolean[] occupancy, int imgW, int imgH, boolean slim) {
        MeshDefinition mesh = PlayerModel.createMesh(CubeDeformation.NONE, slim);
        PartDefinition root = mesh.getRoot();

        voxelizeSlot(root, "head", "hat", occupancy, imgW, imgH,
                32, 0, -4.0F, -8.0F, -4.0F, 8, 8, 8, 0.5F);
        voxelizeSlot(root, "body", "jacket", occupancy, imgW, imgH,
                16, 32, -4.0F, 0.0F, -2.0F, 8, 12, 4, 0.25F);

        int armW = slim ? 3 : 4;
        voxelizeSlot(root, "left_arm", "left_sleeve", occupancy, imgW, imgH,
                48, 48, -1.0F, -2.0F, -2.0F, armW, 12, 4, 0.25F);
        voxelizeSlot(root, "right_arm", "right_sleeve", occupancy, imgW, imgH,
                40, 32, slim ? -2.0F : -3.0F, -2.0F, -2.0F, armW, 12, 4, 0.25F);
        voxelizeSlot(root, "left_leg", "left_pants", occupancy, imgW, imgH,
                0, 48, -2.0F, 0.0F, -2.0F, 4, 12, 4, 0.25F);
        voxelizeSlot(root, "right_leg", "right_pants", occupancy, imgW, imgH,
                0, 32, -2.0F, 0.0F, -2.0F, 4, 12, 4, 0.25F);

        // Same 64x64 material the vanilla player layers bake with, so voxel UVs
        // sample the skin texture exactly like the boxes they replace.
        return new PlayerModel(LayerDefinition.create(mesh, 64, 64).bakeRoot(), slim);
    }

    /**
     * Replaces one vanilla overlay box with a voxel shell. The overlay part
     * keeps its name and {@link PartPose#ZERO} pose, so
     * {@link PlayerModel#setupAnim} visibility flags (showHat, showJacket,
     * showLeftSleeve, ...) and limb parenting keep working unchanged.
     */
    private static void voxelizeSlot(PartDefinition root, String parent, String child,
            boolean[] occupancy, int imgW, int imgH,
            int texU, int texV,
            float minX, float minY, float minZ,
            int sizeX, int sizeY, int sizeZ, float inflate) {
        // Full net extent of one box: U spans two widths plus two depths
        // (back rect ends at u+2w+2d), V spans depth plus height.
        if (texU < 0 || texV < 0
                || texU + 2 * (sizeX + sizeZ) > imgW
                || texV + sizeZ + sizeY > imgH) {
            return;
        }
        final PartDefinition parentDef;
        try {
            parentDef = root.getChild(parent);
            if (parentDef == null) return;
        } catch (RuntimeException e) {
            return;
        }
        CubeListBuilder voxels = CubeListBuilder.create();
        SkinVoxelizer.voxelize(voxels, occupancy, imgW, imgH,
                texU, texV, minX, minY, minZ, sizeX, sizeY, sizeZ, inflate);
        parentDef.clearChild(child);
        parentDef.addOrReplaceChild(child, voxels, PartPose.ZERO);
    }
}
