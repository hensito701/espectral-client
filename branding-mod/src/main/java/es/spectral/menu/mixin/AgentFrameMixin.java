package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

/**
 * Agent capture hook at the end of {@code Minecraft.runTick} — the one frame
 * boundary all three lanes share (26.2/26.3 render inside it through
 * {@code renderFrame}, 1.21.11 renders through {@code GameRenderer.render}
 * then blits). By then the GUI pass is done and the main render target still
 * holds the finished frame (a blit copies; it does not clear), so
 * {@code screenshot} grabs exactly what the player saw, Iris composite
 * included.
 *
 * <p>One volatile check when agent mode is off; nothing else runs.
 */
@Mixin(Minecraft.class)
public abstract class AgentFrameMixin {

    @Inject(method = "runTick(Z)V", at = @At("TAIL"))
    private void espectral$agentFrame(boolean tick, CallbackInfo ci) {
        if (es.spectral.menu.agent.AgentRuntime.enabled()) {
            es.spectral.menu.agent.AgentCapture.onFrameEnd((Minecraft) (Object) this);
            es.spectral.menu.agent.AgentRecorder.onFrameEnd((Minecraft) (Object) this);
            es.spectral.menu.agent.AgentFrames.onFrameEnd();
        }
    }
}
