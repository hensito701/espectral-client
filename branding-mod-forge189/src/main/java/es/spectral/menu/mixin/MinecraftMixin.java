package es.spectral.menu.mixin;

import org.lwjgl.opengl.Display;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

import es.spectral.menu.Version;
import es.spectral.menu.agent.ClientTasks;

/**
 * Window branding: vanilla calls {@code Display.setTitle("Minecraft 1.8.9")}
 * inside {@code createDisplay()}; a TAIL inject runs right after it and renames
 * the window to {@code Espectral Client 1.8.9}.
 *
 * <p>The refmap (mixins.espectralmenu.refmap.json) resolves the mapped
 * {@code createDisplay}/{@code func_175609_am} pair, so this works in the
 * dev workspace and in the remapped production jar alike.
 */
@Mixin(Minecraft.class)
public abstract class MinecraftMixin {

    @Inject(method = "createDisplay", at = @At("TAIL"))
    private void espectral$brandWindow(CallbackInfo ci) {
        try {
            Display.setTitle(Version.clientName());
        } catch (Throwable ignored) {
            // no display yet (headless/test run): branding is cosmetic
        }
    }

    /**
     * Agent tools run at the end of the client tick: runGameLoop calls runTick
     * <em>after</em> draining scheduled tasks and releases that queue's monitor
     * first, which is what makes a network-touching tool (disconnect) safe.
     */
    @Inject(method = "runTick", at = @At("TAIL"))
    private void espectral$agentTick(CallbackInfo ci) {
        ClientTasks.drain();
    }
}
