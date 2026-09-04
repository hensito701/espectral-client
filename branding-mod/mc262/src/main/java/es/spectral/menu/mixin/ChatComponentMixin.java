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

import net.minecraft.client.gui.components.ChatComponent;
import net.minecraft.client.multiplayer.chat.GuiMessage;
import net.minecraft.client.multiplayer.chat.GuiMessageSource;

import es.spectral.menu.ChatHeads;

/**
 * 26.2: maps first-line content instances to senders so the background
 * graphics pass can draw heads without nickname matching.
 *
 * <p>Only {@code PLAYER}-sourced messages are mapped
 * ({@code GuiMessage$Line#parent()} distinguishes player from system, and
 * system lines simply get no head). Within those, only {@code endOfEntry}
 * lines (message first lines) of signed, sender-known messages are mapped.
 * The line map is rebuilt wholesale by {@code refreshTrimmedMessages} and
 * dropped on {@code clearMessages}.
 */
@Mixin(ChatComponent.class)
public abstract class ChatComponentMixin {

    @Shadow
    @Final
    private List<GuiMessage.Line> trimmedMessages;

    @Unique
    private GuiMessage.Line espectral$headBefore;

    @Inject(
            method = "addMessageToDisplayQueue(Lnet/minecraft/client/multiplayer/chat/GuiMessage;)V",
            at = @At("HEAD")
    )
    private void espectral$captureQueueHead(GuiMessage message, CallbackInfo ci) {
        this.espectral$headBefore = this.trimmedMessages.isEmpty() ? null : this.trimmedMessages.get(0);
    }

    @Inject(
            method = "addMessageToDisplayQueue(Lnet/minecraft/client/multiplayer/chat/GuiMessage;)V",
            at = @At("TAIL")
    )
    private void espectral$mapQueuedSenders(GuiMessage message, CallbackInfo ci) {
        if (message.source() != GuiMessageSource.PLAYER) return;
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
