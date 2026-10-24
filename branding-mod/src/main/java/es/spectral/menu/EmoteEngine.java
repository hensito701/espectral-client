package es.spectral.menu;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;

import es.spectral.menu.cosmetics.Cosmetics;
import es.spectral.menu.ui.EmoteWheelScreen;

/**
 * Local-only emotes (MVP cosmetics). The wearer picks one of the five catalog
 * emotes from the {@link EmoteWheelScreen} radial wheel (default {@code B});
 * the pose is applied to the <b>local player's model only</b> at TAIL of
 * {@code PlayerModel#setupAnim(AvatarRenderState)}.
 *
 * <p>The actual angles live in {@link EmotePose} (Minecraft-free, unit-tested
 * by {@code tools/EmotePoseTest.java}); this class only decides <em>whether</em>
 * an emote runs, samples it, and blends the result onto the model. Because
 * {@code Model#setupAnim} calls {@code resetPose()} and the humanoid setup
 * rebuilds the vanilla pose every time — including the draw-time call inside
 * {@code ModelFeatureRenderer#prepareModel} — the values read at TAIL are
 * always the vanilla pose, so a lerp against them is a clean cross-fade in
 * both directions ({@link EmotePose#BLEND_SECONDS}). Root translation is reset
 * by that same {@code resetPose()}, so the sit/invocation offsets simply
 * disappear with the emote.</p>
 *
 * <p>Only the wearer sees the emote (e.g. in third person): the pose is gated
 * on {@code state.id == player.getId()}, never touches other avatars, and no
 * packet is sent. Cancellation: any movement input, jump/sneak, attack, use,
 * damage ({@code hurtTime} increase) or a screen opening stops it — this engine
 * owns the wheel screen lifecycle so the wheel itself cannot cancel an emote.
 * With {@code personalization.reduced_motion} the same emote renders as its
 * held pose with every sine/bob motion frozen.</p>
 *
 * <p>Worn cosmetics are drawn by {@code CosmeticLayer} from the real
 * {@code ModelPart} transforms (head/body/arms/root), so ears, cloak and wraps
 * follow a bow, a sit and a wave without any extra plumbing.</p>
 *
 * <p>Gate: {@link Cosmetics#ownMvp()} AND {@link Cosmetics#emotesEnabled()}
 * (the {@code emotes} flag of {@code espectral-cosmetics.json}). The dev
 * property {@code -Despectral.emotes.preview=true} forces the MVP half of the
 * gate in {@link FabricLoader#isDevelopmentEnvironment() development}
 * environments only, so dev builds can preview poses without a grant.</p>
 */
public final class EmoteEngine {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-emotes");

    /** Dev-only property: forces {@link Cosmetics#ownMvp()} true (never in prod). */
    public static final String PREVIEW_PROPERTY = "espectral.emotes.preview";

    /** Emote catalog ids in wheel order (catalog: cosmetics-catalog.json). */
    public static final String[] EMOTES = {
            EmotePose.WAVE, EmotePose.BOW, EmotePose.CLAP, EmotePose.SIT, EmotePose.INVOCATION};

    private static final EmoteEngine INSTANCE = new EmoteEngine();

    /** Reusable pose scratch — never allocated while rendering. */
    private final float[] pose = new float[EmotePose.PARTS * EmotePose.STRIDE];

    /** Active emote id, or {@code null}. Render thread reads it via {@link #pose}. */
    private volatile String active;
    private volatile long startedMillis;
    /** Last observed hurtTime, so a damage tick cancels the emote. */
    private int lastHurtTime;
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
        if (id == null) return;
        Minecraft minecraft = Minecraft.getInstance();
        LocalPlayer player = minecraft.player;
        if (player == null || state.isSpectator || state.id != player.getId()) return;
        if (!canEmote()) return;

        float t = (System.currentTimeMillis() - startedMillis) / 1000.0f;
        float d = EmotePose.duration(id);
        if (d > 0.0f && t >= d) {
            stop();
            return;
        }
        float weight = EmotePose.blendWeight(id, t);
        if (weight <= 0.0f) return;
        if (!EmotePose.write(id, t, Cosmetics.reducedMotion(), pose)) return;

        for (int part = 0; part < EmotePose.PARTS; part++) {
            int i = part * EmotePose.STRIDE;
            if (pose[i] == 0.0f && pose[i + 1] == 0.0f && pose[i + 2] == 0.0f
                    && pose[i + 3] == 0.0f && pose[i + 4] == 0.0f && pose[i + 5] == 0.0f) {
                continue; // untouched: keep the vanilla animation for this part
            }
            ModelPart p = partOf(model, part);
            if (p == null) continue;
            // Translations are always deltas on the pivot. Rotations are
            // targets for the limbs (the player is standing still, so the idle
            // swing is irrelevant) but offsets for head and chest, which keep
            // following the player's look direction during the emote.
            p.x += pose[i] * weight;
            p.y += pose[i + 1] * weight;
            p.z += pose[i + 2] * weight;
            if (EmotePose.additiveRotation(part)) {
                p.xRot += pose[i + 3] * weight;
                p.yRot += pose[i + 4] * weight;
                p.zRot += pose[i + 5] * weight;
            } else {
                p.xRot = lerp(p.xRot, pose[i + 3], weight);
                p.yRot = lerp(p.yRot, pose[i + 4], weight);
                p.zRot = lerp(p.zRot, pose[i + 5], weight);
            }
        }
    }

    private static ModelPart partOf(PlayerModel model, int part) {
        return switch (part) {
            case EmotePose.BODY -> model.body;
            case EmotePose.HEAD -> model.head;
            case EmotePose.RIGHT_ARM -> model.rightArm;
            case EmotePose.LEFT_ARM -> model.leftArm;
            case EmotePose.RIGHT_LEG -> model.rightLeg;
            case EmotePose.LEFT_LEG -> model.leftLeg;
            case EmotePose.ROOT -> model.root();
            default -> null;
        };
    }

    private static float lerp(float from, float to, float weight) {
        return from + (to - from) * weight;
    }
}
