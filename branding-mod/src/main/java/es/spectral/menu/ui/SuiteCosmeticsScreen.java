package es.spectral.menu.ui;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;

import com.google.gson.JsonNull;
import com.google.gson.JsonObject;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;
import es.spectral.menu.cosmetics.WardrobeBridge;

/**
 * Cosmetics, opened from the "⚙" button on the Suite's Cosméticos row. Two
 * kinds of setting live here:
 *
 * <ul>
 *   <li><b>What you see</b> (local, nothing leaves the game): show my
 *       cosmetics, show other players' cosmetics, which pieces of my own
 *       loadout are drawn, the emote wheel and cosmetic motion. State lives
 *       under {@code features.cosmetics.*} in
 *       {@code config/espectral-client.json} and is read on the render paths
 *       through {@link ClientConfig#cosmeticsState()} (one volatile field, no
 *       I/O).</li>
 *   <li><b>What you wear</b> (launcher-owned): sharing presence, emotes
 *       consent and the item in each slot, edited live through the signed
 *       bridge ({@link WardrobeBridge} → {@code POST /api/bridge/cosmetics}).
 *       The engine validates the change against the catalog with the
 *       account's real flags, stores it and rewrites the file this game reads,
 *       so what the screen shows afterwards is the launcher's state —
 *       refusals included. All HTTP runs on a daemon thread; this screen only
 *       reads volatile snapshots and repaints on {@code tick()}.</li>
 * </ul>
 *
 * <p>Two modes share the sheet so the 240&nbsp;px logical height stays
 * readable: {@link Mode#LIST} (the switches plus one row per slot showing what
 * is worn) and {@link Mode#EDIT} (that slot's pieces, two per line, paged when
 * a slot outgrows the space). A mode change calls {@code rebuildWidgets()}, so
 * neither it nor a resize can stack widgets; geometry is recomputed in
 * {@code init()} from width/height only.
 */
public class SuiteCosmeticsScreen extends Screen implements SuiteSheet {

    private enum Mode { LIST, EDIT }

    /** Slot order (cosmetics catalog) — the same ids the launcher validates. */
    private static final String[] SLOTS = {"espalda", "cuello", "cabeza", "elitros"};
    /** Viewer switches per line in the switch grid. */
    private static final int COLS = 2;
    private static final int ROW_H = 20;
    private static final int SLOT_H = 20;
    private static final int CHIP_H = 18;
    private static final int MSG_H = 10;

    private final Screen parent;
    private Mode mode = Mode.LIST;
    /** Slot being edited in {@link Mode#EDIT}. */
    private String editSlot = SLOTS[0];
    private int page;
    private int lastPages = 1;
    private int pageSize;
    /** Last {@link WardrobeBridge#serial()} painted; drives the tick repaint. */
    private int paintedSerial = -1;

    private StringWidget headerLabel;
    private Button ownButton;
    private Button othersButton;
    private Button emotesButton;
    private Button physicsButton;
    private Button shareButton;
    private Button consentEmotesButton;
    private StringWidget messageLabel;

    /** LIST: one row per slot (opens the picker). EDIT: the slot's pieces. */
    private final List<Button> rowButtons = new ArrayList<>();
    /** EDIT: catalog ids the visible chip buttons stand for ("" = Nada). */
    private final List<String> chipIds = new ArrayList<>();
    private Button visibilityButton;
    private Button pagerPrev;
    private Button pagerNext;
    private StringWidget pagerLabel;

    public SuiteCosmeticsScreen(Screen parent) {
        super(Component.translatable("espectral.cosmetics.title"));
        this.parent = parent;
    }

    /** Parent screen, so a nested dialog can return here. */
    public Screen parentScreen() {
        return parent;
    }

    @Override
    protected void init() {
        super.init();
        ClientConfig.getInstance().load();
        this.clearWidgets();
        this.rowButtons.clear();
        this.chipIds.clear();

        int innerX = SuiteTheme.panelX(this.width) + SuiteTheme.MARGIN;
        int innerW = SuiteTheme.panelWidth(this.width) - 2 * SuiteTheme.MARGIN;
        int footerY = SuiteTheme.footerY(this.height);
        int y = SuiteTheme.PANEL_TOP + SuiteTheme.PAD;

        this.headerLabel = new StringWidget(innerX, y, innerW, SuiteTheme.HEADER_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.headerLabel);
        y += SuiteTheme.HEADER_H + 2;

        if (this.mode == Mode.LIST) {
            initList(innerX, innerW, y, footerY);
        } else {
            initEdit(innerX, innerW, y, footerY);
        }
        // Cheap and signed: asking again on every open keeps a stale screen
        // from claiming a loadout the launcher no longer has.
        WardrobeBridge.requestLoad();
        this.refresh();
    }

    // ---------------------------------------------------------------
    // LIST mode
    // ---------------------------------------------------------------

    private void initList(int innerX, int innerW, int y, int footerY) {
        int colGap = SuiteTheme.GAP;
        int colW = (innerW - colGap) / COLS;
        // "What you see" — the local viewer switches, two per line.
        this.ownButton = localToggle(innerX, y, colW, "espectral.cosmetics.short.own",
                "espectral.cosmetics.own", ClientConfig.COSMETICS_OWN);
        this.othersButton = localToggle(innerX + colW + colGap, y, colW,
                "espectral.cosmetics.short.others", "espectral.cosmetics.others",
                ClientConfig.COSMETICS_OTHERS);
        y += ROW_H + 2;
        this.emotesButton = localToggle(innerX, y, colW, "espectral.cosmetics.short.emotes",
                "espectral.cosmetics.emotes", ClientConfig.COSMETICS_EMOTES);
        this.physicsButton = localToggle(innerX + colW + colGap, y, colW,
                "espectral.cosmetics.short.physics", "espectral.cosmetics.physics",
                ClientConfig.COSMETICS_PHYSICS);
        y += ROW_H + 2;
        // "What you wear" — launcher state through the bridge.
        this.shareButton = bridgeButton(innerX, y, colW, "espectral.cosmetics.short.share",
                "espectral.cosmetics.share", this::toggleShare);
        this.consentEmotesButton = bridgeButton(innerX + colW + colGap, y, colW,
                "espectral.cosmetics.short.consent", "espectral.cosmetics.consent.emotes",
                this::toggleConsentEmotes);
        y += ROW_H + 3;

        this.messageLabel = new StringWidget(innerX, y, innerW, MSG_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.messageLabel);
        y += MSG_H + 3;

        // One row per slot: what is worn; a click opens that slot's pieces.
        int room = footerY - y - SuiteTheme.GAP;
        int rows = Math.max(1, Math.min(SLOTS.length, room / SLOT_H));
        for (int i = 0; i < rows; i++) {
            final String slot = SLOTS[i];
            Button row = Button.builder(Component.empty(), b -> this.openSlot(slot))
                    .bounds(innerX, y + i * SLOT_H, innerW, SLOT_H - 2)
                    .build();
            this.rowButtons.add(row);
            this.addRenderableWidget(row);
        }
        addFooter(innerX, innerW, footerY);
    }

    private void openSlot(String slot) {
        this.editSlot = slot;
        this.page = 0;
        this.mode = Mode.EDIT;
        this.rebuildWidgets();
    }

    // ---------------------------------------------------------------
    // EDIT mode
    // ---------------------------------------------------------------

    private void initEdit(int innerX, int innerW, int y, int footerY) {
        this.messageLabel = new StringWidget(innerX, y, innerW, MSG_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.messageLabel);
        y += MSG_H + 3;

        // "Mostrar esta pieza" (local viewer switch for this slot).
        this.visibilityButton = Button.builder(Component.empty(), b -> {
            ClientConfig config = ClientConfig.getInstance();
            String key = ClientConfig.cosmeticsSlotKey(this.editSlot);
            config.setCosmeticsState(key, !config.getCosmeticsState(key));
            refresh();
        }).bounds(innerX, y, innerW, ROW_H).build();
        this.addRenderableWidget(this.visibilityButton);
        y += ROW_H + 3;

        int pagerY = footerY - SuiteTheme.PAGER_H - SuiteTheme.GAP;
        int chipRoom = pagerY - y - SuiteTheme.GAP;
        int rows = Math.max(1, Math.min(5, chipRoom / (CHIP_H + 3)));
        this.pageSize = rows * COLS;
        int chipW = (innerW - SuiteTheme.GAP) / COLS;
        for (int i = 0; i < this.pageSize; i++) {
            final int index = i;
            Button chip = Button.builder(Component.empty(), b -> this.pickChip(index))
                    .bounds(innerX + (i % COLS) * (chipW + SuiteTheme.GAP),
                            y + (i / COLS) * (CHIP_H + 3), chipW, CHIP_H)
                    .build();
            this.rowButtons.add(chip);
            this.addRenderableWidget(chip);
        }

        int pagerW = 40;
        this.pagerPrev = Button.builder(Component.translatable("espectral.suite.button.prev"), b -> {
            this.page = Math.max(0, this.page - 1);
            refresh();
        }).bounds(innerX, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerPrev);
        this.pagerNext = Button.builder(Component.translatable("espectral.suite.button.next"), b -> {
            this.page = Math.min(this.lastPages - 1, this.page + 1);
            refresh();
        }).bounds(innerX + innerW - pagerW, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerNext);
        this.pagerLabel = new StringWidget(innerX + pagerW + SuiteTheme.GAP, pagerY,
                innerW - 2 * (pagerW + SuiteTheme.GAP), SuiteTheme.PAGER_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.pagerLabel);

        addFooter(innerX, innerW, footerY);
    }

    private void backToList() {
        this.mode = Mode.LIST;
        this.page = 0;
        this.rebuildWidgets();
    }

    /** The one footer button: back a level in EDIT, close the sheet in LIST. */
    private void addFooter(int innerX, int innerW, int footerY) {
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.suite.back"),
                b -> {
                    if (this.mode == Mode.EDIT) {
                        this.backToList();
                    } else {
                        this.onClose();
                    }
                })
                .bounds(innerX, footerY, innerW, SuiteTheme.FOOTER_H).build());
    }

    // ---------------------------------------------------------------
    // Widget builders
    // ---------------------------------------------------------------

    /**
     * A viewer switch stored under {@code features.cosmetics.<stateKey>}. The
     * caption is the short label (it must fit a half-width chip); the tooltip
     * and the narration carry the full one.
     */
    private Button localToggle(int x, int y, int w, String shortKey, String fullKey,
            String stateKey) {
        Button button = Button.builder(Component.empty(), b -> {
            ClientConfig config = ClientConfig.getInstance();
            config.setCosmeticsState(stateKey, !config.getCosmeticsState(stateKey));
            refresh();
        }).bounds(x, y, w, ROW_H)
                .tooltip(Tooltip.create(Component.translatable(fullKey)))
                .createNarration(supplier -> Component.empty()
                        .append(Component.translatable(fullKey))
                        .append(Component.literal(": "))
                        .append(Component.translatable(
                                ClientConfig.getInstance().getCosmeticsState(stateKey)
                                        ? "espectral.suite.state.on"
                                        : "espectral.suite.state.off")).copy())
                .build();
        this.addRenderableWidget(button);
        return button;
    }

    /** A switch whose value lives in the launcher (share, emotes consent). */
    private Button bridgeButton(int x, int y, int w, String shortKey, String fullKey,
            Runnable onClick) {
        Button button = Button.builder(Component.empty(), b -> onClick.run())
                .bounds(x, y, w, ROW_H)
                .tooltip(Tooltip.create(Component.translatable(fullKey)))
                .build();
        this.addRenderableWidget(button);
        return button;
    }

    // ---------------------------------------------------------------
    // Wardrobe actions (bridge)
    // ---------------------------------------------------------------

    private void toggleShare() {
        WardrobeBridge.Wardrobe view = WardrobeBridge.wardrobe();
        if (view == null || WardrobeBridge.busy()) return;
        JsonObject patch = new JsonObject();
        patch.addProperty("share", !view.share());
        WardrobeBridge.requestPatch(patch);
        refresh();
    }

    private void toggleConsentEmotes() {
        WardrobeBridge.Wardrobe view = WardrobeBridge.wardrobe();
        if (view == null || WardrobeBridge.busy()) return;
        JsonObject patch = new JsonObject();
        patch.addProperty("emotes", !view.emotes());
        WardrobeBridge.requestPatch(patch);
        refresh();
    }

    /**
     * Equips (or clears, with {@code ""}) one slot. The patch carries every
     * slot because the engine validates the whole loadout object it receives
     * and an omitted slot would be cleared; slots it answers with stay worn.
     */
    private void equip(String slot, String itemId) {
        WardrobeBridge.Wardrobe view = WardrobeBridge.wardrobe();
        if (view == null || WardrobeBridge.busy()) return;
        JsonObject loadout = new JsonObject();
        for (WardrobeBridge.Slot s : view.slots()) {
            String worn = view.loadout().get(s.id());
            if (s.id().equals(slot)) {
                if (itemId == null || itemId.isEmpty()) loadout.add(s.id(), JsonNull.INSTANCE);
                else loadout.addProperty(s.id(), itemId);
            } else if (worn == null) {
                loadout.add(s.id(), JsonNull.INSTANCE);
            } else {
                loadout.addProperty(s.id(), worn);
            }
        }
        JsonObject patch = new JsonObject();
        patch.add("loadout", loadout);
        WardrobeBridge.requestPatch(patch);
        refresh();
    }

    private void pickChip(int index) {
        if (index >= this.chipIds.size()) return;
        equip(this.editSlot, this.chipIds.get(index));
    }

    // ---------------------------------------------------------------
    // Refresh
    // ---------------------------------------------------------------

    @Override
    public void tick() {
        super.tick();
        if (WardrobeBridge.serial() != this.paintedSerial) {
            // An answer (or a refusal) landed on the bridge thread: repaint.
            this.refresh();
        }
    }

    /** Rewrites every caption from the current state (adds no widgets). */
    private void refresh() {
        this.paintedSerial = WardrobeBridge.serial();
        WardrobeBridge.Wardrobe view = WardrobeBridge.wardrobe();
        boolean usable = view != null && WardrobeBridge.reachable();
        boolean mvp = view != null && view.mvp();
        int innerW = SuiteTheme.panelWidth(this.width) - 2 * SuiteTheme.MARGIN;

        if (this.headerLabel != null) {
            MutableComponent header = Component.translatable("espectral.cosmetics.title")
                    .withStyle(SuiteTheme.accentFormat());
            if (this.mode == Mode.EDIT) {
                header.append(Component.literal("   ·   " + slotLabel(this.editSlot))
                        .withStyle(ChatFormatting.GRAY));
            }
            this.headerLabel.setMessage(header);
        }

        if (this.mode == Mode.LIST) {
            refreshList(view, usable, mvp, innerW);
        } else {
            refreshEdit(view, usable, mvp);
        }
        refreshMessage(view, usable, mvp);
    }

    private void refreshList(WardrobeBridge.Wardrobe view, boolean usable, boolean mvp,
            int innerW) {
        this.ownButton.setMessage(labeled("espectral.cosmetics.short.own",
                ClientConfig.COSMETICS_OWN));
        this.othersButton.setMessage(labeled("espectral.cosmetics.short.others",
                ClientConfig.COSMETICS_OTHERS));
        this.emotesButton.setMessage(labeled("espectral.cosmetics.short.emotes",
                ClientConfig.COSMETICS_EMOTES));
        this.physicsButton.setMessage(labeled("espectral.cosmetics.short.physics",
                ClientConfig.COSMETICS_PHYSICS));

        boolean switches = usable && !WardrobeBridge.busy();
        this.shareButton.setMessage(switchCaption("espectral.cosmetics.short.share",
                view != null && view.share()));
        this.shareButton.active = switches;
        this.consentEmotesButton.setMessage(switchCaption("espectral.cosmetics.short.consent",
                view != null && view.emotes()));
        this.consentEmotesButton.active = switches;

        for (int i = 0; i < this.rowButtons.size(); i++) {
            Button row = this.rowButtons.get(i);
            row.setMessage(slotCaption(SLOTS[i], innerW));
            row.setTooltip(Tooltip.create(Component.translatable(usable && mvp
                    ? "espectral.cosmetics.equip.hint"
                    : usable ? "espectral.cosmetics.mvp" : "espectral.cosmetics.offline")));
            row.active = usable && mvp;
        }
    }

    private void refreshEdit(WardrobeBridge.Wardrobe view, boolean usable, boolean mvp) {
        String visibilityKey = ClientConfig.cosmeticsSlotKey(this.editSlot);
        boolean shown = ClientConfig.getInstance().getCosmeticsState(visibilityKey);
        this.visibilityButton.setMessage(Component.empty()
                .append(Component.translatable("espectral.cosmetics.visible"))
                .append(Component.literal("   ·   "))
                .append(Component.translatable(shown ? "espectral.suite.state.on"
                        : "espectral.suite.state.off")
                        .withStyle(shown ? ChatFormatting.GREEN : ChatFormatting.GRAY)));

        List<String> ids = new ArrayList<>();
        List<String> names = new ArrayList<>();
        // "Nada" first (clears the slot), then the catalog order.
        ids.add("");
        names.add(Component.translatable("espectral.cosmetics.none").getString());
        if (view != null) {
            for (WardrobeBridge.Slot s : view.slots()) {
                if (!s.id().equals(this.editSlot)) continue;
                for (WardrobeBridge.Item item : s.items()) {
                    ids.add(item.id());
                    names.add(itemName(item));
                }
            }
        }
        this.lastPages = Math.max(1, (ids.size() + this.pageSize - 1) / this.pageSize);
        this.page = Math.min(Math.max(0, this.page), this.lastPages - 1);
        String worn = view == null ? null : view.loadout().get(this.editSlot);
        boolean equip = usable && mvp && !WardrobeBridge.busy();

        this.chipIds.clear();
        for (int i = 0; i < this.rowButtons.size(); i++) {
            Button chip = this.rowButtons.get(i);
            int source = this.page * this.pageSize + i;
            if (source >= ids.size()) {
                chip.visible = false;
                continue;
            }
            String id = ids.get(source);
            String name = names.get(source);
            boolean equipped = id.isEmpty() ? worn == null : id.equals(worn);
            this.chipIds.add(id);
            chip.visible = true;
            chip.active = equip && !equipped;
            chip.setMessage(chipCaption(name, equipped));
            chip.setTooltip(Tooltip.create(Component.literal(name)));
        }
        boolean paged = this.lastPages > 1;
        this.pagerPrev.visible = paged;
        this.pagerNext.visible = paged;
        this.pagerLabel.visible = paged;
        if (paged) {
            this.pagerLabel.setMessage(Component.translatable("espectral.suite.page",
                    this.page + 1, this.lastPages));
        }
    }

    /**
     * The one line above the rows: the launcher's last refusal (red) when
     * there is one, else what the user can do right now.
     */
    private void refreshMessage(WardrobeBridge.Wardrobe view, boolean usable, boolean mvp) {
        // A refusal from the engine wins: it explains why the last tap did nothing.
        String errorKey = WardrobeBridge.errorKey();
        if (errorKey != null) {
            this.messageLabel.setMessage(Component.translatable(errorKey)
                    .withStyle(ChatFormatting.RED));
            return;
        }
        String key;
        ChatFormatting colour = ChatFormatting.GRAY;
        if (view == null) {
            key = WardrobeBridge.configured() ? "espectral.cosmetics.loading"
                    : "espectral.cosmetics.offline";
        } else if (!usable) {
            key = "espectral.cosmetics.offline";
        } else if (!mvp) {
            key = "espectral.cosmetics.mvp";
            colour = SuiteTheme.accentFormat();
        } else if (this.mode == Mode.EDIT) {
            key = "espectral.cosmetics.equip.hint";
        } else {
            key = "espectral.cosmetics.wardrobe.hint";
        }
        this.messageLabel.setMessage(Component.translatable(key).withStyle(colour));
    }

    // ---------------------------------------------------------------
    // Captions
    // ---------------------------------------------------------------

    /** {@code Mis cosméticos   ·   Activada}; the tooltip (set where the button is built) carries the full label. */
    private static MutableComponent labeled(String shortKey, String stateKey) {
        return switchCaption(shortKey, ClientConfig.getInstance().getCosmeticsState(stateKey));
    }

    private static MutableComponent switchCaption(String shortKey, boolean on) {
        return Component.empty()
                .append(Component.translatable(shortKey))
                .append(Component.literal("   ·   "))
                .append(Component.translatable(on ? "espectral.suite.state.on"
                        : "espectral.suite.state.off")
                        .withStyle(on ? ChatFormatting.GREEN : ChatFormatting.GRAY));
    }

    private static MutableComponent chipCaption(String name, boolean equipped) {
        return Component.literal(equipped ? "✓ " + name : name)
                .withStyle(equipped ? SuiteTheme.accentFormat() : ChatFormatting.WHITE);
    }

    /** {@code Cabeza · Orejas Felinas}, trimmed to the row so it never overflows. */
    private MutableComponent slotCaption(String slot, int innerW) {
        String label = slotLabel(slot);
        String head = label + "   ";
        String worn = null;
        WardrobeBridge.Wardrobe view = WardrobeBridge.wardrobe();
        if (view != null) {
            String id = view.loadout().get(slot);
            if (id != null) {
                worn = id;
                for (WardrobeBridge.Slot s : view.slots()) {
                    if (!s.id().equals(slot)) continue;
                    for (WardrobeBridge.Item item : s.items()) {
                        if (item.id().equals(id)) {
                            worn = itemName(item);
                            break;
                        }
                    }
                }
            }
        }
        MutableComponent out = Component.literal(head).withStyle(SuiteTheme.accentFormat());
        String text = worn == null
                ? Component.translatable("espectral.cosmetics.none").getString() : worn;
        int room = innerW - 18 - this.font.width(head);
        return out.append(Component.literal(trimTo(text, room))
                .withStyle(worn == null ? ChatFormatting.GRAY : ChatFormatting.WHITE));
    }

    private static String slotLabel(String slot) {
        return Component.translatable("espectral.slot." + slot).getString();
    }

    private static String itemName(WardrobeBridge.Item item) {
        boolean es = language().startsWith("es");
        String name = es ? item.es() : item.en();
        if (name == null) name = es ? item.en() : item.es();
        return name == null ? item.id() : name;
    }

    private static String language() {
        Minecraft minecraft = Minecraft.getInstance();
        return minecraft == null || minecraft.getLanguageManager() == null
                ? "en_us" : minecraft.getLanguageManager().getSelected();
    }

    /** Trims to {@code width} pixels with an ellipsis; never null. */
    private String trimTo(String text, int width) {
        if (text == null) return "";
        if (width <= 0) return "";
        if (this.font.width(text) <= width) return text;
        return this.font.plainSubstrByWidth(text, Math.max(0, width - this.font.width("…"))) + "…";
    }

    // ---------------------------------------------------------------
    // Screen plumbing
    // ---------------------------------------------------------------

    @Override
    public void onClose() {
        ClientConfig.getInstance().save();
        if (this.mode == Mode.EDIT) {
            // Esc leaves the item picker first, like the Macros editor.
            this.backToList();
            return;
        }
        Minecraft minecraft = Minecraft.getInstance();
        if (parent != null && minecraft != null) {
            Compat.open(minecraft, parent);
            return;
        }
        super.onClose();
    }

    @Override
    public Component getNarrationMessage() {
        return Component.translatable("espectral.cosmetics.title");
    }

    @Override
    public void paintSheet(Object gfx, int width, int height, Font font) {
        SuitePainter.paintSheet(gfx, width, height);
    }
}
