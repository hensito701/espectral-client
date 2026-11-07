package es.spectral.menu.ruta;

import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;

/**
 * Localized, player-facing Ruta Espectral messages (es/en come from the
 * generated lang files; see {@code branding-mod/lang-extra.json}). Every text
 * names Ruta Espectral and tells the player how to get out of the failure —
 * "turn Ruta off for that server" — because there is deliberately no silent
 * direct fallback.
 */
public final class RutaMessages {

    private RutaMessages() {}

    /** Ordinary failure text for an error code. */
    public static Component failure(String code) {
        return Component.translatable("espectral.ruta.err." + key(code));
    }

    /** Failure text plus the "turn it off for this server" hint, for screens. */
    public static Component failureWithHint(String code) {
        return Component.translatable("espectral.ruta.err." + key(code))
                .append(Component.literal(" "))
                .append(Component.translatable("espectral.ruta.hint"));
    }

    /** Status-list line: "Ruta Espectral: <reason> · turn it off for this server". */
    public static MutableComponent status(String code) {
        return Component.translatable("espectral.ruta.status",
                Component.translatable("espectral.ruta.short." + key(code)))
                .withStyle(ChatFormatting.DARK_RED);
    }

    /** Short protocol tag ("Ruta") for compact spots. */
    public static Component name() {
        return Component.translatable("espectral.ruta.name");
    }

    static String key(String code) {
        if (code == null) return "proto";
        switch (code) {
            case "proto":
            case "auth":
            case "expired":
            case "notmvp":
            case "dest":
            case "limit":
            case "connect":
            case "busy":
            case "unreachable":
            case "nogrant":
            case "notmvp_local":
            case "timeout":
                return code;
            default:
                return "proto";
        }
    }
}
