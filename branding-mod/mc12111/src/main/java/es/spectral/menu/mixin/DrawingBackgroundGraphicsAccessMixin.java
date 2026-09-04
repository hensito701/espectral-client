package es.spectral.menu.mixin;

import java.util.function.Consumer;

import org.joml.Matrix3x2f;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.entity.player.PlayerSkin;

import es.spectral.menu.ChatHeads;
import es.spectral.menu.Compat;

/**
 * 1.21.11: draws ChatHeads in the background (unfocused) chat pass.
 *
 * <p>The inner class is package-private, so it is targeted by name. On a
 * headed first line the head is drawn at the line's text origin and the
 * text is shifted right by {@link ChatHeads#SHIFT} via
 * {@code updatePose} (restored in {@code TAIL}); unmapped lines take the
 * pure vanilla path. The background-fill pass calls {@code fill}, never
 * {@code handleMessage}, so fills stay unshifted; the focused pass uses a
 * different access class and is untouched.
 */
@Mixin(targets = "net.minecraft.client.gui.components.ChatComponent$DrawingBackgroundGraphicsAccess")
public abstract class DrawingBackgroundGraphicsAccessMixin {

    @Shadow
    @Final
    private GuiGraphics graphics;

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
