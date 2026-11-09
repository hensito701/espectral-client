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

import es.spectral.menu.cosmetics.Loadout;

/**
 * The Espectral cosmetic render layer: every equipped model item, drawn under
 * the real posed body part it attaches to.
 *
 * <p><b>Nothing item-specific lives here.</b> The layer walks the wearer's
 * {@code cabeza}, {@code cuello} and {@code espalda} slots, asks
 * {@link ItemModels} for the baked geometry of each id and submits it. Motion
 * is stepped per wearer by {@link WearerSims} from the item's own
 * {@code motion} block, so a new item is a folder of data — no Java change.
 *
 * <p><b>Anchoring.</b> The layer starts from {@code model.root()} and then
 * enters the real {@link ModelPart} of each attachment ({@code model.head},
 * {@code model.body}, …) with {@link ModelPart#translateAndRotate}, so the
 * <em>posed</em> model carries the cosmetic: vanilla's crouch pitch and body
 * drop, the emote poses applied in {@code PlayerModel#setupAnim}, and the sit
 * root drop all move it with the body instead of leaving it floating.
 *
 * <p><b>Guards</b> (nothing renders otherwise): the wearer is invisible to the
 * viewer, spectating, has no verified loadout, the Suite cosmetics feature is
 * off (inside {@link WearerSims#loadoutFor}), or vanilla would hide the
 * matching skin layer ({@code hide_with}: {@code hat} for head items,
 * {@code cape} for back items). Sneaking is deliberately not a guard.
 *
 * <p>Elytra skins are not drawn here — they hook
 * {@code WingsLayer#getPlayerElytraTexture} instead.
 */
public final class CosmeticLayer extends RenderLayer<AvatarRenderState, PlayerModel> {

    /** Slots drawn by this layer, in submission order. */
    private static final String[] SLOTS = {"espalda", "cuello", "cabeza"};

    public CosmeticLayer(RenderLayerParent<AvatarRenderState, PlayerModel> parent) {
        super(parent);
    }

    @Override
    public void submit(PoseStack poseStack, SubmitNodeCollector collector, int packedLight,
            AvatarRenderState state, float yRot, float xRot) {
        // Sneaking must never hide a cosmetic: `AvatarRenderState.isDiscrete`
        // is `isShiftKeyDown()`, so testing it dropped every cosmetic the
        // instant the wearer held shift. Only the states the design rules
        // actually forbid stay guarded: invisible to this viewer, and spectator.
        if (state.isInvisible || state.isInvisibleToPlayer || state.isSpectator) {
            return;
        }
        UUID uuid = WearerSims.uuidOf(state);
        if (uuid == null) return;
        Loadout loadout = WearerSims.loadoutFor(uuid);
        if (loadout == null || loadout.isEmpty()) return;

        PlayerModel model = getParentModel();
        poseStack.pushPose();
        // Start from the posed root so an emote's sit drop and any root motion
        // carry everything; each item then enters its own part frame.
        model.root().translateAndRotate(poseStack);
        for (String slot : SLOTS) {
            String id = loadout.forSlot(slot);
            if (id == null) continue;
            ItemModels.Item item = ItemModels.item(id);
            if (item == null) continue;
            if (hiddenBySkinLayer(item.hideWith, state)) continue;
            draw(poseStack, collector, packedLight, model, state, item);
        }
        poseStack.popPose();
    }

    /** True when the vanilla skin-layer toggle hides the item. */
    private static boolean hiddenBySkinLayer(String hideWith, AvatarRenderState state) {
        return switch (hideWith) {
            case "hat" -> !state.showHat;
            case "cape" -> !state.showCape;
            default -> false;
        };
    }

    private static void draw(PoseStack pose, SubmitNodeCollector collector, int light,
            PlayerModel model, AvatarRenderState state, ItemModels.Item item) {
        for (int i = 0; i < item.parts.size(); i++) {
            ItemModels.Part part = item.parts.get(i);
            ModelPart attachment = attachmentPart(model, part.part);
            if (attachment == null) continue;
            pose.pushPose();
            attachment.translateAndRotate(pose);
            es.spectral.menu.Compat.submitModel(collector, part, state, pose,
                    part.renderType(ItemModels.texture(item.id)), light,
                    OverlayTexture.NO_OVERLAY, state.outlineColor);
            pose.popPose();
        }
    }

    /** The vanilla {@link ModelPart} an attachment bone is entered from. */
    private static ModelPart attachmentPart(PlayerModel model, String part) {
        return switch (part) {
            case "head" -> model.head;
            case "body" -> model.body;
            case "right_arm" -> model.rightArm;
            case "left_arm" -> model.leftArm;
            case "right_leg" -> model.rightLeg;
            case "left_leg" -> model.leftLeg;
            default -> null;
        };
    }
}
