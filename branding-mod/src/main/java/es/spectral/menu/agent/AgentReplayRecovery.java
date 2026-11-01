package es.spectral.menu.agent;

import java.lang.reflect.Field;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.TitleScreen;

import es.spectral.menu.Compat;

/**
 * Replay Mod asks "recover it?" at startup for every replay a crashed or
 * killed game left unfinished ({@code *.mcpr.tmp}). Its dialog exposes no
 * widgets, so an agent could not answer it, and one prompt per leftover file
 * blocked agent launches before the title screen.
 *
 * <p>Agent launches step over those prompts without answering them: nothing is
 * recovered or discarded, and the files stay for the player's next normal
 * launch, where Replay Mod asks again. Reflection only (soft dependency).
 */
public final class AgentReplayRecovery {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");
    private static final String RESTORE_GUI = "com.replaymod.core.gui.RestoreReplayGui";
    private static final String JGUI_PREFIX = "com.replaymod.lib.de.johni0702.minecraft.gui.";

    private static Screen lastChecked;

    private AgentReplayRecovery() {}

    /** Client tick, agent mode only. Cheap unless a Replay Mod screen is open. */
    static void onClientTick(Minecraft minecraft) {
        if (minecraft.level != null) return;
        Screen screen = Compat.getScreen(minecraft);
        if (screen == null || screen == lastChecked || !isReplayModScreen(screen)) return;
        lastChecked = screen;
        Object gui = outer(screen);
        if (gui == null || !RESTORE_GUI.equals(gui.getClass().getName())) return;
        // Each prompt's parent is the previous prompt; the first one's parent
        // wraps the screen that was open before Replay Mod's startup scan.
        int skipped = 0;
        Object parent = gui;
        while (parent != null && RESTORE_GUI.equals(parent.getClass().getName())) {
            skipped++;
            parent = field(parent, "parent");
        }
        LOGGER.info("agent: left {} unsaved Replay Mod recording(s) for the next normal launch "
                + "(recovery prompt skipped)", skipped);
        Screen under = parent != null ? vanillaScreen(parent) : null;
        Compat.open(minecraft, under != null ? under : new TitleScreen());
    }

    private static boolean isReplayModScreen(Screen screen) {
        return screen.getClass().getName().startsWith(JGUI_PREFIX);
    }

    /** The jGui screen behind its vanilla wrapper ({@code AbstractGuiScreen.MinecraftGuiScreen}). */
    private static Object outer(Object wrapper) {
        for (Class<?> c = wrapper.getClass(); c != null && c != Object.class; c = c.getSuperclass()) {
            for (Field f : c.getDeclaredFields()) {
                if (!f.getName().startsWith("this$")) continue;
                try {
                    f.setAccessible(true);
                    return f.get(wrapper);
                } catch (Throwable t) {
                    return null;
                }
            }
        }
        return null;
    }

    private static Object field(Object target, String name) {
        try {
            return target.getClass().getField(name).get(target);
        } catch (Throwable t) {
            return null;
        }
    }

    /**
     * The vanilla screen the first prompt replaced: Replay Mod wraps it with
     * {@code GuiScreen.wrap(screen)}, whose {@code toMinecraft()} returns it.
     */
    private static Screen vanillaScreen(Object jguiScreen) {
        try {
            // GuiScreen.wrap returns an anonymous (non-public) subclass.
            java.lang.reflect.Method toMinecraft = jguiScreen.getClass().getMethod("toMinecraft");
            toMinecraft.setAccessible(true);
            Object screen = toMinecraft.invoke(jguiScreen);
            return screen instanceof Screen s && !isReplayModScreen(s) ? s : null;
        } catch (Throwable t) {
            LOGGER.warn("agent: could not read the screen under the Replay Mod prompt: {}", t.toString());
            return null;
        }
    }
}
