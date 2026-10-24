package es.spectral.menu.cosmetics.render;

import java.util.Set;

import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.geom.PartPose;
import net.minecraft.client.model.geom.builders.CubeListBuilder;
import net.minecraft.client.model.geom.builders.LayerDefinition;
import net.minecraft.client.model.geom.builders.MeshDefinition;
import net.minecraft.client.model.geom.builders.PartDefinition;
import net.minecraft.core.Direction;
import net.minecraft.util.Mth;

import es.spectral.menu.cosmetics.Cosmetics;

/**
 * Rigid cosmetic geometry: vulpine/feline ears, echo wings and the revenant
 * hood shell.
 *
 * <p>Units here are vanilla model pixels (16 px = 1 block), because the parts
 * are submitted through vanilla {@link ModelPart} machinery. Everything stays
 * inside the cosmetic footprint cap — nothing wider than the shoulders plus one
 * block (|x| = 12.5 px), nothing lower than the knees (y = 18 px) — and every
 * animated rotation is chosen so no reachable angle can leave that box (ears
 * pitch and splay only, wings yaw only).
 *
 * <p><b>Head-top exception (ears only).</b> The cap forbids geometry above the
 * head top (y = −8 px from the neck line). An ear that does not rise above the
 * head reads as a flat box on the skull, which is exactly the playtest defect
 * that forced this redesign, so the two ear items are granted a bounded
 * exception: at most 5 px (0.31 block) above the head top, no further — the
 * vulpine pair reaches 4.9 px and the feline pair 3.3 px. That is no taller
 * than vanilla's own {@code PlayerEarsModel} extra ears and it never grows the
 * collision box. The exception is recorded in {@code docs/espectral-client-v2-vision.md}
 * under "Engineering rules (fixed)".
 *
 * <p>Animated models pose themselves in {@code setupAnim} from the render
 * state plus the wearer's {@link WearerSims.Entry}, so one shared instance is
 * safe with the deferred node renderer (which re-poses each model at draw
 * time).
 */
public final class CosmeticModels {

    /** Vulpine ears: real tapered fox ears, orange-red with cream inner fur. */
    public static final EarModel EARS = new EarModel(buildEars(false), false, 0.22f, 0.06f);
    /** Feline ears: shorter, rounder, dark with a pink inner ear. */
    public static final EarModel CAT_EARS = new EarModel(buildEars(true), true, 0.30f, 0.12f);
    /** Wings: body-attached, open on fall/jump/glide, beat while sprinting. */
    public static final WingModel WINGS = new WingModel(buildWings());
    /** Hood shell: head-attached, open at the front and top. */
    public static final ModelPart HOOD = buildHood();
    /** Neck scarf band: body-attached. */
    public static final ModelPart SCARF_BAND = buildScarfBand();

    /**
     * Texture region for one ear part in the 64x32 ear sheet. Both ear items
     * share this atlas layout so the same UVs serve the vulpine and the feline
     * sheet — only the painted palette differs.
     */
    private static final int UV_SEG0_U = 0, UV_SEG0_V = 0;
    private static final int UV_SEG1_U = 16, UV_SEG1_V = 0;
    private static final int UV_TIP_U = 28, UV_TIP_V = 0;
    private static final int UV_INNER_U = 36, UV_INNER_V = 0;
    private static final int UV_TUFT_U = 48, UV_TUFT_V = 0;

    private CosmeticModels() {}

    private static ModelPart buildHood() {
        MeshDefinition mesh = new MeshDefinition();
        PartDefinition root = mesh.getRoot();
        // Shell around the head: sides + back only (front and head top open).
        root.addOrReplaceChild("hood",
                CubeListBuilder.create().texOffs(0, 0).addBox(
                        -5.3f, -8f, -4.4f, 10.6f, 12.4f, 8.8f,
                        Set.of(Direction.UP, Direction.SOUTH, Direction.EAST, Direction.WEST)),
                PartPose.ZERO);
        // Small cowl flare below the neck line.
        root.addOrReplaceChild("cowl",
                CubeListBuilder.create().texOffs(28, 0).addBox(
                        -6.2f, 3.6f, -3.4f, 12.4f, 3.2f, 8.2f,
                        Set.of(Direction.UP)),
                PartPose.ZERO);
        return LayerDefinition.create(mesh, 64, 32).bakeRoot();
    }

    private static ModelPart buildScarfBand() {
        MeshDefinition mesh = new MeshDefinition();
        PartDefinition root = mesh.getRoot();
        root.addOrReplaceChild("band",
                CubeListBuilder.create().texOffs(0, 0).addBox(
                        -6f, -3f, -4.5f, 12f, 4.6f, 9f),
                PartPose.ZERO);
        root.addOrReplaceChild("fold",
                CubeListBuilder.create().texOffs(0, 16).addBox(
                        -4.5f, 0.8f, -3.4f, 9f, 2.2f, 7f),
                PartPose.ZERO);
        return LayerDefinition.create(mesh, 64, 32).bakeRoot();
    }

    // ---------------------------------------------------------------
    // Ears
    // ---------------------------------------------------------------

    /**
     * Builds the two ear sets out of stacked, progressively narrower boxes —
     * the only taper vanilla cube geometry can express. Three segments give a
     * clean triangle silhouette: a wide base on the skull, a waist, and a dark
     * tip. A slightly proud inner-ear panel sits on the front face of the two
     * lower segments so the cream fur (vulpine) or pink skin (feline) is
     * visible from the front, which is what makes the shape read as an ear
     * rather than a box.
     *
     * <p>Both sets pivot at the top corners of the head (y = −7.5 px, x = ±2.8
     * px) and splay outward, so they sit toward the sides of the skull exactly
     * like a real animal's ears.
     */
    private static ModelPart buildEars(boolean feline) {
        MeshDefinition mesh = new MeshDefinition();
        PartDefinition root = mesh.getRoot();
        // Vulpine: taller, sharper, wider splay. Feline: shorter, rounder, more upright.
        float splay = feline ? 0.30f : 0.22f;
        float yawOut = feline ? 0.10f : 0.16f;
        addEar(root, "right_ear", true, feline, splay, yawOut);
        addEar(root, "left_ear", false, feline, splay, yawOut);
        return LayerDefinition.create(mesh, 64, 32).bakeRoot();
    }

    /**
     * One ear: a wide base box, a narrower waist and a dark tip, plus a proud
     * inner-ear panel on each lower segment. The three stacked boxes are what
     * give the tapered triangle silhouette; the panel is what makes the front
     * read as an ear rather than a slab.
     *
     * <p>Boxes use integer pixel sizes so each face lands on an exact texture
     * rect (vanilla unwraps a box as 2d+2w wide by d+h tall), which is why the
     * generator can paint fur gradients per face.
     *
     * @param right  right ear (mirrored x)
     * @param feline shorter and rounder
     */
    private static void addEar(PartDefinition root, String name, boolean right,
            boolean feline, float splay, float yawOut) {
        float half = right ? -1f : 1f;
        // Pivot on the top corner of the skull, splayed outward and angled forward.
        PartDefinition ear = root.addOrReplaceChild(name, CubeListBuilder.create(),
                PartPose.offsetAndRotation(
                        2.8f * half, -7.5f, 0.4f,
                        0f, -yawOut * half, splay * half));
        // Vulpine: 3 / 2 / 1 px wide, 2 / 2 / 1.4 px tall -> 5.4 px above the
        // pivot, i.e. 4.9 px above the head top (the recorded ear exception).
        // Feline: same widths, 1.4 / 1.4 / 1 px tall -> 3.8 px above the pivot.
        float h0 = feline ? 1.4f : 2f;
        float[][] seg = {
                {1.5f, h0, UV_SEG0_U, UV_SEG0_V},
                {1.0f, h0, UV_SEG1_U, UV_SEG1_V},
                {0.5f, feline ? 1.0f : 1.4f, UV_TIP_U, UV_TIP_V},
        };
        float y = 0f;
        for (int i = 0; i < seg.length; i++) {
            float hw = seg[i][0], h = seg[i][1];
            int u = (int) seg[i][2], v = (int) seg[i][3];
            float hd = hw; // square section: depth equals width
            ear.addOrReplaceChild("seg" + i,
                    CubeListBuilder.create().texOffs(u, v)
                            .addBox(-hw, y - h, -hd, hw * 2f, h, hd * 2f),
                    PartPose.ZERO);
            if (i != seg.length - 1) {
                // Inner ear: a 0.5 px slab straddling the front face, standing
                // 0.25 px proud. Thicker than a z-fighting sliver, still flush;
                // the shell's front face behind it is fur, so the rim reads as
                // fur rather than a hole.
                float iw = hw * 0.62f, ih = h * 0.86f;
                ear.addOrReplaceChild("inner" + i,
                        CubeListBuilder.create()
                                .texOffs(UV_INNER_U, UV_INNER_V + i * 4)
                                .addBox(-iw, y - h + (h - ih) * 0.5f, -hd - 0.25f,
                                        iw * 2f, ih, 0.5f),
                        PartPose.ZERO);
            }
            y -= h;
        }
        if (feline) {
            // Feline bonus: a short fur cuff around the base widens the silhouette.
            ear.addOrReplaceChild("tuft",
                    CubeListBuilder.create().texOffs(UV_TUFT_U, UV_TUFT_V)
                            .addBox(-seg[0][0] - 0.3f, -0.8f, -seg[0][0] * 0.9f,
                                    seg[0][0] * 2f + 0.6f, 0.8f, seg[0][0] * 1.8f),
                    PartPose.ZERO);
        }
    }

    private static ModelPart buildWings() {
        MeshDefinition mesh = new MeshDefinition();
        PartDefinition root = mesh.getRoot();
        root.addOrReplaceChild("right_wing",
                CubeListBuilder.create().texOffs(0, 0).addBox(
                        -8f, 0f, -1.5f, 7f, 17f, 2f),
                PartPose.offsetAndRotation(-3f, -3f, 2f, 0f, 1.4f, 0f));
        root.addOrReplaceChild("left_wing",
                CubeListBuilder.create().texOffs(22, 0).addBox(
                        1f, 0f, -1.5f, 7f, 17f, 2f),
                PartPose.offsetAndRotation(3f, -3f, 2f, 0f, -1.4f, 0f));
        return LayerDefinition.create(mesh, 64, 32).bakeRoot();
    }

    // ---------------------------------------------------------------
    // Models
    // ---------------------------------------------------------------

    /**
     * Ears: pitch/splay-only reactions (cap-safe for any angle), per wearer.
     *
     * <p>Shared by both items; the two instances differ only in their rest pose
     * and in which reaction they amplify — the vulpine set twitches hard on head
     * turns and flops on landing, the feline set flicks gently, perks up while
     * sprinting and folds flatter while sneaking.
     */
    public static final class EarModel extends Model<net.minecraft.client.renderer.entity.state.AvatarRenderState> {

        private final ModelPart right;
        private final ModelPart left;
        private final boolean feline;
        /** Rest splay away from the skull (radians), per item. */
        private final float restSplay;
        /** Rest forward lean (radians, positive tips the tip toward the face). */
        private final float restPitch;

        EarModel(ModelPart root, boolean feline, float restSplay, float restPitch) {
            super(root, CosmeticTextures::cutout);
            this.right = root.getChild("right_ear");
            this.left = root.getChild("left_ear");
            this.feline = feline;
            this.restSplay = restSplay;
            this.restPitch = restPitch;
        }

        @Override
        public void setupAnim(net.minecraft.client.renderer.entity.state.AvatarRenderState state) {
            // Rest pose first: the model instance is shared and re-posed at draw
            // time, so stale angles from the previous wearer must not leak.
            this.right.xRot = restPitch;
            this.left.xRot = restPitch;
            this.right.zRot = -restSplay;
            this.left.zRot = restSplay;

            // Sneaking flattens the ears back against the skull — the reaction
            // the design asks for. It is a pose change, never a hide: the layer
            // draws them regardless of sneak (see CosmeticLayer#submit).
            // Negative pitch lays the tip back along the skull; positive would
            // fold it over the face.
            float flatten = state.isCrouching ? (feline ? -0.62f : -0.48f) : 0f;

            float twitch = 0f;
            float flop = 0f;
            float perk = 0f;
            WearerSims.Entry e = WearerSims.lookup(WearerSims.uuidOf(state));
            if (e != null && Cosmetics.physicsEnabled()) {
                if (feline) {
                    // Gentle flick on head turns, no landing flop; sprinting
                    // sweeps them back with the wind (negative pitch).
                    twitch = Mth.clamp(e.earTwitch * 0.45f, -0.08f, 0.08f);
                    perk = -0.22f * e.catPerk;
                } else {
                    twitch = Mth.clamp(e.earTwitch, -0.12f, 0.12f);
                    flop = Mth.clamp(e.earFlop, 0f, 0.35f);
                }
            }
            // Twitch flicks the ears; landing flops them forward; sprinting
            // sweeps the feline pair back; sneaking lays both flat.
            float pitch = restPitch + flatten + flop + perk - twitch * 2.2f;
            this.right.xRot = pitch;
            this.left.xRot = pitch - twitch * (feline ? 0.5f : 1.3f);
            // Flattening also lays the ears outward against the head. Capped so
            // the splay can never exceed the shoulder-width footprint.
            float widen = -flatten * 0.6f;
            this.right.zRot = -(restSplay + widen);
            this.left.zRot = restSplay + widen;
        }
    }

    /** Wings: yaw-only open/beat (cap-safe for any angle), per wearer. */
    public static final class WingModel extends Model<net.minecraft.client.renderer.entity.state.AvatarRenderState> {

        private final ModelPart right;
        private final ModelPart left;

        WingModel(ModelPart root) {
            super(root, CosmeticTextures::cloth);
            this.right = root.getChild("right_wing");
            this.left = root.getChild("left_wing");
        }

        @Override
        public void setupAnim(net.minecraft.client.renderer.entity.state.AvatarRenderState state) {
            float open = 0.12f;
            boolean still = !Cosmetics.physicsEnabled();
            WearerSims.Entry e = WearerSims.lookup(WearerSims.uuidOf(state));
            if (e != null) open = e.wingOpen;
            if (still) open = 0.25f;
            // Folded at rest (yRot ±1.4), open toward yRot 0 as `open` grows.
            float base = 1.4f - open * 1.55f;
            float beat = still ? 0f : Mth.sin(state.ageInTicks * 0.55f) * 0.22f * open;
            this.right.yRot = base + beat;
            this.left.yRot = -base - beat;
        }
    }
}
