package es.spectral.menu;

/**
 * Voxelizes one skin overlay box (hat, jacket, sleeves, pants) into a 1 px
 * shell of unit cubes hugging the vanilla inflated overlay surface, each
 * cube's outward face sampling exactly the skin pixel vanilla shows there.
 * Geometry identical to the Fabric mod's {@code SkinVoxelizer} (face rects,
 * in-face pixel order and per-face texture offsets -X=(0,1), -Z=(1,1),
 * +X=(2,1), +Z=(3,1), -Y=(1,0), +Y=(2,0)); transparent pixels are skipped.
 * Pure geometry: cubes are emitted through {@link Sink}.
 */
public final class SkinVoxelizer {

    /** Receives one unit cube: texture offset (u, v) and min corner (x, y, z). */
    public interface Sink {
        void cube(int u, int v, float x, float y, float z);
    }

    private SkinVoxelizer() {}

    public static void voxelize(Sink sink, boolean[] occupied, int imgW, int imgH, int texU, int texV,
            float minX, float minY, float minZ, int sizeX, int sizeY, int sizeZ, float inflate) {
        if (sink == null || occupied == null || occupied.length < imgW * imgH) return;
        if (sizeX <= 0 || sizeY <= 0 || sizeZ <= 0) return;
        int w = sizeX;
        int h = sizeY;
        int d = sizeZ;
        float maxX = minX + w + inflate - 1.0f;
        float maxY = minY + h + inflate - 1.0f;
        float maxZ = minZ + d + inflate - 1.0f;
        float loX = minX - inflate;
        float loY = minY - inflate;
        float loZ = minZ - inflate;

        for (int k = 0; k < d; k++) {
            for (int j = 0; j < h; j++) {
                int px = texU + (d - 1 - k);
                int py = texV + d + j;
                if (isSolid(occupied, imgW, imgH, px, py)) sink.cube(px, py - 1, loX, minY + j, minZ + k);
                int ex = texU + d + w + k;
                if (isSolid(occupied, imgW, imgH, ex, py)) sink.cube(ex - 2, py - 1, maxX, minY + j, minZ + k);
            }
        }
        for (int i = 0; i < w; i++) {
            for (int j = 0; j < h; j++) {
                int px = texU + d + i;
                int py = texV + d + j;
                if (isSolid(occupied, imgW, imgH, px, py)) sink.cube(px - 1, py - 1, minX + i, minY + j, loZ);
                int sx = texU + d + w + d + (w - 1 - i);
                if (isSolid(occupied, imgW, imgH, sx, py)) sink.cube(sx - 3, py - 1, minX + i, minY + j, maxZ);
            }
        }
        for (int i = 0; i < w; i++) {
            for (int k = 0; k < d; k++) {
                int px = texU + d + (w - 1 - i);
                int py = texV + (d - 1 - k);
                if (isSolid(occupied, imgW, imgH, px, py)) sink.cube(px - 1, py, minX + i, loY, minZ + k);
                int ux = texU + d + w + (w - 1 - i);
                if (isSolid(occupied, imgW, imgH, ux, py)) sink.cube(ux - 2, py, minX + i, maxY, minZ + k);
            }
        }
    }

    private static boolean isSolid(boolean[] occupied, int imgW, int imgH, int x, int y) {
        return x >= 0 && y >= 0 && x < imgW && y < imgH && occupied[y * imgW + x];
    }
}
