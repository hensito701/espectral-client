package es.spectral.menu.agent;

/**
 * Agent-mode switch. Everything the agent relay does is gated on
 * {@code -Despectral.agent=1}, which the launcher passes only for launches
 * started with {@code agent: true}. Without it: no relay thread, no DNS
 * resolver, no per-tick work beyond one volatile read.
 */
public final class AgentRuntime {

    /** Read once: a JVM property cannot change while the game runs. */
    private static final boolean ENABLED = "1".equals(System.getProperty("espectral.agent"));

    /** Capture root, relative to the game directory (contract-mandated name). */
    public static final String CAPTURES_DIR = "espectral-captures";

    private AgentRuntime() {}

    public static boolean enabled() {
        return ENABLED;
    }

    /** Bridge base URL ({@code …/api/bridge}) or null. */
    public static String bridgeUrl() {
        String url = System.getProperty("espectral.bridge.url");
        if (url == null || url.isEmpty()) return null;
        return url.endsWith("/") ? url.substring(0, url.length() - 1) : url;
    }

    /** Per-launch bridge token or null. */
    public static String bridgeToken() {
        String token = System.getProperty("espectral.bridge.token");
        return token == null || token.isEmpty() ? null : token;
    }
}
