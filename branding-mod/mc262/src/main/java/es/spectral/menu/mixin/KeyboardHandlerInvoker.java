package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

import net.minecraft.client.KeyboardHandler;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;

/**
 * Reaches {@code KeyboardHandler.keyPress}/{@code charTyped} through the same
 * entry points real keyboard input uses. Both are private on 26.2 and
 * 1.21.11 (identical descriptors, so the same invoker serves both lanes);
 * 26.3 made them public and calls them directly from its own
 * {@code AgentInputCompat}, so this mixin is not registered there.
 */
@Mixin(KeyboardHandler.class)
public interface KeyboardHandlerInvoker {

    @Invoker("keyPress")
    void espectral$keyPress(long window, int action, KeyEvent event);

    @Invoker("charTyped")
    void espectral$charTyped(long window, CharacterEvent event);
}
