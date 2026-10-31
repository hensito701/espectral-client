package es.spectral.menu.mixin.agent;

import org.lwjgl.input.Keyboard;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import net.minecraft.client.Minecraft;

import es.spectral.menu.agent.AgentInput;

/**
 * Feeds the agent's synthetic key events through {@code runTick}'s keyboard
 * loop: {@code Keyboard.next()} yields queued synthetic events first, and the
 * event getters answer for the synthetic event while one is current.
 */
@Mixin(Minecraft.class)
public abstract class MinecraftKeyboardMixin {

    @Redirect(method = "runTick", at = @At(value = "INVOKE", target = "Lorg/lwjgl/input/Keyboard;next()Z", remap = false))
    private boolean espectral$next() {
        return AgentInput.nextSynthetic((Minecraft) (Object) this) || Keyboard.next();
    }

    @Redirect(method = "runTick", at = @At(value = "INVOKE", target = "Lorg/lwjgl/input/Keyboard;getEventKey()I", remap = false))
    private int espectral$eventKey() {
        return AgentInput.eventKey();
    }

    @Redirect(method = "runTick", at = @At(value = "INVOKE", target = "Lorg/lwjgl/input/Keyboard;getEventKeyState()Z", remap = false))
    private boolean espectral$eventKeyState() {
        return AgentInput.eventKeyState();
    }

    @Redirect(method = "runTick", at = @At(value = "INVOKE", target = "Lorg/lwjgl/input/Keyboard;getEventCharacter()C", remap = false))
    private char espectral$eventCharacter() {
        return AgentInput.eventCharacter();
    }
}
