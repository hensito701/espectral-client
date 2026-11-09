package es.spectral.menu.cosmetics.render;

/**
 * Standalone harness for {@link MotionDriver} — no Minecraft, no dependencies,
 * plain {@code javac}. It replays each driver exactly the way
 * {@code WearerSims} steps it (one call per client tick) and asserts the
 * behaviour {@code docs/cosmetics-pipeline.md} promises:
 *
 * <ul>
 *   <li><b>swing</b> never leaves {@code [min, max]} — not under a huge speed
 *       or yaw kick — and settles back to {@code rest} once the wearer is
 *       still.</li>
 *   <li><b>react</b> smooths to its declared offset while a state holds, and
 *       back to zero when it ends.</li>
 *   <li><b>flap</b> follows
 *       {@code closed + (open − closed)·amount + beat·sin(t·beat_speed)·amount}:
 *       fully open while gliding, half while sprinting, folded while sneaking.</li>
 *   <li><b>head_turn</b> decays to zero after a turn, and the impulse can never
 *       leave {@code ±max}.</li>
 *   <li><b>land</b> fires the impulse on the landing tick and decays after.</li>
 * </ul>
 *
 * <pre>
 *   javac -d build/motion-test \
 *       src/main/java/es/spectral/menu/cosmetics/render/MotionDriver.java \
 *       tools/MotionDriverTest.java
 *   java -cp build/motion-test es.spectral.menu.cosmetics.render.MotionDriverTest
 * </pre>
 */
public final class MotionDriverTest {

    private static int checks;
    private static int failures;

    private static final MotionDriver.Input IN = new MotionDriver.Input();

    public static void main(String[] args) {
        swingBounds();
        swingSettles();
        reactSmoothing();
        flapFormula();
        headTurnDecay();
        landImpulse();
        summary();
    }

    // ---------------------------------------------------------------
    // swing
    // ---------------------------------------------------------------

    /** Any speed, fall or yaw kick must stay inside the declared clamp. */
    private static void swingBounds() {
        MotionDriver.Spec spec = swing(10f, -5f, 70f);
        MotionDriver.State state = new MotionDriver.State();
        MotionDriver.neutral(spec, state);

        float worstLow = Float.MAX_VALUE, worstHigh = -Float.MAX_VALUE;
        for (int tick = 0; tick < 2000; tick++) {
            reset();
            // Alternate violent extremes so both ends of the clamp are probed.
            IN.speed = tick % 2 == 0 ? 12f : -12f;
            IN.fallSpeed = tick % 3 == 0 ? 6f : 0f;
            IN.yawDelta = tick % 5 == 0 ? 180f : -90f;
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
            worstLow = Math.min(worstLow, state.angle);
            worstHigh = Math.max(worstHigh, state.angle);
        }
        check("swing never drops below min", worstLow >= -5f - 1e-4f, "min reached " + worstLow);
        check("swing never rises above max", worstHigh <= 70f + 1e-4f, "max reached " + worstHigh);
    }

    /** Stillness must settle the spring back onto {@code rest}. */
    private static void swingSettles() {
        MotionDriver.Spec spec = swing(10f, -5f, 70f);
        MotionDriver.State state = new MotionDriver.State();
        MotionDriver.neutral(spec, state);
        // Start displaced, then hold still.
        state.angle = 55f;
        for (int tick = 0; tick < 400; tick++) {
            reset();
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
        }
        check("swing settles to rest when still", Math.abs(state.angle - 10f) < 0.1f,
                "angle " + state.angle);
        check("swing velocity decays to zero", Math.abs(state.vel) < 0.01f, "vel " + state.vel);
    }

    private static MotionDriver.Spec swing(float rest, float min, float max) {
        MotionDriver.Spec spec = new MotionDriver.Spec();
        spec.type = MotionDriver.TYPE_SWING;
        spec.axis = MotionDriver.AXIS_X;
        spec.rest = rest;
        spec.min = min;
        spec.max = max;
        spec.stiffness = 0.18f;
        spec.damping = 0.82f;
        spec.drag = 40f;
        spec.fall = 25f;
        spec.turn = 0.6f;
        spec.sway = 0f;
        return spec;
    }

    // ---------------------------------------------------------------
    // react
    // ---------------------------------------------------------------

    private static void reactSmoothing() {
        MotionDriver.Spec spec = new MotionDriver.Spec();
        spec.type = MotionDriver.TYPE_REACT;
        spec.rate = 0.5f;
        spec.sprint[0] = 10f;
        spec.sneak[2] = -6f;
        MotionDriver.State state = new MotionDriver.State();
        MotionDriver.neutral(spec, state);

        float first = -1f;
        for (int tick = 0; tick < 40; tick++) {
            reset();
            IN.sprint = true;
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
            if (tick == 0) first = state.rx;
        }
        check("react reaches the sprint offset", Math.abs(state.rx - 10f) < 1e-3f, "rx " + state.rx);
        check("react smooths rather than snapping", first > 0f && first < 10f, "first step " + first);
        check("react leaves other axes alone", Math.abs(state.rz) < 1e-4f, "rz " + state.rz);

        for (int tick = 0; tick < 60; tick++) {
            reset();
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
        }
        check("react returns to zero when the state ends", Math.abs(state.rx) < 1e-3f, "rx " + state.rx);

        for (int tick = 0; tick < 40; tick++) {
            reset();
            IN.sneak = true;
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
        }
        check("react applies the sneak offset", Math.abs(state.rz + 6f) < 1e-3f, "rz " + state.rz);
    }

    // ---------------------------------------------------------------
    // flap
    // ---------------------------------------------------------------

    private static void flapFormula() {
        MotionDriver.Spec spec = new MotionDriver.Spec();
        spec.type = MotionDriver.TYPE_FLAP;
        spec.axis = MotionDriver.AXIS_Z;
        spec.closed = 5f;
        spec.open = 40f;
        spec.beat = 15f;
        spec.beatSpeed = 0.5f;
        MotionDriver.State state = new MotionDriver.State();
        MotionDriver.neutral(spec, state);

        // Gliding opens fully; sample the beat only once the easing settled
        // (the first ticks are still ramping up from the closed pose).
        float peak = -Float.MAX_VALUE, trough = Float.MAX_VALUE;
        for (int tick = 0; tick < 200; tick++) {
            reset();
            IN.glide = true;
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
            if (state.amount >= 1f) {
                peak = Math.max(peak, state.angle);
                trough = Math.min(trough, state.angle);
            }
        }
        check("flap opens fully while gliding", Math.abs(state.amount - 1f) < 1e-4f, "amount " + state.amount);
        check("flap beats around the open angle",
                Math.abs(peak - (40f + 15f)) < 0.2f && Math.abs(trough - (40f - 15f)) < 0.2f,
                "peak " + peak + ", trough " + trough);

        // Sprinting holds it half open.
        for (int tick = 0; tick < 200; tick++) {
            reset();
            IN.sprint = true;
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
        }
        check("flap is half open while sprinting", Math.abs(state.amount - 0.5f) < 1e-3f,
                "amount " + state.amount);

        // Sneaking folds it.
        for (int tick = 0; tick < 200; tick++) {
            reset();
            IN.sneak = true;
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
        }
        check("flap folds while sneaking", state.amount == 0f, "amount " + state.amount);
        check("flap folded sits on closed", Math.abs(state.angle - 5f) < 1e-4f, "angle " + state.angle);
    }

    // ---------------------------------------------------------------
    // head_turn / land
    // ---------------------------------------------------------------

    private static void headTurnDecay() {
        MotionDriver.Spec spec = new MotionDriver.Spec();
        spec.type = MotionDriver.TYPE_HEAD_TURN;
        spec.axis = MotionDriver.AXIS_X;
        spec.gain = 0.5f;
        spec.maxAngle = 20f;
        spec.decay = 0.8f;
        MotionDriver.State state = new MotionDriver.State();
        MotionDriver.neutral(spec, state);

        reset();
        IN.headYawDelta = 400f; // absurd turn: the clamp must hold
        MotionDriver.step(spec, state, IN);
        check("head_turn honours max", Math.abs(state.angle) <= 20f + 1e-4f, "angle " + state.angle);

        float previous = state.angle;
        boolean monotonic = true;
        for (int tick = 0; tick < 100; tick++) {
            reset();
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
            if (Math.abs(state.angle) > Math.abs(previous)) monotonic = false;
            previous = state.angle;
        }
        check("head_turn decays monotonically", monotonic, "angle " + state.angle);
        check("head_turn reaches rest", Math.abs(state.angle) < 0.01f, "angle " + state.angle);
    }

    private static void landImpulse() {
        MotionDriver.Spec spec = new MotionDriver.Spec();
        spec.type = MotionDriver.TYPE_LAND;
        spec.axis = MotionDriver.AXIS_X;
        spec.impulse = 12f;
        spec.decay = 0.82f;
        MotionDriver.State state = new MotionDriver.State();
        MotionDriver.neutral(spec, state);

        reset();
        IN.landing = true;
        MotionDriver.step(spec, state, IN);
        check("land fires its full impulse", Math.abs(state.angle - 12f) < 1e-4f, "angle " + state.angle);

        reset();
        MotionDriver.step(spec, state, IN);
        check("land decays one tick later", Math.abs(state.angle - 12f * 0.82f) < 1e-4f, "angle " + state.angle);

        for (int tick = 0; tick < 80; tick++) {
            reset();
            IN.ageInTicks = tick;
            MotionDriver.step(spec, state, IN);
        }
        check("land returns to rest", Math.abs(state.angle) < 0.01f, "angle " + state.angle);
    }

    // ---------------------------------------------------------------
    // helpers
    // ---------------------------------------------------------------

    private static void reset() {
        IN.sneak = false;
        IN.sprint = false;
        IN.airborne = false;
        IN.glide = false;
        IN.swim = false;
        IN.speed = 0f;
        IN.fallSpeed = 0f;
        IN.yawDelta = 0f;
        IN.headYawDelta = 0f;
        IN.landing = false;
        IN.ageInTicks = 0f;
    }

    private static void check(String what, boolean ok, String detail) {
        checks++;
        if (!ok) {
            failures++;
            System.out.println("FAIL " + what + " (" + detail + ")");
        }
    }

    private static void summary() {
        System.out.printf(java.util.Locale.ROOT, "%nMotionDriver: %d checks, %d failures%n", checks, failures);
        if (failures > 0) System.exit(1);
    }
}
