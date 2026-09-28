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
 * scarf, ears, wings, wraps — cloth simulated per wearer by
 * {@link WearerSims}, rigid parts posed at draw time.
 *
 * <p>Guards (nothing renders otherwise): the wearer is invisible to the
 * viewer, spectating, discrete, has no verified loadout, the Suite cosmetics
 * feature is off (inside {@code Cosmetics.loadoutFor}), or vanilla would hide
 * the matching skin layer (cape layer for back items, hat for head items,
 * per-arm sleeve for wraps). The vanilla cape layer itself is never touched.
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
        if (state.isInvisible || state.isInvisibleToPlayer || state.isSpectator || state.isDiscrete) {
            return;
        }
        UUID uuid = WearerSims.uuidOf(state);
        if (uuid == null) return;
        Loadout loadout = WearerSims.loadoutFor(uuid);
        if (loadout == null || loadout.isEmpty()) return;

        WearerSims.Entry entry = WearerSims.entryFor(uuid, loadout);
        entry.ensureCloth();
        float partial = state.ageInTicks - Mth.floor(state.ageInTicks);

        String espalda = loadout.espalda();
        if (espalda != null && state.showCape) {
            switch (espalda) {
                case "manto_espectral" -> mantle(poseStack, collector, packedLight, entry, partial);
                case "capucha_errante" -> revenant(poseStack, collector, packedLight, state, entry, partial);
                case "jirones_ceniza" -> tatters(poseStack, collector, packedLight, state, entry, partial);
                case "alas_eco" -> wings(poseStack, collector, packedLight, state, entry);
                default -> { }
            }
        }
        if ("bufanda_llama_alma".equals(loadout.cuello())) {
            scarf(poseStack, collector, packedLight, state, entry, partial);
        }
        if ("orejas_vulpinas".equals(loadout.cabeza()) && state.showHat) {
            ears(poseStack, collector, packedLight, state, entry);
        }
        if ("vendas_runicas".equals(loadout.brazos())) {
            wraps(poseStack, collector, packedLight, state, entry, partial);
        }
    }

    // ---------------------------------------------------------------
    // Items
    // ---------------------------------------------------------------

    /** Manto Espectral: one-shoulder mantle sheet with a gold-fading hem. */
    private void mantle(PoseStack pose, SubmitNodeCollector collector, int light,
            WearerSims.Entry e, float partial) {
        if (e.mantle == null) return;
        CosmeticGeo.Sheet r = e.mantleRenderer;
        r.sheet = e.mantle;
        r.partial = partial;
        r.light = light;
        collector.submitCustomGeometry(pose, CosmeticTextures.cloth(CosmeticTextures.MANTLE), r);
    }

    /** Capucha Errante: hooded cloak sheet, forked tatters, hood shell. */
    private void revenant(PoseStack pose, SubmitNodeCollector collector, int light,
            AvatarRenderState state, WearerSims.Entry e, float partial) {
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
        // Hood shell follows the head (vanilla has posed the model by now).
        ModelPart head = getParentModel().head;
        pose.pushPose();
        head.translateAndRotate(pose);
        collector.submitModelPart(CosmeticModels.HOOD, pose,
                CosmeticTextures.cutout(CosmeticTextures.REVENANT_HOOD),
                light, OverlayTexture.NO_OVERLAY, null);
        pose.popPose();
    }

    /** Jirones de Ceniza: three long ribbon tails with gold inner edges. */
    private void tatters(PoseStack pose, SubmitNodeCollector collector, int light,
            AvatarRenderState state, WearerSims.Entry e, float partial) {
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
    }

    /** Alas de Eco: translucent wings, pose driven per wearer at draw time. */
    private void wings(PoseStack pose, SubmitNodeCollector collector, int light,
            AvatarRenderState state, WearerSims.Entry e) {
        ModelPart body = getParentModel().body;
        pose.pushPose();
        body.translateAndRotate(pose);
        collector.submitModel(CosmeticModels.WINGS, state, pose,
                CosmeticTextures.cloth(CosmeticTextures.WINGS),
                light, OverlayTexture.NO_OVERLAY, state.outlineColor, null);
        pose.popPose();
    }

    /** Bufanda Llama de Alma: neck band plus two simulated tails. */
    private void scarf(PoseStack pose, SubmitNodeCollector collector, int light,
            AvatarRenderState state, WearerSims.Entry e, float partial) {
        ModelPart body = getParentModel().body;
        pose.pushPose();
        body.translateAndRotate(pose);
        collector.submitModelPart(CosmeticModels.SCARF_BAND, pose,
                CosmeticTextures.cutout(CosmeticTextures.SCARF),
                light, OverlayTexture.NO_OVERLAY, null);
        pose.popPose();
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
    }

    /** Orejas Vulpinas: head-attached ears (twitch/flop/flatten at draw time). */
    private void ears(PoseStack pose, SubmitNodeCollector collector, int light,
            AvatarRenderState state, WearerSims.Entry e) {
        ModelPart head = getParentModel().head;
        pose.pushPose();
        head.translateAndRotate(pose);
        collector.submitModel(CosmeticModels.EARS, state, pose,
                CosmeticTextures.cutout(CosmeticTextures.EARS),
                light, OverlayTexture.NO_OVERLAY, state.outlineColor, null);
        pose.popPose();
    }

    /** Vendas Rúnicas: forearm bands (arms follow the swing) + loose ends. */
    private void wraps(PoseStack pose, SubmitNodeCollector collector, int light,
            AvatarRenderState state, WearerSims.Entry e, float partial) {
        PlayerModel model = getParentModel();
        if (state.showRightSleeve) {
            armBand(pose, collector, light, model.rightArm);
        }
        if (state.showLeftSleeve) {
            armBand(pose, collector, light, model.leftArm);
        }
        for (int i = 0; i < e.wrapEnds.length; i++) {
            ClothSim.Chain c = e.wrapEnds[i];
            if (c == null) continue;
            CosmeticGeo.Ribbon r = e.wrapEndRenderers[i];
            r.chain = c;
            r.partial = partial;
            r.light = light;
            r.halfWidth = 0.03f;
            r.taper = 0.5f;
            r.vStep = 0.5f;
            r.wave = 0.008f;
            r.waveTick = (int) state.ageInTicks + i * 3;
            collector.submitCustomGeometry(pose, CosmeticTextures.cutout(CosmeticTextures.WRAPS), r);
        }
    }

    private void armBand(PoseStack pose, SubmitNodeCollector collector, int light, ModelPart arm) {
        pose.pushPose();
        arm.translateAndRotate(pose);
        collector.submitModelPart(CosmeticModels.WRAP_BAND, pose,
                CosmeticTextures.cutout(CosmeticTextures.WRAPS),
                light, OverlayTexture.NO_OVERLAY, null);
        pose.popPose();
    }
}
