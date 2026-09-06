package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.ClientBrandRetriever;

import es.spectral.menu.EspectralBrand;

/**
 * Reports {@code espectral:fabric[:<version>]} on the
 * {@code minecraft:brand} handshake instead of the loader default.
 *
 * <p>The client sends {@code BrandPayload(ClientBrandRetriever.getClientModName())}
 * once per server join, as the connection switches to the configuration
 * protocol. Server-side plugins read the inbound packet directly — vanilla
 * itself does not persist the client brand — and ours matches a
 * case-insensitive {@code contains("espectral")}.
 *
 * <p>Fabric Loader ASM-patches this same method to return {@code "fabric"}; a
 * HEAD inject runs on top of that patch regardless of what the loader
 * rewrote, so no redirect/overwrite conflict is possible. No refmap ships
 * (same as every other mixin in this mod): the target is
 * {@code DontObfuscate} and production runs Mojang-named — with
 * {@code required: true} a mapping change would fail fast at startup.
 */
@Mixin(ClientBrandRetriever.class)
public abstract class ClientBrandMixin {

    @Inject(method = "getClientModName", at = @At("HEAD"), cancellable = true)
    private static void espectral$brand(CallbackInfoReturnable<String> cir) {
        cir.setReturnValue(EspectralBrand.brand());
    }
}
