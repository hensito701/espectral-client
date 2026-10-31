package es.spectral.menu;

import net.minecraft.client.resources.I18n;

/**
 * Translations from the generated {@code assets/espectralmenu/lang/*.lang}
 * files (es_ES / en_US built from the launcher registry + the Fabric mod's
 * strings + {@code src/lang-1.8.9.json}); follows the game language.
 */
public final class Lang {

    private Lang() {}

    public static String tr(String key, Object... args) {
        return I18n.format(key, args);
    }
}
