package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.EspectralClient;
import es.spectral.menu.FogEngine;
import es.spectral.menu.GammaEngine;
import es.spectral.menu.HudEngine;
import es.spectral.menu.MacroEngine;
import es.spectral.menu.SkinModelCache;
import es.spectral.menu.ZoomEngine;

/**
 * Hooks into Minecraft.tick() to handle the Right Shift GUI keybind, the
 * native zoom/fullbright/nofog engines, macro triggers, and the live
 * launcher-patch config reload.
 */
@Mixin(Minecraft.class)
public abstract class MinecraftMixin {

    /** Tick counter for throttling the launcher-patch config reload. */
    private int espectral$tickCount;

    @Inject(method = "tick()V", at = @At("TAIL"))
    private void espectral$onClientTick(CallbackInfo ci) {
        EspectralClient.ensureClientKeyRegistered();
        Minecraft minecraft = (Minecraft) (Object) this;
        ZoomEngine.getInstance().onTick(minecraft);
        GammaEngine.getInstance().onTick(minecraft);
        FogEngine.getInstance().onTick(minecraft);
        MacroEngine.getInstance().onTick(minecraft);
        HudEngine.getInstance().onTick(minecraft);
        // Launcher PATCH writes the config file while the game runs; pick it
        // up live without hammering the disk. A reload right after our own
        // screen-save just re-reads identical content (harmless).
        if ((espectral$tickCount++ % 20) == 0) {
            ClientConfig.getInstance().maybeReload();
        }
    }
    /**
     * Drops baked 3D Skin Layers voxel models on disconnect. The 2-arg
     * overload delegates to this 3-arg one in both supported versions, so a
     * single hook covers every leave/kick path; entries rebuild lazily on the
     * next world join.
     */
    @Inject(method = "disconnect(Lnet/minecraft/client/gui/screens/Screen;ZZ)V", at = @At("TAIL"))
    private void espectral$onDisconnect(net.minecraft.client.gui.screens.Screen screen,
            boolean showDisconnectScreen, boolean teleporting, CallbackInfo ci) {
        SkinModelCache.clear();
    }
}
