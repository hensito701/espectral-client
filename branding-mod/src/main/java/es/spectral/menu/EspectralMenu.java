package es.spectral.menu;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.multiplayer.JoinMultiplayerScreen;
import net.minecraft.client.gui.screens.worldselection.SelectWorldScreen;
import net.minecraft.network.chat.Component;

/**
 * Title-screen replacement that stays visually close to the vanilla menu:
 * the render path is untouched (panorama, logo and splash are vanilla), and
 * only the button set is built here — the classic Singleplayer / Multiplayer /
 * Options / Quit column with vanilla-styled buttons and translation keys.
 *
 * This is the owned hook: the look is deliberately conservative for now and
 * can be customized later without touching the rest of the client.
 */
public final class EspectralMenu {

    private EspectralMenu() {}

    /** Vanilla-style title buttons (Singleplayer / Multiplayer / Options / Quit). */
    public static List<Button> buttons(Screen screen) {
        Minecraft minecraft = Minecraft.getInstance();
        int x = (screen.width - 200) / 2;
        int y = screen.height / 4 + 48;
        List<Button> out = new ArrayList<>(5);
        out.add(Button.builder(Component.literal("§6★ Espectral Client"),
                b -> Compat.open(minecraft, new EspectralClientScreen(screen)))
                .bounds(x, y - 24, 200, 20).build());
        out.add(Button.builder(Component.translatable("menu.singleplayer"),
                b -> Compat.open(minecraft, new SelectWorldScreen(screen)))
                .bounds(x, y, 200, 20).build());
        out.add(Button.builder(Component.translatable("menu.multiplayer"),
                b -> Compat.open(minecraft, new JoinMultiplayerScreen(screen)))
                .bounds(x, y + 24, 200, 20).build());
        out.add(Button.builder(Component.translatable("menu.options"),
                b -> Compat.open(minecraft, Compat.optionsScreen(minecraft, screen)))
                .bounds(x, y + 48, 200, 20).build());
        out.add(Button.builder(Component.translatable("menu.quit"),
                b -> minecraft.stop())
                .bounds(x, y + 72, 200, 20).build());
        return out;
    }
}
