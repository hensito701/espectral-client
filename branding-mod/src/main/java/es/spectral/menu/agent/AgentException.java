package es.spectral.menu.agent;

/**
 * A tool failure with a protocol error code ({@code TIER_REFUSED},
 * {@code PAUSED}, {@code NO_SCREEN}, …). The relay forwards {@link #code()}
 * and {@link #getMessage()} verbatim to the engine, so the codes must stay in
 * the contract's uppercase set.
 */
public final class AgentException extends Exception {

    private final String code;

    public AgentException(String code, String message) {
        super(message);
        this.code = code;
    }

    public String code() {
        return code;
    }
}
