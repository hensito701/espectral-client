package es.spectral.menu.mixin;

import java.time.Instant;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import com.mojang.authlib.GameProfile;

import net.minecraft.client.multiplayer.chat.ChatListener;
import net.minecraft.network.chat.ChatType;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.PlayerChatMessage;

import es.spectral.menu.ChatHeads;
import es.spectral.menu.ClientConfig;

/**
 * 26.2: captures the signature -&gt; sender link for every signed player
 * message. The intake signature is unchanged from 1.21.11. This is the
 * preferred, exact sender path; the intake never nickname-matches.
 *
 * <p>Unsigned messages carry a null signature and are skipped here. If such a
 * line still reaches the chat display, {@code ChatComponentMixin} falls back
 * to {@link ChatHeads#senderForText}, which only accepts an exact name from
 * the current tab list inside the line's sender prefix — the shape chat
 * plugins produce when they broadcast player chat as a system message.
 */
@Mixin(ChatListener.class)
public abstract class ChatListenerMixin {

    @Inject(
            method = "showMessageToPlayer(Lnet/minecraft/network/chat/ChatType$Bound;"
                    + "Lnet/minecraft/network/chat/PlayerChatMessage;"
                    + "Lnet/minecraft/network/chat/Component;"
                    + "Lcom/mojang/authlib/GameProfile;ZLjava/time/Instant;)Z",
            at = @At("HEAD")
    )
    private void espectral$captureChatSender(ChatType.Bound bound, PlayerChatMessage message,
            Component decorated, GameProfile profile, boolean onlyShowSecureChat, Instant timestamp,
            CallbackInfoReturnable<Boolean> cir) {
        if (!ClientConfig.getInstance().isFeatureEnabled("chatheads")) return;
        ChatHeads.onPlayerMessage(message.signature(), profile.id());
    }
}
