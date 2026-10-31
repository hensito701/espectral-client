package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.ClientBrandRetriever;
import net.minecraft.client.network.NetHandlerPlayClient;

import es.spectral.menu.Version;

/**
 * {@code MC|Brand} sent on join: Forge's brand ({@code fml,forge}) plus
 * {@code espectral:forge:<version>}, so Espectral servers detect the client
 * (they match {@code contains("espectral")}, like the Fabric/NeoForge brands)
 * while servers looking for Forge still see it. {@code ClientBrandRetriever}
 * itself is loaded by FML before mixins apply, so the call site is redirected.
 * Self-reported identification, never authentication.
 */
@Mixin(NetHandlerPlayClient.class)
public abstract class NetHandlerPlayClientBrandMixin {

    @Redirect(method = "handleJoinGame", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/ClientBrandRetriever;getClientModName()Ljava/lang/String;", remap = false))
    private String espectral$brand() {
        return ClientBrandRetriever.getClientModName() + ",espectral:forge:" + Version.version();
    }
}
