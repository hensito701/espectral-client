package es.spectral.menu.mixin;

import java.util.function.Consumer;

import org.joml.Matrix3x2f;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.entity.player.PlayerSkin;

import es.spectral.menu.ChatHeads;
import es.spectral.menu.Compat;

/**
 * 26.2: draws ChatHeads in the <em>focused</em> (chat screen open) pass.
 *
 * <p>{@code ChatComponent#extractRenderState} builds this access class when the
 * display mode is a foreground one (i.e. the chat screen is open); the
 * unfocused HUD path uses {@code DrawingBackgroundGraphicsAccess} instead.
 * Without this mixin heads simply vanish the moment chat is opened, because the
 * background pass never runs while the screen is up.
 *
 * <p>The geometry is identical in both passes ({@code handleMessage} places the
 * text at x = 0), so the same treatment applies: draw the head at the line's
 * text origin and shift the text right by {@link ChatHeads#SHIFT} for the
 * duration of the call, restoring the pose in {@code TAIL}. Because the shift is
 * scoped to {@code handleMessage}, the per-line background fill
 * ({@code fill}, called outside it) stays unshifted. The line instances come
 * from {@code trimmedMessages}, so scrolled lines keep their own heads.
 *
 * <p>{@code ClickableTextOnlyGraphicsAccessMixin} applies the same shift to the
 * click/tooltip capture pass so hitboxes stay on the shifted glyphs.
 */
@Mixin(targets = "net.minecraft.client.gui.components.ChatComponent$DrawingFocusedGraphicsAccess")
public abstract class DrawingFocusedGraphicsAccessMixin {

    @Shadow
    @Final
    private GuiGraphicsExtractor graphics;

    @Shadow
    public abstract void updatePose(Consumer<Matrix3x2f> consumer);

    @Inject(
            method = "handleMessage(IFLnet/minecraft/util/FormattedCharSequence;)Z",
            at = @At("HEAD")
    )
    private void espectral$drawChatHead(int y, float alpha, FormattedCharSequence content,
            CallbackInfoReturnable<Boolean> cir) {
        PlayerSkin skin = ChatHeads.skinForContent(content);
        if (skin == null) return;
        Compat.drawChatHead(this.graphics, skin, 0, y, ChatHeads.HEAD_SIZE);
        this.updatePose(pose -> pose.translate(ChatHeads.SHIFT, 0));
    }

    @Inject(
            method = "handleMessage(IFLnet/minecraft/util/FormattedCharSequence;)Z",
            at = @At("TAIL")
    )
    private void espectral$unshiftChatHead(int y, float alpha, FormattedCharSequence content,
            CallbackInfoReturnable<Boolean> cir) {
        if (ChatHeads.skinForContent(content) == null) return;
        this.updatePose(pose -> pose.translate(-ChatHeads.SHIFT, 0));
    }
}
