package es.spectral.menu.cosmetics.render;

/**
 * Motion drivers: the Minecraft-free math behind every animated cosmetic bone
 * (contract: {@code docs/cosmetics-pipeline.md}, "Motion drivers").
 *
 * <p>One {@link Spec} is the parsed metadata for a single driver, one
 * {@link State} is the live value a wearer carries, and {@link #step} advances
 * it exactly one client tick. Nothing here touches Minecraft, so
 * {@code branding-mod/tools/MotionDriverTest.java} compiles this file alone
 * with plain {@code javac}.
 *
 * <p><b>Inputs.</b> Only publicly visible pose state, as the contract
 * demands: sneaking, sprinting, airborne, vertical speed, gliding, swimming,
 * horizontal speed in the body frame, body/head yaw change, landing and time.
 * Every driver rests at {@link #neutral} when cosmetics physics is off, under
 * reduced motion, or when the wearer is outside the simulation set.
 */
public final class MotionDriver {

    /** Rotation axis of a single-axis driver (the bone's own axes). */
    public static final byte AXIS_X = 0;
    public static final byte AXIS_Y = 1;
    public static final byte AXIS_Z = 2;

    public static final byte TYPE_SWING = 0;
    public static final byte TYPE_REACT = 1;
    public static final byte TYPE_HEAD_TURN = 2;
    public static final byte TYPE_LAND = 3;
    public static final byte TYPE_FLAP = 4;

    /** Contract driver names, index = {@code TYPE_*}. */
    private static final String[] NAMES = {"swing", "react", "head_turn", "land", "flap"};

    /** Idle sway angular rate (rad per tick): one slow breath, ~1.3 s period. */
    static final float SWAY_RATE = 0.08f;
    /** Horizontal speed (blocks/tick) at which idle sway has fully faded. */
    static final float SWAY_SPEED = 0.08f;
    /** Flap open-amount easing per tick. */
    static final float FLAP_RATE = 0.2f;
    /** Downward speed (blocks/tick) at which the wings count as open. */
    static final float FALLING_SPEED = 0.12f;

    private MotionDriver() {}

    // ---------------------------------------------------------------
    // Spec (parsed metadata, written once at load)
    // ---------------------------------------------------------------

    /** One driver declaration for one bone. Fields default to neutral values. */
    public static final class Spec {

        public byte type;
        public byte axis = AXIS_X;

        // swing
        public float rest, min, max;
        public float stiffness = 0.18f;
        public float damping = 0.82f;
        public float drag, fall, turn, sway;

        // react: one offset per state, in degrees
        public final float[] sneak = new float[3];
        public final float[] sprint = new float[3];
        public final float[] airborne = new float[3];
        public final float[] glide = new float[3];
        public final float[] swim = new float[3];
        public float rate = 0.2f;

        // head_turn
        public float gain, maxAngle = 30f, decay = 0.8f;

        // land
        public float impulse;

        // flap
        public float closed, open, beat, beatSpeed = 0.3f;
    }

    /** Live value of one driver for one wearer. */
    public static final class State {
        /** Swing / head_turn / land / flap angle (degrees). */
        public float angle;
        /** Swing angular velocity (degrees per tick). */
        public float vel;
        /** React offset per axis (degrees). */
        public float rx, ry, rz;
        /** Flap open amount 0..1. */
        public float amount;
    }

    /** Per-tick inputs, one reused instance per wearer (never allocated later). */
    public static final class Input {
        public boolean sneak, sprint, airborne, glide, swim;
        /** Signed forward speed in the body frame, blocks/tick. */
        public float speed;
        /** Downward speed, blocks/tick (0 when not falling). */
        public float fallSpeed;
        /** Body yaw change this tick, degrees. */
        public float yawDelta;
        /** Head yaw change this tick, degrees. */
        public float headYawDelta;
        /** This tick ended a fall. */
        public boolean landing;
        /** Wearer time in ticks (beat phase, idle sway). */
        public float ageInTicks;
    }

    // ---------------------------------------------------------------
    // Parsing helpers
    // ---------------------------------------------------------------

    /** Driver type index for a contract name, or -1 when unknown. */
    public static int typeOf(String name) {
        if (name == null) return -1;
        for (int i = 0; i < NAMES.length; i++) {
            if (NAMES[i].equals(name)) return i;
        }
        return -1;
    }

    /** Axis index for {@code x}/{@code y}/{@code z}, default {@link #AXIS_X}. */
    public static byte axisOf(String name) {
        if ("y".equals(name)) return AXIS_Y;
        if ("z".equals(name)) return AXIS_Z;
        return AXIS_X;
    }

    // ---------------------------------------------------------------
    // Stepping
    // ---------------------------------------------------------------

    /** Advances {@code st} one client tick from {@code in}. */
    public static void step(Spec s, State st, Input in) {
        switch (s.type) {
            case TYPE_SWING -> stepSwing(s, st, in);
            case TYPE_REACT -> stepReact(s, st, in);
            case TYPE_HEAD_TURN -> stepHeadTurn(s, st, in);
            case TYPE_LAND -> stepLand(s, st, in);
            case TYPE_FLAP -> stepFlap(s, st, in);
            default -> { }
        }
    }

    private static void stepSwing(Spec s, State st, Input in) {
        float target = s.rest + s.drag * in.speed + s.fall * in.fallSpeed + idleSway(s, in);
        st.vel += (target - st.angle) * s.stiffness;
        st.vel *= s.damping;
        // A yaw change kicks the spring; the damping settles it a few ticks
        // later, which is what makes a tail trail a turn instead of snapping.
        st.vel += s.turn * in.yawDelta;
        st.angle += st.vel;
        float lo = Math.min(s.min, s.max), hi = Math.max(s.min, s.max);
        if (st.angle < lo) {
            st.angle = lo;
            if (st.vel < 0f) st.vel = 0f;
        } else if (st.angle > hi) {
            st.angle = hi;
            if (st.vel > 0f) st.vel = 0f;
        }
    }

    /**
     * Idle sway: {@code sway} is the idle amplitude, so it fades out as the
     * wearer starts moving (wind already dominates the target by then).
     */
    private static float idleSway(Spec s, Input in) {
        if (s.sway == 0f) return 0f;
        float idle = 1f - Math.min(1f, Math.abs(in.speed) / SWAY_SPEED);
        if (idle <= 0f) return 0f;
        return s.sway * (float) Math.sin(in.ageInTicks * SWAY_RATE) * idle;
    }

    private static void stepReact(Spec s, State st, Input in) {
        float tx = 0f, ty = 0f, tz = 0f;
        if (in.sneak) { tx += s.sneak[0]; ty += s.sneak[1]; tz += s.sneak[2]; }
        if (in.sprint) { tx += s.sprint[0]; ty += s.sprint[1]; tz += s.sprint[2]; }
        if (in.airborne) { tx += s.airborne[0]; ty += s.airborne[1]; tz += s.airborne[2]; }
        if (in.glide) { tx += s.glide[0]; ty += s.glide[1]; tz += s.glide[2]; }
        if (in.swim) { tx += s.swim[0]; ty += s.swim[1]; tz += s.swim[2]; }
        float r = Math.min(1f, Math.max(0.001f, s.rate));
        st.rx += (tx - st.rx) * r;
        st.ry += (ty - st.ry) * r;
        st.rz += (tz - st.rz) * r;
    }

    private static void stepHeadTurn(Spec s, State st, Input in) {
        float limit = Math.abs(s.maxAngle);
        st.angle = st.angle * s.decay + s.gain * in.headYawDelta;
        if (st.angle > limit) st.angle = limit;
        else if (st.angle < -limit) st.angle = -limit;
    }

    private static void stepLand(Spec s, State st, Input in) {
        // The impulse lands once and the decay flops it back to rest. Decay
        // runs first so the tick it fires shows the full angle.
        st.angle *= s.decay;
        if (in.landing) st.angle += s.impulse;
    }

    private static void stepFlap(Spec s, State st, Input in) {
        float target;
        if (in.sneak) target = 0f;
        else if (in.glide || in.fallSpeed >= FALLING_SPEED) target = 1f;
        else if (in.sprint) target = 0.5f;
        else target = 0f;
        st.amount += (target - st.amount) * FLAP_RATE;
        if (st.amount < 0.0001f) st.amount = 0f;
        else if (st.amount > 0.9999f) st.amount = 1f;
        float base = s.closed + (s.open - s.closed) * st.amount;
        float beat = s.beat * (float) Math.sin(in.ageInTicks * s.beatSpeed) * st.amount;
        st.angle = base + beat;
    }

    /** Restores the driver's neutral (static) pose. */
    public static void neutral(Spec s, State st) {
        st.vel = 0f;
        st.rx = st.ry = st.rz = 0f;
        st.amount = 0f;
        st.angle = s.type == TYPE_SWING ? s.rest : s.type == TYPE_FLAP ? s.closed : 0f;
    }

    /**
     * Adds this driver's contribution (radians) to {@code out[0..2]} =
     * x/y/z rotation of the bone. A react driver can move all three axes; the
     * rest move only their own.
     */
    public static void accumulate(Spec s, State st, float[] out) {
        float rx = 0f, ry = 0f, rz = 0f;
        if (s.type == TYPE_REACT) {
            rx = st.rx;
            ry = st.ry;
            rz = st.rz;
        } else {
            switch (s.axis) {
                case AXIS_Y -> ry = st.angle;
                case AXIS_Z -> rz = st.angle;
                default -> rx = st.angle;
            }
        }
        out[0] = (float) Math.toRadians(rx);
        out[1] = (float) Math.toRadians(ry);
        out[2] = (float) Math.toRadians(rz);
    }
}
