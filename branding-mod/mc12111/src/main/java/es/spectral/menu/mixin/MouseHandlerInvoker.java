package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

import net.minecraft.client.MouseHandler;
import net.minecraft.client.input.MouseButtonInfo;

/**
 * Reaches {@code MouseHandler.onButton}/{@code onMove} so agent clicks behave
 * exactly like a real mouse: hover state, double-click detection and screen
 * routing all run through vanilla.
 *
 * <p>26.2/1.21.11 only: both methods are public on 26.3 (whose {@code onMove}
 * also takes relative deltas), and that lane calls them directly from its own
 * {@code AgentInputCompat} instead of registering this mixin.
 */
@Mixin(MouseHandler.class)
public interface MouseHandlerInvoker {

    @Invoker("onButton")
    void espectral$onButton(long window, MouseButtonInfo button, int action);

    @Invoker("onMove")
    void espectral$onMove(long window, double x, double y);
}
