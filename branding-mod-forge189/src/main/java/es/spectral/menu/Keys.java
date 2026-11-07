package es.spectral.menu;

import org.lwjgl.input.Keyboard;

import net.minecraft.client.settings.KeyBinding;
import net.minecraftforge.fml.client.registry.ClientRegistry;

import es.spectral.menu.agent.AgentRuntime;

/**
 * The mod's key bindings, listed under "Espectral" in Controls and saved in
 * options.txt: Right Shift opens the Suite, Z (the registry's zoom keybind)
 * zooms, and — in agent launches only — F8 pauses/resumes the agent.
 * K plays or pauses the Media Player; period and comma skip to the next and previous track.
 */
public final class Keys {

    private static final String CATEGORY = "key.categories.espectral";

    public static KeyBinding CLIENT;
    public static KeyBinding ZOOM;
    public static KeyBinding AGENT_PAUSE;
    public static KeyBinding MEDIA_PLAY;
    public static KeyBinding MEDIA_NEXT;
    public static KeyBinding MEDIA_PREV;

    private Keys() {}

    public static void register() {
        CLIENT = new KeyBinding("key.espectral.client", Keyboard.KEY_RSHIFT, CATEGORY);
        FeatureRegistry.Feature zoom = FeatureRegistry.findById("zoom");
        int zoomKey = zoom != null && zoom.keybind != null ? KeyNames.keyCode(zoom.keybind) : -1;
        ZOOM = new KeyBinding("key.espectral.zoom", zoomKey > 0 ? zoomKey : Keyboard.KEY_Z, CATEGORY);
        ClientRegistry.registerKeyBinding(CLIENT);
        ClientRegistry.registerKeyBinding(ZOOM);
        MEDIA_PLAY = new KeyBinding("key.espectral.media.play", Keyboard.KEY_K, CATEGORY);
        MEDIA_NEXT = new KeyBinding("key.espectral.media.next", Keyboard.KEY_PERIOD, CATEGORY);
        MEDIA_PREV = new KeyBinding("key.espectral.media.prev", Keyboard.KEY_COMMA, CATEGORY);
        ClientRegistry.registerKeyBinding(MEDIA_PLAY);
        ClientRegistry.registerKeyBinding(MEDIA_NEXT);
        ClientRegistry.registerKeyBinding(MEDIA_PREV);
        if (AgentRuntime.enabled()) {
            AGENT_PAUSE = new KeyBinding("key.espectral.agent.pause", Keyboard.KEY_F8, CATEGORY);
            ClientRegistry.registerKeyBinding(AGENT_PAUSE);
        }
    }
}
