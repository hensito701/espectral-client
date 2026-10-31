package es.spectral.menu.mixin.agent;

import java.util.List;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;
import org.spongepowered.asm.mixin.gen.Invoker;

import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;

/** Agent act tools: the protected input entry points and button list of any screen. */
@Mixin(GuiScreen.class)
public interface GuiScreenInvoker {

    @Accessor("buttonList")
    List<GuiButton> espectral$buttons();

    @Invoker("mouseClicked")
    void espectral$mouseClicked(int mouseX, int mouseY, int mouseButton);

    @Invoker("mouseReleased")
    void espectral$mouseReleased(int mouseX, int mouseY, int state);

    @Invoker("keyTyped")
    void espectral$keyTyped(char typedChar, int keyCode);
}
