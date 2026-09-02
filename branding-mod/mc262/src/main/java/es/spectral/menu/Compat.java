package es.spectral.menu;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.options.OptionsScreen;

/** Screen-opening compatibility for 26.2 (setScreen moved to Gui). */
public final class Compat {

    private Compat() {}

    public static void open(Minecraft minecraft, Screen screen) {
        minecraft.setScreenAndShow(screen);
    }

    public static Screen optionsScreen(Minecraft minecraft, Screen back) {
        return new OptionsScreen(back, minecraft.options, false);
    }

    public static boolean isScreenOpen(Minecraft minecraft) {
        return minecraft.gui != null && minecraft.gui.screen() != null;
    }

    public static Screen getScreen(Minecraft minecraft) {
        return minecraft.gui != null ? minecraft.gui.screen() : null;
    }

    public static void sendChat(Minecraft minecraft, String text) {
        if (minecraft.player != null && minecraft.player.connection != null) {
            if (text.startsWith("/")) {
                minecraft.player.connection.sendCommand(text.substring(1));
            } else {
                minecraft.player.connection.sendChat(text);
            }
        }
    }
}
