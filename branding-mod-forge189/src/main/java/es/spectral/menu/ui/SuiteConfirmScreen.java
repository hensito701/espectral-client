package es.spectral.menu.ui;

import java.util.ArrayList;
import java.util.List;

import org.lwjgl.Sys;
import org.lwjgl.input.Keyboard;

import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Lang;

/**
 * Two-button confirmation dialog for the destructive or external Suite
 * actions (reset-to-defaults, opening the support URL). Only the support
 * confirm opens a browser, and the URL never carries parameters. Esc / Cancel
 * return to the parent screen.
 */
public class SuiteConfirmScreen extends GuiScreen {

    /** Canonical support URL, quoted without parameters. */
    public static final String SUPPORT_URL = "https://espectral.uhce.top/donaciones";
    public static final int PANEL_H = 132;

    private final GuiScreen parent;
    private final String titleKey;
    private final String message;
    private final String confirmKey;
    private final Runnable onConfirm;
    private List<String> lines = new ArrayList<String>();

    public SuiteConfirmScreen(GuiScreen parent, String titleKey, String message, String confirmKey, Runnable onConfirm) {
        this.parent = parent;
        this.titleKey = titleKey;
        this.message = message;
        this.confirmKey = confirmKey;
        this.onConfirm = onConfirm;
    }

    /** Support flow: quotes the canonical URL; only Confirm opens it. */
    public static SuiteConfirmScreen support(GuiScreen parent) {
        return new SuiteConfirmScreen(parent, "espectral.suite.support.title",
                Lang.tr("espectral.suite.support.message", SUPPORT_URL), "espectral.suite.support.confirm",
                () -> Sys.openURL(SUPPORT_URL));
    }

    /** Reset flow: registry defaults (master untouched), then a fresh Suite list. */
    public static SuiteConfirmScreen reset(SuiteScreen suite) {
        return new SuiteConfirmScreen(suite, "espectral.suite.reset.title",
                Lang.tr("espectral.suite.reset.message"), "espectral.suite.reset.confirm",
                () -> ClientConfig.getInstance().resetFeaturesToDefaults());
    }

    public static int panelWidth(int screenWidth) {
        return Math.min(screenWidth - 32, 300);
    }

    @Override
    public void initGui() {
        buttonList.clear();
        int cw = panelWidth(width);
        lines = SuitePainter.wrap(message, fontRendererObj, cw - 2 * SuiteTheme.PAD);
        int cx = (width - cw) / 2;
        int cy = (height - PANEL_H) / 2;
        int bw = (cw - 2 * SuiteTheme.PAD - SuiteTheme.GAP) / 2;
        int by = cy + PANEL_H - SuiteTheme.PAD - SuiteTheme.FOOTER_H;
        buttonList.add(new SuiteButton(0, cx + SuiteTheme.PAD, by, bw, SuiteTheme.FOOTER_H, Lang.tr(confirmKey)));
        buttonList.add(new SuiteButton(1, cx + SuiteTheme.PAD + bw + SuiteTheme.GAP, by, bw, SuiteTheme.FOOTER_H,
                Lang.tr("espectral.common.cancel")));
    }

    @Override
    protected void actionPerformed(GuiButton button) {
        if (button.id == 0 && onConfirm != null) {
            onConfirm.run();
        }
        mc.displayGuiScreen(parent);
    }

    @Override
    protected void keyTyped(char typedChar, int keyCode) {
        if (keyCode == Keyboard.KEY_ESCAPE) {
            mc.displayGuiScreen(parent);
        }
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        if (mc.theWorld == null) {
            MenuSkin.paintBackdrop(width, height);
        }
        SuitePainter.paintConfirm(width, height, panelWidth(width), PANEL_H, Lang.tr(titleKey), lines);
        super.drawScreen(mouseX, mouseY, partialTicks);
    }

    /** Title shown by the agent screen tree. */
    public String titleText() {
        return Lang.tr(titleKey);
    }
}
