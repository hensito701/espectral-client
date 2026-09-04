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

/**
 * 26.2: captures the signature -&gt; sender link for every signed player
 * message. The intake signature is unchanged from 1.21.11. Unsigned
 * messages carry a null signature and are skipped (known limitation: those
 * lines get no head). Never nickname-matches.
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
        ChatHeads.onPlayerMessage(message.signature(), profile.id());
    }
}
