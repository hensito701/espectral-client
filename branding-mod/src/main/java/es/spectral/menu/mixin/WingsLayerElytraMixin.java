package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.renderer.entity.layers.WingsLayer;
import net.minecraft.client.renderer.entity.state.HumanoidRenderState;
import net.minecraft.resources.Identifier;

import es.spectral.menu.cosmetics.render.CosmeticElytra;

/**
 * Elytra skin hook: {@code WingsLayer#getPlayerElytraTexture} (private static,
 * identical descriptor in 26.2 and 1.21.11) picks the per-player elytra
 * texture vanilla feeds to the equipment renderer. Overriding only the return
 * value keeps vanilla's equipment checks, glow and outline handling intact and
 * never touches the cape layer. Shared by both lane mixin configs.
 */
@Mixin(WingsLayer.class)
public abstract class WingsLayerElytraMixin {

    @Inject(method = "getPlayerElytraTexture", at = @At("RETURN"), cancellable = true)
    private static void espectral$cosmeticElytra(HumanoidRenderState state,
            CallbackInfoReturnable<Identifier> cir) {
        Identifier skin = CosmeticElytra.skinFor(state);
        if (skin != null) {
            cir.setReturnValue(skin);
        }
    }
}
