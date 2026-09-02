package es.spectral.menu;

import java.util.Arrays;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.fabricmc.api.ClientModInitializer;
import net.minecraft.client.Minecraft;
import es.spectral.menu.mixin.OptionsAccessor;

/**
 * Mod entrypoint for the Espectral Client Fabric mod.
 */
public final class EspectralClient implements ClientModInitializer {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    public static net.minecraft.client.KeyMapping CLIENT_KEY;

    /** Hold-to-zoom key for the native ZoomEngine (default Z). */
    public static net.minecraft.client.KeyMapping ZOOM_KEY;

    private static boolean clientKeyRegistered = false;

    @Override
    public void onInitializeClient() {
        CLIENT_KEY = new net.minecraft.client.KeyMapping("key.espectral.client", com.mojang.blaze3d.platform.InputConstants.Type.KEYSYM, org.lwjgl.glfw.GLFW.GLFW_KEY_RIGHT_SHIFT, net.minecraft.client.KeyMapping.Category.GAMEPLAY);
        ZOOM_KEY = new net.minecraft.client.KeyMapping("key.espectral.zoom", com.mojang.blaze3d.platform.InputConstants.Type.KEYSYM, org.lwjgl.glfw.GLFW.GLFW_KEY_Z, net.minecraft.client.KeyMapping.Category.GAMEPLAY);
        String version = getClass().getPackage().getImplementationVersion();
        if (version == null) version = "1.3.0";
        LOGGER.info("Initializing Espectral Client mod (v{})", version);
        ClientConfig.getInstance().load();
    }

    /**
     * Appends CLIENT_KEY and ZOOM_KEY to Options#keyMappings once the options
     * instance exists, so they are listed in Controls and saved to options.txt.
     * Idempotent; called from the client tick until it succeeds.
     */
    public static void ensureClientKeyRegistered() {
        if (clientKeyRegistered || CLIENT_KEY == null || ZOOM_KEY == null) return;
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null || minecraft.options == null) return;
        net.minecraft.client.KeyMapping[] current = minecraft.options.keyMappings;
        boolean clientPresent = false;
        boolean zoomPresent = false;
        for (net.minecraft.client.KeyMapping mapping : current) {
            if (mapping == CLIENT_KEY) clientPresent = true;
            if (mapping == ZOOM_KEY) zoomPresent = true;
        }
        if (clientPresent && zoomPresent) {
            clientKeyRegistered = true;
            return;
        }
        int missing = (clientPresent ? 0 : 1) + (zoomPresent ? 0 : 1);
        net.minecraft.client.KeyMapping[] extended = Arrays.copyOf(current, current.length + missing);
        int at = current.length;
        if (!clientPresent) extended[at++] = CLIENT_KEY;
        if (!zoomPresent) extended[at] = ZOOM_KEY;
        ((OptionsAccessor) minecraft.options).espectral$setKeyMappings(extended);
    }
}
