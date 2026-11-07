package es.spectral.menu.ruta;

import es.spectral.menu.Lang;

/**
 * Localized, player-facing Ruta Espectral texts.
 *
 * <p>Strings live in the generated 1.8.9 {@code .lang} files:
 * {@code espectral.ruta.name}, {@code espectral.ruta.status} ({@code %s}),
 * {@code espectral.ruta.hint}, the long {@code espectral.ruta.err.<code>}, the
 * short {@code espectral.ruta.short.<code>} and the toggle tooltips
 * {@code espectral.ruta.tooltip.on|off|blocked}. Every failure text names Ruta
 * Espectral and says how to get out — turn it off for that server — because
 * there is deliberately no silent direct fallback.
 */
public final class RutaMessages {

    private RutaMessages() {}

    /**「Ruta Espectral」, the product name (never translated away). */
    public static String name() {
        return Lang.tr("espectral.ruta.name");
    }

    /** Long failure text for an error code. */
    public static String failure(String code) {
        return Lang.tr("espectral.ruta.err." + key(code));
    }

    /** Failure text plus the "turn it off for this server" hint, for screens. */
    public static String failureWithHint(String code) {
        return failure(code) + " " + Lang.tr("espectral.ruta.hint");
    }

    /** Short reason tag for the row status line. */
    public static String shortReason(String code) {
        return Lang.tr("espectral.ruta.short." + key(code));
    }

    /** Status line: {@code espectral.ruta.status} with the short reason. */
    public static String status(String code) {
        return Lang.tr("espectral.ruta.status", shortReason(code));
    }

    /**
     * Maps any code the client can produce onto one the lang files carry.
     * Unknown codes read as {@code proto}, which is what the relay means by
     * "I could not understand you".
     */
    public static String key(String code) {
        if (code == null) return "proto";
        if (code.equals("proto") || code.equals("auth") || code.equals("expired")
                || code.equals("notmvp") || code.equals("dest") || code.equals("limit")
                || code.equals("connect") || code.equals("busy") || code.equals("unreachable")
                || code.equals("nogrant") || code.equals("notmvp_local") || code.equals("timeout")) {
            return code;
        }
        return "proto";
    }
}
