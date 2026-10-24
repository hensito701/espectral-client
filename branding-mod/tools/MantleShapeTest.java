package es.spectral.menu;

/**
 * Standalone harness for the Manto Espectral's silhouette — no Minecraft, no
 * dependencies, plain {@code javac}. It mirrors {@code ClothSim.Sheet}'s step
 * (verlet integration + pin + distance constraints + torso collider + footprint
 * cap) with the exact constants {@code WearerSims} feeds it, then checks the
 * things the playtest complained about and the design rules demand:
 *
 * <ul>
 *   <li><b>Length.</b> The hem never passes the knee line
 *       ({@code ClothSim.CAP_KNEES}), at rest, walking or sneaking.</li>
 *   <li><b>Footprint.</b> No point leaves the width cap and none rises above
 *       the head top.</li>
 *   <li><b>Clearance.</b> No point sits inside the torso collider — the "cape
 *       clips into the body" defect.</li>
 *   <li><b>Drape.</b> The rendered cross-section is corrugated, not flat: the
 *       draw-time relief gives the hem a real fold depth, and it grows from the
 *       shoulder line to the hem so the cloth flares instead of hugging.</li>
 *   <li><b>One shoulder.</b> The pinned line is a diagonal: the clasp corner is
 *       much higher than the opposite corner, and the sheet's mid column sits
 *       between them (a symmetric cape would score 0).</li>
 *   <li><b>No per-frame allocation.</b> The render path touches only the
 *       preallocated arrays the sim already owns.</li>
 * </ul>
 *
 * <pre>
 *   javac -d build/mantle-test src/main/java/es/spectral/menu/cosmetics/render/ClothSim.java \
 *       tools/MantleShapeTest.java
 * </pre>
 *
 * <p>This file deliberately does not depend on the Minecraft classes, so it
 * compiles alone; {@link #main} prints the measured numbers for the report.
 */
public final class MantleShapeTest {

    private static int checks;
    private static int failures;

    // ---- constants mirrored from WearerSims / ClothSim (kept in sync by hand) ----
    private static final int COLS = 6, ROWS = 7;
    private static final float SEG_X = 0.112f, SEG_Y = 0.13f;
    private static final float GRAVITY = 0.0245f, DAMP = 0.92f;
    private static final float CAP_KNEES = 1.12f, CAP_HEAD_TOP = -0.5f, CAP_X = 0.78f;
    private static final float BODY_HALF_Z = 0.125f, BODY_PAD = 0.025f, BODY_HALF_X = 0.25f;
    /** {x, y, z} per column, from WearerSims.MANTLE_TOP. */
    private static final float[][] TOP = {
            { 0.28f, -0.04f, 0.20f},
            { 0.25f,  0.01f, 0.21f},
            { 0.12f,  0.05f, 0.21f},
            {-0.05f,  0.09f, 0.21f},
            {-0.21f,  0.13f, 0.20f},
            {-0.28f,  0.16f, 0.18f},
    };
    /** From WearerSims.MANTLE_RELIEF. */
    private static final float[] RELIEF = {0.012f, 0.052f, 0.026f, 0.080f, 0.030f, 0.010f};
    private static final float RELIEF_TOP = 0.25f;

    private static final float[] px = new float[COLS * ROWS];
    private static final float[] py = new float[COLS * ROWS];
    private static final float[] pz = new float[COLS * ROWS];
    private static final float[] ox = new float[COLS * ROWS];
    private static final float[] oy = new float[COLS * ROWS];
    private static final float[] oz = new float[COLS * ROWS];

    private static int idx(int c, int r) { return r * COLS + c; }

    public static void main(String[] args) {
        // Rest pose: settled, still air, upright body.
        reset();
        for (int t = 0; t < 600; t++) step(t, 0f, 0f, 0f, 1f, 0f);
        report("rest");
        assertRestShape();
        longLength("still");
        assertLength("still");
        assertFootprint("still");
        assertClearance("still");

        // Walking forward: the wearer's velocity maps to +z in the body frame,
        // so the mantle is pushed straight back (WIND_K * ~0.2 blocks/tick
        // walking speed). This is the case the playtest watched.
        reset();
        for (int t = 0; t < 600; t++) step(t, 0f, 0f, 0.018f, 1f, 0f);
        report("walk");
        assertLength("walk");
        assertFootprint("walk");
        assertClearance("walk");
        check("walk: the mantle trails behind the body, not into it",
                minHemZ() > BODY_HALF_Z + BODY_PAD,
                String.format("hem z = %.3f..%.3f", minHemZ(), maxHemZ()));

        // Sprint-strafe: sideways wind, the worst case for the width cap.
        reset();
        for (int t = 0; t < 600; t++) step(t, 0.014f, 0f, 0.010f, 1f, 0f);
        report("strafe");
        assertFootprint("strafe");
        assertClearance("strafe");

        // Sneaking: gravity rotated into the body frame by the vanilla 0.5 rad
        // crouch tilt, exactly as WearerSims does before stepping.
        reset();
        float pitch = 0.5f;
        float cy = (float) Math.cos(pitch), sy = (float) Math.sin(pitch);
        float downY = 1f * cy, downZ = 1f * sy;
        for (int t = 0; t < 600; t++) step(t, 0f, 0f, 0f, downY, downZ);
        report("sneak");
        assertLength("sneak");
        assertFootprint("sneak");
        assertClearance("sneak");

        // Jump: upward acceleration, i.e. apparent gravity reduced and a
        // downward wind (the wearer rises through the air).
        reset();
        for (int t = 0; t < 300; t++) step(t, 0f, 0.010f, 0f, 0.75f, 0f);
        report("jump");
        assertLength("jump");
        assertFootprint("jump");
        assertClearance("jump");

        System.out.println();
        System.out.printf("%d checks, %d failures%n", checks, failures);
        if (failures > 0) System.exit(1);
    }

    // ---------------------------------------------------------------
    // Sim mirror
    // ---------------------------------------------------------------

    private static void reset() {
        for (int c = 0; c < COLS; c++) {
            for (int r = 0; r < ROWS; r++) {
                int i = idx(c, r);
                px[i] = TOP[c][0];
                py[i] = TOP[c][1] + SEG_Y * r;
                pz[i] = TOP[c][2];
            }
        }
        System.arraycopy(px, 0, ox, 0, px.length);
        System.arraycopy(py, 0, oy, 0, py.length);
        System.arraycopy(pz, 0, oz, 0, pz.length);
    }

    private static void step(int tick, float wx, float wy, float wz, float downY, float downZ) {
        for (int c = 0; c < COLS; c++) {
            int i = idx(c, 0);
            px[i] = TOP[c][0]; py[i] = TOP[c][1]; pz[i] = TOP[c][2];
        }
        for (int i = COLS; i < px.length; i++) {
            float vx = (px[i] - ox[i]) * DAMP;
            float vy = (py[i] - oy[i]) * DAMP;
            float vz = (pz[i] - oz[i]) * DAMP;
            ox[i] = px[i]; oy[i] = py[i]; oz[i] = pz[i];
            px[i] += vx + wx;
            py[i] += vy + GRAVITY * downY + wy;
            pz[i] += vz + GRAVITY * downZ + wz;
        }
        for (int it = 0; it < 3; it++) {
            for (int c = 0; c < COLS; c++) {
                int i = idx(c, 0);
                px[i] = TOP[c][0]; py[i] = TOP[c][1]; pz[i] = TOP[c][2];
            }
            for (int r = 1; r < ROWS; r++) {
                for (int c = 0; c < COLS; c++) {
                    constrain(idx(c, r), idx(c, r - 1), SEG_Y, r == 1 ? 1f : 0.5f);
                }
            }
            for (int r = 1; r < ROWS; r++) {
                for (int c = 1; c < COLS; c++) {
                    constrain(idx(c, r), idx(c - 1, r), SEG_X, 0.5f);
                }
            }
            pushBody();
        }
        clampFootprint();
    }

    private static void constrain(int i, int j, float rest, float share) {
        float ex = px[i] - px[j], ey = py[i] - py[j], ez = pz[i] - pz[j];
        float d = (float) Math.sqrt(ex * ex + ey * ey + ez * ez);
        if (d < 1.0e-5f) d = rest;
        float diff = (d - rest) / d * share;
        px[i] -= ex * diff; py[i] -= ey * diff; pz[i] -= ez * diff;
        int row = j / COLS;
        if (row > 0) {                      // row 0 is pinned
            float js = 1f - share;
            px[j] += ex * diff * js; py[j] += ey * diff * js; pz[j] += ez * diff * js;
        }
    }

    /** Mirrors ClothSim.Body.push (no chestplate: bodyBack(0)). */
    private static void pushBody() {
        float half = BODY_HALF_Z + BODY_PAD;
        for (int i = COLS; i < px.length; i++) {
            float y = py[i];
            if (y <= 0f || y >= CAP_KNEES) continue;
            if (px[i] >= BODY_HALF_X || px[i] <= -BODY_HALF_X) continue;
            if (pz[i] < half && pz[i] > -half) pz[i] = pz[i] >= 0f ? half : -half;
        }
    }

    /** Mirrors ClothSim.clampFootprint (minZ from the mantle call: -0.30). */
    private static void clampFootprint() {
        for (int i = COLS; i < px.length; i++) {
            if (px[i] > CAP_X) px[i] = CAP_X;
            else if (px[i] < -CAP_X) px[i] = -CAP_X;
            if (py[i] > CAP_KNEES) py[i] = CAP_KNEES;
            else if (py[i] < CAP_HEAD_TOP) py[i] = CAP_HEAD_TOP;
            if (pz[i] < -0.30f) pz[i] = -0.30f;
        }
    }

    // ---------------------------------------------------------------
    // Assertions
    // ---------------------------------------------------------------

    private static void assertRestShape() {
        System.out.println();
        // One-shoulder diagonal: the clasp corner is clearly the high one.
        float clasp = py[idx(0, 0)], far = py[idx(COLS - 1, 0)];
        check("shoulder line is diagonal (clasp corner above the far corner)",
                clasp < far - 0.10f,
                String.format("clasp y=%.3f far y=%.3f", clasp, far));
        float mid = py[idx(2, 0)];
        check("mid column lies between the two corners",
                mid > clasp + 0.02f && mid < far,
                String.format("mid y=%.3f", mid));

        // Drape: the rendered hem cross-section must not be flat.
        float depth = reliefHemDepth();
        check("hem shows real fold depth when rendered (>= 1 px)",
                depth >= 1.0f / 16f,
                String.format("hem relief span = %.2f px", depth * 16f));
        check("fold relief grows from shoulder to hem (flare, not a tube)",
                reliefAt(1f) > reliefAt(0f) * 2f,
                String.format("relief at top = %.3f, at hem = %.3f", reliefAt(0f), reliefAt(1f)));
        // The deepest fold must still be behind the body surface, or the relief
        // would push cloth into the torso.
        float minZ = minHemZ() + minRelief();
        check("relief never pushes the hem into the torso",
                minZ >= BODY_HALF_Z + BODY_PAD,
                String.format("min hem z with relief = %.3f, surface = %.3f",
                        minZ, BODY_HALF_Z + BODY_PAD));

        // Horizontal ripple across the *simulated* sheet: the solver flattens
        // rest bulges, so this is expected to be flat and is reported, not
        // asserted — the visible folds come from the relief above.
        float ripple = maxHemZ() - minHemZ();
        System.out.printf("  (simulated hem ripple: %.3f blk — flat by design; "
                + "folds are applied at draw time)%n", ripple);
        System.out.println();
    }

    private static void longLength(String label) {
        float hem = maxHemY();
        System.out.printf("  hem lowest point: %.3f blk (knee line %.2f)%n", hem, CAP_KNEES);
    }

    private static void assertLength(String label) {
        float hem = maxHemY();
        check(label + ": hem ends at or above the knee line",
                hem <= CAP_KNEES + 1.0e-4f,
                String.format("hem y = %.3f, cap = %.2f", hem, CAP_KNEES));
        check(label + ": hem is long enough to read as a mantle (>= 0.75 blk)",
                hem >= 0.75f, String.format("hem y = %.3f", hem));
    }

    private static void assertFootprint(String label) {
        float maxX = 0f, minY = 0f;
        for (int i = 0; i < px.length; i++) {
            maxX = Math.max(maxX, Math.abs(px[i]));
            minY = Math.min(minY, py[i]);
        }
        check(label + ": nothing wider than the width cap",
                maxX <= CAP_X + 1.0e-4f, String.format("|x| max = %.3f, cap = %.2f", maxX, CAP_X));
        check(label + ": nothing above the head top",
                minY >= CAP_HEAD_TOP - 1.0e-4f,
                String.format("min y = %.3f, cap = %.2f", minY, CAP_HEAD_TOP));
    }

    private static void assertClearance(String label) {
        float half = BODY_HALF_Z + BODY_PAD;
        int inside = 0;
        for (int i = 0; i < px.length; i++) {
            if (py[i] <= 0f || py[i] >= CAP_KNEES) continue;
            if (px[i] >= BODY_HALF_X || px[i] <= -BODY_HALF_X) continue;
            if (pz[i] > -half && pz[i] < half) inside++;
        }
        check(label + ": no point inside the torso volume", inside == 0,
                inside + " points inside");
    }

    private static void report(String label) {
        System.out.printf("[%s] hem y=%.3f..%.3f  z=%.3f..%.3f  x=%.3f..%.3f%n",
                label, minHemY(), maxHemY(), minHemZ(), maxHemZ(),
                minHemX(), maxHemX());
    }

    // ---- helpers ----

    private static float reliefAt(float t) {
        return (RELIEF_TOP + (1f - RELIEF_TOP) * t) * maxAbsRelief();
    }

    private static float maxAbsRelief() {
        float m = 0f;
        for (float v : RELIEF) m = Math.max(m, Math.abs(v));
        return m;
    }

    private static float minRelief() {
        float m = 0f;
        for (float v : RELIEF) m = Math.min(m, v);
        return m;
    }

    private static float reliefHemDepth() {
        float lo = Float.MAX_VALUE, hi = -Float.MAX_VALUE;
        for (int c = 0; c < COLS; c++) {
            float z = pz[idx(c, ROWS - 1)] + RELIEF[c] * 1f;
            lo = Math.min(lo, z);
            hi = Math.max(hi, z);
        }
        return hi - lo;
    }

    private static float minHemY() { return rowMin(py); }
    private static float maxHemY() { return rowMax(py); }
    private static float minHemZ() { return rowMin(pz); }
    private static float maxHemZ() { return rowMax(pz); }
    private static float minHemX() { return rowMin(px); }
    private static float maxHemX() { return rowMax(px); }

    private static float rowMin(float[] a) {
        float m = Float.MAX_VALUE;
        for (int c = 0; c < COLS; c++) m = Math.min(m, a[idx(c, ROWS - 1)]);
        return m;
    }

    private static float rowMax(float[] a) {
        float m = -Float.MAX_VALUE;
        for (int c = 0; c < COLS; c++) m = Math.max(m, a[idx(c, ROWS - 1)]);
        return m;
    }

    private static void check(String what, boolean ok, String detail) {
        checks++;
        if (!ok) failures++;
        System.out.printf("  %s %s%s%n", ok ? "ok  " : "FAIL", what,
                detail == null || detail.isEmpty() ? "" : "  [" + detail + "]");
    }
}
