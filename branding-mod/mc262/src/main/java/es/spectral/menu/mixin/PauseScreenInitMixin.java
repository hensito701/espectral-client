package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

import es.spectral.menu.Compat;
import es.spectral.menu.ui.SuiteConfirmScreen;
import es.spectral.menu.ui.SuiteScreen;

/**
 * 26.2 only: inserts the Espectral Client and Apoyar buttons into the vanilla
 * Esc-screen button column, below the Options row and above Disconnect, at
 * the vanilla full-button width and position. Vanilla buttons are never
 * removed, covered or reordered: the two new rows take the disconnect slot
 * and the slot below it, and only the disconnect button moves (down two
 * pitches). Every {@code init()} starts from a cleared widget list (vanilla
 * {@code rebuildWidgets}), so re-init/resize never stacks widgets.
 */
@Mixin(PauseScreen.class)
public abstract class PauseScreenInitMixin extends Screen {

    /**
     * Vanilla row pitch (20 px button + 4 px top cell padding); only a
     * fallback when the live buttons cannot be measured.
     */
    private static final int FALLBACK_ROW_PITCH = 24;

    @Shadow
    private Button disconnectButton;

    private PauseScreenInitMixin(Component title) {
        super(title);
    }

    @Inject(method = "init()V", at = @At("RETURN"))
    private void espectral$insertSuiteActions(CallbackInfo ci) {
        if (this.disconnectButton == null) {
            return;
        }
        Minecraft minecraft = Minecraft.getInstance();
        int dx = this.disconnectButton.getX();
        int dw = this.disconnectButton.getWidth();
        int dh = this.disconnectButton.getHeight();
        int dy = this.disconnectButton.getY();
        int pitch = espectral$rowPitch(dy);
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.pause.client").withStyle(ChatFormatting.GOLD),
                b -> Compat.open(minecraft, new SuiteScreen(this)))
                .bounds(dx, dy, dw, dh).build());
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.pause.support").withStyle(ChatFormatting.GOLD),
                b -> Compat.open(minecraft, SuiteConfirmScreen.support(this)))
                .bounds(dx, dy + pitch, dw, dh).build());
        this.disconnectButton.setY(dy + 2 * pitch);
    }

    /**
     * Vertical distance between the disconnect row and the options row above
     * it, measured from the live vanilla buttons so both pause layouts work
     * (singleplayer half-row and full-width Options).
     */
    private int espectral$rowPitch(int disconnectY) {
        int best = Integer.MIN_VALUE;
        for (GuiEventListener child : this.children()) {
            if (child instanceof Button button && child != this.disconnectButton) {
                int y = button.getY();
                if (y < disconnectY && y > best) {
                    best = y;
                }
            }
        }
        if (best == Integer.MIN_VALUE) {
            return FALLBACK_ROW_PITCH;
        }
        int pitch = disconnectY - best;
        return pitch > 0 ? pitch : FALLBACK_ROW_PITCH;
    }
}
