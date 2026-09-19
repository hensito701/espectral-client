package es.spectral.menu;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.TitleScreen;

import es.spectral.menu.ui.SuiteConfirmScreen;
import es.spectral.menu.ui.SuiteScreen;

/**
 * Owns the Right Shift recovery shortcut ({@code EspectralClient.CLIENT_KEY}):
 * it opens the Suite only when no screen is open, exactly like the previous
 * behaviour. Also honours the two screenshot-harness QA properties; both are
 * inert when unset.
 *
 * <p>Called once per client tick from the Minecraft tick mixin.
 */
public final class SuiteShortcut {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-menu");
    /** Ticks since the first non-null screen; bounds the QA re-apply window. */
    private static final int QA_TICK_BUDGET = 400;
    private static boolean qaSatisfied;
    private static boolean qaLogged;
    private static boolean qaReopenLogged;
    private static boolean qaTitleSeen;
    private static boolean qaWaitingLogged;
    private static int qaTicks;
    /** Cached reflective Esc-screen class; null where the class is absent. */
    private static Class<?> pauseClass;

    private SuiteShortcut() {}

    /**
     * Tick entry point: shortcut first, then the sticky QA opener. A
     * non-null screen is not "the game is ready" (a screen already exists
     * during the resource reload, and forcing a custom screen in then stalls
     * the boot), so while the QA property is set the hook waits for the
     * vanilla title screen to actually be up. From that moment the target
     * screen is re-applied every tick until the current screen already is
     * the target type — the hook ends only on the tick budget, never on
     * success, so vanilla's late title set can never win for good.
     */
    public static void onClientTick(Minecraft minecraft) {
        if (minecraft == null) return;
        boolean screenOpen = Compat.isScreenOpen(minecraft);
        if (EspectralClient.CLIENT_KEY != null
                && EspectralClient.CLIENT_KEY.consumeClick() && !screenOpen) {
            Compat.open(minecraft, new SuiteScreen(null));
        }
        if (!qaSatisfied) {
            tickQaHook(minecraft);
        }
    }

    private static void tickQaHook(Minecraft minecraft) {
        String which = System.getProperty(SuiteScreen.QA_SCREEN_PROPERTY);
        if (which == null) {
            qaSatisfied = true;
            return;
        }
        Screen current = Compat.getScreen(minecraft);
        if (current == null) {
            logQaWaiting();
            return;
        }
        if (!qaTitleSeen) {
            if (!(current instanceof TitleScreen)) {
                logQaWaiting();
                return;
            }
            qaTitleSeen = true;
        }
        qaTicks++;
        if (qaTicks > QA_TICK_BUDGET) {
            qaSatisfied = true;
            return;
        }
        if (isQaTarget(which, current)) {
            return;
        }
        // The SuiteScreen constructor picks up the QA search prefill, and
        // Compat.open inits it right away, so the rendered list is already
        // filtered from the first visible frame.
        openQaScreen(minecraft, which);
    }

    /** One INFO line while the hook waits for the title screen to be up. */
    private static void logQaWaiting() {
        if (!qaWaitingLogged) {
            qaWaitingLogged = true;
            LOGGER.info("QA: waiting for the title screen");
        }
    }

    private static boolean isQaTarget(String which, Screen current) {
        switch (which) {
            case "suite":
                return current instanceof SuiteScreen;
            case "support", "reset":
                return current instanceof SuiteConfirmScreen;
            case "pause":
                return pauseClass() != null && pauseClass().isInstance(current);
            default:
                // Unknown values are not a target: execution reaches
                // openQaScreen, which warns once and ends the hook.
                return false;
        }
    }

    private static void openQaScreen(Minecraft minecraft, String which) {
        switch (which) {
            case "suite" -> {
                Compat.open(minecraft, new SuiteScreen(null));
                logQaOpen(which, "suite");
            }
            case "support" -> {
                Compat.open(minecraft, SuiteConfirmScreen.support(null));
                logQaOpen(which, "support confirm");
            }
            case "reset" -> {
                Compat.open(minecraft,
                        SuiteConfirmScreen.reset(new SuiteScreen(null)));
                logQaOpen(which, "reset confirm");
            }
            case "pause" -> {
                // Always shown, even without a world: this is the only way to
                // screenshot the Esc-screen additions on a worldless box. A
                // construction failure must never kill the tick. The failure
                // is deterministic (missing class), so it also ends the hook
                // instead of logging every tick for the whole budget.
                try {
                    Compat.open(minecraft, newPauseScreen());
                    logQaOpen(which, "pause screen");
                } catch (Exception e) {
                    LOGGER.info("QA: pause screen failed: {}",
                            String.valueOf(e.getMessage()));
                    qaSatisfied = true;
                }
            }
            default -> {
                LOGGER.warn("QA: unknown {} value '{}' (want suite|support|reset|pause)",
                        SuiteScreen.QA_SCREEN_PROPERTY, which);
                qaSatisfied = true;
            }
        }
    }

    /**
     * QA open logging: one INFO line on the first successful open, one more
     * the first time a later tick has to re-open after vanilla replaced the
     * screen, silence after that.
     */
    private static void logQaOpen(String which, String what) {
        if (!qaLogged) {
            qaLogged = true;
            LOGGER.info("QA: opened {} ({})", what, SuiteScreen.QA_SCREEN_PROPERTY);
            return;
        }
        if (!qaReopenLogged) {
            qaReopenLogged = true;
            LOGGER.info("QA: re-opened {} after vanilla replaced the screen", which);
        }
    }

    /**
     * The Esc screen is {@code PauseScreen} on 26.2 but a differently named
     * class on older versions, so it is resolved reflectively to keep this
     * shared source set version-neutral. Null where the class is absent.
     */
    private static Class<?> pauseClass() {
        if (pauseClass == null) {
            try {
                pauseClass = Class.forName("net.minecraft.client.gui.screens.PauseScreen");
            } catch (ClassNotFoundException e) {
                return null;
            }
        }
        return pauseClass;
    }

    /**
     * Builds the Esc screen reflectively (see {@link #pauseClass}). Throws
     * instead of returning null so the caller logs the outcome in one place.
     */
    private static Screen newPauseScreen() throws ReflectiveOperationException {
        Class<?> target = pauseClass();
        if (target == null) {
            throw new ClassNotFoundException("net.minecraft.client.gui.screens.PauseScreen");
        }
        return (Screen) target.getConstructor(boolean.class).newInstance(true);
    }
}
