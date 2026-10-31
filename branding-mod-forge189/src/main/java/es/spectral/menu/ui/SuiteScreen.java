package es.spectral.menu.ui;

import java.io.IOException;
import java.text.Normalizer;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

import org.lwjgl.input.Keyboard;

import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.gui.GuiTextField;
import net.minecraft.util.EnumChatFormatting;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.FeatureRegistry;
import es.spectral.menu.Lang;
import es.spectral.menu.agent.AgentLabeled;

/**
 * The Suite control panel (1.8.9 port of the Fabric lanes' screen): header
 * with the {@code suite.enabled} master chip, live search, category tabs,
 * paginated feature rows with explicit enabled/disabled words and the "⚙"
 * settings button for Macros, and a Move HUD / Reset / Support / Done footer.
 * Only features the 1.8.9 port implements are listed
 * ({@link FeatureRegistry#available}). Toggles persist immediately; Esc and
 * Done return to the parent screen.
 */
public class SuiteScreen extends GuiScreen implements AgentLabeled {

    private static final String CATEGORY_ALL = "all";
    private static final int ID_MASTER = 1;
    private static final int ID_PREV = 2;
    private static final int ID_NEXT = 3;
    private static final int ID_MOVE = 4;
    private static final int ID_RESET = 5;
    private static final int ID_SUPPORT = 6;
    private static final int ID_DONE = 7;
    private static final int ID_CATEGORY = 100;
    private static final int ID_TOGGLE = 200;
    private static final int ID_CONFIG = 300;

    private final GuiScreen parent;
    private String search = "";
    private String category = CATEGORY_ALL;
    private int page;
    private int pageSize = 5;
    private int lastPages = 1;
    private int rowsTop;
    private int innerX;
    private int innerW;
    private GuiTextField searchBox;
    private SuiteButton master;
    private SuiteButton prev;
    private SuiteButton next;
    private final List<String> categories = new ArrayList<String>();
    private final List<SuiteButton> categoryButtons = new ArrayList<SuiteButton>();
    private final List<SuiteButton> toggles = new ArrayList<SuiteButton>();
    private final List<SuiteButton> configs = new ArrayList<SuiteButton>();
    private final List<FeatureRegistry.Feature> slots = new ArrayList<FeatureRegistry.Feature>();
    private List<FeatureRegistry.Feature> filtered = new ArrayList<FeatureRegistry.Feature>();

    public SuiteScreen(GuiScreen parent) {
        this.parent = parent;
    }

    public GuiScreen parentScreen() {
        return parent;
    }

    @Override
    public void initGui() {
        ClientConfig.getInstance().load();
        Keyboard.enableRepeatEvents(true);
        buttonList.clear();
        categories.clear();
        categoryButtons.clear();
        toggles.clear();
        configs.clear();

        innerX = SuiteTheme.panelX(width) + SuiteTheme.MARGIN;
        innerW = SuiteTheme.panelWidth(width) - 2 * SuiteTheme.MARGIN;
        int y = SuiteTheme.PANEL_TOP + SuiteTheme.PAD;
        master = new SuiteButton(ID_MASTER, innerX + innerW - SuiteTheme.MASTER_W, y, SuiteTheme.MASTER_W,
                SuiteTheme.HEADER_H, "");
        buttonList.add(master);
        y += SuiteTheme.HEADER_H + 2;

        String previous = searchBox != null ? searchBox.getText() : search;
        searchBox = new GuiTextField(0, fontRendererObj, innerX + 1, y + 1, innerW - 2, SuiteTheme.SEARCH_H - 2);
        searchBox.setMaxStringLength(64);
        searchBox.setEnableBackgroundDrawing(false);
        searchBox.setText(previous);
        searchBox.setFocused(true);
        y += SuiteTheme.SEARCH_H + 2;

        categories.add(CATEGORY_ALL);
        categories.addAll(FeatureRegistry.availableCategories());
        int catW = (innerW - SuiteTheme.GAP * (categories.size() - 1)) / categories.size();
        for (int i = 0; i < categories.size(); i++) {
            String id = categories.get(i);
            String caption = CATEGORY_ALL.equals(id) ? Lang.tr("espectral.suite.category.all")
                    : Lang.tr(FeatureRegistry.categoryKey(id));
            SuiteButton cat = new SuiteButton(ID_CATEGORY + i, innerX + i * (catW + SuiteTheme.GAP), y, catW,
                    SuiteTheme.CAT_H, caption);
            categoryButtons.add(cat);
            buttonList.add(cat);
        }
        y += SuiteTheme.CAT_H + 3;

        rowsTop = y;
        int footerY = SuiteTheme.footerY(height);
        int rowsBottom = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H - 2;
        pageSize = Math.max(1, (rowsBottom - rowsTop) / SuiteTheme.ROW_H);
        for (int i = 0; i < pageSize; i++) {
            int rowY = rowsTop + i * SuiteTheme.ROW_H;
            SuiteButton gear = new SuiteButton(ID_CONFIG + i,
                    innerX + innerW - SuiteTheme.TOGGLE_W - SuiteTheme.CONFIG_W - SuiteTheme.GAP, rowY,
                    SuiteTheme.CONFIG_W, SuiteTheme.FOOTER_H, "\u2699");
            gear.tooltip = Lang.tr("espectral.suite.config");
            configs.add(gear);
            buttonList.add(gear);
            SuiteButton toggle = new SuiteButton(ID_TOGGLE + i, innerX + innerW - SuiteTheme.TOGGLE_W, rowY,
                    SuiteTheme.TOGGLE_W, SuiteTheme.FOOTER_H, "");
            toggles.add(toggle);
            buttonList.add(toggle);
        }

        int pagerY = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H;
        prev = new SuiteButton(ID_PREV, innerX, pagerY, 64, SuiteTheme.PAGER_H, Lang.tr("espectral.suite.button.prev"));
        next = new SuiteButton(ID_NEXT, innerX + innerW - 64, pagerY, 64, SuiteTheme.PAGER_H,
                Lang.tr("espectral.suite.button.next"));
        buttonList.add(prev);
        buttonList.add(next);

        int footerW = (innerW - 3 * SuiteTheme.GAP) / 4;
        String[] keys = {"espectral.suite.button.move", "espectral.suite.button.reset",
                "espectral.suite.button.support", "espectral.suite.button.done"};
        int[] ids = {ID_MOVE, ID_RESET, ID_SUPPORT, ID_DONE};
        for (int i = 0; i < 4; i++) {
            buttonList.add(new SuiteButton(ids[i], innerX + i * (footerW + SuiteTheme.GAP), footerY, footerW,
                    SuiteTheme.FOOTER_H, Lang.tr(keys[i])));
        }
        refresh();
    }

    /** Recomputes the filtered rows and updates the existing widgets in place. */
    private void refresh() {
        ClientConfig config = ClientConfig.getInstance();
        search = searchBox.getText();
        filtered = filtered();
        lastPages = Math.max(1, (filtered.size() + pageSize - 1) / pageSize);
        page = Math.min(Math.max(0, page), lastPages - 1);
        slots.clear();
        for (int i = page * pageSize; i < filtered.size() && slots.size() < pageSize; i++) {
            slots.add(filtered.get(i));
        }
        master.displayString = Lang.tr(config.isSuiteEnabled() ? "espectral.suite.master.on" : "espectral.suite.master.off");
        for (int i = 0; i < pageSize; i++) {
            SuiteButton toggle = toggles.get(i);
            SuiteButton gear = configs.get(i);
            if (i < slots.size()) {
                FeatureRegistry.Feature f = slots.get(i);
                boolean on = config.isFeatureEnabledRaw(f.id);
                toggle.visible = true;
                toggle.displayString = Lang.tr(on ? "espectral.suite.state.on" : "espectral.suite.state.off");
                toggle.tooltip = Lang.tr(FeatureRegistry.descriptionKey(f.id));
                gear.visible = "macros".equals(f.id);
            } else {
                toggle.visible = false;
                gear.visible = false;
            }
        }
        boolean paged = lastPages > 1;
        prev.visible = paged;
        next.visible = paged;
        for (int i = 0; i < categoryButtons.size(); i++) {
            boolean current = categories.get(i).equals(category);
            categoryButtons.get(i).enabled = !current;
            categoryButtons.get(i).selected = current;
        }
    }

    private List<FeatureRegistry.Feature> filtered() {
        List<FeatureRegistry.Feature> base = FeatureRegistry.available(CATEGORY_ALL.equals(category) ? null : category);
        String needle = fold(search);
        if (needle.isEmpty()) return base;
        List<FeatureRegistry.Feature> out = new ArrayList<FeatureRegistry.Feature>();
        for (FeatureRegistry.Feature f : base) {
            String hay = fold(f.id + " " + Lang.tr(FeatureRegistry.nameKey(f.id)) + " "
                    + Lang.tr(FeatureRegistry.descriptionKey(f.id)));
            if (hay.contains(needle)) out.add(f);
        }
        return out;
    }

    /** Case- and accent-insensitive folding for the live search. */
    static String fold(String value) {
        String lower = value == null ? "" : value.toLowerCase(Locale.ROOT);
        return Normalizer.normalize(lower, Normalizer.Form.NFD).replaceAll("\\p{M}", "");
    }

    @Override
    protected void actionPerformed(GuiButton button) {
        ClientConfig config = ClientConfig.getInstance();
        int id = button.id;
        if (id == ID_MASTER) {
            config.setSuiteEnabled(!config.isSuiteEnabled());
        } else if (id == ID_PREV) {
            page = Math.max(0, page - 1);
        } else if (id == ID_NEXT) {
            page = Math.min(lastPages - 1, page + 1);
        } else if (id == ID_MOVE) {
            mc.displayGuiScreen(new HudEditScreen(this));
            return;
        } else if (id == ID_RESET) {
            mc.displayGuiScreen(SuiteConfirmScreen.reset(this));
            return;
        } else if (id == ID_SUPPORT) {
            mc.displayGuiScreen(SuiteConfirmScreen.support(this));
            return;
        } else if (id == ID_DONE) {
            close();
            return;
        } else if (id >= ID_CONFIG && id - ID_CONFIG < slots.size()) {
            if ("macros".equals(slots.get(id - ID_CONFIG).id)) {
                mc.displayGuiScreen(new SuiteMacrosScreen(this));
            }
            return;
        } else if (id >= ID_TOGGLE && id - ID_TOGGLE < slots.size()) {
            String featureId = slots.get(id - ID_TOGGLE).id;
            config.setFeatureEnabled(featureId, !config.isFeatureEnabledRaw(featureId));
        } else if (id >= ID_CATEGORY && id - ID_CATEGORY < categories.size()) {
            category = categories.get(id - ID_CATEGORY);
            page = 0;
        }
        refresh();
    }

    private void close() {
        ClientConfig.getInstance().save();
        mc.displayGuiScreen(parent);
    }

    @Override
    protected void keyTyped(char typedChar, int keyCode) throws IOException {
        if (keyCode == Keyboard.KEY_ESCAPE) {
            close();
            return;
        }
        String before = searchBox.getText();
        searchBox.textboxKeyTyped(typedChar, keyCode);
        if (!before.equals(searchBox.getText())) {
            page = 0;
            refresh();
        }
    }

    @Override
    protected void mouseClicked(int mouseX, int mouseY, int mouseButton) throws IOException {
        super.mouseClicked(mouseX, mouseY, mouseButton);
        searchBox.mouseClicked(mouseX, mouseY, mouseButton);
    }

    @Override
    public void updateScreen() {
        searchBox.updateCursorCounter();
    }

    @Override
    public void onGuiClosed() {
        Keyboard.enableRepeatEvents(false);
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        if (mc.theWorld == null) MenuSkin.paintBackdrop(width, height);
        SuitePainter.paintPanel(width, height);
        ClientConfig config = ClientConfig.getInstance();
        int headerY = SuiteTheme.PANEL_TOP + SuiteTheme.PAD + (SuiteTheme.HEADER_H - 8) / 2;
        String title = Lang.tr("espectral.suite.title");
        Gfx.textGradient(title, innerX, headerY, SuiteTheme.accentGradientLeft(), SuiteTheme.accentGradientRight(), true);
        Gfx.text(" \u00b7 " + filtered.size() + "/" + FeatureRegistry.available(null).size(),
                innerX + fontRendererObj.getStringWidth(title), headerY, SuiteTheme.TEXT_DIM, false);

        int sy = SuiteTheme.PANEL_TOP + SuiteTheme.PAD + SuiteTheme.HEADER_H + 2;
        Gfx.fill(innerX, sy, innerX + innerW, sy + SuiteTheme.SEARCH_H, 0xA0000000);
        Gfx.outline(innerX, sy, innerW, SuiteTheme.SEARCH_H,
                searchBox.isFocused() ? SuiteTheme.panelEdge() : SuiteTheme.accentHairline());
        searchBox.yPosition = sy + (SuiteTheme.SEARCH_H - 8) / 2;
        searchBox.xPosition = innerX + 4;
        searchBox.drawTextBox();
        if (searchBox.getText().isEmpty()) {
            Gfx.text(Lang.tr("espectral.suite.search.hint"), innerX + 8, searchBox.yPosition, SuiteTheme.TEXT_DIM, false);
        }

        boolean masterOn = config.isSuiteEnabled();
        int labelW = innerW - SuiteTheme.TOGGLE_W - SuiteTheme.CONFIG_W - 2 * SuiteTheme.GAP;
        for (int i = 0; i < slots.size(); i++) {
            FeatureRegistry.Feature f = slots.get(i);
            int rowY = rowsTop + i * SuiteTheme.ROW_H;
            boolean hot = mouseX >= innerX && mouseX < innerX + innerW && mouseY >= rowY && mouseY < rowY + SuiteTheme.FOOTER_H;
            Gfx.fill(innerX, rowY, innerX + labelW, rowY + SuiteTheme.FOOTER_H, hot ? SuiteTheme.ROW_HOVER : SuiteTheme.ROW);
            boolean on = config.isFeatureEnabledRaw(f.id);
            String label = Lang.tr(FeatureRegistry.nameKey(f.id)) + " "
                    + (on ? EnumChatFormatting.GREEN : EnumChatFormatting.GRAY)
                    + Lang.tr(on ? "espectral.suite.state.on" : "espectral.suite.state.off")
                    + (masterOn ? "" : " " + EnumChatFormatting.GRAY + Lang.tr("espectral.suite.state.suppressed"));
            Gfx.text(fontRendererObj.trimStringToWidth(label, labelW - 8), innerX + 5, rowY + 6, SuiteTheme.TEXT, false);
        }
        if (filtered.isEmpty()) {
            Gfx.text(Lang.tr("espectral.suite.empty"), innerX + 5, rowsTop + 6, SuiteTheme.TEXT_DIM, false);
        }
        if (lastPages > 1) {
            Gfx.textCentered(Lang.tr("espectral.suite.page", page + 1, lastPages), innerX + innerW / 2,
                    prev.yPosition + 6, SuiteTheme.TEXT_DIM, false);
        }
        super.drawScreen(mouseX, mouseY, partialTicks);
        SuiteButton.drawTooltips(buttonList, mouseX, mouseY, width, height);
    }

    @Override
    public boolean doesGuiPauseGame() {
        return true;
    }

    /** Row toggles and gears carry their feature's name for the agent. */
    @Override
    public String agentLabel(GuiButton b) {
        int toggle = b.id - ID_TOGGLE;
        if (toggle >= 0 && toggle < slots.size() && b.id < ID_CONFIG) {
            return Lang.tr(FeatureRegistry.nameKey(slots.get(toggle).id)) + ": " + b.displayString;
        }
        int gear = b.id - ID_CONFIG;
        if (gear >= 0 && gear < slots.size()) {
            return Lang.tr("espectral.suite.config") + " " + Lang.tr(FeatureRegistry.nameKey(slots.get(gear).id));
        }
        return null;
    }
}
