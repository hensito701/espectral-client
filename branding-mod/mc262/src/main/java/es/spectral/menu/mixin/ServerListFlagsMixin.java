package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.multiplayer.ServerSelectionList;
import net.minecraft.client.multiplayer.ServerData;

import es.spectral.menu.geoip.RowExtras;
import es.spectral.menu.geoip.RowExtrasLayout;

/**
 * Multiplayer server list, 26.2/26.3 flavour: appends the ping in milliseconds
 * and the country flag to each online row, right of the vanilla player count.
 *
 * <p>26.x split rendering into an extraction pass, so the hook sits at the tail
 * of {@code OnlineServerEntry.extractContent} — after the name, MOTD, icon,
 * ping bars and player count were submitted. Records are emitted in call order,
 * so the extras land on top, and the explicit mouse hit-tests in
 * {@link RowExtras} queue the flag tooltip last.
 *
 * <p>Geometry comes from the entry itself ({@code getContentX/Y/Right()}), so
 * the layout can never drift from the row being drawn. See {@link RowExtras}
 * for the pixel offsets. 26.3 shares this source unchanged.
 */
@Mixin(ServerSelectionList.OnlineServerEntry.class)
public abstract class ServerListFlagsMixin {

    /** Per-entry slot rectangles, reused every frame (no per-frame allocation). */
    @Unique
    private final RowExtrasLayout espectral$rowExtras = new RowExtrasLayout();

    @Inject(
            method = "extractContent(Lnet/minecraft/client/gui/GuiGraphicsExtractor;IIZF)V",
            at = @At("TAIL")
    )
    private void espectral$extractRowExtras(GuiGraphicsExtractor gfx, int mouseX, int mouseY,
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
