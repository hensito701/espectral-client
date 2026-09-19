package es.spectral.menu;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

import es.spectral.menu.ui.SuiteConfirmScreen;
import es.spectral.menu.ui.SuiteScreen;
import es.spectral.menu.ui.SuiteTheme;

/**
 * Bottom action row for the title screen: {@code Espectral Client...} and
 * {@code Apoyar} as one right-aligned row at the top-right corner (y = 6,
 * widths 120 + 100, gap 4). Vanilla {@code init()} keeps building every
 * vanilla widget; the mixin only appends these two, so vanilla navigation,
 * splash, panorama, icon row and footer lines stay intact.
 */
public final class EspectralMenu {

    private EspectralMenu() {}

    /** Top-right action row (Espectral Client... / Apoyar) for the title screen. */
    public static List<Button> buttons(Screen screen) {
        Minecraft minecraft = Minecraft.getInstance();
        int rowX = SuiteTheme.actionRowX(screen.width);
        int y = SuiteTheme.ACTION_Y;
        List<Button> out = new ArrayList<>(2);
        out.add(Button.builder(
                Component.translatable("espectral.title.client").withStyle(ChatFormatting.GOLD),
                b -> Compat.open(minecraft, new SuiteScreen(screen)))
                .bounds(rowX, y, SuiteTheme.ACTION_CLIENT_W, 20).build());
        out.add(Button.builder(
                Component.translatable("espectral.title.support").withStyle(ChatFormatting.GOLD),
                b -> Compat.open(minecraft, SuiteConfirmScreen.support(screen)))
                .bounds(rowX + SuiteTheme.ACTION_CLIENT_W + SuiteTheme.GAP, y,
                        SuiteTheme.ACTION_SUPPORT_W, 20).build());
        return out;
    }
}
