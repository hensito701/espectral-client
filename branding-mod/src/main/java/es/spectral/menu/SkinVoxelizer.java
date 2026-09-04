package es.spectral.menu;

import net.minecraft.client.model.geom.builders.CubeDeformation;
import net.minecraft.client.model.geom.builders.CubeListBuilder;

/**
 * Voxelizes player-skin overlay regions into per-pixel 1x1x1 boxes.
 *
 * <p>For one overlay box (hat, jacket, sleeves, pants) this emits a 1px-thick
 * shell hugging the vanilla inflated overlay surface: each box face is covered
 * by a grid of unit cubes whose outer faces sit flush with the inflated bounds
 * (base box + inflation). Every voxel's outward face samples exactly the skin
 * pixel vanilla would show at that spot: face rectangles, in-face pixel order
 * and per-face UV offsets were calibrated empirically against baked vanilla
 * boxes (headless probe of {@code ModelPart$Cube} polygons), not assumed:</p>
 *
 * <ul>
 *   <li>-X: rect (u, v+d); columns run toward -Z, rows toward +Y.</li>
 *   <li>-Z: rect (u+d, v+d); columns toward +X, rows toward +Y.</li>
 *   <li>+X: rect (u+d+w, v+d); columns toward +Z, rows toward +Y.</li>
 *   <li>+Z: rect (u+d+w+d, v+d); columns toward -X, rows toward +Y.</li>
 *   <li>-Y: rect (u+d, v); columns toward -X, rows toward -Z.</li>
 *   <li>+Y: rect (u+d+w, v); columns toward -X, rows toward -Z.</li>
 * </ul>
 *
 * <p>(Note the -Z/+Z roles: vanilla shows the skin's "front" rect on -Z, and
 * rows run bottom-up on every side face.) A unit cube's outward face samples
 * the 1x1 texel at (texU+ou, texV+ov) with per-face offsets -X=(0,1), -Z=(1,1),
 * +X=(2,1), +Z=(3,1), -Y=(1,0), +Y=(2,0), so each voxel is emitted with
 * {@code texOffs(pixel - offset)} and its visible face lands on its own
 * pixel exactly.</p>
 *
 * <p>Transparent pixels are skipped, so lace, gaps and fringe details stay
 * see-through exactly like the flat overlay. Voxel cubes intentionally
 * intersect (never coincide): neighbouring faces overlap in volume around
 * edges instead of sharing planes, so there is no coplanar z-fighting — only
 * solid Minecraft-style corners. In-plane edge strips (<= inflation wide) are
 * covered by the sides of the perpendicular faces; only sub-pixel corner
 * notches can show the base box beneath.</p>
 *
 * <p>Pure geometry helper: no texture or renderer references, safe to call from
 * the render thread during per-skin model baking ({@link SkinModelCache}).</p>
 */
public final class SkinVoxelizer {

    private SkinVoxelizer() {}

    /**
     * Appends a voxel shell for one overlay box to {@code builder}.
     *
     * @param builder   target cube list (fresh per overlay part)
     * @param occupied  row-major opacity snapshot of the skin image
     * @param imgW      skin image width in pixels
     * @param imgH      skin image height in pixels
     * @param texU      overlay region texture origin U
     * @param texV      overlay region texture origin V
     * @param minX      uninflated box minimum, in model units (part space)
     * @param minY      uninflated box minimum, in model units (part space)
     * @param minZ      uninflated box minimum, in model units (part space)
     * @param sizeX     uninflated box width in pixels
     * @param sizeY     uninflated box height in pixels
     * @param sizeZ     uninflated box depth in pixels
     * @param inflate   vanilla overlay inflation (0.5 hat, 0.25 elsewhere)
     */
    public static void voxelize(CubeListBuilder builder, boolean[] occupied, int imgW, int imgH,
            int texU, int texV,
            float minX, float minY, float minZ,
            int sizeX, int sizeY, int sizeZ, float inflate) {
        if (builder == null || occupied == null || occupied.length < imgW * imgH) return;
        if (sizeX <= 0 || sizeY <= 0 || sizeZ <= 0) return;
        int w = sizeX;
        int h = sizeY;
        int d = sizeZ;
        // Inflated bounds are [min - infl, min + size + infl]; each outer slab
        // is 1px thick with its outer face flush: origin = max - 1.
        float maxX = minX + w + inflate - 1.0F;
        float maxY = minY + h + inflate - 1.0F;
        float maxZ = minZ + d + inflate - 1.0F;
        float loX = minX - inflate;
        float loY = minY - inflate;
        float loZ = minZ - inflate;

        // -X (west) and +X (east) slabs.
        for (int k = 0; k < d; k++) {
            for (int j = 0; j < h; j++) {
                int px = texU + (d - 1 - k);
                int py = texV + d + j;
                if (isSolid(occupied, imgW, imgH, px, py)) {
                    builder.texOffs(px, py - 1)
                            .addBox(loX, minY + j, minZ + k, 1.0F, 1.0F, 1.0F, CubeDeformation.NONE);
                }
                int ex = texU + d + w + k;
                if (isSolid(occupied, imgW, imgH, ex, py)) {
                    builder.texOffs(ex - 2, py - 1)
                            .addBox(maxX, minY + j, minZ + k, 1.0F, 1.0F, 1.0F, CubeDeformation.NONE);
                }
            }
        }

        // -Z (north) and +Z (south) slabs.
        for (int i = 0; i < w; i++) {
            for (int j = 0; j < h; j++) {
                int px = texU + d + i;
                int py = texV + d + j;
                if (isSolid(occupied, imgW, imgH, px, py)) {
                    builder.texOffs(px - 1, py - 1)
                            .addBox(minX + i, minY + j, loZ, 1.0F, 1.0F, 1.0F, CubeDeformation.NONE);
                }
                int sx = texU + d + w + d + (w - 1 - i);
                if (isSolid(occupied, imgW, imgH, sx, py)) {
                    builder.texOffs(sx - 3, py - 1)
                            .addBox(minX + i, minY + j, maxZ, 1.0F, 1.0F, 1.0F, CubeDeformation.NONE);
                }
            }
        }

        // -Y (down) and +Y (up) slabs.
        for (int i = 0; i < w; i++) {
            for (int k = 0; k < d; k++) {
                int px = texU + d + (w - 1 - i);
                int py = texV + (d - 1 - k);
                if (isSolid(occupied, imgW, imgH, px, py)) {
                    builder.texOffs(px - 1, py)
                            .addBox(minX + i, loY, minZ + k, 1.0F, 1.0F, 1.0F, CubeDeformation.NONE);
                }
                int ux = texU + d + w + (w - 1 - i);
                if (isSolid(occupied, imgW, imgH, ux, py)) {
                    builder.texOffs(ux - 2, py)
                            .addBox(minX + i, maxY, minZ + k, 1.0F, 1.0F, 1.0F, CubeDeformation.NONE);
                }
            }
        }
    }

    private static boolean isSolid(boolean[] occupied, int imgW, int imgH, int x, int y) {
        if (x < 0 || y < 0 || x >= imgW || y >= imgH) return false;
        return occupied[y * imgW + x];
    }
}
