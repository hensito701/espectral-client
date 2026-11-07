package es.spectral.menu.ruta;

/**
 * Failure of the Ruta Espectral relay handshake. Carries the relay's error
 * code ({@code proto|auth|expired|notmvp|dest|limit|connect|busy}) or one of the
 * client-only codes ({@code unreachable}, {@code nogrant}, {@code notmvp_local},
 * {@code timeout}) so the UI can localize it. No Minecraft types here: the
 * relay test harness throws and catches this class directly.
 */
public final class RutaHandshakeException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    private final String code;

    public RutaHandshakeException(String code, String message) {
        super(message == null ? "Ruta Espectral: " + code : message);
        this.code = code == null ? "proto" : code;
    }

    public String code() {
        return this.code;
    }
}
