package es.spectral.menu.agent;

/**
 * Agent-mode configuration, read from JVM properties exactly like the modern
 * mod ({@code -Despectral.agent=1 -Despectral.bridge.url=...}); the bridge
 * token comes from the {@code ESPECTRAL_BRIDGE_TOKEN} environment variable.
 *
 * <p>Java 8: no {@code System.getProperty(name, fallback)} sugar beyond the
 * plain overloads, and no {@code readAllBytes}/{@code String.isBlank}.
 */
public final class AgentRuntime {

    /** Where screenshots land, relative to the game directory. */
    public static final String CAPTURES_DIR = "espectral-captures";

    private AgentRuntime() {}

    /** {@code -Despectral.agent=1}. */
    public static boolean enabled() {
        return "1".equals(prop("espectral.agent"));
    }

    /** {@code -Despectral.agent.hidden}: the launch should stay invisible. */
    public static boolean hidden() {
        return "1".equals(prop("espectral.agent.hidden"));
    }

    /** {@code -Despectral.bridge.url}, e.g. http://127.0.0.1:1234/api/bridge. */
    public static String bridgeUrl() {
        String url = prop("espectral.bridge.url");
        if (url == null || url.isEmpty()) return null;
        while (url.endsWith("/")) url = url.substring(0, url.length() - 1);
        return url;
    }

    /** Per-launch bridge token (environment, never the command line) or null. */
    public static String bridgeToken() {
        return es.spectral.menu.BridgeAuth.tokenFromEnv();
    }

    private static String prop(String name) {
        try {
            return System.getProperty(name);
        } catch (SecurityException e) {
            return null;
        }
    }
}
