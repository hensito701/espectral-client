package es.spectral.menu;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.util.Mth;

import es.spectral.menu.cosmetics.Cosmetics;
import es.spectral.menu.ui.EmoteWheelScreen;

/**
 * Local-only emotes (MVP cosmetics). The wearer picks one of the five catalog
 * emotes from the {@link EmoteWheelScreen} radial wheel (default {@code B});
 * the pose is applied to the <b>local player's model only</b> at TAIL of
 * {@code PlayerModel#setupAnim(AvatarRenderState)} — the same "pose after
 * setupAnim" point the 3D skin feature uses, which matters because the
 * deferred render pipeline re-runs {@code setupAnim} at draw time
 * ({@code ModelFeatureRenderer#prepareModel}), so a pose injected inside
 * {@code submit} would be overwritten before rasterisation.
 *
 * <p>Only the wearer sees the emote (e.g. in third person): the pose is
 * gated on {@code state.id == player.getId()}, never touches other avatars,
 * and no packet is sent. Cancellation: any movement input, attack, damage
 * ({@code hurtTime} increase) or a screen opening stops it — this engine
 * owns the wheel screen lifecycle so the wheel itself cannot cancel an
 * emote. With {@code personalization.reduced_motion} the same emote renders
 * as its held (static) pose with all sine/bob motion frozen.</p>
 *
 * <p>Gate: {@link Cosmetics#ownMvp()} AND {@link Cosmetics#emotesEnabled()}
 * (the {@code emotes} flag of {@code espectral-cosmetics.json}). The dev
 * property {@code -Despectral.emotes.preview=true} forces the MVP half of
 * the gate in {@link FabricLoader#isDevelopmentEnvironment() development}
 * environments only, so dev builds can preview poses without a grant.</p>
 */
public final class EmoteEngine {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-emotes");

    /** Dev-only property: forces {@link Cosmetics#ownMvp()} true (never in prod). */
    public static final String PREVIEW_PROPERTY = "espectral.emotes.preview";

    /** Emote catalog ids in wheel order (catalog: cosmetics-catalog.json). */
    public static final String[] EMOTES = {
            "saludo", "reverencia", "aplauso", "sentarse", "invocacion"};

    /** Duration (seconds) of the one-shot emotes; loops run until cancelled. */
    private static final double SALUDO_SECONDS = 1.8;
    private static final double REVERENCIA_SECONDS = 2.4;
    private static final double APLAUSO_SECONDS = 3.2;

    private static final EmoteEngine INSTANCE = new EmoteEngine();

    /** Active emote id, or {@code null}. Render thread reads it via {@link #pose}. */
    private volatile String active;
    private volatile long startedMillis;
    /** Last observed hurtTime, so a damage tick cancels the emote. */
    private int lastHurtTime;
    /**
     * Set while {@code sentarse} offsets {@code root().y}: vanilla
     * {@code setupAnim} rewrites rotations but never translations, and the
     * deferred draw path does not call {@code resetPose()}, so the flag
     * makes {@link #pose} restore the root pose exactly once when the emote
     * ends instead of the offset sticking forever.
     */
    private boolean rootTouched;
    /** Wall-clock when the wheel last closed; suppresses click re-opens. */
    private long wheelClosedAt;

    private EmoteEngine() {}

    public static EmoteEngine getInstance() {
        return INSTANCE;
    }

    /** Active emote id for the local player, or {@code null}. */
    public String active() {
        return active;
    }

    /** Called by the wheel when it closes; stamps the re-open suppression. */
    public void noteWheelClosed() {
        this.wheelClosedAt = System.currentTimeMillis();
    }

    /**
     * MVP gate + cosmetics emotes flag, plus the dev preview override.
     * {@code PREVIEW_PROPERTY} only counts when the game runs inside a
     * development environment; it forces the ownMvp() half of the gate so
     * QA can pose without an identity grant.
     */
    public static boolean canEmote() {
        boolean mvp = Cosmetics.ownMvp();
        if (!mvp && FabricLoader.getInstance().isDevelopmentEnvironment()
                && Boolean.getBoolean(PREVIEW_PROPERTY)) {
            mvp = true;
        }
        return mvp && Cosmetics.emotesEnabled();
    }

    /**
     * Per-client-tick driver, called from {@code MinecraftMixin}: keybind
     * polling (consumeClick toggles the wheel), then cancellation checks for
     * the active emote.
     */
    public void onClientTick(Minecraft minecraft) {
        if (EspectralClient.EMOTE_KEY == null) return;

        Screen current = Compat.getScreen(minecraft);
        if (EspectralClient.EMOTE_KEY.consumeClick()) {
            if (current instanceof EmoteWheelScreen wheel) {
                wheel.onClose();
            } else if (minecraft.player != null && current == null
                    && System.currentTimeMillis() - wheelClosedAt > 250) {
                if (canEmote()) {
                    Compat.open(minecraft, new EmoteWheelScreen());
                } else {
                    // Silent by design: the key is listed in Controls for
                    // everyone, but the cosmetic line requires MVP.
                    LOGGER.debug("Emote wheel suppressed: emotes not unlocked");
                }
            }
        }

        String id = active;
        if (id == null) {
            if (minecraft.player != null) this.lastHurtTime = minecraft.player.hurtTime;
            return;
        }
        if (stopCheck(minecraft, current)) {
            stop();
        } else if (minecraft.player != null) {
            this.lastHurtTime = minecraft.player.hurtTime;
        }
    }

    /** Starts an emote; called by the wheel on selection. */
    public void start(String id) {
        this.active = id;
        this.startedMillis = System.currentTimeMillis();
        Minecraft minecraft = Minecraft.getInstance();
        this.lastHurtTime = minecraft.player != null ? minecraft.player.hurtTime : 0;
    }

    /** Stops the active emote immediately (idempotent). */
    public void stop() {
        this.active = null;
    }

    /**
     * True when the emote must cancel this tick: movement, jump, sneak,
     * attack or use input, damage taken (hurtTime grew), a screen other than
     * the wheel being open, spectator/death, or no world.
     */
    private boolean stopCheck(Minecraft minecraft, Screen current) {
        LocalPlayer player = minecraft.player;
        if (player == null || minecraft.level == null) return true;
        if (player.isSpectator() || player.isDeadOrDying() || player.isRemoved()) return true;
        if (player.hurtTime > this.lastHurtTime) return true;
        if (current != null && !(current instanceof EmoteWheelScreen)) return true;
        Options options = minecraft.options;
        return options.keyUp.isDown() || options.keyDown.isDown()
                || options.keyLeft.isDown() || options.keyRight.isDown()
                || options.keyJump.isDown() || options.keyShift.isDown()
                || options.keyAttack.isDown() || options.keyUse.isDown();
    }

    /**
     * Poses the player model for the active emote. Called from the
     * {@code PlayerModel#setupAnim(AvatarRenderState)} TAIL injection — both
     * the {@code submit}-time call and the draw-time call inside
     * {@code prepareModel} reach it, so layers and the deferred node see the
     * same pose. Only the local player's own render state poses.
     *
     * <p>Must stay allocation-free: this runs once per rendered avatar.</p>
     */
    public void pose(PlayerModel model, AvatarRenderState state) {
        String id = active;
        Minecraft minecraft = Minecraft.getInstance();
        LocalPlayer player = minecraft.player;
        boolean own = player != null && !state.isSpectator && state.id == player.getId();
        if (!own) return;
        // Root translation isn't rewritten by setupAnim and prepareModel
        // never calls resetPose(): undo the sit offset exactly once.
        if (rootTouched) {
            model.root().resetPose();
            rootTouched = false;
        }
        if (id == null || !canEmote()) return;

        float t = (float) ((System.currentTimeMillis() - startedMillis) / 1000.0);
        boolean still = Cosmetics.reducedMotion();
        switch (id) {
            case "saludo" -> poseWave(model, t, still);
            case "reverencia" -> poseBow(model, t, still);
            case "aplauso" -> poseClap(model, t, still);
            case "sentarse" -> { poseSit(model, still); rootTouched = true; }
            case "invocacion" -> poseInvocation(model, t, still);
            default -> { return; }
        }

        if (("saludo".equals(id) && t > SALUDO_SECONDS)
                || ("reverencia".equals(id) && t > REVERENCIA_SECONDS)
                || ("aplauso".equals(id) && t > APLAUSO_SECONDS)) {
            stop();
        }
    }

    /**
     * Saludo / Wave: right arm raised overhead, swaying side to side; slight
     * head tilt. Reduced motion holds the arm up with a fixed tilt.
     */
    private static void poseWave(PlayerModel m, float t, boolean still) {
        float sway = still ? 0.0f : Mth.sin(t * 7.0f) * 0.35f;
        m.rightArm.xRot = 0.0f;
        m.rightArm.zRot = -(float) Math.PI + 0.45f + sway;
        m.rightArm.yRot = 0.0f;
        m.head.zRot = still ? 0.0f : Mth.sin(t * 3.5f) * 0.06f;
        m.head.xRot = -0.15f;
    }

    /**
     * Reverencia / Bow: torso folds forward (head + arms follow), eases in,
     * holds, releases. Reduced motion holds the deepest point.
     */
    private static void poseBow(PlayerModel m, float t, boolean still) {
        float phase;
        if (still) {
            phase = 1.0f;
        } else if (t < 0.4f) {
            phase = t / 0.4f;                       // bend in
        } else if (t < 1.6f) {
            phase = 1.0f;                           // hold
        } else {
            phase = Math.max(0.0f, 1.0f - (t - 1.6f) / 0.6f); // release
        }
        phase = phase * phase * (3.0f - 2.0f * phase); // smoothstep
        float bend = 0.9f * phase;
        m.body.xRot = bend;
        m.head.xRot = bend * 0.7f;
        m.leftArm.xRot = bend;
        m.rightArm.xRot = bend;
        m.leftLeg.xRot = -bend * 0.3f;
        m.rightLeg.xRot = -bend * 0.3f;
    }

    /**
     * Aplauso / Clap: both arms raised forward at shoulder height, swinging
     * together/apart so the hands meet. Reduced motion holds them apart.
     */
    private static void poseClap(PlayerModel m, float t, boolean still) {
        float gap = still ? 0.35f : 0.18f + 0.30f * (0.5f + 0.5f * Mth.sin(t * 9.0f));
        m.leftArm.xRot = -1.45f;
        m.rightArm.xRot = -1.45f;
        m.leftArm.yRot = -0.55f + gap;
        m.rightArm.yRot = 0.55f - gap;
        m.leftArm.zRot = 0.15f;
        m.rightArm.zRot = -0.15f;
    }

    /**
     * Sentarse / Sit: whole model drops 10 px (model space +y is down-world
     * because submit applies scale(-1,-1,1) + translate(0,-1.501,0)), legs
     * fold forward horizontally. Loops until cancelled.
     */
    private static void poseSit(PlayerModel m, boolean still) {
        m.root().y = 10.0f;
        m.leftLeg.xRot = -1.5f;
        m.rightLeg.xRot = -1.5f;
        m.leftArm.xRot = -0.35f;
        m.rightArm.xRot = -0.35f;
    }

    /**
     * Invocación / Invocation: both arms straight up in a V, head tipped
     * back, a slow bob. Reduced motion holds the V without the bob.
     */
    private static void poseInvocation(PlayerModel m, float t, boolean still) {
        float bob = still ? 0.0f : Mth.sin(t * 2.4f) * 0.06f;
        m.leftArm.zRot = (float) Math.PI - 0.55f;
        m.rightArm.zRot = -((float) Math.PI - 0.55f);
        m.leftArm.xRot = 0.0f;
        m.rightArm.xRot = 0.0f;
        m.leftArm.yRot = -0.15f;
        m.rightArm.yRot = 0.15f;
        m.head.xRot = -0.35f + bob;
        m.body.xRot = -0.06f + bob;
    }
}
