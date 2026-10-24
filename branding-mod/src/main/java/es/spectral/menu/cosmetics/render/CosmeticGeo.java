package es.spectral.menu.cosmetics.render;

import org.joml.Vector3f;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;

import net.minecraft.client.renderer.SubmitNodeCollector.CustomGeometryRenderer;
import net.minecraft.client.renderer.texture.OverlayTexture;

/**
 * Allocation-free emitters for the cloth cosmetics.
 *
 * <p>Coordinates are model/layer space: +X is the wearer's left, +Y is down,
 * +Z is behind the wearer, the origin is the model root (neck line) and one
 * unit is one block — the same frame vanilla model parts live in. Each vertex
 * is pushed through the captured {@link PoseStack.Pose} exactly like
 * {@code ModelPart.Cube} does (transformPosition + transformNormal), so the
 * deferred node renderer sees finished geometry.
 *
 * <p>Renderers are reusable: one instance per submitted ribbon/sheet, its
 * configuration fields set right before {@code submitCustomGeometry}. The node
 * captures the instance and invokes it later in the same frame; the sim never
 * touches it between submit and draw.
 */
public final class CosmeticGeo {

    private static final Vector3f POS = new Vector3f();
    private static final Vector3f NRM = new Vector3f();

    /** ARGB white, fully opaque. */
    public static final int WHITE = 0xFFFFFFFF;

    /** Lowest vertex alpha any cosmetic may use (footprint cap: min opacity). */
    public static final int MIN_ALPHA = 128;

    private CosmeticGeo() {}

    /** Emits one vertex: position + normal through the pose, everything else raw. */
    public static void vertex(PoseStack.Pose pose, VertexConsumer buf,
            float x, float y, float z, int argb, float u, float v, int light,
            float nx, float ny, float nz) {
        pose.pose().transformPosition(x, y, z, POS);
        pose.transformNormal(nx, ny, nz, NRM);
        buf.addVertex(POS.x(), POS.y(), POS.z(), argb, u, v,
                OverlayTexture.NO_OVERLAY, light, NRM.x(), NRM.y(), NRM.z());
    }

    /** Vertex with the normal computed from a triangle's edges. */
    private static void normalOf(
            float ax, float ay, float az, float bx, float by, float bz,
            float cx, float cy, float cz, Vector3f out) {
        float ux = bx - ax, uy = by - ay, uz = bz - az;
        float vx = cx - ax, vy = cy - ay, vz = cz - az;
        out.set(uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
        float len = (float) Math.sqrt(out.x() * out.x() + out.y() * out.y() + out.z() * out.z());
        if (len > 1.0e-6f) out.mul(1.0f / len);
    }

    /**
     * Cloth ribbon along a simulated chain. Points are interpolated between the
     * previous and current tick with {@link #partial}. Width tapers toward the
     * tip by {@link #taper}; the hem is tinted gold and fades to
     * {@link #MIN_ALPHA} at the very end.
     */
    public static final class Ribbon implements CustomGeometryRenderer {
        /** Source chain; points are read as lerp(prev, cur, partial). */
        public ClothSim.Chain chain;
        public float partial;
        /** Half width at the anchor (blocks). */
        public float halfWidth;
        /** Width scale at the tip (0..1). */
        public float taper = 0.55f;
        /** UV window: u across the strip, v advances by vStep per segment. */
        public float u0 = 0f, u1 = 1f, vStep = 0.5f;
        /** Base ARGB; the hem is tinted toward this gold. */
        public int color = WHITE;
        public int goldTint = 0xFFFFC864;
        /** Hem fade start (fraction of the strip, 0..1). */
        public float fadeFrom = 0.72f;
        /** Gold hem brightness multiplier (flame-edge flicker). */
        public float goldScale = 1.0f;
        public int light;
        /** Extra lateral wave, animated by the wearer tick (0 = off). */
        public float wave;
        public int waveTick;
        /** Rigid offset applied to every point (e.g. re-anchoring for hands). */
        public float offX, offY, offZ;

        @Override
        public void render(PoseStack.Pose pose, VertexConsumer buf) {
            ClothSim.Chain c = chain;
            int n = c.n;
            float p = partial;
            float inv = 1.0f / (n - 1);
            for (int i = 0; i < n - 1; i++) {
                float t0 = i * inv, t1 = (i + 1) * inv;
                float w0 = halfWidth * (1.0f + (taper - 1.0f) * t0);
                float w1 = halfWidth * (1.0f + (taper - 1.0f) * t1);
                float ax = lerp(c, i, 0, p) + offX, ay = lerp(c, i, 1, p) + offY, az = lerp(c, i, 2, p) + offZ;
                float bx = lerp(c, i + 1, 0, p) + offX, by = lerp(c, i + 1, 1, p) + offY, bz = lerp(c, i + 1, 2, p) + offZ;
                // lateral = the strip's width direction: perpendicular to the
                // segment in the XZ plane (the ribbon hangs roughly vertically).
                float sx = bz - az, sz = -(bx - ax);
                float sl = (float) Math.sqrt(sx * sx + sz * sz);
                if (sl < 1.0e-5f) { sx = 1f; sz = 0f; sl = 1f; }
                sx /= sl; sz /= sl;

                float wv0 = wave * (float) Math.sin(waveTick * 0.22 + t0 * 2.6f);
                float wv1 = wave * (float) Math.sin(waveTick * 0.22 + t1 * 2.6f);
                float p0x = ax + sx * wv0, p0z = az + sz * wv0;
                float p1x = bx + sx * wv1, p1z = bz + sz * wv1;

                int col0 = hemColor(color, goldTint, fadeFrom, t0, goldScale);
                int col1 = hemColor(color, goldTint, fadeFrom, t1, goldScale);
                float v0 = i * vStep;
                float v1 = (i + 1) * vStep;

                float q0x = p0x - sx * w0, q0z = p0z - sz * w0;
                float q1x = p0x + sx * w0, q1z = p0z + sz * w0;
                float r0x = p1x - sx * w1, r0z = p1z - sz * w1;
                float r1x = p1x + sx * w1, r1z = p1z + sz * w1;

                quad(pose, buf,
                        q0x, ay, q0z, q1x, ay, q1z, r1x, by, r1z, r0x, by, r0z,
                        u0, v0, u1, v0, u1, v1, u0, v1,
                        col0, col0, col1, col1, light);
            }
        }

        private float lerp(ClothSim.Chain c, int i, int axis, float p) {
            float a, b;
            if (axis == 0) { a = c.qx[i]; b = c.px[i]; }
            else if (axis == 1) { a = c.qy[i]; b = c.py[i]; }
            else { a = c.qz[i]; b = c.pz[i]; }
            return a + (b - a) * p;
        }
    }

    /**
     * Cloth sheet (mantle / cloak body) as a double-sided quad grid with the
     * same hem treatment as {@link Ribbon}.
     */
    public static final class Sheet implements CustomGeometryRenderer {
        public ClothSim.Sheet sheet;
        public float partial;
        public int color = WHITE;
        public int goldTint = 0xFFFFC864;
        public float fadeFrom = 0.7f;
        public int light;
        /**
         * Per-column depth relief in blocks, applied at draw time only. The
         * solver flattens any rest-shape bulge it is given (the in-plane links
         * win), so a garment that must read as hanging cloth — the mantle —
         * gets its folds here instead: the simulation stays honest and the
         * silhouette is still corrugated. Empty = flat, and then nothing is
         * added at all.
         */
        public float[] relief;
        /** Relief strength at the shoulder line (1 = full, 0 = flush). */
        public float reliefTop = 0.35f;

        @Override
        public void render(PoseStack.Pose pose, VertexConsumer buf) {
            ClothSim.Sheet s = sheet;
            int cols = s.cols, rows = s.rows;
            float p = partial;
            float[] rel = relief;
            boolean bump = rel != null && rel.length == cols;
            for (int r = 0; r < rows - 1; r++) {
                float v0 = r / (float) (rows - 1);
                float v1 = (r + 1) / (float) (rows - 1);
                int c0 = hemColor(color, goldTint, fadeFrom, v0);
                int c1 = hemColor(color, goldTint, fadeFrom, v1);
                float g0 = reliefTop + (1f - reliefTop) * v0;
                float g1 = reliefTop + (1f - reliefTop) * v1;
                for (int c = 0; c < cols - 1; c++) {
                    float u0 = c / (float) (cols - 1);
                    float u1 = (c + 1) / (float) (cols - 1);
                    float z00 = bump ? rel[c] * g0 : 0f;
                    float z01 = bump ? rel[c + 1] * g0 : 0f;
                    float z11 = bump ? rel[c + 1] * g1 : 0f;
                    float z10 = bump ? rel[c] * g1 : 0f;
                    int i00 = r * cols + c, i01 = r * cols + c + 1;
                    int i10 = (r + 1) * cols + c, i11 = (r + 1) * cols + c + 1;
                    quad(pose, buf,
                            at(s, i00, 0, p), at(s, i00, 1, p), at(s, i00, 2, p) + z00,
                            at(s, i01, 0, p), at(s, i01, 1, p), at(s, i01, 2, p) + z01,
                            at(s, i11, 0, p), at(s, i11, 1, p), at(s, i11, 2, p) + z11,
                            at(s, i10, 0, p), at(s, i10, 1, p), at(s, i10, 2, p) + z10,
                            u0, v0, u1, v0, u1, v1, u0, v1,
                            c0, c0, c1, c1, light);
                }
            }
        }

        private float at(ClothSim.Sheet s, int i, int axis, float p) {
            float a, b;
            if (axis == 0) { a = s.qx[i]; b = s.px[i]; }
            else if (axis == 1) { a = s.qy[i]; b = s.py[i]; }
            else { a = s.qz[i]; b = s.pz[i]; }
            return a + (b - a) * p;
        }
    }

    /** Gold-tinted, opacity-faded hem colour for cloth ends (min opacity kept). */
    public static int hemColor(int color, int goldTint, float fadeFrom, float t) {
        return hemColor(color, goldTint, fadeFrom, t, 1.0f);
    }

    /** As {@link #hemColor(int, int, float, float)} with a gold brightness scale. */
    public static int hemColor(int color, int goldTint, float fadeFrom, float t, float goldScale) {
        if (t <= fadeFrom) return color;
        float k = (t - fadeFrom) / (1.0f - fadeFrom);
        int a = (int) ((color >>> 24) * (1.0f - k * 0.45f));
        if (a < MIN_ALPHA) a = MIN_ALPHA;
        float gs = goldScale < 0f ? 0f : (goldScale > 1.6f ? 1.6f : goldScale);
        int gr = Math.min(255, (int) (((goldTint >> 16) & 0xFF) * gs));
        int gg = Math.min(255, (int) (((goldTint >> 8) & 0xFF) * gs));
        int gb = Math.min(255, (int) ((goldTint & 0xFF) * gs));
        int r = (int) (((color >> 16) & 0xFF) * (1.0f - k) + gr * k);
        int g = (int) (((color >> 8) & 0xFF) * (1.0f - k) + gg * k);
        int b = (int) ((color & 0xFF) * (1.0f - k) + gb * k);
        return (a << 24) | (r << 16) | (g << 8) | b;
    }

    /**
     * One double-sided quad (front and back windings), normals per side.
     * Corner order a,b,c,d goes counter-clockwise seen from the front.
     */
    public static void quad(PoseStack.Pose pose, VertexConsumer buf,
            float ax, float ay, float az, float bx, float by, float bz,
            float cx, float cy, float cz, float dx, float dy, float dz,
            float u0, float v0, float u1, float v1, float u2, float v2, float u3, float v3,
            int c0, int c1, int c2, int c3, int light) {
        normalOf(ax, ay, az, bx, by, bz, cx, cy, cz, NRM);
        float nx = NRM.x(), ny = NRM.y(), nz = NRM.z();
        vertex(pose, buf, ax, ay, az, c0, u0, v0, light, nx, ny, nz);
        vertex(pose, buf, bx, by, bz, c1, u1, v1, light, nx, ny, nz);
        vertex(pose, buf, cx, cy, cz, c2, u2, v2, light, nx, ny, nz);
        vertex(pose, buf, ax, ay, az, c0, u0, v0, light, nx, ny, nz);
        vertex(pose, buf, cx, cy, cz, c2, u2, v2, light, nx, ny, nz);
        vertex(pose, buf, dx, dy, dz, c3, u3, v3, light, nx, ny, nz);
        // back face
        float bx1 = -nx, by1 = -ny, bz1 = -nz;
        vertex(pose, buf, ax, ay, az, c0, u0, v0, light, bx1, by1, bz1);
        vertex(pose, buf, dx, dy, dz, c3, u3, v3, light, bx1, by1, bz1);
        vertex(pose, buf, cx, cy, cz, c2, u2, v2, light, bx1, by1, bz1);
        vertex(pose, buf, ax, ay, az, c0, u0, v0, light, bx1, by1, bz1);
        vertex(pose, buf, cx, cy, cz, c2, u2, v2, light, bx1, by1, bz1);
        vertex(pose, buf, bx, by, bz, c1, u1, v1, light, bx1, by1, bz1);
    }
}
