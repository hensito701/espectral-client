package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl;
import net.minecraft.network.DisconnectionDetails;

import es.spectral.menu.ChatHeads;

/**
 * 1.21.11: drops cached ChatHeads sender state on disconnect so nothing
 * leaks across servers. Lives in its own mixin because the disconnect hook
 * sits on the packet listener, not on the chat classes.
 */
@Mixin(ClientCommonPacketListenerImpl.class)
public abstract class ChatDisconnectMixin {

    @Inject(
            method = "onDisconnect(Lnet/minecraft/network/DisconnectionDetails;)V",
            at = @At("HEAD")
    )
    private void espectral$clearChatHeads(DisconnectionDetails details, CallbackInfo ci) {
        ChatHeads.clear();
    }
}
