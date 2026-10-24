package es.spectral.menu.mixin;

import java.util.function.Consumer;

import org.joml.Matrix3x2f;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.util.FormattedCharSequence;

import es.spectral.menu.ChatHeads;

/**
 * 26.2: keeps click/tooltip hitboxes on the shifted glyphs.
 *
 * <p>When the chat screen opens, {@code ChatScreen#mouseClicked} runs the chat
 * through {@code ChatComponent#captureClickableText}, which walks the same lines
 * via {@code ClickableTextOnlyGraphicsAccess} and records where each clickable
 * span landed. A headed first line is drawn {@link ChatHeads#SHIFT} pixels to the
 * right by {@link DrawingFocusedGraphicsAccessMixin}, so this pass has to apply
 * the same shift or links would be unclickable where they appear to be.
 *
 * <p>Only the pose is adjusted — nothing is drawn here.
 */
@Mixin(targets = "net.minecraft.client.gui.components.ChatComponent$ClickableTextOnlyGraphicsAccess")
public abstract class ClickableTextOnlyGraphicsAccessMixin {

    @Shadow
    public abstract void updatePose(Consumer<Matrix3x2f> consumer);

    @Inject(
            method = "handleMessage(IFLnet/minecraft/util/FormattedCharSequence;)Z",
            at = @At("HEAD")
    )
    private void espectral$shiftForChatHead(int y, float alpha, FormattedCharSequence content,
            CallbackInfoReturnable<Boolean> cir) {
        if (ChatHeads.skinForContent(content) == null) return;
        this.updatePose(pose -> pose.translate(ChatHeads.SHIFT, 0));
    }

    @Inject(
            method = "handleMessage(IFLnet/minecraft/util/FormattedCharSequence;)Z",
            at = @At("TAIL")
    )
    private void espectral$unshiftForChatHead(int y, float alpha, FormattedCharSequence content,
            CallbackInfoReturnable<Boolean> cir) {
        if (ChatHeads.skinForContent(content) == null) return;
        this.updatePose(pose -> pose.translate(-ChatHeads.SHIFT, 0));
    }
}
