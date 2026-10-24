package es.spectral.menu;

/**
 * Standalone geometry harness for {@link EmotePose} — no Minecraft, no
 * dependencies, plain {@code javac}. It samples every emote of the wheel over
 * its whole duration, maps the posed parts into the model-root frame with the
 * same math {@code ModelPart#translateAndRotate} performs (translate the pivot,
 * then {@code Rz*Ry*Rx}) and asserts the properties the playtest asked for:
 *
 * <ul>
 *   <li>Saludo: the waving hand and forearm never enter the head cube, and the
 *       hand stays well outboard of the head in its own height band.</li>
 *   <li>Aplauso: the palms actually meet — the inner faces are &lt;= 1 px apart
 *       on every contact frame, and never overlap.</li>
 *   <li>Invocación: neither arm intersects the head cube, and the V opens
 *       outwards (hands end up further out than the shoulders).</li>
 *   <li>Sentarse: the whole model is lowered by the agreed 11 px through the
 *       root, hips sit above the ground plane, feet stay on it.</li>
 *   <li>Reverencia: a true hip hinge — the hip point is invariant under the
 *       pose, the head pitches no more than the torso plus the small nod, and
 *       no arm enters the torso or the head.</li>
 *   <li>Every emote blends from and back to the vanilla pose (weight 0 at both
 *       ends) and cross-fades over at least 150 ms.</li>
 * </ul>
 *
 * <pre>
 *   javac -d build/emote-test src/main/java/es/spectral/menu/EmotePose.java \
 *       tools/EmotePoseTest.java
 *   java -cp build/emote-test es.spectral.menu.EmotePoseTest
 * </pre>
 */
public final class EmotePoseTest {

    private static int checks;
    private static int failures;

    private static final float STEP = 0.01f;
    private static final float[] POSE = new float[EmotePose.PARTS * EmotePose.STRIDE];
    private static final float[] P = new float[3];
    private static final float[] CUBE_A = new float[6];
    private static final float[] CUBE_B = new float[6];

    public static void main(String[] args) {
        wave();
        clap();
        invocation();
        sit();
        bow();
        blending();
        frames();
        detectorControl();
        summary();
        System.out.printf(java.util.Locale.ROOT, "%n%d checks, %d failures%n", checks, failures);
        if (failures > 0) System.exit(1);
    }

    // ---------------------------------------------------------------
    // 1. Saludo / Wave
    // ---------------------------------------------------------------
    private static void wave() {
        float worstHand = -1.0f;      // px inside the head cube (negative = clear)
        float worstForearm = -1.0f;
        float minAbsX = Float.MAX_VALUE;
        float minY = Float.MAX_VALUE;
        float maxY = -Float.MAX_VALUE;
        // The held phase (after the fade-in, before the fade-out) is what the
        // player sees; the 250 ms cross-fade is checked separately.
        for (float t = 0.35f; t <= EmotePose.WAVE_SECONDS - 0.40f; t += STEP) {
            EmotePose.write(EmotePose.WAVE, t, false, POSE);
            worstHand = Math.max(worstHand, penetration(EmotePose.RIGHT_ARM, 6.0f, 10.0f));
            worstForearm = Math.max(worstForearm, penetration(EmotePose.RIGHT_ARM, 4.0f, 10.0f));
            EmotePose.toRoot(EmotePose.RIGHT_ARM, 0.0f, 10.0f, 0.0f, POSE, P);
            minAbsX = Math.min(minAbsX, Math.abs(P[0]));
            minY = Math.min(minY, P[1]);
            maxY = Math.max(maxY, P[1]);
        }
        ok(worstHand <= 0.0f, "wave: hand cube never enters the head (depth %.2f px)", worstHand);
        ok(worstForearm <= 0.0f, "wave: forearm above the elbow never enters the head (depth %.2f px)",
                worstForearm);
        ok(minAbsX > 4.5f, "wave: hand centre stays outboard of the head (min |x| %.2f px)", minAbsX);
        ok(minY > -9.0f && maxY > -4.0f,
                "wave: hand waves at face height beside the head, y %.2f..%.2f px", minY, maxY);
        // The whole arm must be out of the head at the held pose as well; only
        // the 250 ms cross-fade may graze the head's lower corner by < 0.5 px.
        EmotePose.write(EmotePose.WAVE, 0.9f, false, POSE);
        float heldPen = penetration(EmotePose.RIGHT_ARM, -2.0f, 10.0f);
        ok(heldPen <= 0.0f, "wave: whole arm clear of the head at the held pose (depth %.2f px)", heldPen);

        // Whole-timeline guard: through the 250 ms cross-fades the hand cube
        // must stay out of the head too; the upper arm may graze the head's
        // lower corner by less than half a pixel while it swings up.
        float fadeHand = -1.0f;
        float fadeArm = -1.0f;
        for (float t = 0.0f; t <= EmotePose.WAVE_SECONDS; t += STEP) {
            EmotePose.write(EmotePose.WAVE, t, false, POSE);
            fadeHand = Math.max(fadeHand, penetration(EmotePose.RIGHT_ARM, 6.0f, 10.0f));
            fadeArm = Math.max(fadeArm, penetration(EmotePose.RIGHT_ARM, 2.0f, 10.0f));
        }
        ok(fadeHand <= 0.0f, "wave: hand never enters the head, fades included (depth %.2f px)", fadeHand);
        ok(fadeArm < 0.5f, "wave: forearm never enters the head, fades included (depth %.2f px)", fadeArm);

        // The head keeps following the player's look direction during an emote
        // (its rotations are additive), so the raised arm must stay clear of the
        // head over a plausible look range too — hand and forearm only; the
        // upper arm at the shoulder is allowed to graze like the vanilla arms do.
        float lookWorst = -1.0f;
        for (float lookX = -0.6f; lookX <= 0.601f; lookX += 0.2f) {
            for (float lookY = -0.6f; lookY <= 0.601f; lookY += 0.3f) {
                EmotePose.write(EmotePose.WAVE, 0.9f, false, POSE);
                POSE[EmotePose.HEAD * EmotePose.STRIDE + EmotePose.X_ROT] += lookX;
                POSE[EmotePose.HEAD * EmotePose.STRIDE + EmotePose.Y_ROT] += lookY;
                lookWorst = Math.max(lookWorst, penetration(EmotePose.RIGHT_ARM, 4.0f, 10.0f));
            }
        }
        ok(lookWorst <= 0.0f,
                "wave: hand and forearm clear the head for look pitch/yaw +-0.6 rad (depth %.2f px)",
                lookWorst);

        // The arm is actually up: the hand ends up above the shoulder.
        EmotePose.write(EmotePose.WAVE, 0.9f, false, POSE);
        EmotePose.toRoot(EmotePose.RIGHT_ARM, 0.0f, 10.0f, 0.0f, POSE, P);
        ok(P[1] < EmotePose.SHOULDER_Y - 2.0f, "wave: hand raised %.1f px above the shoulder",
                EmotePose.SHOULDER_Y - P[1]);
    }

    // ---------------------------------------------------------------
    // 2. Aplauso / Clap
    // ---------------------------------------------------------------
    private static void clap() {
        float minGap = Float.MAX_VALUE;   // inner face distance, px
        float maxGap = -Float.MAX_VALUE;
        float minContactGap = Float.MAX_VALUE;
        for (float t = 0.0f; t <= EmotePose.CLAP_SECONDS; t += STEP / 4.0f) {
            EmotePose.write(EmotePose.CLAP, t, false, POSE);
            float rightInner = innerFaceX(EmotePose.RIGHT_ARM, 1.0f);
            float leftInner = innerFaceX(EmotePose.LEFT_ARM, -1.0f);
            float gap = leftInner - rightInner;
            // Only frames where the swing is at its closed extreme count as contact.
            boolean contact = gap <= 0.25f;
            if (contact) minContactGap = Math.min(minContactGap, Math.abs(gap));
            if (t > 0.4f && t < EmotePose.CLAP_SECONDS - 0.4f) {
                minGap = Math.min(minGap, gap);
                maxGap = Math.max(maxGap, gap);
            }
        }
        ok(minContactGap <= 1.0f, "clap: palms meet within 1 px (closest %.2f px)", minContactGap);
        ok(minGap >= -0.05f, "clap: palms never pass through each other (min gap %.2f px)", minGap);
        ok(maxGap >= 2.0f && maxGap <= 5.0f,
                "clap: hands open %.2f px between claps (3 px target)", maxGap);

        // Contact also happens at chest height in front of the body, not at the face.
        EmotePose.write(EmotePose.CLAP, 1.0f, false, POSE);
        EmotePose.toRoot(EmotePose.RIGHT_ARM, 0.0f, 10.0f, 0.0f, POSE, P);
        ok(P[1] > 0.0f && P[1] < 8.0f, "clap: palms at chest height (y %.2f px)", P[1]);
        ok(P[2] < -4.0f, "clap: palms in front of the torso (z %.2f px)", P[2]);
    }

    // ---------------------------------------------------------------
    // 3. Invocación / Invocation
    // ---------------------------------------------------------------
    private static void invocation() {
        float worstRight = -1.0f;
        float worstLeft = -1.0f;
        float handX = 0.0f;
        float lastBob = 0.0f;
        float minBob = Float.MAX_VALUE;
        float maxBob = -Float.MAX_VALUE;
        for (float t = 0.0f; t <= 4.0f; t += STEP) {
            EmotePose.write(EmotePose.INVOCATION, t, false, POSE);
            worstRight = Math.max(worstRight, penetration(EmotePose.RIGHT_ARM, -2.0f, 10.0f));
            worstLeft = Math.max(worstLeft, penetration(EmotePose.LEFT_ARM, -2.0f, 10.0f));
            EmotePose.toRoot(EmotePose.RIGHT_ARM, 0.0f, 10.0f, 0.0f, POSE, P);
            handX = P[0];
            lastBob = POSE[EmotePose.ROOT * EmotePose.STRIDE + EmotePose.Y];
            minBob = Math.min(minBob, lastBob);
            maxBob = Math.max(maxBob, lastBob);
        }
        // Head rotations are additive (the player still looks around), so the
        // raised arms must stay clear over a plausible look range as well.
        float lookWorst = -1.0f;
        for (float lookX = -0.6f; lookX <= 0.601f; lookX += 0.2f) {
            for (float lookY = -0.6f; lookY <= 0.601f; lookY += 0.3f) {
                EmotePose.write(EmotePose.INVOCATION, 3.0f, false, POSE);
                POSE[EmotePose.HEAD * EmotePose.STRIDE + EmotePose.X_ROT] += lookX;
                POSE[EmotePose.HEAD * EmotePose.STRIDE + EmotePose.Y_ROT] += lookY;
                lookWorst = Math.max(lookWorst, penetration(EmotePose.RIGHT_ARM, 4.0f, 10.0f));
                lookWorst = Math.max(lookWorst, penetration(EmotePose.LEFT_ARM, 4.0f, 10.0f));
            }
        }
        ok(lookWorst <= 0.0f,
                "invocation: forearms and hands clear the head over the look range (depth %.2f px)",
                lookWorst);
        ok(worstRight <= 0.0f, "invocation: right arm never enters the head (depth %.2f px)", worstRight);
        ok(worstLeft <= 0.0f, "invocation: left arm never enters the head (depth %.2f px)", worstLeft);
        ok(handX < -(EmotePose.SHOULDER_X + 2.0f),
                "invocation: arms open into a V (hand x %.2f px, shoulder %.2f)", handX, -EmotePose.SHOULDER_X);
        ok(minBob < 0.0f && maxBob - minBob <= EmotePose.INVOCATION_BOB + 0.01f,
                "invocation: hover bob %.2f..%.2f px (cap %.2f)", minBob, maxBob, EmotePose.INVOCATION_BOB);
    }

    // ---------------------------------------------------------------
    // 4. Sentarse / Sit
    // ---------------------------------------------------------------
    private static void sit() {
        EmotePose.write(EmotePose.SIT, 2.0f, false, POSE);
        float rootY = POSE[EmotePose.ROOT * EmotePose.STRIDE + EmotePose.Y];
        ok(Math.abs(rootY - EmotePose.SIT_DROP) < 0.01f,
                "sit: model lowered by %.2f px (agreed %.2f)", rootY, EmotePose.SIT_DROP);

        // Hips just above the ground, feet on it.
        float hipY = EmotePose.HIP_Y + rootY;
        ok(hipY >= EmotePose.GROUND_Y - 2.0f && hipY <= EmotePose.GROUND_Y,
                "sit: hips at y %.2f (ground %.0f)", hipY, EmotePose.GROUND_Y);
        EmotePose.toRoot(EmotePose.RIGHT_LEG, 0.0f, 12.0f, 0.0f, POSE, P);
        ok(P[1] >= EmotePose.GROUND_Y - 1.0f && P[1] <= EmotePose.GROUND_Y + 1.0f,
                "sit: foot at y %.2f (ground %.0f)", P[1], EmotePose.GROUND_Y);

        // Legs forward, not folded backwards through the body.
        EmotePose.toRoot(EmotePose.RIGHT_LEG, 0.0f, 12.0f, 0.0f, POSE, P);
        ok(P[2] < -4.0f, "sit: legs point forward (knee z %.2f px)", P[2]);
        // Arms rest outside the thighs.
        float armLeg = gap(EmotePose.RIGHT_ARM, true, EmotePose.RIGHT_LEG);
        ok(armLeg > 0.0f, "sit: arm rests clear of the thigh (gap %.2f px)", armLeg);

        // The drop is fully in place after the ramp.
        EmotePose.write(EmotePose.SIT, 0.0f, false, POSE);
        ok(Math.abs(POSE[EmotePose.ROOT * EmotePose.STRIDE + EmotePose.Y]) < 0.01f,
                "sit: starts at the vanilla height (no pop)");
    }

    // ---------------------------------------------------------------
    // 5. Reverencia / Bow
    // ---------------------------------------------------------------
    private static void bow() {
        float worstPitch = 0.0f;
        float hipDrift = 0.0f;
        float worstArmBody = -1.0f;
        float worstArmHead = -1.0f;
        for (float t = 0.0f; t <= EmotePose.BOW_SECONDS; t += STEP) {
            EmotePose.write(EmotePose.BOW, t, false, POSE);
            float pitch = POSE[EmotePose.BODY * EmotePose.STRIDE + EmotePose.X_ROT];
            float headPitch = POSE[EmotePose.HEAD * EmotePose.STRIDE + EmotePose.X_ROT];
            worstPitch = Math.max(worstPitch, pitch);
            ok(headPitch <= pitch + EmotePose.BOW_NOD + 0.001f,
                    "bow: head never bends past torso + nod (%.3f vs %.3f + %.3f)",
                    headPitch, pitch, EmotePose.BOW_NOD);
            ok(pitch <= EmotePose.BOW_PITCH + 0.001f, "bow: torso pitch stays at %.2f rad", pitch);

            // The hip point (body-local 0,12,0) must not move: that is the hinge.
            EmotePose.toRoot(EmotePose.BODY, 0.0f, 12.0f, 0.0f, POSE, P);
            hipDrift = Math.max(hipDrift, (float) Math.sqrt(P[0] * P[0]
                    + (P[1] - EmotePose.HIP_Y) * (P[1] - EmotePose.HIP_Y) + P[2] * P[2]));

            worstArmBody = Math.max(worstArmBody, penetration(EmotePose.BODY, EmotePose.RIGHT_ARM, -2.0f, 10.0f));
            worstArmHead = Math.max(worstArmHead, penetration(EmotePose.HEAD, EmotePose.RIGHT_ARM, -2.0f, 10.0f));
        }
        ok(hipDrift < 0.02f, "bow: hinges at the hip (hip drift %.3f px)", hipDrift);
        ok(worstPitch > 0.4f, "bow: torso actually bends (max pitch %.2f rad)", worstPitch);
        ok(worstArmBody <= 0.0f, "bow: arms clear the tilted torso (depth %.2f px)", worstArmBody);
        ok(worstArmHead <= 0.0f, "bow: arms clear the head (depth %.2f px)", worstArmHead);

        // Legs stay straight and planted.
        EmotePose.write(EmotePose.BOW, 1.0f, false, POSE);
        EmotePose.toRoot(EmotePose.RIGHT_LEG, 0.0f, 12.0f, 0.0f, POSE, P);
        ok(Math.abs(P[1] - EmotePose.GROUND_Y) < 0.01f, "bow: legs stay straight (foot y %.2f)", P[1]);
    }

    // ---------------------------------------------------------------
    // 6. Blending
    // ---------------------------------------------------------------
    private static void blending() {
        ok(EmotePose.BLEND_SECONDS >= 0.15f, "blend window %.2f s (>= 150 ms)", EmotePose.BLEND_SECONDS);
        for (String id : new String[]{EmotePose.WAVE, EmotePose.BOW, EmotePose.CLAP}) {
            float d = EmotePose.duration(id);
            ok(EmotePose.blendWeight(id, 0.0f) <= 0.001f, "%s: starts at the vanilla pose", id);
            ok(EmotePose.blendWeight(id, d) <= 0.001f, "%s: ends at the vanilla pose", id);
            ok(EmotePose.blendWeight(id, d * 0.5f) > 0.99f, "%s: fully blended in the middle", id);
        }
        for (String id : new String[]{EmotePose.SIT, EmotePose.INVOCATION}) {
            ok(EmotePose.loops(id), "%s: loops until cancelled", id);
            ok(EmotePose.blendWeight(id, 0.0f) <= 0.001f, "%s: starts at the vanilla pose", id);
            ok(EmotePose.blendWeight(id, 5.0f) > 0.99f, "%s: holds after fading in", id);
        }
        // Equal weights across parts: the whole body leaves the vanilla pose together.
        EmotePose.write(EmotePose.BOW, 1.0f, false, POSE);
        ok(POSE[EmotePose.BODY * EmotePose.STRIDE + EmotePose.Y] > 1.0f,
                "bow: the torso translation follows the pitch (y %.2f px)",
                POSE[EmotePose.BODY * EmotePose.STRIDE + EmotePose.Y]);
    }

    /**
     * Anti-false-negative control: the <em>old</em> wave pose (arm rotated to
     * {@code -pi + 0.45}, i.e. across the face) must be reported as penetrating
     * the head. If this ever passes, the clearance checks below are blind.
     */
    private static void detectorControl() {
        java.util.Arrays.fill(POSE, 0.0f);
        POSE[EmotePose.RIGHT_ARM * EmotePose.STRIDE + EmotePose.Z_ROT] = (float) (-Math.PI + 0.45);
        float legacy = penetration(EmotePose.RIGHT_ARM, 6.0f, 10.0f);
        ok(legacy > 0.5f, "control: the old wave pose is detected inside the head (depth %.2f px)", legacy);
        // And the vanilla rest pose must be clean.
        java.util.Arrays.fill(POSE, 0.0f);
        float rest = penetration(EmotePose.RIGHT_ARM, -2.0f, 10.0f);
        ok(rest <= 0.0f, "control: the vanilla rest pose is clear of the head (depth %.2f px)", rest);
    }

    /**
     * Sanity check on the frame math itself: {@link EmotePose#toRoot} and
     * {@link EmotePose#toLocal} must be exact inverses for every part, or the
     * clearance numbers above would be meaningless.
     */
    private static void frames() {
        String[] ids = {EmotePose.WAVE, EmotePose.BOW, EmotePose.CLAP, EmotePose.SIT, EmotePose.INVOCATION};
        float worst = 0.0f;
        for (String id : ids) {
            for (float t = 0.0f; t < 2.0f; t += 0.15f) {
                EmotePose.write(id, t, false, POSE);
                for (int part = 0; part < EmotePose.PARTS; part++) {
                    EmotePose.toRoot(part, 3.0f, -5.0f, 2.0f, POSE, P);
                    float wx = P[0];
                    float wy = P[1];
                    float wz = P[2];
                    EmotePose.toLocal(part, wx, wy, wz, POSE, P);
                    worst = Math.max(worst, Math.abs(P[0] - 3.0f));
                    worst = Math.max(worst, Math.abs(P[1] + 5.0f));
                    worst = Math.max(worst, Math.abs(P[2] - 2.0f));
                }
            }
        }
        ok(worst < 0.001f, "frame math: toLocal inverts toRoot (max error %.4f px)", worst);
    }

    /** Prints the measured numbers, so a run doubles as a pose report. */
    private static void summary() {
        System.out.println("measured poses (model px, root frame):");
        printWave();
        printClap();
        printSit();
        printBow();
        printInvocation();
    }

    private static void printWave() {
        EmotePose.write(EmotePose.WAVE, 0.9f, false, POSE);
        EmotePose.toRoot(EmotePose.RIGHT_ARM, 0.0f, 10.0f, 0.0f, POSE, P);
        System.out.printf(java.util.Locale.ROOT, "  saludo:     hand (%.1f, %.1f, %.1f), arm zRot %.2f rad +-%.2f%n",
                P[0], P[1], P[2], EmotePose.WAVE_ZROT, EmotePose.WAVE_SWAY);
    }

    private static void printClap() {
        EmotePose.write(EmotePose.CLAP, 1.0f, false, POSE);
        float right = innerFaceX(EmotePose.RIGHT_ARM, 1.0f);
        float left = innerFaceX(EmotePose.LEFT_ARM, -1.0f);
        EmotePose.toRoot(EmotePose.RIGHT_ARM, 0.0f, 10.0f, 0.0f, POSE, P);
        System.out.printf(java.util.Locale.ROOT, "  aplauso:    palms at (%.1f, %.1f, %.1f), inner faces gap %.2f px, yRot %.3f rad%n",
                P[0], P[1], P[2], Math.abs(left - right), EmotePose.CLAP_CONTACT);
    }

    private static void printSit() {
        EmotePose.write(EmotePose.SIT, 2.0f, false, POSE);
        float rootY = POSE[EmotePose.ROOT * EmotePose.STRIDE + EmotePose.Y];
        EmotePose.toRoot(EmotePose.RIGHT_LEG, 0.0f, 12.0f, 0.0f, POSE, P);
        System.out.printf(java.util.Locale.ROOT, "  sentarse:   root +%.1f px, hips y %.1f, foot (%.1f, %.1f, %.1f)%n",
                rootY, EmotePose.HIP_Y + rootY, P[0], P[1], P[2]);
    }

    private static void printBow() {
        EmotePose.write(EmotePose.BOW, 1.0f, false, POSE);
        EmotePose.toRoot(EmotePose.HEAD, 0.0f, -8.0f, 0.0f, POSE, P);
        float headTopY = P[1];
        float headTopZ = P[2];
        float pitch = POSE[EmotePose.BODY * EmotePose.STRIDE + EmotePose.X_ROT];
        System.out.printf(java.util.Locale.ROOT, "  reverencia: torso pitch %.2f rad, head top (%.1f, %.1f), hip fixed%n",
                pitch, headTopY, headTopZ);
    }

    private static void printInvocation() {
        EmotePose.write(EmotePose.INVOCATION, 3.0f, false, POSE);
        EmotePose.toRoot(EmotePose.RIGHT_ARM, 0.0f, 10.0f, 0.0f, POSE, P);
        System.out.printf(java.util.Locale.ROOT, "  invocacion: hand (%.1f, %.1f, %.1f), xRot %.2f zRot %.2f, bob %.2f px%n",
                P[0], P[1], P[2], EmotePose.INVOCATION_XROT, EmotePose.INVOCATION_ZROT,
                EmotePose.INVOCATION_BOB);
    }

    // ---------------------------------------------------------------
    // Geometry
    // ---------------------------------------------------------------

    /** Maps the local point through {@code part} into {@code head}-local space. */
    private static void intoHead(int part, float lx, float ly, float lz, float[] dst) {
        EmotePose.toRoot(part, lx, ly, lz, POSE, dst);
        EmotePose.toLocal(EmotePose.HEAD, dst[0], dst[1], dst[2], POSE, dst);
    }

    /**
     * Deepest penetration (px) of the sampled volume of {@code part} (local y
     * band yMin..yMax) into the head cube; 0 when they do not overlap.
     */
    private static float penetration(int part, float yMin, float yMax) {
        return penetration(EmotePose.HEAD, part, yMin, yMax);
    }

    private static float penetration(int box, int part, float yMin, float yMax) {
        EmotePose.cube(box, CUBE_A);
        EmotePose.cube(part, CUBE_B);
        float worst = 0.0f;
        int nx = 7;
        int ny = 9;
        int nz = 5;
        for (int i = 0; i < nx; i++) {
            float lx = CUBE_B[0] + (CUBE_B[3] - CUBE_B[0]) * i / (nx - 1.0f);
            for (int j = 0; j < ny; j++) {
                float ly = yMin + (yMax - yMin) * j / (ny - 1.0f);
                for (int k = 0; k < nz; k++) {
                    float lz = CUBE_B[2] + (CUBE_B[5] - CUBE_B[2]) * k / (nz - 1.0f);
                    EmotePose.toRoot(part, lx, ly, lz, POSE, P);
                    EmotePose.toLocal(box, P[0], P[1], P[2], POSE, P);
                    if (inside(CUBE_A, P)) {
                        worst = Math.max(worst, boxDepth(CUBE_A, P));
                    }
                }
            }
        }
        return worst;
    }

    private static boolean inside(float[] cube, float[] p) {
        return p[0] > cube[0] && p[0] < cube[3]
                && p[1] > cube[1] && p[1] < cube[4]
                && p[2] > cube[2] && p[2] < cube[5];
    }

    private static float boxDepth(float[] cube, float[] p) {
        return Math.min(
                Math.min(p[0] - cube[0], cube[3] - p[0]),
                Math.min(Math.min(p[1] - cube[1], cube[4] - p[1]),
                        Math.min(p[2] - cube[2], cube[5] - p[2])));
    }

    /** Root-frame x of an arm's palm face ({@code faceX} = +-1 in local space). */
    private static float innerFaceX(int part, float faceX) {
        EmotePose.toRoot(part, faceX, 10.0f, 0.0f, POSE, P);
        return P[0];
    }

    /**
     * Axis-aligned-free separation between two posed boxes in the root frame
     * (positive = gap between their surfaces, px).
     */
    private static float gap(int partA, boolean right, int partB) {
        float best = -Float.MAX_VALUE;
        EmotePose.cube(partA, CUBE_A);
        EmotePose.cube(partB, CUBE_B);
        float[][] axes = new float[3][];
        for (int i = 0; i < 3; i++) axes[i] = new float[3];
        // Only the vertical axis matters for the sit (arm above the thigh),
        // which keeps this dependency-free; a full SAT lives in the engine test
        // harness of the cosmetics workstream.
        EmotePose.toRoot(partA, CUBE_A[0], CUBE_A[1], CUBE_A[2], POSE, P);
        float[] aMin = {P[0], P[1], P[2]};
        EmotePose.toRoot(partA, CUBE_A[3], CUBE_A[4], CUBE_A[5], POSE, P);
        float[] aMax = {P[0], P[1], P[2]};
        EmotePose.toRoot(partB, CUBE_B[0], CUBE_B[1], CUBE_B[2], POSE, P);
        float[] bMin = {P[0], P[1], P[2]};
        EmotePose.toRoot(partB, CUBE_B[3], CUBE_B[4], CUBE_B[5], POSE, P);
        float[] bMax = {P[0], P[1], P[2]};
        for (int axis = 0; axis < 3; axis++) {
            float lo = Math.min(aMin[axis], aMax[axis]);
            float hi = Math.max(aMin[axis], aMax[axis]);
            float blo = Math.min(bMin[axis], bMax[axis]);
            float bhi = Math.max(bMin[axis], bMax[axis]);
            best = Math.max(best, Math.max(blo - hi, lo - bhi));
        }
        return best;
    }

    // ---------------------------------------------------------------

    private static void ok(boolean condition, String format, Object... args) {
        checks++;
        if (!condition) {
            failures++;
            System.out.println("FAIL " + String.format(java.util.Locale.ROOT, format, args));
        }
    }
}
