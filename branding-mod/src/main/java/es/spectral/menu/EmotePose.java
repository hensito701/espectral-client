package es.spectral.menu;

/**
 * Minecraft-free pose math for the five local emotes. {@link EmoteEngine}
 * samples {@link #write} every rendered frame and blends the result onto the
 * {@code PlayerModel} part transforms; {@code tools/EmotePoseTest.java} samples
 * the same function with plain {@code javac} to prove the poses are sound (the
 * waving hand never enters the head, the clap palms meet, the bow hinges at the
 * hips, the sit drop is the agreed amount), so nothing here may import
 * {@code net.minecraft}.
 *
 * <h2>Model facts (player model, model units = 1/16 block)</h2>
 * <ul>
 *   <li>{@code ModelPart.translateAndRotate} applies {@code R = Rz * Ry * Rx}
 *       to the part's <em>local</em> vector, then adds the pivot offset.</li>
 *   <li>Head pivot at the neck (0,0,0), cube x[-4,4] y[-8,0] z[-4,4].
 *       Positive {@code xRot} looks down, positive {@code yRot} turns the face
 *       towards -x, positive {@code zRot} tips the head top towards +x.</li>
 *   <li>Arms pivot at (±5,2,0); the cube is local x[-3,1] (right) / [-1,3]
 *       (left), y[-2,10], z[-2,2]: the hand centre is local (0,10,0), 10 px
 *       below the shoulder pivot.</li>
 *   <li>Body pivot is the <em>neck</em>, cube y[0,12] (hips at y=12) — there is
 *       no hip joint, so the hip-hinged bow is emulated by translating body,
 *       head and arms to the hip-rotated pivot (see {@link #bow}).</li>
 *   <li>Legs pivot at (±1.9,12,0), cube y[0,12]; the feet rest on y=24, the
 *       vanilla ground plane.</li>
 * </ul>
 *
 * <h2>Arm signs (right arm pivot x = -5)</h2>
 * {@code zRot} maps the hand direction (0,10,0) to {@code (-10 sin z, 10 cos z)}:
 * a <b>positive</b> {@code zRot} swings the right arm <b>outward</b> (-x), a
 * <b>negative</b> one across the head. The previous wave used {@code -pi+0.45}
 * (= -2.69), i.e. the hand crossed in front of the face and ended up inside the
 * head — that is the bug this class removes. With the arm hanging down,
 * {@code yRot} < 0 turns the right hand inward (+x); with the arm pointing
 * forward ({@code xRot} ≈ -1.42) {@code yRot} < 0 also moves it towards the
 * centre line, which is what makes the clap palms meet.
 */
public final class EmotePose {

    public static final String WAVE = "saludo";
    public static final String BOW = "reverencia";
    public static final String CLAP = "aplauso";
    public static final String SIT = "sentarse";
    public static final String INVOCATION = "invocacion";

    /** Pose slots, in the order the engine copies them onto the model. */
    public static final int BODY = 0;
    public static final int HEAD = 1;
    public static final int RIGHT_ARM = 2;
    public static final int LEFT_ARM = 3;
    public static final int RIGHT_LEG = 4;
    public static final int LEFT_LEG = 5;
    public static final int ROOT = 6;
    public static final int PARTS = 7;
    /** Floats per part: x, y, z, xRot, yRot, zRot. */
    public static final int STRIDE = 6;
    public static final int X = 0;
    public static final int Y = 1;
    public static final int Z = 2;
    public static final int X_ROT = 3;
    public static final int Y_ROT = 4;
    public static final int Z_ROT = 5;

    // ---------------------------------------------------------------
    // Geometry constants (also used by the verification harness)
    // ---------------------------------------------------------------

    public static final float SHOULDER_X = 5.0f;
    public static final float SHOULDER_Y = 2.0f;
    public static final float HIP_Y = 12.0f;
    /** Head cube in head-local units. */
    public static final float HEAD_MIN_X = -4.0f;
    public static final float HEAD_MIN_Y = -8.0f;
    public static final float HEAD_MIN_Z = -4.0f;
    public static final float HEAD_MAX_X = 4.0f;
    public static final float HEAD_MAX_Y = 0.0f;
    public static final float HEAD_MAX_Z = 4.0f;
    /** Ground plane in the model-root frame (feet rest here). */
    public static final float GROUND_Y = 24.0f;

    // ---------------------------------------------------------------
    // Timings
    // ---------------------------------------------------------------

    /** One-shot lengths; looping emotes return -1 from {@link #duration}. */
    public static final float WAVE_SECONDS = 1.8f;
    public static final float BOW_SECONDS = 2.4f;
    public static final float CLAP_SECONDS = 3.2f;
    /** Cross-fade from (and back to) the vanilla animation, seconds. */
    public static final float BLEND_SECONDS = 0.25f;

    // ---------------------------------------------------------------
    // Saludo / Wave
    // ---------------------------------------------------------------

    /** Raised arm, right arm swung 2.44 rad out to the side: hand x -10.6, y -6.2. */
    public static final float WAVE_ZROT = 2.44f;
    /** Sway amplitude (rad) around {@link #WAVE_ZROT}: hand sweeps x -12.0..-9.4. */
    public static final float WAVE_SWAY = 0.25f;
    /** Sway rate (rad/s). */
    public static final float WAVE_SWAY_RATE = 7.0f;
    /** Arm tilted slightly forward so the wave reads from the front. */
    public static final float WAVE_XROT = -0.30f;
    /**
     * Shoulder pivot nudged outwards (px) while the arm is raised. Without it
     * the arm's upper cap grazes the head's lower corner once the arm is past
     * ~2.2 rad. 1.5 px on a 4 px-wide arm is invisible in third person and
     * clears the whole swing even when the player turns their head towards the
     * raised hand (measured penetration 0.0 px over a +-0.6 rad look range).
     */
    public static final float WAVE_SHOULDER_OUT = 1.5f;
    /** Shoulder pivot nudged up (px) with the arm (a small shrug). */
    public static final float WAVE_SHOULDER_UP = 0.9f;
    /** Head turns towards the raised hand (yaw only — a roll tilts it into the arm). */
    public static final float WAVE_HEAD_YROT = 0.10f;
    /** Head roll kept tiny, away from the raised arm. */
    public static final float WAVE_HEAD_ZROT = -0.04f;
    public static final float WAVE_HEAD_XROT = -0.10f;

    // ---------------------------------------------------------------
    // Aplauso / Clap
    // ---------------------------------------------------------------

    /** Arms forward at chest height: hand y 3.5, z -9.9 px. */
    public static final float CLAP_XROT = -1.42f;
    /**
     * |yRot| that puts both inner palm faces exactly on x = 0. Solved
     * numerically for the pivot shift below; at contact the right hand's inner
     * face (local x=+1, y=10) maps to x=0 and the mirror does the same on the
     * other side, so the palms touch at the centre line.
     */
    public static final float CLAP_CONTACT = 0.4979f;
    /** yRot released at the open phase -> palms 2.9 px apart (3 px target). */
    public static final float CLAP_OPEN = 0.17f;
    /** 3 claps per second. */
    public static final float CLAP_HZ = 3.0f;
    /**
     * Both shoulders shift forward (px) so the upper arm clears the chest while
     * the forearm swings across — a 2 px reach forward reads as leaning into
     * the clap and removes the arm/torso intersection completely.
     */
    public static final float CLAP_SHOULDER_FWD = 1.2f;
    /** Both shoulders shift outwards (px) so the elbow never enters the ribcage. */
    public static final float CLAP_SHOULDER_OUT = 0.6f;

    // ---------------------------------------------------------------
    // Reverencia / Bow
    // ---------------------------------------------------------------

    /** Torso pitch about the hip, 0.55 rad (~31.5 deg). */
    public static final float BOW_PITCH = 0.55f;
    /** Extra nod on top of the torso pitch (the head never folds further). */
    public static final float BOW_NOD = 0.10f;
    /** Arms hang almost gravity-vertical instead of following the torso. */
    public static final float BOW_ARM = -0.10f;
    /**
     * Shoulders shift outwards (px) with the bow: the chest pitches away from
     * the vertical arms, so without this the upper arm clips into the ribs.
     */
    public static final float BOW_ARM_OUT = 0.6f;

    // ---------------------------------------------------------------
    // Sentarse / Sit
    // ---------------------------------------------------------------

    /** Whole model lowered so the hips (y=12, plus drop) sit just above the ground. */
    public static final float SIT_DROP = 11.0f;
    /** Legs forward: feet land at y ~= 23.9 (ground is 24). */
    public static final float SIT_LEG = -1.50f;
    /** Legs spread slightly (yRot, applied after the forward pitch). */
    public static final float SIT_SPREAD = 0.14f;
    /** Arms angled forward to rest on the knees (not through the thighs). */
    public static final float SIT_ARM = -1.05f;
    /** Shoulders shift outwards (px) so the arms lie on the outside of the knees. */
    public static final float SIT_ARM_OUT = 0.6f;

    // ---------------------------------------------------------------
    // Invocación / Invocation
    // ---------------------------------------------------------------

    /** Arms up and forward (in front of the torso, never over the head). */
    public static final float INVOCATION_XROT = -2.60f;
    /** V spread outwards. */
    public static final float INVOCATION_ZROT = 0.50f;
    /** Head tipped back. */
    public static final float INVOCATION_HEAD = -0.25f;
    /** Chest opened a hair. */
    public static final float INVOCATION_BODY = -0.05f;
    /** Hover bob amplitude (px, upwards, i.e. negative y). */
    public static final float INVOCATION_BOB = 0.5f;
    /**
     * Shoulders shift outwards (px) with the V so the raised upper arms clear
     * the chest; without it they graze the ribcage at the top of the swing.
     */
    public static final float INVOCATION_SHOULDER_OUT = 1.2f;
    /** Shoulders rise (px) with the arms. */
    public static final float INVOCATION_SHOULDER_UP = 0.4f;

    private EmotePose() {}

    /** One-shot length in seconds, or -1 for the loops (sit, invocation). */
    public static float duration(String id) {
        if (WAVE.equals(id)) return WAVE_SECONDS;
        if (BOW.equals(id)) return BOW_SECONDS;
        if (CLAP.equals(id)) return CLAP_SECONDS;
        return -1.0f;
    }

    /**
     * True when the part's rotations are <em>offsets</em> on top of the vanilla
     * pose instead of absolute replacements. The head and chest must stay
     * additive because the player keeps looking around during an emote (vanilla
     * writes {@code head.xRot/yRot} from the look direction every frame); arms
     * and legs are replaced outright, since the player is standing still and the
     * idle swing would otherwise fight the pose.
     */
    public static boolean additiveRotation(int part) {
        return part == HEAD || part == BODY;
    }

    /** True for the emotes that loop until cancelled. */
    public static boolean loops(String id) {
        return SIT.equals(id) || INVOCATION.equals(id);
    }

    // ---------------------------------------------------------------
    // Pose sampling
    // ---------------------------------------------------------------

    /**
     * Writes the <em>absolute</em> pose for {@code id} at {@code t} seconds into
     * {@code out} (length {@code PARTS*STRIDE}), zeroing it first. Returns false
     * for an unknown id.
     *
     * <p>{@code reducedMotion} keeps the same held pose with every oscillation
     * frozen — the rule cloth physics and cosmetics already follow.</p>
     */
    public static boolean write(String id, float t, boolean reducedMotion, float[] out) {
        java.util.Arrays.fill(out, 0, PARTS * STRIDE, 0.0f);
        if (WAVE.equals(id)) { wave(t, reducedMotion, out); return true; }
        if (BOW.equals(id)) { bow(t, reducedMotion, out); return true; }
        if (CLAP.equals(id)) { clap(t, reducedMotion, out); return true; }
        if (SIT.equals(id)) { sit(t, reducedMotion, out); return true; }
        if (INVOCATION.equals(id)) { invocation(t, reducedMotion, out); return true; }
        return false;
    }

    /**
     * Blend weight of the emote against the vanilla animation for this frame:
     * eases in over {@link #BLEND_SECONDS} and, for one-shots, back out over the
     * same window, so the first and last frames are the vanilla pose again.
     */
    public static float blendWeight(String id, float t) {
        float w = smoothstep(t / BLEND_SECONDS);
        float d = duration(id);
        if (d > 0.0f) w = Math.min(w, smoothstep((d - t) / BLEND_SECONDS));
        return clamp01(w);
    }

    /**
     * Saludo / Wave: the right arm rises out to the side (not across the face)
     * and waves by oscillating {@code zRot}; the head turns towards the raised
     * hand. Reduced motion holds the arm up at the middle of the sway.
     */
    private static void wave(float t, boolean reducedMotion, float[] out) {
        float env = envelope(t, 0.30f, 0.35f, WAVE_SECONDS);
        // The shoulder nudge arrives ahead of the swing and leaves last, so the
        // arm never sweeps past the head on its way up or down.
        float lead = clamp01(Math.min(smoothstep(t / 0.10f),
                smoothstep((WAVE_SECONDS - t) / 0.14f)));
        float sway = reducedMotion ? 0.0f : (float) Math.sin(t * WAVE_SWAY_RATE) * WAVE_SWAY;
        float up = env * (WAVE_ZROT + sway);
        set(out, RIGHT_ARM,
                -WAVE_SHOULDER_OUT * lead, WAVE_SHOULDER_UP * lead, 0.0f,
                WAVE_XROT * env, 0.0f, up);
        set(out, HEAD, 0.0f, 0.0f, 0.0f,
                WAVE_HEAD_XROT * env, WAVE_HEAD_YROT * env, WAVE_HEAD_ZROT * env);
        // Small counter-roll of the chest so the raised arm does not look stiff.
        set(out, BODY, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, -0.03f * env);
    }

    /**
     * Aplauso / Clap: both arms forward and angled inwards so the palms meet on
     * the centre line ({@link #CLAP_CONTACT} is the solved angle at which the
     * right hand's inner face x=+1 lands on x=0, the left one mirrored), then
     * open by {@link #CLAP_OPEN} and close again three times a second.
     */
    private static void clap(float t, boolean reducedMotion, float[] out) {
        float env = envelope(t, 0.25f, 0.25f, CLAP_SECONDS);
        // 0 = closed (contact), 1 = open.
        float open = reducedMotion
                ? 0.65f
                : 0.5f - 0.5f * (float) Math.cos(t * CLAP_HZ * 2.0 * Math.PI);
        float inward = (CLAP_CONTACT - CLAP_OPEN * open) * env;
        set(out, RIGHT_ARM, -CLAP_SHOULDER_OUT * env, 0.0f, -CLAP_SHOULDER_FWD * env,
                CLAP_XROT * env, -inward, 0.0f);
        set(out, LEFT_ARM, CLAP_SHOULDER_OUT * env, 0.0f, -CLAP_SHOULDER_FWD * env,
                CLAP_XROT * env, inward, 0.0f);
        // A hair of torso counter-motion sells the impact without moving the palms.
        float beat = reducedMotion ? 0.0f : (1.0f - open) * 0.03f * env;
        set(out, BODY, 0.0f, 0.0f, 0.0f, beat, 0.0f, 0.0f);
        set(out, HEAD, 0.0f, 0.0f, 0.0f, -0.06f * env - beat * 0.5f, 0.0f, 0.0f);
    }

    /**
     * Reverencia / Bow: a hip-hinged bow. The vanilla body part pivots at the
     * <em>neck</em>, so the hip hinge is emulated by rotating every upper part
     * about the hip point (0,12,0) and translating its pivot into place:
     * {@code body/head.y = 12(1-cos a)}, {@code z = -12 sin a} (the neck travels
     * forward and down), {@code arm.y = 10(1-cos a)}, {@code z = -10 sin a} (the
     * shoulders follow the same arc). The hip point itself is invariant, the
     * legs stay straight, and the head adds only a small nod on top of the torso
     * pitch — nothing folds or inverts.
     */
    private static void bow(float t, boolean reducedMotion, float[] out) {
        float env = envelope(t, 0.50f, 0.60f, BOW_SECONDS);
        float a = BOW_PITCH * env;
        float c = (float) Math.cos(a);
        float s = (float) Math.sin(a);
        float neckY = HIP_Y * (1.0f - c);
        float neckZ = -HIP_Y * s;
        set(out, BODY, 0.0f, neckY, neckZ, a, 0.0f, 0.0f);
        set(out, HEAD, 0.0f, neckY, neckZ, a + BOW_NOD * env, 0.0f, 0.0f);
        float shoulderY = (HIP_Y - SHOULDER_Y) * (1.0f - c);
        float shoulderZ = -(HIP_Y - SHOULDER_Y) * s;
        // Parts are siblings: the arms' own xRot decides their world direction,
        // so they hang almost vertically instead of following the torso, and a
        // small outward shift keeps the upper arm out of the pitched chest.
        set(out, RIGHT_ARM, -BOW_ARM_OUT * env, shoulderY, shoulderZ,
                BOW_ARM * env, 0.0f, 0.0f);
        set(out, LEFT_ARM, BOW_ARM_OUT * env, shoulderY, shoulderZ,
                BOW_ARM * env, 0.0f, 0.0f);
    }

    /**
     * Sentarse / Sit: the whole model drops {@link #SIT_DROP} px through the
     * model root (cosmetics anchored to the root transform follow — see the
     * RenderFix anchoring contract), legs swing forward and spread, arms rest on
     * the lap. Loops until cancelled.
     */
    private static void sit(float t, boolean reducedMotion, float[] out) {
        float env = smoothstep(t / 0.30f);
        set(out, ROOT, 0.0f, SIT_DROP * env, 0.0f, 0.0f, 0.0f, 0.0f);
        set(out, RIGHT_LEG, 0.0f, 0.0f, 0.0f, SIT_LEG * env, SIT_SPREAD * env, 0.0f);
        set(out, LEFT_LEG, 0.0f, 0.0f, 0.0f, SIT_LEG * env, -SIT_SPREAD * env, 0.0f);
        set(out, RIGHT_ARM, -SIT_ARM_OUT * env, 0.0f, 0.0f, SIT_ARM * env, 0.0f, 0.0f);
        set(out, LEFT_ARM, SIT_ARM_OUT * env, 0.0f, 0.0f, SIT_ARM * env, 0.0f, 0.0f);
        set(out, BODY, 0.0f, 0.0f, 0.0f, 0.06f * env, 0.0f, 0.0f);
        set(out, HEAD, 0.0f, 0.0f, 0.0f, -0.04f * env, 0.0f, 0.0f);
    }

    /**
     * Invocación / Invocation: both arms swing forward then up into a V (in
     * front of the torso, where they cannot reach the head), the head tips back,
     * the chest opens, and the whole model hovers with a 0.5 px bob driven
     * through the root. Loops until cancelled.
     */
    private static void invocation(float t, boolean reducedMotion, float[] out) {
        float rise = smoothstep(t / 0.55f);
        float spread = smoothstep((t - 0.45f) / 0.60f);
        float bob = reducedMotion
                ? -INVOCATION_BOB
                : -INVOCATION_BOB * (0.5f + 0.5f * (float) Math.sin(t * 1.9f));
        set(out, ROOT, 0.0f, bob * rise, 0.0f, 0.0f, 0.0f, 0.0f);
        set(out, RIGHT_ARM, -INVOCATION_SHOULDER_OUT * rise, INVOCATION_SHOULDER_UP * rise, 0.0f,
                INVOCATION_XROT * rise, 0.0f, -INVOCATION_ZROT * spread);
        set(out, LEFT_ARM, INVOCATION_SHOULDER_OUT * rise, INVOCATION_SHOULDER_UP * rise, 0.0f,
                INVOCATION_XROT * rise, 0.0f, INVOCATION_ZROT * spread);
        set(out, BODY, 0.0f, 0.0f, 0.0f, INVOCATION_BODY * rise, 0.0f, 0.0f);
        set(out, HEAD, 0.0f, 0.0f, 0.0f, INVOCATION_HEAD * rise, 0.0f, 0.0f);
    }

    // ---------------------------------------------------------------
    // Shared math
    // ---------------------------------------------------------------

    private static void set(float[] out, int part, float x, float y, float z,
            float xRot, float yRot, float zRot) {
        int i = part * STRIDE;
        out[i] = x;
        out[i + 1] = y;
        out[i + 2] = z;
        out[i + 3] = xRot;
        out[i + 4] = yRot;
        out[i + 5] = zRot;
    }

    /** Ease in over {@code in} s, hold, ease out over {@code out} s. */
    private static float envelope(float t, float in, float out, float duration) {
        return clamp01(Math.min(smoothstep(t / in), smoothstep((duration - t) / out)));
    }

    private static float smoothstep(float x) {
        float v = clamp01(x);
        return v * v * (3.0f - 2.0f * v);
    }

    private static float clamp01(float v) {
        return v < 0.0f ? 0.0f : (v > 1.0f ? 1.0f : v);
    }

    // ---------------------------------------------------------------
    // Geometry helpers (used by the verification harness only)
    // ---------------------------------------------------------------

    /** Rest pivot of a part in the model-root frame (px). */
    public static void pivot(int part, float[] dst) {
        switch (part) {
            case RIGHT_ARM -> { dst[0] = -SHOULDER_X; dst[1] = SHOULDER_Y; dst[2] = 0.0f; }
            case LEFT_ARM -> { dst[0] = SHOULDER_X; dst[1] = SHOULDER_Y; dst[2] = 0.0f; }
            case RIGHT_LEG -> { dst[0] = -1.9f; dst[1] = HIP_Y; dst[2] = 0.0f; }
            case LEFT_LEG -> { dst[0] = 1.9f; dst[1] = HIP_Y; dst[2] = 0.0f; }
            default -> { dst[0] = 0.0f; dst[1] = 0.0f; dst[2] = 0.0f; }
        }
    }

    /**
     * Maps a part-local point to the model-root frame for the pose in
     * {@code pose}, including the root translation — the same composition
     * {@code translateAndRotate} performs (translate the pivot, then Rz*Ry*Rx).
     */
    public static void toRoot(int part, float lx, float ly, float lz,
            float[] pose, float[] dst) {
        pivot(part, dst);
        int i = part * STRIDE;
        float cx = (float) Math.cos(pose[i + X_ROT]);
        float sx = (float) Math.sin(pose[i + X_ROT]);
        float cy = (float) Math.cos(pose[i + Y_ROT]);
        float sy = (float) Math.sin(pose[i + Y_ROT]);
        float cz = (float) Math.cos(pose[i + Z_ROT]);
        float sz = (float) Math.sin(pose[i + Z_ROT]);
        float y1 = ly * cx - lz * sx;
        float z1 = ly * sx + lz * cx;
        float x2 = lx * cy + z1 * sy;
        float z2 = -lx * sy + z1 * cy;
        float x3 = x2 * cz - y1 * sz;
        float y3 = x2 * sz + y1 * cz;
        int r = ROOT * STRIDE;
        dst[0] += pose[r] + pose[i + X] + x3;
        dst[1] += pose[r + 1] + pose[i + Y] + y3;
        dst[2] += pose[r + 2] + pose[i + Z] + z2;
    }

    /** Maps a model-root point into a part's local frame (inverse of {@link #toRoot}). */
    public static void toLocal(int part, float wx, float wy, float wz,
            float[] pose, float[] dst) {
        int i = part * STRIDE;
        int r = ROOT * STRIDE;
        pivot(part, dst);
        float x = wx - pose[r] - pose[i + X] - dst[0];
        float y = wy - pose[r + 1] - pose[i + Y] - dst[1];
        float z = wz - pose[r + 2] - pose[i + Z] - dst[2];
        float cx = (float) Math.cos(pose[i + X_ROT]);
        float sx = (float) Math.sin(pose[i + X_ROT]);
        float cy = (float) Math.cos(pose[i + Y_ROT]);
        float sy = (float) Math.sin(pose[i + Y_ROT]);
        float cz = (float) Math.cos(pose[i + Z_ROT]);
        float sz = (float) Math.sin(pose[i + Z_ROT]);
        // Inverse of Rz*Ry*Rx is Rz^-1 * Ry^-1 * Rx^-1, i.e. the same three
        // elementary rotations applied with negated angles in reverse order.
        float x2 = x * cz + y * sz;
        float y1 = -x * sz + y * cz;
        float z1 = z;
        float x1 = x2 * cy - z1 * sy;
        float z0 = x2 * sy + z1 * cy;
        float y0 = y1 * cx + z0 * sx;
        float z3 = -y1 * sx + z0 * cx;
        dst[0] = x1;
        dst[1] = y0;
        dst[2] = z3;
    }

    /** Local cube extents of a part: min at [0..2], max at [3..5]. */
    public static void cube(int part, float[] dst) {
        switch (part) {
            case HEAD -> {
                dst[0] = HEAD_MIN_X; dst[1] = HEAD_MIN_Y; dst[2] = HEAD_MIN_Z;
                dst[3] = HEAD_MAX_X; dst[4] = HEAD_MAX_Y; dst[5] = HEAD_MAX_Z;
            }
            case BODY -> {
                dst[0] = -4.0f; dst[1] = 0.0f; dst[2] = -2.0f;
                dst[3] = 4.0f; dst[4] = 12.0f; dst[5] = 2.0f;
            }
            case RIGHT_ARM -> {
                dst[0] = -3.0f; dst[1] = -2.0f; dst[2] = -2.0f;
                dst[3] = 1.0f; dst[4] = 10.0f; dst[5] = 2.0f;
            }
            case LEFT_ARM -> {
                dst[0] = -1.0f; dst[1] = -2.0f; dst[2] = -2.0f;
                dst[3] = 3.0f; dst[4] = 10.0f; dst[5] = 2.0f;
            }
            case RIGHT_LEG, LEFT_LEG -> {
                dst[0] = -2.0f; dst[1] = 0.0f; dst[2] = -2.0f;
                dst[3] = 2.0f; dst[4] = 12.0f; dst[5] = 2.0f;
            }
            default -> {
                dst[0] = 0.0f; dst[1] = 0.0f; dst[2] = 0.0f;
                dst[3] = 0.0f; dst[4] = 0.0f; dst[5] = 0.0f;
            }
        }
    }
}
