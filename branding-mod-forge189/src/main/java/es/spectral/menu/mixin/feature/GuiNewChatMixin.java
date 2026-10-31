package es.spectral.menu.mixin.feature;

import java.util.List;
import java.util.UUID;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import net.minecraft.client.gui.ChatLine;
import net.minecraft.client.gui.FontRenderer;
import net.minecraft.client.gui.GuiNewChat;
import net.minecraft.client.network.NetworkPlayerInfo;
import net.minecraft.util.IChatComponent;

import es.spectral.menu.ChatHeads;

/**
 * Chat heads ({@code chatheads}): the sender is resolved once per message in
 * {@code setChatLine} and bound to the message's first display line; while
 * drawing, a mapped line gets the sender's head and its text shifted right by
 * {@link ChatHeads#SHIFT}, with the line's fade alpha.
 */
@Mixin(GuiNewChat.class)
public abstract class GuiNewChatMixin {

    @Unique private UUID espectral$sender;
    @Unique private boolean espectral$firstLine;
    @Unique private ChatLine espectral$drawing;

    @Inject(method = "setChatLine", at = @At("HEAD"))
    private void espectral$resolveSender(IChatComponent component, int chatLineId, int updateCounter,
            boolean displayOnly, CallbackInfo ci) {
        espectral$sender = ChatHeads.senderForText(component.getUnformattedText());
        espectral$firstLine = true;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    @Redirect(method = "setChatLine",
            at = @At(value = "INVOKE", target = "Ljava/util/List;add(ILjava/lang/Object;)V", ordinal = 0))
    private void espectral$mapFirstLine(List list, int index, Object line) {
        list.add(index, line);
        if (espectral$firstLine) {
            espectral$firstLine = false;
            ChatHeads.mapFirstLine((ChatLine) line, espectral$sender);
        }
    }

    @Inject(method = "clearChatMessages", at = @At("HEAD"))
    private void espectral$clear(CallbackInfo ci) {
        ChatHeads.clear();
    }

    @SuppressWarnings("rawtypes")
    @Redirect(method = "drawChat", at = @At(value = "INVOKE", target = "Ljava/util/List;get(I)Ljava/lang/Object;"))
    private Object espectral$trackLine(List list, int index) {
        Object line = list.get(index);
        espectral$drawing = (ChatLine) line;
        return line;
    }

    @Redirect(method = "drawChat", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/FontRenderer;drawStringWithShadow(Ljava/lang/String;FFI)I"))
    private int espectral$drawWithHead(FontRenderer font, String text, float x, float y, int color) {
        NetworkPlayerInfo info = ChatHeads.infoFor(espectral$drawing);
        if (info == null) return font.drawStringWithShadow(text, x, y, color);
        ChatHeads.drawHead(info, (int) x, (int) y, color >>> 24);
        return font.drawStringWithShadow(text, x + ChatHeads.SHIFT, y, color);
    }
}
