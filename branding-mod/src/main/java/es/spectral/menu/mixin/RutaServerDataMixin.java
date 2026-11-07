package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ServerData;

import es.spectral.menu.ruta.Ruta;
import es.spectral.menu.ruta.RutaMessages;

/**
 * Keeps the Ruta reason visible when a routed status ping fails through the
 * exception path: the server row marks the entry {@code UNREACHABLE} and writes
 * vanilla's "Can't connect to server." text. Our reason is recorded by the
 * connect hook; this rewrites the row text on the game thread, because the
 * state change happens on the pinger pool thread while the row reads it on the
 * render thread.
 */
@Mixin(ServerData.class)
public abstract class RutaServerDataMixin {

    @Inject(method = "setState", at = @At("TAIL"))
    private void espectral$rutaUnreachableText(ServerData.State state, CallbackInfo ci) {
        if (state != ServerData.State.UNREACHABLE) return;
        ServerData self = (ServerData) (Object) this;
        String code = Ruta.recentFailure(self.ip);
        if (code == null) return;
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null) {
            self.motd = RutaMessages.status(code);
            return;
        }
        minecraft.execute(() -> self.motd = RutaMessages.status(code));
    }
}
