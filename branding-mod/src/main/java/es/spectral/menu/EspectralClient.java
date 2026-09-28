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

    /** Radial emote wheel key for local-only MVP emotes (default B). */
    public static net.minecraft.client.KeyMapping EMOTE_KEY;

    private static final net.minecraft.client.KeyMapping.Category ESPECTRAL_CATEGORY =
            net.minecraft.client.KeyMapping.Category.register(
                    net.minecraft.resources.Identifier.fromNamespaceAndPath(
                            "espectral-menu", "espectral"));
    private static boolean clientKeyRegistered = false;

    @Override
    public void onInitializeClient() {
        CLIENT_KEY = new net.minecraft.client.KeyMapping("key.espectral.client", com.mojang.blaze3d.platform.InputConstants.Type.KEYSYM, org.lwjgl.glfw.GLFW.GLFW_KEY_RIGHT_SHIFT, net.minecraft.client.KeyMapping.Category.GAMEPLAY);
        ZOOM_KEY = new net.minecraft.client.KeyMapping("key.espectral.zoom", com.mojang.blaze3d.platform.InputConstants.Type.KEYSYM, org.lwjgl.glfw.GLFW.GLFW_KEY_Z, net.minecraft.client.KeyMapping.Category.GAMEPLAY);
        EMOTE_KEY = new net.minecraft.client.KeyMapping("key.espectral.emotes", com.mojang.blaze3d.platform.InputConstants.Type.KEYSYM, org.lwjgl.glfw.GLFW.GLFW_KEY_B, ESPECTRAL_CATEGORY);
        String version = EspectralBrand.modVersion();
        if (version == null) version = "dev";
        LOGGER.info("Initializing Espectral Client mod (v{})", version);
        ClientConfig.getInstance().load();
    }

    /**
     * Appends CLIENT_KEY, ZOOM_KEY and EMOTE_KEY to Options#keyMappings once
     * the options instance exists, so they are listed in Controls and saved
     * to options.txt. Idempotent; called from the client tick until it
     * succeeds.
     */
    public static void ensureClientKeyRegistered() {
        if (clientKeyRegistered || CLIENT_KEY == null || ZOOM_KEY == null
                || EMOTE_KEY == null) return;
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null || minecraft.options == null) return;
        net.minecraft.client.KeyMapping[] all = {CLIENT_KEY, ZOOM_KEY, EMOTE_KEY};
        net.minecraft.client.KeyMapping[] current = minecraft.options.keyMappings;
        int missing = 0;
        for (net.minecraft.client.KeyMapping key : all) {
            boolean present = false;
            for (net.minecraft.client.KeyMapping mapping : current) {
                if (mapping == key) {
                    present = true;
                    break;
                }
            }
            if (!present) missing++;
        }
        if (missing == 0) {
            clientKeyRegistered = true;
            return;
        }
        net.minecraft.client.KeyMapping[] extended =
                Arrays.copyOf(current, current.length + missing);
        int at = current.length;
        for (net.minecraft.client.KeyMapping key : all) {
            boolean present = false;
            for (net.minecraft.client.KeyMapping mapping : current) {
                if (mapping == key) {
                    present = true;
                    break;
                }
            }
            if (!present) extended[at++] = key;
        }
        ((OptionsAccessor) minecraft.options).espectral$setKeyMappings(extended);
    }
}
