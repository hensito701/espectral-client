package es.spectral.menu.agent;

/** Tool failure with a protocol error code; the relay forwards both verbatim. */
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
