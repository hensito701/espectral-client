package es.spectral.menu.cosmetics.render;

import com.mojang.blaze3d.vertex.PoseStack;

import net.minecraft.client.Minecraft;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.util.Mth;

import es.spectral.menu.cosmetics.Loadout;

/**
 * Own-player first-person hands: the runic wrap band on the visible arm plus
 * one simulated loose end per hand. Called from the {@code renderHand} TAIL
 * injection, so the arm part still carries the exact pose vanilla just drew
 * (reset pose + the slight zRot) — the band is submitted in the arm's own
 * frame, the loose end is the wearer's simulated chain re-anchored at the
 * wrist.
 */
public final class CosmeticHand {

    private static final CosmeticGeo.Ribbon RIGHT_END = new CosmeticGeo.Ribbon();
    private static final CosmeticGeo.Ribbon LEFT_END = new CosmeticGeo.Ribbon();

    private CosmeticHand() {}

    /**
     * @param armPart the arm vanilla just submitted ({@code x < 0} = right)
     */
    public static void render(PoseStack pose, SubmitNodeCollector collector, int light, ModelPart armPart) {
        Minecraft mc = Minecraft.getInstance();
        if (mc == null || mc.player == null || armPart == null) return;
        Loadout loadout = WearerSims.loadoutFor(mc.player.getUUID());
        if (loadout == null || !"vendas_runicas".equals(loadout.brazos())) return;
        boolean right = armPart.x < 0f;

        // Wrap band around the forearm, in the arm's own frame.
        pose.pushPose();
        armPart.translateAndRotate(pose);
        if (right) {
            collector.submitModelPart(CosmeticModels.WRAP_BAND, pose,
                    CosmeticTextures.cutout(CosmeticTextures.WRAPS),
                    light, OverlayTexture.NO_OVERLAY, null);
        } else {
            // The band is authored for the right arm; mirror it to the left.
            pose.pushPose();
            pose.translate(0.125f, 0f, 0f);
            collector.submitModelPart(CosmeticModels.WRAP_BAND, pose,
                    CosmeticTextures.cutout(CosmeticTextures.WRAPS),
                    light, OverlayTexture.NO_OVERLAY, null);
            pose.popPose();
        }
        pose.popPose();

        // Loose end: the wearer's simulated wrist chain, re-anchored to the
        // drawn wrist (blocks; the arm part is pixel-authored).
        WearerSims.Entry entry = WearerSims.lookup(mc.player.getUUID());
        if (entry == null) return;
        ClothSim.Chain chain = entry.wrapEnds[right ? 0 : 1];
        if (chain == null) return;
        float partial = Mth.clamp(mc.getDeltaTracker().getGameTimeDeltaPartialTick(true), 0f, 1f);
        CosmeticGeo.Ribbon r = right ? RIGHT_END : LEFT_END;
        r.chain = chain;
        r.partial = partial;
        r.light = light;
        r.halfWidth = 0.03f;
        r.taper = 0.5f;
        r.vStep = 0.5f;
        r.wave = 0.006f;
        r.waveTick = mc.player.tickCount + (right ? 0 : 11);
        float wx = (right ? -1f : 1f) / 16f;   // wrist, arm-part frame (blocks)
        float wy = 0.5f;
        float wz = 0f;
        r.offX = wx - Mth.lerp(partial, chain.qx[0], chain.px[0]);
        r.offY = wy - Mth.lerp(partial, chain.qy[0], chain.py[0]);
        r.offZ = wz - Mth.lerp(partial, chain.qz[0], chain.pz[0]);
        pose.pushPose();
        armPart.translateAndRotate(pose);
        collector.submitCustomGeometry(pose, CosmeticTextures.cutout(CosmeticTextures.WRAPS), r);
        pose.popPose();
    }
}
