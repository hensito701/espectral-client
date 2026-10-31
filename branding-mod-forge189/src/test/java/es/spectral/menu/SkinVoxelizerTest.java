package es.spectral.menu;

import static org.junit.Assert.assertEquals;

import java.util.ArrayList;
import java.util.List;

import org.junit.Test;

/** Voxel shell geometry for 3D skin layers (same rules as the Fabric mod). */
public class SkinVoxelizerTest {

    private static List<float[]> voxelize(boolean[] occupied) {
        final List<float[]> cubes = new ArrayList<float[]>();
        SkinVoxelizer.voxelize(new SkinVoxelizer.Sink() {
            @Override
            public void cube(int u, int v, float x, float y, float z) {
                cubes.add(new float[] {u, v, x, y, z});
            }
        }, occupied, 64, 64, 32, 0, -4f, -8f, -4f, 8, 8, 8, 0.5f);
        return cubes;
    }

    @Test
    public void opaqueHatIsAFullShellAndTransparentIsEmpty() {
        boolean[] all = new boolean[64 * 64];
        java.util.Arrays.fill(all, true);
        assertEquals(6 * 64, voxelize(all).size());
        assertEquals(0, voxelize(new boolean[64 * 64]).size());
    }

    @Test
    public void frontFacePixelLandsOnTheFrontSlab() {
        boolean[] one = new boolean[64 * 64];
        // Hat front rect starts at (u+d, v+d) = (40, 8); its first column sits at -X.
        one[8 * 64 + 40] = true;
        List<float[]> cubes = voxelize(one);
        assertEquals(1, cubes.size());
        float[] c = cubes.get(0);
        assertEquals(39f, c[0], 0f);
        assertEquals(7f, c[1], 0f);
        assertEquals(-4f, c[2], 0f);
        assertEquals(-8f, c[3], 0f);
        assertEquals(-4.5f, c[4], 0f);
    }
}
