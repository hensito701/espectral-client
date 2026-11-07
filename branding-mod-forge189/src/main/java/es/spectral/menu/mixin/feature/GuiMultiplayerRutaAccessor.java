package es.spectral.menu.mixin.feature;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

import net.minecraft.client.gui.GuiMultiplayer;

/**
 * Ruta Espectral: re-runs the server list ping after the toggle flips, so the
 * displayed number (and the flag / status text) reflect the routed connection
 * instead of the direct one.
 */
@Mixin(GuiMultiplayer.class)
public interface GuiMultiplayerRutaAccessor {

    @Invoker("refreshServerList")
    void espectral$refreshServerList();
}
