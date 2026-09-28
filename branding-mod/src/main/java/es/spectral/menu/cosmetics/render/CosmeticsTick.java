package es.spectral.menu.cosmetics.render;

import net.minecraft.client.Minecraft;

import es.spectral.menu.cosmetics.Cosmetics;

/**
 * Client-tick driver for the cosmetic renderers: the shared cosmetics state
 * service (MOD-CORE) plus the per-wearer cloth simulation. Wired from
 * {@code CosmeticsTickMixin}; does no I/O itself.
 */
public final class CosmeticsTick {

    private CosmeticsTick() {}

    public static void onClientTick(Minecraft minecraft) {
        Cosmetics.onClientTick(minecraft);
        WearerSims.tick(minecraft);
    }

    /** World change / disconnect: every wearer's sim state is stale. */
    public static void onDisconnect() {
        WearerSims.clear();
    }
}
