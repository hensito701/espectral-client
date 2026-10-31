package es.spectral.menu.mixin;

import org.lwjgl.glfw.GLFW;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import com.mojang.blaze3d.platform.Window;

/**
 * Hidden agent launches ({@code -Despectral.agent.hidden=1}, set by the
 * launcher only for agent launches): hide the GLFW window right after it is
 * created, before the first frame is presented. Rendering continues into the
 * main render target, which is what agent captures read.
 */
@Mixin(Window.class)
public abstract class AgentHiddenWindowMixin {

    @Inject(method = "createWindow", at = @At("RETURN"))
    private void espectral$agentHide(CallbackInfoReturnable<Long> cir) {
        if (es.spectral.menu.agent.AgentRuntime.hidden()) {
            GLFW.glfwHideWindow(cir.getReturnValueJ());
        }
    }
}
