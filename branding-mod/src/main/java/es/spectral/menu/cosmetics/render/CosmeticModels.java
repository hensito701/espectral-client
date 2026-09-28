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
 * Rigid cosmetic geometry: vulpine ears, echo wings, the revenant hood shell
 * and the runic wrap bands.
 *
 * <p>Units here are vanilla model pixels (16 px = 1 block), because the parts
 * are submitted through vanilla {@link ModelPart} machinery. Everything stays
 * inside the cosmetic footprint cap: nothing above the head top (y = -8 px),
 * beyond |x| = 12.5 px (shoulders + 1 block) or below the knees (y = 18 px) —
 * and every animated rotation is chosen so no reachable angle can leave that
 * box (ears pitch only, wings yaw only).
 *
 * <p>Animated models pose themselves in {@code setupAnim} from the render
 * state plus the wearer's {@link WearerSims.Entry}, so one shared instance is
 * safe with the deferred node renderer (which re-poses each model at draw
 * time).
 */
public final class CosmeticModels {

    /** Ears: head-attached, twitch on head-yaw deltas, flatten while sneaking. */
    public static final EarModel EARS = new EarModel(buildEars());
    /** Wings: body-attached, open on fall/jump/glide, beat while sprinting. */
    public static final WingModel WINGS = new WingModel(buildWings());
    /** Hood shell: head-attached, open at the front and top. */
    public static final ModelPart HOOD = buildHood();
    /** Neck scarf band: body-attached. */
    public static final ModelPart SCARF_BAND = buildScarfBand();
    /** Forearm wrap band: arm-attached (mirrored by the arm transform). */
    public static final ModelPart WRAP_BAND = buildWrapBand();

    private CosmeticModels() {}

    // ---------------------------------------------------------------
    // Static geometry
    // ---------------------------------------------------------------

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

    private static ModelPart buildWrapBand() {
        MeshDefinition mesh = new MeshDefinition();
        PartDefinition root = mesh.getRoot();
        // Slightly oversized band around the forearm (wide-arm silhouette).
        root.addOrReplaceChild("band",
                CubeListBuilder.create().texOffs(0, 0).addBox(
                        -3.4f, 1.8f, -2.4f, 4.8f, 5.4f, 4.8f),
                PartPose.ZERO);
        root.addOrReplaceChild("knot",
                CubeListBuilder.create().texOffs(0, 14).addBox(
                        -1.2f, 6.6f, -1.2f, 2.4f, 2.2f, 2.4f),
                PartPose.ZERO);
        return LayerDefinition.create(mesh, 64, 32).bakeRoot();
    }

    // ---------------------------------------------------------------
    // Animated geometry
    // ---------------------------------------------------------------

    private static ModelPart buildEars() {
        MeshDefinition mesh = new MeshDefinition();
        PartDefinition root = mesh.getRoot();
        // Wedges on the upper sides of the head; tips flush with the head top.
        root.addOrReplaceChild("right_ear",
                CubeListBuilder.create().texOffs(0, 0).addBox(
                        -3.5f, -5f, -2.5f, 3.5f, 5f, 5f),
                PartPose.offset(-4.5f, -2f, 0f));
        root.addOrReplaceChild("left_ear",
                CubeListBuilder.create().texOffs(14, 0).addBox(
                        0f, -5f, -2.5f, 3.5f, 5f, 5f),
                PartPose.offset(4.5f, -2f, 0f));
        return LayerDefinition.create(mesh, 64, 32).bakeRoot();
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

    /** Ears: pitch-only reaction (cap-safe for any angle), per wearer. */
    public static final class EarModel extends Model<net.minecraft.client.renderer.entity.state.AvatarRenderState> {

        private final ModelPart right;
        private final ModelPart left;

        EarModel(ModelPart root) {
            super(root, CosmeticTextures::cutout);
            this.right = root.getChild("right_ear");
            this.left = root.getChild("left_ear");
        }

        @Override
        public void setupAnim(net.minecraft.client.renderer.entity.state.AvatarRenderState state) {
            float twitch = 0f;
            float flop = 0f;
            float flatten = state.isCrouching ? 0.55f : 0f;
            WearerSims.Entry e = WearerSims.lookup(WearerSims.uuidOf(state));
            if (e != null) {
                if (Cosmetics.reducedMotion()) {
                    twitch = 0f;
                    flop = 0f;
                } else {
                    twitch = Mth.clamp(e.earTwitch, -0.12f, 0.12f);
                    flop = Mth.clamp(e.earFlop, 0f, 0.35f);
                }
            }
            // Twitch flicks the ears forward/back; landing flops them further;
            // sneaking flattens them against the head.
            float pitch = flatten + flop - twitch * 2.2f;
            this.right.xRot = pitch;
            this.left.xRot = pitch - twitch * 1.3f;
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
            boolean still = Cosmetics.reducedMotion();
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
