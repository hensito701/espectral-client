package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.main.GameConfig;

/**
 * Neutralizes Minecraft's client shutdown watchdog during AOT training.
 *
 * MC 26.2 arms a 15-second ClientShutdownWatchdog when the client returns
 * from main ("post-main"): if the JVM is still alive 15s later, the watchdog
 * writes a crash report and calls System.exit(-8). A launcher AOT training run
 * (-XX:AOTCacheOutput, JEP 514) closes the game at the menu and the JVM then
 * dumps a 100+MB AOT config on the normal-exit path — which ALWAYS exceeds
 * the watchdog budget. The watchdog fires mid-write, aborts the shutdown, and
 * the cache is discarded (observed 2026-08-31: crash-2026-08-31_00.02.14,
 * "Client shutdown from post-main", game.aot never materialized).
 *
 * This mixin cancels the watchdog thread spawn entirely when the launcher set
 * -Despectral.aot-training=1 on the training JVM (launch.mjs adds the property
 * for every train-mode launch). Normal (player) launches are untouched —
 * the watchdog keeps protecting them.
 *
 * 26.2-only: the class does not exist in 1.21.11 (verified against the 1.21.11
 * client jar), so this mixin lives in the mc262 source set, not the shared one.
 */
@Mixin(targets = "com.mojang.blaze3d.platform.ClientShutdownWatchdog")
public abstract class ClientShutdownWatchdogMixin {

    @Inject(method = "startShutdownWatchdog", at = @At("HEAD"), cancellable = true)
    private static void espectral$skipDuringAotTraining(String reason, boolean exitOnTimeout,
            Minecraft minecraft, GameConfig gameConfig, long threadId, CallbackInfo ci) {
        if (System.getProperty("espectral.aot-training") != null) {
            ci.cancel();
        }
    }
}
