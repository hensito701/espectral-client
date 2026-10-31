package es.spectral.menu;

import java.io.InputStream;
import java.util.Properties;

/**
 * Runtime version metadata: the launcher's package.json version (1.7.0 at the
 * time of writing) is injected into {@code espectral-menu.properties} at build
 * time by {@code processResources}, so it is correct both in a remapped jar and
 * in a {@code runClient} dev launch.
 */
public final class Version {

    private static final String VERSION;
    private static final String MINECRAFT;

    static {
        String version = "dev";
        String minecraft = "1.8.9";
        try (InputStream in = Version.class.getResourceAsStream("/espectral-menu.properties")) {
            if (in != null) {
                Properties props = new Properties();
                props.load(in);
                version = props.getProperty("version", version);
                minecraft = props.getProperty("minecraft", minecraft);
            }
        } catch (Exception ignored) {
            // fall through to the defaults: a broken resource must never kill the client
        }
        VERSION = version;
        MINECRAFT = minecraft;
    }

    private Version() {}

    /** Espectral Client release, from the launcher's package.json. */
    public static String version() {
        return VERSION;
    }

    public static String minecraft() {
        return MINECRAFT;
    }

    /** What the window title and F3 line advertise. */
    public static String clientName() {
        return "Espectral Client " + MINECRAFT;
    }
}
