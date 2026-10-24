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
import net.minecraft.client.GuiMessageTag;
import net.minecraft.client.gui.components.ChatComponent;

import es.spectral.menu.ChatHeads;
import es.spectral.menu.ClientConfig;

/**
 * 1.21.11: maps first-line content instances to senders so the background
 * graphics pass can draw heads without nickname guessing.
 *
 * <p>Vanilla {@code GuiMessage$Line} carries no sender reference, so the head
 * line recorded at {@code HEAD} delimits the lines the current call prepends:
 * they are always inserted at the front, so scanning from index 0 until the
 * recorded line (or the end, when the display list was empty) covers exactly
 * this message. That holds while {@code refreshTrimmedMessages} replays every
 * message through this same method, since each replay still prepends its own
 * lines in front of the ones already replayed.
 *
 * <p>Two sender sources, in order of preference:
 *
 * <ol>
 *   <li>Signed player messages: the signature was captured at intake
 *       ({@code ChatListenerMixin} → {@link ChatHeads#senderFor}), so the
 *       sender is exact.</li>
 *   <li>Server system messages. This lane has no {@code GuiMessageSource};
 *       {@link GuiMessageTag#system()} / {@link GuiMessageTag#systemSinglePlayer()}
 *       mark system chat, and servers that broadcast player chat through a
 *       chat plugin deliver it there — {@code <[ADMIN] henMV> asd} — with no
 *       signature at all. {@link ChatHeads#senderForText} resolves the sender
 *       by matching an exact tab-list name inside the line's sender
 *       prefix.</li>
 * </ol>
 *
 * <p>Only {@code endOfEntry} lines (message first lines) are mapped, so
 * continuation lines never draw heads. Lines that resolve to nothing
 * (broadcasts, unknown names, unsigned player chat) stay unmapped and render
 * vanilla. The line map is rebuilt wholesale by
 * {@code refreshTrimmedMessages} and dropped on {@code clearMessages}.
 */
@Mixin(ChatComponent.class)
public abstract class ChatComponentMixin {

    @Shadow
    @Final
    private List<GuiMessage.Line> trimmedMessages;

    /** Head line before the current call, or null when the list was empty. */
    @Unique
    private GuiMessage.Line espectral$headBefore;

    @Inject(
            method = "addMessageToDisplayQueue(Lnet/minecraft/client/GuiMessage;)V",
            at = @At("HEAD")
    )
    private void espectral$captureQueueHead(GuiMessage message, CallbackInfo ci) {
        this.espectral$headBefore =
                this.trimmedMessages.isEmpty() ? null : this.trimmedMessages.get(0);
    }

    @Inject(
            method = "addMessageToDisplayQueue(Lnet/minecraft/client/GuiMessage;)V",
            at = @At("TAIL")
    )
    private void espectral$mapQueuedSenders(GuiMessage message, CallbackInfo ci) {
        GuiMessage.Line stop = this.espectral$headBefore;
        this.espectral$headBefore = null;
        if (!ClientConfig.getInstance().isFeatureEnabled("chatheads")) return;
        UUID senderId = ChatHeads.senderFor(message.signature());
        if (senderId == null && espectral$isSystemLine(message)) {
            senderId = ChatHeads.senderForText(message.content().getString());
        }
        if (senderId == null) return;
        for (GuiMessage.Line line : this.trimmedMessages) {
            if (line == stop) break;
            if (line.endOfEntry()) {
                ChatHeads.mapFirstLine(senderId, line.content());
            }
        }
    }

    /**
     * True for a signature-less line tagged as system chat. Signed player
     * messages never carry these tags, and unsigned player chat carries a
     * null or not-secure tag, so this cannot steal the exact signed path.
     * Both factories return static singletons, so identity is the fast path.
     */
    @Unique
    private static boolean espectral$isSystemLine(GuiMessage message) {
        if (message.signature() != null) return false;
        GuiMessageTag tag = message.tag();
        if (tag == null) return false;
        GuiMessageTag system = GuiMessageTag.system();
        GuiMessageTag singlePlayer = GuiMessageTag.systemSinglePlayer();
        return tag == system || tag == singlePlayer || tag.equals(system) || tag.equals(singlePlayer);
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
