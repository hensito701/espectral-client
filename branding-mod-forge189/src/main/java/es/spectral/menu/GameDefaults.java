package es.spectral.menu;

import net.minecraft.client.Minecraft;
import net.minecraft.client.settings.GameSettings;

/**
 * Game settings Espectral turns on once per game dir (ids recorded in the
 * config's {@code "applied_defaults"}), so a player who switches one back
 * off keeps that choice.
 *
 * <p>"Use VBOs" ships OFF in vanilla 1.8.9 (a 2015 workaround for old
 * drivers); ON draws the same image with ~25% more fps in the bench scenes.
 * It is applied on the first client tick: Forge re-reads options.txt after
 * the mods load (which would undo a preInit change), and the title screen
 * comes before any world, so {@code RenderGlobal.loadRenderers()} switches
 * to VBO rendering on the first world load — the same path as the
 * Video Settings button.
 */
final class GameDefaults {

    private static final String VBO = "vbo";
    private static boolean checked;

    private GameDefaults() {}

    /** Client tick: a single check per session. */
    static void onTick(Minecraft mc) {
        if (checked) return;
        checked = true;
        ClientConfig config = ClientConfig.getInstance();
        if (config.isDefaultApplied(VBO)) return;
        GameSettings settings = mc.gameSettings;
        if (!settings.useVbo) {
            settings.useVbo = true;
            settings.saveOptions();
        }
        config.markDefaultApplied(VBO);
    }
}
