package es.spectral.menu.mixin;

import java.util.List;
import java.util.UUID;

import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.components.ChatComponent;
import net.minecraft.client.multiplayer.chat.GuiMessage;
import net.minecraft.client.multiplayer.chat.GuiMessageSource;

import es.spectral.menu.ChatHeads;
import es.spectral.menu.ClientConfig;

/**
 * 26.2: maps first-line content instances to senders so the background
 * graphics pass can draw heads without nickname guessing.
 *
 * <p>Two sender sources, in order of preference:
 *
 * <ol>
 *   <li>{@code PLAYER} messages: the signature was captured at intake
 *       ({@code ChatListenerMixin} → {@link ChatHeads#senderFor}), so the
 *       sender is exact.</li>
 *   <li>{@code SYSTEM_SERVER} messages: servers that broadcast player chat
 *       through a chat plugin (the Espectral ones do) deliver it as system
 *       chat — {@code <[ADMIN] henMV> asd} — with no signature at all.
 *       {@link ChatHeads#senderForText} resolves the sender by matching an
 *       exact tab-list name inside the line's sender prefix.</li>
 * </ol>
 *
 * <p>{@code GuiMessage$Line#parent()} ties every queued line back to its
 * message, so only this message's {@code endOfEntry} (first) lines are
 * mapped. The line map is rebuilt wholesale by
 * {@code refreshTrimmedMessages} and dropped on {@code clearMessages}.
 */
@Mixin(ChatComponent.class)
public abstract class ChatComponentMixin {

    @Shadow
    @Final
    private List<GuiMessage.Line> trimmedMessages;

    @Inject(
            method = "addMessageToDisplayQueue(Lnet/minecraft/client/multiplayer/chat/GuiMessage;)V",
            at = @At("TAIL")
    )
    private void espectral$mapQueuedSenders(GuiMessage message, CallbackInfo ci) {
        if (!ClientConfig.getInstance().isFeatureEnabled("chatheads")) return;
        UUID senderId = ChatHeads.senderFor(message.signature());
        if (senderId == null && message.source() == GuiMessageSource.SYSTEM_SERVER) {
            senderId = ChatHeads.senderForText(message.content().getString());
        }
        if (senderId == null) return;
        for (GuiMessage.Line line : this.trimmedMessages) {
            if (line.parent() != message) break;
            if (line.endOfEntry()) {
                ChatHeads.mapFirstLine(senderId, line.content());
            }
        }
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
