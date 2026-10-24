package es.spectral.menu.ui;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;
import es.spectral.menu.MacroEngine;

/**
 * In-game macro editor, opened from the "⚙" button on the Suite's Macros row.
 *
 * <p>Two modes share one screen so the 240&nbsp;px logical height stays
 * readable: {@link Mode#LIST} (paginated rows, click a row to edit) and
 * {@link Mode#EDIT} (name, key capture, message/command). Switching mode calls
 * {@code rebuildWidgets()}, which clears the widget list before {@code init()}
 * rebuilds it, so neither mode change nor resize can stack widgets. Draft
 * values live in fields, so a resize mid-edit keeps what was typed.
 *
 * <p>Keys are stored in the vanilla keybind format
 * ({@code key.keyboard.<name>}, see {@link MacroEngine#keybindFor(int)}) which
 * already expresses non-US keys: on a Spanish layout the Ñ and Ç keys carry
 * the US keysyms {@code semicolon} / {@code apostrophe}, and the label shown
 * here comes from the layout-aware vanilla display name, so the user sees
 * {@code ñ} and {@code ç}. Esc during capture cancels the capture instead of
 * closing the screen.
 *
 * <p>Only chat output is produced: text starting with {@code /} runs as a
 * command, everything else as chat (see {@code MacroEngine.executeMacro}).
 */
public class SuiteMacrosScreen extends Screen implements SuiteSheet {

    /** Which view the screen is showing. */
    private enum Mode { LIST, EDIT }

    private static final int HEADER_H = SuiteTheme.HEADER_H;
    private static final int ROW_H = SuiteTheme.ROW_H;
    private static final int FIELD_H = 18;
    private static final int BUTTON_H = SuiteTheme.FOOTER_H;

    private final Screen parent;
    private Mode mode = Mode.LIST;

    // List state.
    private int page;
    private int pageSize = 5;
    private int lastPages = 1;
    private final List<Button> rowButtons = new ArrayList<>();
    private final List<ClientConfig.MacroConfig> rowMacros = new ArrayList<>();
    private StringWidget emptyLabel;
    private Button pagerPrev;
    private Button pagerNext;
    private StringWidget pagerLabel;
    private StringWidget headerLabel;
    private StringWidget statusLabel;

    // Edit state (survives a resize: rebuilt widgets read these back).
    private String draftId;
    private String draftName = "";
    private String draftKey = "";
    private String draftText = "";
    private boolean captureActive;
    /** Set after a non-fatal conflict was shown; the next Save binds anyway. */
    private boolean confirmPending;
    private Button keyButton;
    private Button saveButton;
    private Button deleteButton;

    public SuiteMacrosScreen(Screen parent) {
        super(Component.translatable("espectral.macros.title"));
        this.parent = parent;
    }

    /** Parent screen for Esc/return navigation. */
    public Screen parentScreen() {
        return parent;
    }

    @Override
    protected void init() {
        super.init();
        ClientConfig.getInstance().load();
        this.clearWidgets();
        this.rowButtons.clear();
        this.rowMacros.clear();

        int innerX = SuiteTheme.panelX(this.width) + SuiteTheme.MARGIN;
        int innerW = SuiteTheme.panelWidth(this.width) - 2 * SuiteTheme.MARGIN;
        int y = SuiteTheme.PANEL_TOP + SuiteTheme.PAD;

        this.headerLabel = new StringWidget(innerX, y, innerW, HEADER_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.headerLabel);
        y += HEADER_H + 2;

        int footerY = SuiteTheme.footerY(this.height);
        if (this.mode == Mode.LIST) {
            initList(innerX, innerW, y, footerY);
        } else {
            initEdit(innerX, innerW, y, footerY);
        }
        this.refresh();
    }

    // ---------------------------------------------------------------
    // LIST mode
    // ---------------------------------------------------------------

    private void initList(int innerX, int innerW, int rowsTop, int footerY) {
        int rowsBottom = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H - 2;
        this.pageSize = Math.max(1, (rowsBottom - rowsTop) / ROW_H);
        for (int i = 0; i < this.pageSize; i++) {
            final int slot = i;
            Button row = Button.builder(Component.empty(), b -> this.editSlot(slot))
                    .bounds(innerX, rowsTop + i * ROW_H, innerW, ROW_H - 2)
                    .createNarration(supplier -> this.rowNarration(slot))
                    .build();
            this.rowButtons.add(row);
            this.addRenderableWidget(row);
        }

        this.emptyLabel = new StringWidget(innerX, rowsTop + 4, innerW, 12,
                Component.translatable("espectral.macros.empty"), this.font);
        this.addRenderableWidget(this.emptyLabel);

        int pagerY = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H;
        int pagerW = 40;
        this.pagerPrev = Button.builder(Component.translatable("espectral.suite.button.prev"), b -> {
            this.page = Math.max(0, this.page - 1);
            this.refresh();
        }).bounds(innerX, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerPrev);
        this.pagerNext = Button.builder(Component.translatable("espectral.suite.button.next"), b -> {
            this.page = Math.min(this.lastPages - 1, this.page + 1);
            this.refresh();
        }).bounds(innerX + innerW - pagerW, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerNext);
        this.pagerLabel = new StringWidget(innerX + pagerW + SuiteTheme.GAP, pagerY,
                innerW - 2 * (pagerW + SuiteTheme.GAP), SuiteTheme.PAGER_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.pagerLabel);

        int btnW = (innerW - SuiteTheme.GAP) / 2;
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.macros.add"),
                b -> this.startNew())
                .bounds(innerX, footerY, btnW, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.suite.back"),
                b -> this.onClose())
                .bounds(innerX + btnW + SuiteTheme.GAP, footerY, btnW, BUTTON_H).build());
    }

    private void refreshListWidgets() {
        ClientConfig config = ClientConfig.getInstance();
        List<ClientConfig.MacroConfig> all = config.getMacros();
        this.lastPages = Math.max(1, (all.size() + this.pageSize - 1) / this.pageSize);
        this.page = Math.min(Math.max(0, this.page), this.lastPages - 1);
        this.rowMacros.clear();
        for (int i = this.page * this.pageSize;
                i < all.size() && this.rowMacros.size() < this.pageSize; i++) {
            this.rowMacros.add(all.get(i));
        }
        for (int i = 0; i < this.rowButtons.size(); i++) {
            Button button = this.rowButtons.get(i);
            if (i < this.rowMacros.size()) {
                ClientConfig.MacroConfig macro = this.rowMacros.get(i);
                button.setMessage(rowCaption(macro));
                button.setTooltip(Tooltip.create(Component.literal(macroSummary(macro))));
                button.active = true;
                button.visible = true;
            } else {
                button.visible = false;
            }
        }
        this.emptyLabel.visible = all.isEmpty();
        boolean paged = this.lastPages > 1;
        this.pagerPrev.visible = paged;
        this.pagerNext.visible = paged;
        this.pagerLabel.visible = paged;
        if (paged) {
            this.pagerLabel.setMessage(Component.translatable("espectral.suite.page",
                    this.page + 1, this.lastPages));
        }
    }

    private MutableComponent rowNarration(int slot) {
        if (slot >= this.rowMacros.size()) return Component.translatable("espectral.macros.title").copy();
        ClientConfig.MacroConfig macro = this.rowMacros.get(slot);
        return Component.translatable("espectral.macros.edit").copy()
                .append(Component.literal(": " + macroSummary(macro)));
    }

    /** {@code [ñ]  /helpop ayuda} — key chip then the action text. */
    private static MutableComponent rowCaption(ClientConfig.MacroConfig macro) {
        MutableComponent out = Component.literal("[")
                .withStyle(ChatFormatting.GRAY)
                .append(MacroEngine.keyDisplayName(macro.keybind)
                        .copy().withStyle(SuiteTheme.accentFormat()))
                .append(Component.literal("] ").withStyle(ChatFormatting.GRAY));
        String text = firstActionText(macro);
        out.append(Component.literal(truncate(text, 32))
                .withStyle(text.startsWith("/") ? ChatFormatting.YELLOW : ChatFormatting.WHITE));
        return out;
    }

    private static String macroSummary(ClientConfig.MacroConfig macro) {
        String name = macro.name == null || macro.name.isBlank() ? macro.id : macro.name;
        return name + " · " + macro.keybind + " · " + firstActionText(macro);
    }

    private static String firstActionText(ClientConfig.MacroConfig macro) {
        if (macro.actions == null || macro.actions.isEmpty()) return "";
        String text = macro.actions.get(0).text;
        return text == null ? "" : text;
    }

    private static String truncate(String text, int max) {
        if (text == null) return "";
        return text.length() <= max ? text : text.substring(0, Math.max(0, max - 1)) + "…";
    }

    // ---------------------------------------------------------------
    // EDIT mode
    // ---------------------------------------------------------------

    private void initEdit(int innerX, int innerW, int y, int footerY) {
        int labelW = 62;
        int fieldX = innerX + labelW + SuiteTheme.GAP;
        int fieldW = innerW - labelW - SuiteTheme.GAP;

        StringWidget nameLabel = new StringWidget(innerX, y + 5, labelW, 12,
                Component.translatable("espectral.macros.name"), this.font);
        this.addRenderableWidget(nameLabel);
        EditBox nameBox = new EditBox(this.font, fieldX, y, fieldW, FIELD_H,
                Component.translatable("espectral.macros.name"));
        nameBox.setMaxLength(ClientConfig.MAX_MACRO_NAME);
        nameBox.setHint(Component.translatable("espectral.macros.name.hint"));
        nameBox.setValue(this.draftName);
        nameBox.setResponder(value -> this.draftName = value);
        this.addRenderableWidget(nameBox);
        y += FIELD_H + 3;

        StringWidget keyLabel = new StringWidget(innerX, y + 4, labelW, 12,
                Component.translatable("espectral.macros.key"), this.font);
        this.addRenderableWidget(keyLabel);
        this.keyButton = Button.builder(Component.empty(), b -> {
            this.captureActive = !this.captureActive;
            if (this.captureActive) this.confirmPending = false;
            this.refresh();
        }).bounds(fieldX, y, fieldW, BUTTON_H)
                .tooltip(Tooltip.create(Component.translatable("espectral.macros.key.hint")))
                .build();
        this.addRenderableWidget(this.keyButton);
        y += BUTTON_H + 3;

        StringWidget textLabel = new StringWidget(innerX, y + 5, labelW, 12,
                Component.translatable("espectral.macros.text"), this.font);
        this.addRenderableWidget(textLabel);
        EditBox textBox = new EditBox(this.font, fieldX, y, fieldW, FIELD_H,
                Component.translatable("espectral.macros.text"));
        textBox.setMaxLength(ClientConfig.MAX_MACRO_TEXT);
        textBox.setHint(Component.translatable("espectral.macros.text.hint"));
        textBox.setValue(this.draftText);
        textBox.setResponder(value -> {
            this.draftText = value;
            this.confirmPending = false;
        });
        this.addRenderableWidget(textBox);
        y += FIELD_H + 3;

        this.statusLabel = new StringWidget(innerX, y, innerW, 12,
                Component.empty(), this.font);
        this.addRenderableWidget(this.statusLabel);

        int btnW = (innerW - 2 * SuiteTheme.GAP) / 3;
        this.saveButton = Button.builder(Component.translatable("espectral.macros.save"),
                b -> this.saveDraft())
                .bounds(innerX, footerY, btnW, BUTTON_H).build();
        this.addRenderableWidget(this.saveButton);
        this.deleteButton = Button.builder(Component.translatable("espectral.macros.delete"),
                b -> this.confirmDelete())
                .bounds(innerX + btnW + SuiteTheme.GAP, footerY, btnW, BUTTON_H).build();
        this.addRenderableWidget(this.deleteButton);
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.macros.cancel"),
                b -> this.backToList())
                .bounds(innerX + 2 * (btnW + SuiteTheme.GAP), footerY, btnW, BUTTON_H).build());

        this.deleteButton.active = this.draftId != null;
    }

    private void refreshEditWidgets() {
        this.keyButton.setMessage(this.captureActive
                ? Component.translatable("espectral.macros.key.capture")
                        .withStyle(SuiteTheme.accentFormat())
                : (this.draftKey == null || this.draftKey.isEmpty()
                        ? Component.translatable("espectral.macros.key.capture")
                                .withStyle(ChatFormatting.GRAY)
                        : MacroEngine.keyDisplayName(this.draftKey)
                                .copy().withStyle(SuiteTheme.accentFormat())));
        this.saveButton.setMessage(Component.translatable(this.confirmPending
                ? "espectral.macros.confirm" : "espectral.macros.save"));
    }

    private void startNew() {
        this.mode = Mode.EDIT;
        this.draftId = null;
        this.draftName = "";
        this.draftKey = "";
        this.draftText = "";
        this.captureActive = false;
        this.confirmPending = false;
        this.lastStatus = null;
        this.rebuildWidgets();
    }

    private void editSlot(int slot) {
        if (slot >= this.rowMacros.size()) return;
        ClientConfig.MacroConfig macro = this.rowMacros.get(slot);
        this.mode = Mode.EDIT;
        this.draftId = macro.id;
        this.draftName = macro.name;
        this.draftKey = macro.keybind;
        this.draftText = firstActionText(macro);
        this.captureActive = false;
        this.confirmPending = false;
        this.lastStatus = null;
        this.rebuildWidgets();
    }

    private void backToList() {
        this.mode = Mode.LIST;
        this.captureActive = false;
        this.confirmPending = false;
        this.lastStatus = null;
        this.rebuildWidgets();
    }

    private void saveDraft() {
        String text = this.draftText == null ? "" : this.draftText.trim();
        if (text.isEmpty()) {
            this.setStatus(Component.translatable("espectral.macros.needtext"), true);
            return;
        }
        if (this.draftKey == null || this.draftKey.isEmpty()) {
            this.setStatus(Component.translatable("espectral.macros.needkey"), true);
            return;
        }
        if (MacroEngine.isReserved(MacroEngine.getGlfwKeyCode(this.draftKey))) {
            this.setStatus(Component.translatable("espectral.macros.reserved"), true);
            return;
        }
        MacroEngine.Conflict conflict = MacroEngine.findConflict(this.draftKey, this.draftId);
        if (conflict != null) {
            if (conflict.fromMacro()) {
                this.setStatus(Component.translatable("espectral.macros.dupkey",
                        conflict.label()), true);
                return;
            }
            if (!this.confirmPending) {
                this.confirmPending = true;
                this.setStatus(Component.translatable("espectral.macros.conflict",
                        conflict.label()), true);
                return;
            }
        }

        String name = this.draftName == null ? "" : this.draftName.trim();
        if (name.isEmpty()) name = deriveName(text);
        String id = this.draftId != null ? this.draftId
                : ClientConfig.getInstance().newMacroId(name);
        boolean ok = ClientConfig.getInstance().putMacro(id, name, this.draftKey,
                text.startsWith("/") ? "command" : "chat", text);
        if (!ok) {
            this.setStatus(Component.translatable("espectral.macros.full",
                    ClientConfig.MAX_MACROS), true);
            return;
        }
        this.backToList();
    }

    /** Fallback label when the name field is left empty. */
    private static String deriveName(String text) {
        String base = text.startsWith("/") ? text.substring(1) : text;
        base = base.trim();
        if (base.isEmpty()) base = "macro";
        return base.length() <= ClientConfig.MAX_MACRO_NAME
                ? base : base.substring(0, ClientConfig.MAX_MACRO_NAME);
    }

    private void confirmDelete() {
        if (this.draftId == null) return;
        String label = this.draftName == null || this.draftName.isBlank()
                ? this.draftId : this.draftName;
        final String id = this.draftId;
        Compat.open(this.minecraft, new SuiteConfirmScreen(this,
                "espectral.macros.delete.title",
                "espectral.macros.delete.message",
                "espectral.macros.delete.confirm",
                () -> {
                    ClientConfig.getInstance().removeMacro(id);
                    Minecraft minecraft = Minecraft.getInstance();
                    if (minecraft != null) {
                        Compat.open(minecraft, new SuiteMacrosScreen(this.parent));
                    }
                },
                label));
    }

    /** Last status line text (kept across a resize rebuild). */
    private Component lastStatus;
    private boolean lastStatusIsError;

    private void setStatus(Component text, boolean error) {
        this.lastStatus = text;
        this.lastStatusIsError = error;
        this.refresh();
    }

    // ---------------------------------------------------------------
    // Shared refresh
    // ---------------------------------------------------------------

    private void refresh() {
        if (this.headerLabel != null) {
            String titleKey = this.mode == Mode.EDIT
                    ? (this.draftId != null ? "espectral.macros.edit" : "espectral.macros.new")
                    : "espectral.macros.title";
            MutableComponent header = Component.translatable(titleKey)
                    .withStyle(SuiteTheme.accentFormat());
            int count = ClientConfig.getInstance().getMacros().size();
            header.append(Component.literal("  " + count + "/" + ClientConfig.MAX_MACROS)
                    .withStyle(ChatFormatting.GRAY));
            this.headerLabel.setMessage(header);
        }
        if (this.mode == Mode.LIST) {
            this.refreshListWidgets();
        } else {
            this.refreshEditWidgets();
            if (this.statusLabel != null) {
                if (this.lastStatus == null) {
                    this.statusLabel.setMessage(Component.translatable(
                            "espectral.macros.key.hint").withStyle(ChatFormatting.GRAY));
                } else {
                    this.statusLabel.setMessage(this.lastStatus.copy().withStyle(
                            this.lastStatusIsError ? SuiteTheme.accentFormat()
                                    : ChatFormatting.GRAY));
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // Input
    // ---------------------------------------------------------------

    @Override
    public boolean keyPressed(KeyEvent event) {
        if (this.captureActive) {
            int code = event.key();
            if (code == org.lwjgl.glfw.GLFW.GLFW_KEY_ESCAPE) {
                // Esc aborts the capture, never the screen.
                this.captureActive = false;
                this.refresh();
                return true;
            }
            if (MacroEngine.isReserved(code)) {
                this.setStatus(Component.translatable("espectral.macros.reserved"), true);
                return true;
            }
            String stored = MacroEngine.keybindFor(code);
            if (stored.isEmpty()) {
                // GLFW_KEY_UNKNOWN: no keysym to poll, so it cannot be bound.
                this.setStatus(Component.translatable("espectral.macros.needkey"), true);
                return true;
            }
            this.draftKey = stored;
            this.captureActive = false;
            this.confirmPending = false;
            this.lastStatus = null;
            this.refresh();
            return true;
        }
        return super.keyPressed(event);
    }

    @Override
    public void onClose() {
        ClientConfig.getInstance().save();
        if (this.mode == Mode.EDIT) {
            this.backToList();
            return;
        }
        Minecraft minecraft = Minecraft.getInstance();
        if (this.parent != null && minecraft != null) {
            Compat.open(minecraft, this.parent);
            return;
        }
        super.onClose();
    }

    @Override
    public Component getNarrationMessage() {
        return Component.translatable("espectral.macros.title");
    }

    @Override
    public void paintSheet(Object gfx, int width, int height, Font font) {
        SuitePainter.paintSheet(gfx, width, height);
    }
}
