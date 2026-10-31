package es.spectral.menu.mixin;

import org.lwjgl.sdl.SDLVideo;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import com.mojang.blaze3d.platform.Window;

/**
 * Hidden agent launches ({@code -Despectral.agent.hidden=1}, set by the
 * launcher only for agent launches): 26.3 creates its window through SDL3, so
 * hide the SDL window right after creation, before the first frame is
 * presented. Rendering continues into the main render target, which is what
 * agent captures read.
 */
@Mixin(Window.class)
public abstract class AgentHiddenWindowMixin {

    @Inject(method = "createWindow", at = @At("RETURN"))
    private void espectral$agentHide(CallbackInfoReturnable<Long> cir) {
        if (es.spectral.menu.agent.AgentRuntime.hidden()) {
            SDLVideo.SDL_HideWindow(cir.getReturnValueJ());
        }
    }
}
