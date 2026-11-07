package es.spectral.menu.ruta;

/**
 * What the Netty side needs to know once a connection is routed: the resolved
 * destination (the relay never does DNS) and the entitlement grant verbatim.
 * Immutable, created on the connector/ping thread, read on the IO thread.
 */
public final class RutaTarget {

    /** Normalized config key of the server the player chose to route. */
    public final String serverKey;
    /** Destination IP literal (no brackets), already resolved by Minecraft. */
    public final String destIp;
    /** Destination port. */
    public final int destPort;
    /** Compact JWS of the player's verified entitlement. */
    public final String grant;

    public RutaTarget(String serverKey, String destIp, int destPort, String grant) {
        this.serverKey = serverKey;
        this.destIp = destIp;
        this.destPort = destPort;
        this.grant = grant;
    }

    /** The single ASCII line the relay expects as the first bytes. */
    public String handshakeLine() {
        return RutaProtocol.handshakeLine(this.grant, this.destIp, this.destPort);
    }

    @Override
    public String toString() {
        return "ruta -> " + this.destIp + ":" + this.destPort + " for " + this.serverKey;
    }
}
