package es.spectral.menu.cosmetics.render;

/**
 * Verlet/spring cloth primitives for the cosmetic renderers.
 *
 * <p>Everything here runs in the wearer "emit frame": the coordinate space a
 * {@code RenderLayer} (or {@code submitCustomGeometry} callback) writes into
 * while submitting an avatar. In that space +X is the wearer's RIGHT, +Y is
 * DOWN (the model is drawn flipped), +Z is BEHIND the wearer, the origin sits
 * on the model root (the neck line of the humanoid) and one unit is one block —
 * exactly the space {@code ModelPart#translateAndRotate} leaves the pose stack
 * in. Gravity and wind are handed in already rotated into the frame the caller
 * simulates in, so the same code serves upright, sneaking, bowing, swimming and
 * gliding poses.
 *
 * <p>Chains keep a render copy ({@code q*}) of the previous tick so the layer
 * can interpolate per frame; simulation itself happens once per client tick.
 */
public final class ClothSim {

    private ClothSim() {}

    /** Per-tick gravity magnitude (blocks/tick^2), tuned for 1.2 m capes. */
    public static final float GRAVITY = 0.0245f;
    /** Base velocity damping per tick. */
    public static final float DAMP = 0.92f;

    // Cosmetic footprint caps (model units: +Y down, origin at the neck line).
    /** Nothing above the head top. */
    public static final float CAP_HEAD_TOP = -0.5f;
    /** Nothing lower than the knees. */
    public static final float CAP_KNEES = 1.12f;
    /** Nothing wider than the shoulders plus one block. */
    public static final float CAP_X = 0.78f;

    // ---------------------------------------------------------------
    // Body collision (the torso and upper legs are not penetrable)
    // ---------------------------------------------------------------

    /** Torso surface: 2 px behind/in front of the neck line. */
    public static final float BODY_HALF_Z = 0.125f;
    /** Torso half width (4 px). */
    public static final float BODY_HALF_X = 0.25f;
    /** Skin offset: cloth floats 0.4 px clear of the surface it drapes on. */
    public static final float BODY_PAD = 0.025f;
    /**
     * Extra clearance a chestplate needs: the outer armor deformation inflates
     * the torso by 1 px, and vanilla's own cape layer adds another 1.1 px on
     * top of that ({@code CapeLayer} translates its model 0.06875 blocks).
     */
    public static final float ARMOR_CLEARANCE = 0.0625f + 0.06875f;
    /** Bottom of the collidable volume: the knee cap (upper legs share the torso's z span). */
    public static final float BODY_BOTTOM = CAP_KNEES;

    /**
     * Plane distance for cloth: {@code BODY_HALF_Z + BODY_PAD} plus the armor
     * clearance when the wearer has a chestplate.
     */
    public static float bodyBack(float armorClearance) {
        return BODY_HALF_Z + BODY_PAD + armorClearance;
    }

    /**
     * Pushes one cloth point out of the torso volume to whichever z face it is
     * closest to (behind for cloaks, in front for a mantle's chest drape). The
     * point keeps its x and y, so the sheet slides along the body instead of
     * collapsing. Cheap: a handful of compares and at most one store.
     *
     * @param half   half depth of the torso box along z (see {@link #bodyBack})
     * @param top    y of the shoulder line in the sim frame (0 = neck root)
     * @param bottom y of the bottom of the collidable volume
     */
    static void pushOutOfBody(float[] px, float[] py, float[] pz, int i,
            float half, float top, float bottom) {
        if (half <= 0f) return;
        float y = py[i];
        if (y <= top || y >= bottom) return;
        float x = px[i];
        if (x >= BODY_HALF_X || x <= -BODY_HALF_X) return;
        float z = pz[i];
        if (z < half) pz[i] = z >= 0f ? half : -half;
    }

    /**
     * The wearer's torso as a collidable box in the <b>body part's local
     * frame</b> — the frame the cloth simulates in, so the box is axis-aligned
     * and never needs rotating: the crouch pitch, an emote's bow and the sit
     * drop are all part of the frame itself. One reusable instance per
     * {@code WearerSims.Entry}; {@link #set} runs once per tick, so the
     * simulation path allocates nothing.
     *
     * <p>A point inside the box is clamped to the nearer z face and keeps its
     * x/y, so a sheet slides along the chest instead of collapsing. Row 0 is
     * never pushed — it is pinned to the shoulders by definition.
     */
    public static final class Body {
        /** Half depth (z) of the torso volume, armor-aware. */
        public float halfZ = bodyBack(0f);
        /** Hip line in body-local units (12 px below the neck pivot). */
        public static final float HIP = 0.75f;

        /**
         * @param zHalf     torso half depth including the armor clearance
         * @param bodyPitch the part's {@code xRot}: while upright the upper legs
         *                  sit in the same z span, once it hinges at the hip
         *                  they swing away and the box stops at the hip line
         */
        public void set(float zHalf, float bodyPitch) {
            this.halfZ = zHalf;
            this.legs = bodyPitch > -0.25f && bodyPitch < 0.25f;
        }

        /** True while the upper legs share the torso's z span. */
        private boolean legs = true;

        /** Clamps points {@code [from, n)} out of the torso (and upper legs). */
        void push(float[] px, float[] py, float[] pz, int from, int n) {
            float half = halfZ;
            float botY = legs ? BODY_BOTTOM : HIP;
            for (int i = from; i < n; i++) {
                float y = py[i];
                if (y <= 0f || y >= botY) continue;
                float x = px[i];
                if (x >= BODY_HALF_X || x <= -BODY_HALF_X) continue;
                float z = pz[i];
                if (z < half && z > -half) pz[i] = z >= 0f ? half : -half;
            }
        }
    }

    /** Keeps one cloth point inside the cosmetic footprint cap. */
    static void clampFootprint(float[] px, float[] py, float[] pz, int i,
            float minZ, float capY) {
        if (px[i] > CAP_X) px[i] = CAP_X;
        else if (px[i] < -CAP_X) px[i] = -CAP_X;
        float low = capY < CAP_KNEES ? capY : CAP_KNEES;
        if (py[i] > low) py[i] = low;
        else if (py[i] < CAP_HEAD_TOP) py[i] = CAP_HEAD_TOP;
        if (minZ > -900f && pz[i] < minZ) pz[i] = minZ;
    }

    /** A pinned-at-0 verlet rope. {@code n} points, equal rest length. */
    public static final class Chain {
        /** Current tick position (emit frame). */
        public final float[] px, py, pz;
        /** Previous tick position (lerp source for the render thread). */
        public final float[] qx, qy, qz;
        private final float[] ox, oy, oz;
        public final int n;
        public final float seg;

        public Chain(int n, float seg) {
            this.n = n;
            this.seg = seg;
            px = new float[n]; py = new float[n]; pz = new float[n];
            qx = new float[n]; qy = new float[n]; qz = new float[n];
            ox = new float[n]; oy = new float[n]; oz = new float[n];
        }

        /** Straightens the chain from {@code (ax,ay,az)} along {@code (dx,dy,dz)}. */
        public void reset(float ax, float ay, float az, float dx, float dy, float dz) {
            for (int i = 0; i < n; i++) {
                px[i] = ax + dx * seg * i;
                py[i] = ay + dy * seg * i;
                pz[i] = az + dz * seg * i;
            }
            System.arraycopy(px, 0, qx, 0, n);
            System.arraycopy(py, 0, qy, 0, n);
            System.arraycopy(pz, 0, qz, 0, n);
            System.arraycopy(px, 0, ox, 0, n);
            System.arraycopy(py, 0, oy, 0, n);
            System.arraycopy(pz, 0, oz, 0, n);
        }

        /**
         * Rotates every point in the XZ plane about the origin by {@code rad}
         * (current, velocity and render-lerp states together).
         */
        public void yawAll(float rad) {
            float c = (float) Math.cos(rad), s = (float) Math.sin(rad);
            for (int i = 0; i < n; i++) {
                float x = px[i], z = pz[i];
                px[i] = x * c + z * s;
                pz[i] = -x * s + z * c;
                x = ox[i]; z = oz[i];
                ox[i] = x * c + z * s;
                oz[i] = -x * s + z * c;
                x = qx[i]; z = qz[i];
                qx[i] = x * c + z * s;
                qz[i] = -x * s + z * c;
            }
        }

        /**
         * One simulation tick.
         *
         * @param anchor      pinned point 0 (emit frame)
         * @param down        unit-ish gravity direction in emit frame
         * @param wind        wearer-relative airflow push (emit frame,
         *                    blocks/tick^2), usually {@code -wearerVelocity}
         * @param damp        velocity retention, {@link #DAMP} is a good base
         * @param flutter     ambient flutter amplitude (0 = off)
         * @param phase       per-chain phase for the flutter wave
         * @param tick        wearer tick counter (drives the flutter wave)
         * @param minZ        points are pushed to {@code z >= minZ} (keeps back
         *                    cloth behind the chest); pass a very negative
         *                    value to disable
         * @param capY        low end of the footprint (knees); points are kept
         *                    at {@code y <= capY}
         * @param body        torso collider; points are pushed out of its
         *                    volume after every constraint pass, or
         *                    {@code null} to disable collision
         */
        public void step(float ax, float ay, float az,
                         float dx, float dy, float dz,
                         float wx, float wy, float wz,
                         float damp, float flutter, float phase, int tick,
                         float minZ, float capY, Body body) {
            // Keep the pre-step state as the render-lerp source: at partial
            // tick 0 the wearer renders at its previous-tick position, so the
            // cloth interpolates between the same two tick states.
            System.arraycopy(px, 0, qx, 0, n);
            System.arraycopy(py, 0, qy, 0, n);
            System.arraycopy(pz, 0, qz, 0, n);
            px[0] = ax; py[0] = ay; pz[0] = az;
            for (int i = 1; i < n; i++) {
                float vx = (px[i] - ox[i]) * damp;
                float vy = (py[i] - oy[i]) * damp;
                float vz = (pz[i] - oz[i]) * damp;
                ox[i] = px[i]; oy[i] = py[i]; oz[i] = pz[i];
                float fl = flutter > 0
                        ? flutter * (float) Math.sin(tick * 0.31 + phase + i * 0.9)
                        : 0f;
                px[i] += vx + dx * GRAVITY + wx + fl * 0.4f;
                py[i] += vy + dy * GRAVITY + wy + fl * 0.15f;
                pz[i] += vz + dz * GRAVITY + wz + fl;
            }
            // Distance constraints, pin 0. The torso collider runs inside the
            // iteration so the cloth resolves against the body and the
            // distance links together, instead of the body winning the last
            // word and leaving the chain stretched.
            for (int it = 0; it < 3; it++) {
                px[0] = ax; py[0] = ay; pz[0] = az;
                for (int i = 1; i < n; i++) {
                    float ex = px[i] - px[i - 1];
                    float ey = py[i] - py[i - 1];
                    float ez = pz[i] - pz[i - 1];
                    float d = (float) Math.sqrt(ex * ex + ey * ey + ez * ez);
                    if (d < 1.0e-5f) d = seg;
                    float diff = (d - seg) / d;
                    px[i] -= ex * diff;
                    py[i] -= ey * diff;
                    pz[i] -= ez * diff;
                }
                if (body != null) body.push(px, py, pz, 1, n);
            }
            for (int i = 1; i < n; i++) {
                clampFootprint(px, py, pz, i, minZ, capY);
            }
            if (body != null) body.push(px, py, pz, 1, n);
        }

        /**
         * Freezes the render lerp: copies the current state over the
         * render-source copy so the cloth shows a static pose (no per-tick
         * oscillation between two tick states).
         */
        public void freeze() {
            System.arraycopy(px, 0, qx, 0, n);
            System.arraycopy(py, 0, qy, 0, n);
            System.arraycopy(pz, 0, qz, 0, n);
        }
    }

    /**
     * A rectangular cloth sheet: {@code cols} x {@code rows} points, the whole
     * top row pinned to an anchor line. Used by the mantle and the hooded
     * cloak's body.
     */
    public static final class Sheet {
        public final float[] px, py, pz, qx, qy, qz;
        private final float[] ox, oy, oz;
        public final int cols, rows;
        public final float segX, segY;

        public Sheet(int cols, int rows, float segX, float segY) {
            this.cols = cols;
            this.rows = rows;
            this.segX = segX;
            this.segY = segY;
            int n = cols * rows;
            px = new float[n]; py = new float[n]; pz = new float[n];
            qx = new float[n]; qy = new float[n]; qz = new float[n];
            ox = new float[n]; oy = new float[n]; oz = new float[n];
        }

        private int idx(int c, int r) { return r * cols + c; }

        /**
         * Lays the sheet flat: row 0 pinned to {@code top} (per-column anchor
         * positions), later rows hanging along {@code (dx,dy,dz)}.
         */
        public void reset(float[][] top, float dx, float dy, float dz) {
            for (int c = 0; c < cols; c++) {
                resetColumn(c, top[c][0], top[c][1], top[c][2], dx, dy, dz);
            }
            int n = cols * rows;
            System.arraycopy(px, 0, qx, 0, n);
            System.arraycopy(py, 0, qy, 0, n);
            System.arraycopy(pz, 0, qz, 0, n);
            System.arraycopy(px, 0, ox, 0, n);
            System.arraycopy(py, 0, oy, 0, n);
            System.arraycopy(pz, 0, oz, 0, n);
        }

        /** Lays the sheet flat from explicit per-column anchors. */
        public void reset(float[] topX, float[] topY, float[] topZ,
                          float dx, float dy, float dz) {
            for (int c = 0; c < cols; c++) {
                resetColumn(c, topX[c], topY[c], topZ[c], dx, dy, dz);
            }
            int n = cols * rows;
            System.arraycopy(px, 0, qx, 0, n);
            System.arraycopy(py, 0, qy, 0, n);
            System.arraycopy(pz, 0, qz, 0, n);
            System.arraycopy(px, 0, ox, 0, n);
            System.arraycopy(py, 0, oy, 0, n);
            System.arraycopy(pz, 0, oz, 0, n);
        }

        private void resetColumn(int c, float ax, float ay, float az,
                                 float dx, float dy, float dz) {
            for (int r = 0; r < rows; r++) {
                int i = idx(c, r);
                px[i] = ax + dx * segY * r;
                py[i] = ay + dy * segY * r;
                pz[i] = az + dz * segY * r;
            }
        }

        /** Rotates every point in the XZ plane about the origin by {@code rad}. */
        public void yawAll(float rad) {
            float c = (float) Math.cos(rad), s = (float) Math.sin(rad);
            int n = cols * rows;
            for (int i = 0; i < n; i++) {
                float x = px[i], z = pz[i];
                px[i] = x * c + z * s;
                pz[i] = -x * s + z * c;
                x = ox[i]; z = oz[i];
                ox[i] = x * c + z * s;
                oz[i] = -x * s + z * c;
                x = qx[i]; z = qz[i];
                qx[i] = x * c + z * s;
                qz[i] = -x * s + z * c;
            }
        }

        /**
         * One tick. {@code topX/topY/topZ} hold the {@code cols} pinned
         * anchor positions of row 0 (left-to-right in emit frame). Other
         * params behave like {@link Chain#step}.
         */
        public void step(float[] topX, float[] topY, float[] topZ,
                         float dx, float dy, float dz,
                         float wx, float wy, float wz,
                         float damp, float flutter, float phase, int tick,
                         float minZ, float capY, Body body) {
            int total = cols * rows;
            System.arraycopy(px, 0, qx, 0, total);
            System.arraycopy(py, 0, qy, 0, total);
            System.arraycopy(pz, 0, qz, 0, total);
            for (int c = 0; c < cols; c++) {
                px[c] = topX[c]; py[c] = topY[c]; pz[c] = topZ[c];
            }
            int n = cols * rows;
            for (int i = cols; i < n; i++) {
                float vx = (px[i] - ox[i]) * damp;
                float vy = (py[i] - oy[i]) * damp;
                float vz = (pz[i] - oz[i]) * damp;
                ox[i] = px[i]; oy[i] = py[i]; oz[i] = pz[i];
                float fl = flutter > 0
                        ? flutter * (float) Math.sin(tick * 0.31 + phase + i * 0.53)
                        : 0f;
                px[i] += vx + dx * GRAVITY + wx + fl * 0.35f;
                py[i] += vy + dy * GRAVITY + wy;
                pz[i] += vz + dz * GRAVITY + wz + fl;
            }
            for (int it = 0; it < 3; it++) {
                for (int c = 0; c < cols; c++) {
                    px[c] = topX[c]; py[c] = topY[c]; pz[c] = topZ[c];
                }
                // vertical links
                for (int r = 1; r < rows; r++) {
                    for (int c = 0; c < cols; c++) {
                        constrain(idx(c, r), idx(c, r - 1), segY,
                                r == 1 ? 1f : 0.5f);
                    }
                }
                // horizontal links keep the hem coherent
                for (int r = 1; r < rows; r++) {
                    for (int c = 1; c < cols; c++) {
                        constrain(idx(c, r), idx(c - 1, r), segX, 0.5f);
                    }
                }
                if (body != null) body.push(px, py, pz, cols, n);
            }
            for (int i = cols; i < n; i++) {
                clampFootprint(px, py, pz, i, minZ, capY);
            }
            if (body != null) body.push(px, py, pz, cols, n);
        }

        private void constrain(int i, int j, float rest, float share) {
            float ex = px[i] - px[j];
            float ey = py[i] - py[j];
            float ez = pz[i] - pz[j];
            float d = (float) Math.sqrt(ex * ex + ey * ey + ez * ez);
            if (d < 1.0e-5f) d = rest;
            float diff = (d - rest) / d * share;
            // j may be pinned (row 0): then i takes the full correction.
            float js = j < cols ? 0f : 1f - share;
            px[i] -= ex * diff;
            py[i] -= ey * diff;
            pz[i] -= ez * diff;
            if (js > 0) {
                px[j] += ex * diff * js;
                py[j] += ey * diff * js;
                pz[j] += ez * diff * js;
            }
        }

        /**
         * Freezes the render lerp (see {@link Chain#freeze()}).
         */
        public void freeze() {
            int n = cols * rows;
            System.arraycopy(px, 0, qx, 0, n);
            System.arraycopy(py, 0, qy, 0, n);
            System.arraycopy(pz, 0, qz, 0, n);
        }
    }
}
