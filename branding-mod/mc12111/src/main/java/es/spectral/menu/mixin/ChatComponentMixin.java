package es.spectral.menu.mixin;

import java.util.List;
import java.util.UUID;

import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.GuiMessage;
import net.minecraft.client.gui.components.ChatComponent;

import es.spectral.menu.ChatHeads;

/**
 * 1.21.11: maps first-line content instances to senders so the background
 * graphics pass can draw heads without nickname matching.
 *
 * <p>Vanilla {@code GuiMessage$Line} carries no sender reference, so the
 * freshly queued lines are captured here: the head sentinel recorded at
 * {@code HEAD} delimits exactly the lines the current call prepends, and
 * only {@code endOfEntry} lines (message first lines) of signed,
 * sender-known messages are mapped. System/deleted/unsigned lines stay
 * unmapped and render vanilla. The line map is rebuilt wholesale by
 * {@code refreshTrimmedMessages} and dropped on {@code clearMessages}.
 */
@Mixin(ChatComponent.class)
public abstract class ChatComponentMixin {

    @Shadow
    @Final
    private List<GuiMessage.Line> trimmedMessages;

    @Unique
    private GuiMessage.Line espectral$headBefore;

    @Inject(
            method = "addMessageToDisplayQueue(Lnet/minecraft/client/GuiMessage;)V",
            at = @At("HEAD")
    )
    private void espectral$captureQueueHead(GuiMessage message, CallbackInfo ci) {
        this.espectral$headBefore = this.trimmedMessages.isEmpty() ? null : this.trimmedMessages.get(0);
    }

    @Inject(
            method = "addMessageToDisplayQueue(Lnet/minecraft/client/GuiMessage;)V",
            at = @At("TAIL")
    )
    private void espectral$mapQueuedSenders(GuiMessage message, CallbackInfo ci) {
        UUID senderId = ChatHeads.senderFor(message.signature());
        if (senderId == null) return;
        for (GuiMessage.Line line : this.trimmedMessages) {
            if (line == this.espectral$headBefore) break;
            if (line.endOfEntry()) {
                ChatHeads.mapFirstLine(senderId, line.content());
            }
        }
        this.espectral$headBefore = null;
    }

    @Inject(method = "refreshTrimmedMessages()V", at = @At("HEAD"))
    private void espectral$dropStaleLineSenders(CallbackInfo ci) {
        ChatHeads.clearLineSenders();
    }

    @Inject(method = "clearMessages(Z)V", at = @At("TAIL"))
    private void espectral$clearChatHeads(boolean clearHistory, CallbackInfo ci) {
        ChatHeads.clear();
    }
}
