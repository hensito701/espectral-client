package es.spectral.menu.ui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;

/**
 * Two-button confirmation dialog for the destructive or external Suite
 * actions (reset-to-defaults, opening the support URL). Confirming the reset
 * reloads the list; confirming support is the only path that calls
 * {@code Compat.openUri}, and the URL never carries parameters. Esc returns
 * to the parent screen.
 */
public class SuiteConfirmScreen extends Screen {

    /** Fixed dialog height; the painter frames exactly this box. */
    public static final int PANEL_H = 132;

    private final Screen parent;
    private final String titleKey;
    private final String messageKey;
    private final Object[] messageArgs;
    private final String confirmKey;
    private final Runnable onConfirm;

    public SuiteConfirmScreen(Screen parent, String titleKey, String messageKey,
            String confirmKey, Runnable onConfirm) {
        this(parent, titleKey, messageKey, confirmKey, onConfirm, new Object[0]);
    }

    /**
     * Variant for messages with format arguments (e.g. the support message
     * quoting the URL via {@code %s}). The frozen 5-arg constructor stays
     * for argument-free messages like the reset confirm.
     */
    public SuiteConfirmScreen(Screen parent, String titleKey, String messageKey,
            String confirmKey, Runnable onConfirm, Object... messageArgs) {
        super(Component.translatable(titleKey));
        this.parent = parent;
        this.titleKey = titleKey;
        this.messageKey = messageKey;
        this.messageArgs = messageArgs != null ? messageArgs : new Object[0];
        this.confirmKey = confirmKey;
        this.onConfirm = onConfirm;
    }

    /**
     * Apoyar flow: quotes the canonical support URL; only the confirm action
     * opens it, with no UUID, token or query parameter appended.
     */
    public static SuiteConfirmScreen support(Screen parent) {
        return new SuiteConfirmScreen(parent,
                "espectral.suite.support.title",
                "espectral.suite.support.message",
                "espectral.suite.support.confirm",
                () -> Compat.openUri(SuiteScreen.SUPPORT_URL),
                SuiteScreen.SUPPORT_URL);
    }

    /**
     * Reset flow: confirming restores registry defaults (master untouched)
     * and reopens the Suite list so the new state is visible immediately.
     */
    public static SuiteConfirmScreen reset(SuiteScreen suite) {
        Screen grandparent = suite != null ? suite.parentScreen() : null;
        return new SuiteConfirmScreen(suite,
                "espectral.suite.reset.title",
                "espectral.suite.reset.message",
                "espectral.suite.reset.confirm",
                () -> {
                    ClientConfig.getInstance().resetFeaturesToDefaults();
                    Minecraft minecraft = Minecraft.getInstance();
                    if (minecraft != null) {
                        Compat.open(minecraft, new SuiteScreen(grandparent));
                    }
                });
    }

    /** Panel width for a screen of the given logical width. */
    public static int panelWidth(int screenWidth) {
        return Math.min(screenWidth - 32, 300);
    }

    /** Title text, drawn centred by the lane painter. */
    public Component getTitleText() {
        return Component.translatable(titleKey);
    }

    /** Message text, drawn centred by the lane painter. */
    public Component getMessageText() {
        return Component.translatable(messageKey, messageArgs);
    }

    @Override
    protected void init() {
        super.init();
        // All geometry from width/height; re-init clears first so resize
        // never stacks widgets.
        this.clearWidgets();
        int cw = panelWidth(this.width);
        int cx = (this.width - cw) / 2;
        int cy = (this.height - PANEL_H) / 2;
        int bw = (cw - 2 * SuiteTheme.PAD - SuiteTheme.GAP) / 2;
        int by = cy + PANEL_H - SuiteTheme.PAD - SuiteTheme.FOOTER_H;
        this.addRenderableWidget(Button.builder(Component.translatable(confirmKey), b -> {
            if (onConfirm != null) {
                onConfirm.run();
            }
        }).bounds(cx + SuiteTheme.PAD, by, bw, SuiteTheme.FOOTER_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.common.cancel"),
                b -> this.onClose()).bounds(
                cx + SuiteTheme.PAD + bw + SuiteTheme.GAP, by, bw, SuiteTheme.FOOTER_H).build());
    }

    @Override
    public void onClose() {
        Minecraft minecraft = Minecraft.getInstance();
        if (parent != null && minecraft != null) {
            Compat.open(minecraft, parent);
            return;
        }
        super.onClose();
    }

    @Override
    public Component getNarrationMessage() {
        return Component.translatable(titleKey);
    }
}
