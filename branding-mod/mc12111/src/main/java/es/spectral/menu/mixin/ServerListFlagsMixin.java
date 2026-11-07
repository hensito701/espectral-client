package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.multiplayer.ServerSelectionList;
import net.minecraft.client.multiplayer.ServerData;

import es.spectral.menu.geoip.RowExtras;
import es.spectral.menu.geoip.RowExtrasLayout;

/**
 * Multiplayer server list, 1.21.11 flavour: appends the ping in milliseconds
 * and the country flag to each online row, right of the vanilla player count.
 *
 * <p>The injection sits at the tail of {@code
 * OnlineServerEntry.renderContent}, i.e. after vanilla drew the name, MOTD,
 * icon, ping bars, player count and its own tooltips. The extras therefore
 * paint above them, and the explicit mouse hit-tests in {@link RowExtras}
 * (the same shape vanilla uses there) queue our flag tooltip last, which is
 * why it wins.
 *
 * <p>Geometry is read from the entry itself ({@code getContentX/Y/Right()})
 * rather than the vanilla locals, so the layout can never drift from the row
 * the user is looking at. See {@link RowExtras} for the exact pixel offsets.
 */
@Mixin(ServerSelectionList.OnlineServerEntry.class)
public abstract class ServerListFlagsMixin {

    /** Per-entry slot rectangles, reused every frame (no per-frame allocation). */
    @Unique
    private final RowExtrasLayout espectral$rowExtras = new RowExtrasLayout();

    @Inject(
            method = "renderContent(Lnet/minecraft/client/gui/GuiGraphics;IIZF)V",
            at = @At("TAIL")
    )
    private void espectral$drawRowExtras(GuiGraphics gfx, int mouseX, int mouseY,
            boolean hovered, float delta, CallbackInfo ci) {
        ServerSelectionList.OnlineServerEntry self = (ServerSelectionList.OnlineServerEntry) (Object) this;
        ServerData data = self.getServerData();
        Minecraft minecraft = Minecraft.getInstance();
        if (data == null || minecraft == null || minecraft.font == null) {
            return;
        }
        RowExtras.render(gfx, minecraft.font, espectral$rowExtras, data,
                self.getContentX(), self.getContentY(), self.getContentRight(),
                mouseX, mouseY);
    }
}
