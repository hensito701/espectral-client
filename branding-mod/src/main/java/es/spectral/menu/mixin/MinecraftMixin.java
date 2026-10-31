package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.EspectralClient;
import es.spectral.menu.EmoteEngine;
import es.spectral.menu.HudEngine;
import es.spectral.menu.MacroEngine;
import es.spectral.menu.SkinModelCache;
import es.spectral.menu.SuiteShortcut;
import es.spectral.menu.ZoomEngine;

/**
 * Hooks into Minecraft.tick() to handle the Right Shift GUI keybind, native
 * zoom, macro triggers, visual-feature transition logs, and the live
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
        SuiteShortcut.onClientTick(minecraft);
        ZoomEngine.getInstance().onTick(minecraft);
        MacroEngine.getInstance().onTick(minecraft);
        HudEngine.getInstance().onTick(minecraft);
        EmoteEngine.getInstance().onClientTick(minecraft);
        es.spectral.menu.ui.MenuSkin.onClientTick(minecraft);
        // Launcher PATCH writes the config file while the game runs; pick it
        // up live without hammering the disk. A reload right after our own
        // screen-save just re-reads identical content (harmless).
        if ((espectral$tickCount++ % 20) == 0) {
            ClientConfig.getInstance().maybeReload();
        }
    }

    /**
     * Agent relay housekeeping (expiring key leases, the F8 pause sample,
     * starting the relay thread). One volatile read when agent mode is off.
     */
    @Inject(method = "tick()V", at = @At("TAIL"))
    private void espectral$agentTick(CallbackInfo ci) {
        es.spectral.menu.agent.AgentTick.onClientTick((Minecraft) (Object) this);
    }

    /**
     * Leaving a server or world. Drops baked 3D Skin Layers voxel models
     * (they rebuild lazily on the next join) and logs the marker the
     * launcher watches to put Discord presence back to "main menu"
     * ({@code PRESENCE_MENU_MARKER} in routes/launch.mjs — keep in sync).
     * The 2-arg overload delegates to this 3-arg one in both supported
     * versions, so a single hook covers every leave/kick path.
     */
    @Inject(method = "disconnect(Lnet/minecraft/client/gui/screens/Screen;ZZ)V", at = @At("TAIL"))
    private void espectral$onDisconnect(net.minecraft.client.gui.screens.Screen screen,
            boolean showDisconnectScreen, boolean teleporting, CallbackInfo ci) {
        SkinModelCache.clear();
        org.slf4j.LoggerFactory.getLogger("espectral-client").info("[espectral] presence: menu");
    }
}
