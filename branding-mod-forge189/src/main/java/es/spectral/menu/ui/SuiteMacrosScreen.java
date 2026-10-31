package es.spectral.menu.ui;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

import org.lwjgl.input.Keyboard;

import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.gui.GuiTextField;
import net.minecraft.util.EnumChatFormatting;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.KeyNames;
import es.spectral.menu.Lang;
import es.spectral.menu.MacroEngine;

/**
 * In-game macro editor (opened from the "⚙" of the Suite's Macros row), the
 * 1.8.9 port of the Fabric lanes' screen: {@link Mode#LIST} (paginated rows,
 * click to edit) and {@link Mode#EDIT} (name, key capture, message/command).
 * Keys are stored in the launcher's {@code key.keyboard.<name>} format
 * ({@link KeyNames}); Esc during capture cancels the capture, not the screen.
 * Text starting with {@code /} runs as a command, anything else as chat.
 */
public class SuiteMacrosScreen extends GuiScreen {

    private enum Mode { LIST, EDIT }

    private static final int FIELD_H = 18;
    private static final int ID_ROW = 100;
    private static final int ID_PREV = 1;
    private static final int ID_NEXT = 2;
    private static final int ID_ADD = 3;
    private static final int ID_BACK = 4;
    private static final int ID_KEY = 5;
    private static final int ID_SAVE = 6;
    private static final int ID_DELETE = 7;
    private static final int ID_CANCEL = 8;

    private final GuiScreen parent;
    private Mode mode = Mode.LIST;
    private int page;
    private int pageSize = 5;
    private int lastPages = 1;
    private final List<ClientConfig.MacroConfig> rowMacros = new ArrayList<ClientConfig.MacroConfig>();
    private final List<SuiteButton> rowButtons = new ArrayList<SuiteButton>();
    private SuiteButton prev;
    private SuiteButton next;

    private String draftId;
    private String draftName = "";
    private String draftKey = "";
    private String draftText = "";
    private boolean captureActive;
    private boolean confirmPending;
    private String status;
    private boolean statusError;
    private GuiTextField nameBox;
    private GuiTextField textBox;
    private SuiteButton keyButton;
    private SuiteButton saveButton;
    private int innerX;
    private int innerW;
    private int fieldsTop;

    public SuiteMacrosScreen(GuiScreen parent) {
        this.parent = parent;
    }

    @Override
    public void initGui() {
        ClientConfig.getInstance().load();
        Keyboard.enableRepeatEvents(true);
        buttonList.clear();
        rowButtons.clear();
        nameBox = null;
        textBox = null;
        innerX = SuiteTheme.panelX(width) + SuiteTheme.MARGIN;
        innerW = SuiteTheme.panelWidth(width) - 2 * SuiteTheme.MARGIN;
        int y = SuiteTheme.PANEL_TOP + SuiteTheme.PAD + SuiteTheme.HEADER_H + 2;
        int footerY = SuiteTheme.footerY(height);
        if (mode == Mode.LIST) {
            initList(y, footerY);
        } else {
            initEdit(y, footerY);
        }
        refresh();
    }

    private void initList(int rowsTop, int footerY) {
        int rowsBottom = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H - 2;
        pageSize = Math.max(1, (rowsBottom - rowsTop) / SuiteTheme.ROW_H);
        for (int i = 0; i < pageSize; i++) {
            SuiteButton row = new SuiteButton(ID_ROW + i, innerX, rowsTop + i * SuiteTheme.ROW_H, innerW,
                    SuiteTheme.ROW_H - 2, "");
            rowButtons.add(row);
            buttonList.add(row);
        }
        int pagerY = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H;
        prev = new SuiteButton(ID_PREV, innerX, pagerY, 40, SuiteTheme.PAGER_H, Lang.tr("espectral.suite.button.prev"));
        next = new SuiteButton(ID_NEXT, innerX + innerW - 40, pagerY, 40, SuiteTheme.PAGER_H,
                Lang.tr("espectral.suite.button.next"));
        buttonList.add(prev);
        buttonList.add(next);
        int btnW = (innerW - SuiteTheme.GAP) / 2;
        buttonList.add(new SuiteButton(ID_ADD, innerX, footerY, btnW, SuiteTheme.FOOTER_H, Lang.tr("espectral.macros.add")));
        buttonList.add(new SuiteButton(ID_BACK, innerX + btnW + SuiteTheme.GAP, footerY, btnW, SuiteTheme.FOOTER_H,
                Lang.tr("espectral.suite.back")));
    }

    /** Label column: the widest of the three edit labels, at most a third of the panel. */
    private int labelWidth() {
        int w = 0;
        for (String key : new String[] {"espectral.macros.name", "espectral.macros.key", "espectral.macros.text"}) {
            w = Math.max(w, fontRendererObj.getStringWidth(Lang.tr(key)));
        }
        return Math.min(w + 8, innerW / 3);
    }

    private void initEdit(int y, int footerY) {
        int labelW = labelWidth();
        int fieldX = innerX + labelW + SuiteTheme.GAP;
        int fieldW = innerW - labelW - SuiteTheme.GAP;
        fieldsTop = y;
        nameBox = new GuiTextField(0, fontRendererObj, fieldX + 4, y + 5, fieldW - 8, 10);
        nameBox.setEnableBackgroundDrawing(false);
        nameBox.setMaxStringLength(ClientConfig.MAX_MACRO_NAME);
        nameBox.setText(draftName);
        y += FIELD_H + 3;
        keyButton = new SuiteButton(ID_KEY, fieldX, y, fieldW, SuiteTheme.FOOTER_H, "");
        keyButton.tooltip = Lang.tr("espectral.macros.key.hint");
        buttonList.add(keyButton);
        y += SuiteTheme.FOOTER_H + 3;
        textBox = new GuiTextField(1, fontRendererObj, fieldX + 4, y + 5, fieldW - 8, 10);
        textBox.setEnableBackgroundDrawing(false);
        textBox.setMaxStringLength(ClientConfig.MAX_MACRO_TEXT);
        textBox.setText(draftText);
        nameBox.setFocused(true);
        int btnW = (innerW - 2 * SuiteTheme.GAP) / 3;
        saveButton = new SuiteButton(ID_SAVE, innerX, footerY, btnW, SuiteTheme.FOOTER_H, "");
        buttonList.add(saveButton);
        SuiteButton delete = new SuiteButton(ID_DELETE, innerX + btnW + SuiteTheme.GAP, footerY, btnW,
                SuiteTheme.FOOTER_H, Lang.tr("espectral.macros.delete"));
        delete.enabled = draftId != null;
        buttonList.add(delete);
        buttonList.add(new SuiteButton(ID_CANCEL, innerX + 2 * (btnW + SuiteTheme.GAP), footerY, btnW,
                SuiteTheme.FOOTER_H, Lang.tr("espectral.macros.cancel")));
    }

    private void refresh() {
        if (mode == Mode.LIST) {
            List<ClientConfig.MacroConfig> all = ClientConfig.getInstance().getMacros();
            lastPages = Math.max(1, (all.size() + pageSize - 1) / pageSize);
            page = Math.min(Math.max(0, page), lastPages - 1);
            rowMacros.clear();
            for (int i = page * pageSize; i < all.size() && rowMacros.size() < pageSize; i++) {
                rowMacros.add(all.get(i));
            }
            for (int i = 0; i < rowButtons.size(); i++) {
                SuiteButton b = rowButtons.get(i);
                if (i < rowMacros.size()) {
                    ClientConfig.MacroConfig m = rowMacros.get(i);
                    String text = firstActionText(m);
                    b.displayString = EnumChatFormatting.GRAY + "[" + SuiteTheme.accentFormat()
                            + KeyNames.displayName(m.keybind) + EnumChatFormatting.GRAY + "] "
                            + (text.startsWith("/") ? EnumChatFormatting.YELLOW : EnumChatFormatting.WHITE)
                            + truncate(text, 32);
                    b.tooltip = summary(m);
                    b.visible = true;
                } else {
                    b.visible = false;
                }
            }
            prev.visible = lastPages > 1;
            next.visible = lastPages > 1;
        } else {
            keyButton.displayString = captureActive ? SuiteTheme.accentFormat() + Lang.tr("espectral.macros.key.capture")
                    : draftKey.isEmpty() ? EnumChatFormatting.GRAY + Lang.tr("espectral.macros.key.capture")
                    : SuiteTheme.accentFormat() + KeyNames.displayName(draftKey);
            saveButton.displayString = Lang.tr(confirmPending ? "espectral.macros.confirm" : "espectral.macros.save");
        }
    }

    private static String firstActionText(ClientConfig.MacroConfig m) {
        return m.actions.isEmpty() || m.actions.get(0).text == null ? "" : m.actions.get(0).text;
    }

    private static String summary(ClientConfig.MacroConfig m) {
        String name = m.name == null || m.name.trim().isEmpty() ? m.id : m.name;
        return name + " \u00b7 " + m.keybind + " \u00b7 " + firstActionText(m);
    }

    private static String truncate(String text, int max) {
        return text.length() <= max ? text : text.substring(0, Math.max(0, max - 1)) + "\u2026";
    }

    @Override
    protected void actionPerformed(GuiButton button) {
        switch (button.id) {
            case ID_PREV: page = Math.max(0, page - 1); break;
            case ID_NEXT: page = Math.min(lastPages - 1, page + 1); break;
            case ID_ADD: edit(null); return;
            case ID_BACK: close(); return;
            case ID_KEY:
                captureActive = !captureActive;
                if (captureActive) confirmPending = false;
                break;
            case ID_SAVE: save(); return;
            case ID_DELETE: confirmDelete(); return;
            case ID_CANCEL: backToList(); return;
            default:
                if (button.id >= ID_ROW && button.id - ID_ROW < rowMacros.size()) {
                    edit(rowMacros.get(button.id - ID_ROW));
                    return;
                }
                break;
        }
        refresh();
    }

    private void edit(ClientConfig.MacroConfig macro) {
        mode = Mode.EDIT;
        draftId = macro == null ? null : macro.id;
        draftName = macro == null ? "" : macro.name;
        draftKey = macro == null ? "" : macro.keybind;
        draftText = macro == null ? "" : firstActionText(macro);
        captureActive = false;
        confirmPending = false;
        status = null;
        initGui();
    }

    private void backToList() {
        mode = Mode.LIST;
        captureActive = false;
        confirmPending = false;
        status = null;
        initGui();
    }

    private void setStatus(String text, boolean error) {
        status = text;
        statusError = error;
        refresh();
    }

    private void save() {
        draftName = nameBox.getText();
        draftText = textBox.getText();
        String text = draftText.trim();
        if (text.isEmpty()) {
            setStatus(Lang.tr("espectral.macros.needtext"), true);
            return;
        }
        if (draftKey.isEmpty()) {
            setStatus(Lang.tr("espectral.macros.needkey"), true);
            return;
        }
        if (MacroEngine.isReserved(KeyNames.keyCode(draftKey))) {
            setStatus(Lang.tr("espectral.macros.reserved"), true);
            return;
        }
        MacroEngine.Conflict conflict = MacroEngine.findConflict(draftKey, draftId);
        if (conflict != null) {
            if (conflict.fromMacro) {
                setStatus(Lang.tr("espectral.macros.dupkey", conflict.label), true);
                return;
            }
            if (!confirmPending) {
                confirmPending = true;
                setStatus(Lang.tr("espectral.macros.conflict", conflict.label), true);
                return;
            }
        }
        String name = draftName.trim();
        if (name.isEmpty()) {
            String base = text.startsWith("/") ? text.substring(1).trim() : text;
            name = base.isEmpty() ? "macro" : base.length() <= ClientConfig.MAX_MACRO_NAME ? base
                    : base.substring(0, ClientConfig.MAX_MACRO_NAME);
        }
        ClientConfig config = ClientConfig.getInstance();
        String id = draftId != null ? draftId : config.newMacroId(name);
        if (!config.putMacro(id, name, draftKey, text.startsWith("/") ? "command" : "chat", text)) {
            setStatus(Lang.tr("espectral.macros.full", ClientConfig.MAX_MACROS), true);
            return;
        }
        backToList();
    }

    private void confirmDelete() {
        if (draftId == null) return;
        final String id = draftId;
        String label = draftName == null || draftName.trim().isEmpty() ? draftId : draftName;
        final GuiScreen grandparent = parent;
        mc.displayGuiScreen(new SuiteConfirmScreen(new SuiteMacrosScreen(grandparent), "espectral.macros.delete.title",
                Lang.tr("espectral.macros.delete.message", label), "espectral.macros.delete.confirm",
                () -> ClientConfig.getInstance().removeMacro(id)));
    }

    private void close() {
        ClientConfig.getInstance().save();
        mc.displayGuiScreen(parent);
    }

    @Override
    protected void keyTyped(char typedChar, int keyCode) throws IOException {
        if (captureActive) {
            if (keyCode == Keyboard.KEY_ESCAPE) {
                captureActive = false;
            } else if (MacroEngine.isReserved(keyCode)) {
                setStatus(Lang.tr("espectral.macros.reserved"), true);
                return;
            } else {
                String stored = KeyNames.keybindFor(keyCode);
                if (stored.isEmpty()) {
                    setStatus(Lang.tr("espectral.macros.needkey"), true);
                    return;
                }
                draftKey = stored;
                captureActive = false;
                confirmPending = false;
                status = null;
            }
            refresh();
            return;
        }
        if (keyCode == Keyboard.KEY_ESCAPE) {
            if (mode == Mode.EDIT) {
                backToList();
            } else {
                close();
            }
            return;
        }
        if (mode == Mode.EDIT) {
            if (keyCode == Keyboard.KEY_TAB) {
                boolean nameFocused = nameBox.isFocused();
                nameBox.setFocused(!nameFocused);
                textBox.setFocused(nameFocused);
                return;
            }
            String before = textBox.getText();
            nameBox.textboxKeyTyped(typedChar, keyCode);
            textBox.textboxKeyTyped(typedChar, keyCode);
            if (!before.equals(textBox.getText())) confirmPending = false;
            draftName = nameBox.getText();
            draftText = textBox.getText();
            refresh();
        }
    }

    /**
     * Mode switches rebuild {@code buttonList} from inside {@code actionPerformed};
     * vanilla's click loop would keep iterating the rebuilt list and press the
     * new button under the cursor too. So one click presses at most one button,
     * found on a snapshot of the list.
     */
    @Override
    protected void mouseClicked(int mouseX, int mouseY, int mouseButton) throws IOException {
        boolean editing = mode == Mode.EDIT;
        if (mouseButton == 0) {
            for (GuiButton button : new ArrayList<GuiButton>(buttonList)) {
                if (button.mousePressed(mc, mouseX, mouseY)) {
                    button.playPressSound(mc.getSoundHandler());
                    actionPerformed(button);
                    return;
                }
            }
        }
        if (editing && mode == Mode.EDIT) {
            nameBox.mouseClicked(mouseX, mouseY, mouseButton);
            textBox.mouseClicked(mouseX, mouseY, mouseButton);
        }
    }

    @Override
    public void updateScreen() {
        if (nameBox != null) nameBox.updateCursorCounter();
        if (textBox != null) textBox.updateCursorCounter();
    }

    @Override
    public void onGuiClosed() {
        Keyboard.enableRepeatEvents(false);
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        if (mc.theWorld == null) MenuSkin.paintBackdrop(width, height);
        SuitePainter.paintPanel(width, height);
        String titleKey = mode == Mode.EDIT ? (draftId != null ? "espectral.macros.edit" : "espectral.macros.new")
                : "espectral.macros.title";
        int headerY = SuiteTheme.PANEL_TOP + SuiteTheme.PAD + (SuiteTheme.HEADER_H - 8) / 2;
        String title = Lang.tr(titleKey);
        Gfx.textGradient(title, innerX, headerY, SuiteTheme.accentGradientLeft(), SuiteTheme.accentGradientRight(), true);
        Gfx.text("  " + ClientConfig.getInstance().getMacros().size() + "/" + ClientConfig.MAX_MACROS,
                innerX + fontRendererObj.getStringWidth(title), headerY, SuiteTheme.TEXT_DIM, false);
        if (mode == Mode.LIST) {
            if (rowMacros.isEmpty()) {
                Gfx.text(Lang.tr("espectral.macros.empty"), innerX + 4, fieldsTopForList() + 6, SuiteTheme.TEXT_DIM, false);
            }
            if (lastPages > 1) {
                Gfx.textCentered(Lang.tr("espectral.suite.page", page + 1, lastPages), innerX + innerW / 2,
                        prev.yPosition + 6, SuiteTheme.TEXT_DIM, false);
            }
        } else {
            int labelW = labelWidth();
            int y = fieldsTop;
            drawField(nameBox, Lang.tr("espectral.macros.name"), Lang.tr("espectral.macros.name.hint"), y, labelW);
            Gfx.text(Lang.tr("espectral.macros.key"), innerX, y + FIELD_H + 3 + 6, SuiteTheme.TEXT, false);
            y += FIELD_H + 3 + SuiteTheme.FOOTER_H + 3;
            drawField(textBox, Lang.tr("espectral.macros.text"), Lang.tr("espectral.macros.text.hint"), y, labelW);
            y += FIELD_H + 3;
            String line = status != null ? status : Lang.tr("espectral.macros.key.hint");
            for (String part : SuitePainter.wrap(line, fontRendererObj, innerW)) {
                Gfx.text(part, innerX, y, status != null && statusError ? SuiteTheme.accentBright() : SuiteTheme.TEXT_DIM, false);
                y += 10;
            }
        }
        super.drawScreen(mouseX, mouseY, partialTicks);
        SuiteButton.drawTooltips(buttonList, mouseX, mouseY, width, height);
    }

    private int fieldsTopForList() {
        return SuiteTheme.PANEL_TOP + SuiteTheme.PAD + SuiteTheme.HEADER_H + 2;
    }

    private void drawField(GuiTextField box, String label, String hint, int y, int labelW) {
        int fieldX = innerX + labelW + SuiteTheme.GAP;
        int fieldW = innerW - labelW - SuiteTheme.GAP;
        Gfx.text(label, innerX, y + 5, SuiteTheme.TEXT, false);
        Gfx.fill(fieldX, y, fieldX + fieldW, y + FIELD_H, 0xA0000000);
        Gfx.outline(fieldX, y, fieldW, FIELD_H, box.isFocused() ? SuiteTheme.panelEdge() : SuiteTheme.accentHairline());
        box.drawTextBox();
        if (box.getText().isEmpty()) {
            Gfx.text(hint, fieldX + 8, y + 5, SuiteTheme.TEXT_DIM, false);
        }
    }

    @Override
    public boolean doesGuiPauseGame() {
        return true;
    }
}
