package es.spectral.menu.cosmetics.render;

import java.util.UUID;

import com.mojang.blaze3d.vertex.PoseStack;

import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.player.PlayerModel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.RenderLayerParent;
import net.minecraft.client.renderer.entity.layers.RenderLayer;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.util.Mth;

import es.spectral.menu.cosmetics.Loadout;

/**
 * The Espectral cosmetic render layer for avatars: cloaks/mantle, tatters,
 * scarf, ears, wings — cloth simulated per wearer by
 * {@link WearerSims}, rigid parts posed at draw time.
 *
 * <p><b>Anchoring.</b> Nothing is drawn at a fixed model-root offset. Every
 * piece is entered from the real {@link ModelPart} it belongs to (head for ears
 * and the hood, body for cloak/scarf/tatters/wings), and the whole layer starts
 * from {@code model.root()} so the <em>posed</em> model carries the cosmetics:
 * vanilla's crouch pitch and body drop, the emote poses
 * applied in {@code PlayerModel#setupAnim} TAIL, and the sit root drop all move
 * the cloth with the body instead of leaving it floating at standing height.
 *
 * <p>Cloth primitives are simulated in the body part's local frame, and
 * {@link ModelPart#translateAndRotate} already converts pixels to blocks, so
 * entering the part <em>is</em> the whole conversion — no inverse rotation to
 * get wrong, which keeps the crouch and the bow exact.
 *
 * <p>Guards (nothing renders otherwise): the wearer is invisible to the viewer,
 * spectating, has no verified loadout, the Suite cosmetics feature is off
 * (inside {@code Cosmetics.loadoutFor}), or vanilla would hide the matching
 * skin layer (cape layer for back items, hat for head items). <b>Sneaking is
 * deliberately not a guard</b> — see {@link #submit}. The vanilla cape layer
 * itself is never touched.
 *
 * <p>Elytra skins are not drawn here — they hook
 * {@code WingsLayer#getPlayerElytraTexture} instead.
 */
public final class CosmeticLayer extends RenderLayer<AvatarRenderState, PlayerModel> {

    public CosmeticLayer(RenderLayerParent<AvatarRenderState, PlayerModel> parent) {
        super(parent);
    }

    @Override
    public void submit(PoseStack poseStack, SubmitNodeCollector collector, int packedLight,
            AvatarRenderState state, float yRot, float xRot) {
        // Sneaking must never hide a cosmetic. In 26.2 `AvatarRenderState` has
        // no sneak flag of its own, and the old guard here tested
        // `state.isDiscrete` — which `Entity#isDiscrete()` defines as
        // `isShiftKeyDown()`. Every cosmetic therefore vanished the instant the
        // wearer held shift, which read in play as "sneaking disappears all of
        // it". Only the states the design rules actually forbid stay guarded:
        // invisible to this viewer, and spectator.
        if (state.isInvisible || state.isInvisibleToPlayer || state.isSpectator) {
            return;
        }
        UUID uuid = WearerSims.uuidOf(state);
        if (uuid == null) return;
        Loadout loadout = WearerSims.loadoutFor(uuid);
        if (loadout == null || loadout.isEmpty()) return;

        WearerSims.Entry entry = WearerSims.entryFor(uuid, loadout);
        entry.ensureCloth();
        float partial = state.ageInTicks - Mth.floor(state.ageInTicks);
        PlayerModel model = getParentModel();

        poseStack.pushPose();
        // Start from the posed root so an emote's sit drop and any root motion
        // carry everything; each item then enters its own part frame.
        model.root().translateAndRotate(poseStack);

        // Back items still honour the vanilla cape layer toggle: an equipped
        // elytra (which clears `showCape`) must not get a cloak drawn over it.
        switch (loadout.espalda() != null && state.showCape ? loadout.espalda() : "") {
            case "manto_espectral" -> mantle(poseStack, collector, packedLight, model, entry, partial);
            case "capucha_errante" -> revenant(poseStack, collector, packedLight, model, state, entry, partial);
            case "jirones_ceniza" -> tatters(poseStack, collector, packedLight, model, state, entry, partial);
            case "alas_eco" -> wings(poseStack, collector, packedLight, model, state, entry);
            default -> { }
        }
        if ("bufanda_llama_alma".equals(loadout.cuello())) {
            scarf(poseStack, collector, packedLight, model, state, entry, partial);
        }
        // Head items replace each other; the hat layer toggle still governs them
        // (a helmet hides the ears exactly like it hides the skin's hat layer).
        if (state.showHat) {
            if ("orejas_vulpinas".equals(loadout.cabeza())) {
                ears(poseStack, collector, packedLight, model, state, CosmeticModels.EARS, CosmeticTextures.EARS);
            } else if ("orejas_felinas".equals(loadout.cabeza())) {
                ears(poseStack, collector, packedLight, model, state, CosmeticModels.CAT_EARS, CosmeticTextures.CAT_EARS);
            }
        }
        poseStack.popPose();
    }

    // ---------------------------------------------------------------
    // Frames
    // ---------------------------------------------------------------

    /**
     * Enters a model part's frame. {@link ModelPart#translateAndRotate} already
     * divides its offsets by 16, so the pose stack is in <b>blocks</b> after it
     * with {@code y = 0} on the model root and {@code +y} down — which is
     * exactly what both the cloth simulation and the rigid geometry are written
     * in (rigid parts are authored in the same pixels as the vanilla model, so
     * their cubes land at the right scale automatically).
     *
     * <p>The body part's pivot is the neck line, and that is also where the
     * cloth anchors are measured from, so the crouch pitch, an emote's bow hinge
     * and the sit root drop are carried by the part with no inverse to get
     * wrong.
     */
    private static void partFrame(PoseStack pose, ModelPart part) {
        part.translateAndRotate(pose);
    }

    // ---------------------------------------------------------------
    // Items
    // ---------------------------------------------------------------

    /** Manto Espectral: one-shoulder mantle sheet with a gold-fading hem. */
    private void mantle(PoseStack pose, SubmitNodeCollector collector, int light,
            PlayerModel model, WearerSims.Entry e, float partial) {
        if (e.mantle == null) return;
        CosmeticGeo.Sheet r = e.mantleRenderer;
        r.sheet = e.mantle;
        r.partial = partial;
        r.light = light;
        // The sim cannot hold a ripple (its in-plane links straighten one), so
        // the mantle's folds are added here: three soft ridges down the sheet,
        // fading to nothing at the pinned shoulder line.
        r.relief = WearerSims.MANTLE_RELIEF;
        r.reliefTop = 0.25f;
        pose.pushPose();
        partFrame(pose, model.body);
        collector.submitCustomGeometry(pose, CosmeticTextures.cloth(CosmeticTextures.MANTLE), r);
        pose.popPose();
    }

    /** Capucha Errante: hooded cloak sheet, forked tatters, hood shell. */
    private void revenant(PoseStack pose, SubmitNodeCollector collector, int light,
            PlayerModel model, AvatarRenderState state, WearerSims.Entry e, float partial) {
        if (e.cloak != null || e.hoodTails[0] != null) {
            pose.pushPose();
            partFrame(pose, model.body);
            if (e.cloak != null) {
                CosmeticGeo.Sheet r = e.cloakRenderer;
                r.sheet = e.cloak;
                r.partial = partial;
                r.light = light;
                collector.submitCustomGeometry(pose, CosmeticTextures.cloth(CosmeticTextures.REVENANT), r);
            }
            for (int i = 0; i < e.hoodTails.length; i++) {
                ClothSim.Chain c = e.hoodTails[i];
                if (c == null) continue;
                CosmeticGeo.Ribbon r = e.hoodTailRenderers[i];
                r.chain = c;
                r.partial = partial;
                r.light = light;
                r.halfWidth = 0.06f;
                r.taper = 0.45f;
                r.vStep = 0.5f;
                r.wave = 0.012f;
                r.waveTick = (int) state.ageInTicks;
                collector.submitCustomGeometry(pose, CosmeticTextures.cloth(CosmeticTextures.REVENANT), r);
            }
            pose.popPose();
        }
        // Hood shell: under the head part, so head yaw/pitch, the crouch drop
        // and an emote's nod all carry it. Submitted as geometry, not as a model
        // call, so nothing re-poses it out from under the head.
        pose.pushPose();
        partFrame(pose, model.head);
        collector.submitModelPart(CosmeticModels.HOOD, pose,
                CosmeticTextures.cutout(CosmeticTextures.REVENANT_HOOD),
                light, OverlayTexture.NO_OVERLAY, null);
        pose.popPose();
    }

    /** Jirones de Ceniza: three long ribbon tails with gold inner edges. */
    private void tatters(PoseStack pose, SubmitNodeCollector collector, int light,
            PlayerModel model, AvatarRenderState state, WearerSims.Entry e, float partial) {
        if (e.tatters[0] == null) return;
        pose.pushPose();
        partFrame(pose, model.body);
        for (int i = 0; i < e.tatters.length; i++) {
            ClothSim.Chain c = e.tatters[i];
            if (c == null) continue;
            CosmeticGeo.Ribbon r = e.tatterRenderers[i];
            r.chain = c;
            r.partial = partial;
            r.light = light;
            r.halfWidth = 0.055f;
            r.taper = 0.5f;
            r.vStep = 0.2f;
            r.wave = 0.02f;
            r.waveTick = (int) state.ageInTicks + i * 7;
            collector.submitCustomGeometry(pose, CosmeticTextures.cloth(CosmeticTextures.TATTERS), r);
        }
        pose.popPose();
    }

    /** Alas de Eco: translucent wings, pose driven per wearer at draw time. */
    private void wings(PoseStack pose, SubmitNodeCollector collector, int light,
            PlayerModel model, AvatarRenderState state, WearerSims.Entry e) {
        pose.pushPose();
        partFrame(pose, model.body);
        collector.submitModel(CosmeticModels.WINGS, state, pose,
                CosmeticTextures.cloth(CosmeticTextures.WINGS),
                light, OverlayTexture.NO_OVERLAY, state.outlineColor, null);
        pose.popPose();
    }

    /** Bufanda Llama de Alma: neck band plus two simulated tails. */
    private void scarf(PoseStack pose, SubmitNodeCollector collector, int light,
            PlayerModel model, AvatarRenderState state, WearerSims.Entry e, float partial) {
        pose.pushPose();
        partFrame(pose, model.body);
        collector.submitModelPart(CosmeticModels.SCARF_BAND, pose,
                CosmeticTextures.cutout(CosmeticTextures.SCARF),
                light, OverlayTexture.NO_OVERLAY, null);
        pose.popPose();
        if (e.scarfTails[0] == null) return;
        pose.pushPose();
        partFrame(pose, model.body);
        for (int i = 0; i < e.scarfTails.length; i++) {
            ClothSim.Chain c = e.scarfTails[i];
            if (c == null) continue;
            CosmeticGeo.Ribbon r = e.scarfTailRenderers[i];
            r.chain = c;
            r.partial = partial;
            r.light = light;
            r.halfWidth = 0.05f;
            r.taper = 0.6f;
            r.vStep = 0.25f;
            r.wave = 0.014f;
            r.waveTick = (int) state.ageInTicks + i * 5;
            r.fadeFrom = 0.55f;
            // The gold flame edge only flickers while the wearer moves.
            r.goldScale = e.walkSpeed > 0.08f
                    ? 0.75f + 0.55f * Mth.sin(state.ageInTicks * 0.6f + i * 1.7f)
                    : 0.9f;
            collector.submitCustomGeometry(pose, CosmeticTextures.cloth(CosmeticTextures.SCARF_TAIL), r);
        }
        pose.popPose();
    }

    /**
     * Orejas (vulpine or feline): the animated ear model, anchored under the
     * head part. The part transform supplies the carried pose (head yaw/pitch,
     * crouch drop, emote nod); the model's own {@code setupAnim} supplies the
     * local reactions.
     */
    private void ears(PoseStack pose, SubmitNodeCollector collector, int light,
            PlayerModel model, AvatarRenderState state,
            CosmeticModels.EarModel kind, net.minecraft.resources.Identifier texture) {
        pose.pushPose();
        partFrame(pose, model.head);
        collector.submitModel(kind, state, pose,
                CosmeticTextures.cutout(texture),
                light, OverlayTexture.NO_OVERLAY, state.outlineColor, null);
        pose.popPose();
    }
}
