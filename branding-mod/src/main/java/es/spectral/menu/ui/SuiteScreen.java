package es.spectral.menu.ui;

import java.text.Normalizer;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;
import es.spectral.menu.FeatureRegistry;

/**
 * The Suite control panel: header with the {@code suite.enabled} master chip,
 * live search, category filter, paginated feature rows with explicit
 * enabled/disabled words, and a Reiniciar / Apoyar / Listo footer.
 *
 * <p>All widget geometry is recomputed in {@code init()} from width/height
 * only. The search responder never adds or removes widgets: it only mutates
 * the existing row slots in place (via {@link #refreshList()}), so typing
 * never drops EditBox focus and re-init/resize never duplicates widgets.
 * Esc returns to the parent screen.
 */
public class SuiteScreen extends Screen {

    /** QA hook: {@code -Despectral.suite.screen=suite} opens this screen. */
    public static final String QA_SCREEN_PROPERTY = "espectral.suite.screen";
    /** QA hook: {@code -Despectral.suite.search=<text>} prefills search. */
    public static final String QA_SEARCH_PROPERTY = "espectral.suite.search";
    /** Canonical support URL, quoted without parameters. */
    public static final String SUPPORT_URL = "https://espectral.es/donaciones";

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-menu");
    private static final String CATEGORY_ALL = "all";
    private static boolean qaSearchLogged;

    private final Screen parent;
    private String search = "";
    private String category = CATEGORY_ALL;
    private int page;

    // Dynamic zone, rebuilt in place by refreshList().
    private int pageSize = 5;
    private int lastPages = 1;
    private final List<FeatureRegistry.Feature> slotFeatures = new ArrayList<>();
    private StringWidget headerLabel;
    private Button masterChip;
    private final List<StringWidget> rowLabels = new ArrayList<>();
    private final List<Button> rowToggles = new ArrayList<>();
    private final List<Button> categoryButtons = new ArrayList<>();
    private final List<String> categoryButtonIds = new ArrayList<>();
    private StringWidget emptyLabel;
    private Button pagerPrev;
    private Button pagerNext;
    private StringWidget pagerLabel;

    public SuiteScreen(Screen parent) {
        super(Component.translatable("espectral.suite.title"));
        this.parent = parent;
        String prefill = System.getProperty(QA_SEARCH_PROPERTY);
        if (prefill != null) {
            this.search = prefill;
            if (!qaSearchLogged) {
                qaSearchLogged = true;
                LOGGER.info("QA: suite search prefilled ({} chars)", prefill.length());
            }
        }
    }

    /** Parent screen for Esc/return navigation. */
    public Screen parentScreen() {
        return parent;
    }

    @Override
    protected void init() {
        super.init();
        ClientConfig.getInstance().load();
        // Re-init (open/resize) rebuilds everything from scratch, so widgets
        // can never stack.
        this.clearWidgets();
        this.rowLabels.clear();
        this.rowToggles.clear();
        this.categoryButtons.clear();
        this.categoryButtonIds.clear();
        this.slotFeatures.clear();

        int innerX = SuiteTheme.panelX(this.width) + SuiteTheme.MARGIN;
        int innerW = SuiteTheme.panelWidth(this.width) - 2 * SuiteTheme.MARGIN;
        int y = SuiteTheme.PANEL_TOP + SuiteTheme.PAD;

        this.headerLabel = new StringWidget(innerX, y,
                innerW - SuiteTheme.MASTER_W - SuiteTheme.GAP, SuiteTheme.HEADER_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.headerLabel);
        this.masterChip = Button.builder(Component.translatable("espectral.suite.master.on"), b -> {
            ClientConfig config = ClientConfig.getInstance();
            config.setSuiteEnabled(!config.isSuiteEnabled());
            this.refreshList();
        }).bounds(innerX + innerW - SuiteTheme.MASTER_W, y,
                SuiteTheme.MASTER_W, SuiteTheme.HEADER_H).build();
        this.addRenderableWidget(this.masterChip);
        // Tight vertical rhythm (2/2/3) so 5 rows plus the pager fit at 240
        // logical px; horizontal gaps stay at the spec'd 4 (2 for categories).
        y += SuiteTheme.HEADER_H + 2;

        EditBox searchBox = new EditBox(this.font, innerX, y, innerW, SuiteTheme.SEARCH_H,
                Component.translatable("espectral.suite.subtitle"));
        searchBox.setHint(Component.translatable("espectral.suite.search.hint"));
        searchBox.setMaxLength(64);
        // Value before responder: attaching comes last so the initial fill
        // never triggers a refresh.
        searchBox.setValue(this.search);
        searchBox.setResponder(value -> {
            this.search = value;
            this.page = 0;
            this.refreshList();
        });
        this.addRenderableWidget(searchBox);
        y += SuiteTheme.SEARCH_H + 2;

        List<String> categories = new ArrayList<>();
        categories.add(CATEGORY_ALL);
        categories.addAll(FeatureRegistry.categoryIds());
        int catW = (innerW - SuiteTheme.GAP * (categories.size() - 1)) / categories.size();
        for (int i = 0; i < categories.size(); i++) {
            final String id = categories.get(i);
            Component caption = CATEGORY_ALL.equals(id)
                    ? Component.translatable("espectral.suite.category.all")
                    : Component.translatable(FeatureRegistry.categoryKey(id));
            Button cat = Button.builder(caption, b -> {
                this.category = id;
                this.page = 0;
                this.refreshList();
            }).bounds(innerX + i * (catW + SuiteTheme.GAP), y, catW, SuiteTheme.CAT_H).build();
            this.categoryButtons.add(cat);
            this.categoryButtonIds.add(id);
            this.addRenderableWidget(cat);
        }
        y += SuiteTheme.CAT_H + 3;

        int rowsTop = y;
        int footerY = SuiteTheme.footerY(this.height);
        // Pager space is always reserved, so the pager row can never overlap
        // the last feature row whatever the filter state becomes after init.
        int rowsBottom = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H - 2;
        this.pageSize = Math.max(1, (rowsBottom - rowsTop) / SuiteTheme.ROW_H);
        int labelW = innerW - SuiteTheme.TOGGLE_W - SuiteTheme.GAP;
        for (int i = 0; i < this.pageSize; i++) {
            final int slot = i;
            int rowY = rowsTop + i * SuiteTheme.ROW_H;
            StringWidget label = new StringWidget(innerX, rowY + 5, labelW, 12,
                    Component.empty(), this.font);
            this.rowLabels.add(label);
            this.addRenderableWidget(label);
            Button toggle = Button.builder(Component.translatable("espectral.suite.state.off"), b -> {
                if (slot < this.slotFeatures.size()) {
                    FeatureRegistry.Feature feature = this.slotFeatures.get(slot);
                    ClientConfig config = ClientConfig.getInstance();
                    config.setFeatureEnabled(feature.id(),
                            !config.isFeatureEnabledRaw(feature.id()));
                    this.refreshList();
                }
            }).bounds(innerX + innerW - SuiteTheme.TOGGLE_W, rowY,
                    SuiteTheme.TOGGLE_W, SuiteTheme.FOOTER_H)
                    .tooltip(Tooltip.create(Component.empty()))
                    .createNarration(supplier -> toggleNarration(slot))
                    .build();
            this.rowToggles.add(toggle);
            this.addRenderableWidget(toggle);
        }

        this.emptyLabel = new StringWidget(innerX, rowsTop, innerW, 12,
                Component.translatable("espectral.suite.empty"), this.font);
        this.addRenderableWidget(this.emptyLabel);

        int pagerY = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H;
        int pagerW = 64;
        this.pagerPrev = Button.builder(Component.translatable("espectral.suite.button.prev"), b -> {
            this.page = Math.max(0, this.page - 1);
            this.refreshList();
        }).bounds(innerX, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerPrev);
        this.pagerNext = Button.builder(Component.translatable("espectral.suite.button.next"), b -> {
            this.page = Math.min(this.lastPages - 1, this.page + 1);
            this.refreshList();
        }).bounds(innerX + innerW - pagerW, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerNext);
        this.pagerLabel = new StringWidget(innerX + pagerW + SuiteTheme.GAP, pagerY,
                innerW - 2 * (pagerW + SuiteTheme.GAP), SuiteTheme.PAGER_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.pagerLabel);

        int footerW = (innerW - 2 * SuiteTheme.GAP) / 3;
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.suite.button.reset"),
                b -> Compat.open(Minecraft.getInstance(), SuiteConfirmScreen.reset(this)))
                .bounds(innerX, footerY, footerW, SuiteTheme.FOOTER_H).build());
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.suite.button.support"),
                b -> Compat.open(Minecraft.getInstance(), SuiteConfirmScreen.support(this)))
                .bounds(innerX + footerW + SuiteTheme.GAP, footerY,
                        footerW, SuiteTheme.FOOTER_H).build());
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.suite.button.done"),
                b -> this.onClose())
                .bounds(innerX + 2 * (footerW + SuiteTheme.GAP), footerY,
                        footerW, SuiteTheme.FOOTER_H).build());

        this.refreshList();
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
        ClientConfig config = ClientConfig.getInstance();
        return Component.translatable("espectral.narration.suite",
                FeatureRegistry.all().size(),
                Component.translatable(config.isSuiteEnabled()
                        ? "espectral.suite.master.on" : "espectral.suite.master.off"));
    }

    /**
     * Recomputes the filtered list and mutates the widgets built in
     * {@code init()} in place. Adds and removes nothing, so the search box
     * keeps focus while typing.
     */
    private void refreshList() {
        ClientConfig config = ClientConfig.getInstance();
        List<FeatureRegistry.Feature> filtered = filtered();
        int total = FeatureRegistry.all().size();
        this.lastPages = Math.max(1,
                (filtered.size() + this.pageSize - 1) / this.pageSize);
        this.page = Math.min(Math.max(0, this.page), this.lastPages - 1);
        this.slotFeatures.clear();
        for (int i = this.page * this.pageSize;
                i < filtered.size() && this.slotFeatures.size() < this.pageSize; i++) {
            this.slotFeatures.add(filtered.get(i));
        }

        boolean master = config.isSuiteEnabled();
        MutableComponent header = Component.translatable("espectral.suite.title")
                .withStyle(ChatFormatting.GOLD).append(Component.literal(
                        " \u00b7 " + filtered.size() + "/" + total)
                        .withStyle(ChatFormatting.GRAY));
        this.headerLabel.setMessage(header);
        this.masterChip.setMessage(Component.translatable(master
                ? "espectral.suite.master.on" : "espectral.suite.master.off"));

        for (int i = 0; i < this.pageSize; i++) {
            StringWidget label = this.rowLabels.get(i);
            Button toggle = this.rowToggles.get(i);
            if (i < this.slotFeatures.size()) {
                FeatureRegistry.Feature feature = this.slotFeatures.get(i);
                boolean on = config.isFeatureEnabledRaw(feature.id());
                label.setMessage(rowLabel(feature, on, master));
                label.visible = true;
                toggle.setMessage(Component.translatable(on
                        ? "espectral.suite.state.on" : "espectral.suite.state.off"));
                toggle.setTooltip(Tooltip.create(Component.translatable(
                        FeatureRegistry.descriptionKey(feature.id()))));
                toggle.visible = true;
            } else {
                label.visible = false;
                toggle.visible = false;
            }
        }

        this.emptyLabel.visible = filtered.isEmpty();
        boolean paged = this.lastPages > 1;
        this.pagerPrev.visible = paged;
        this.pagerNext.visible = paged;
        this.pagerLabel.visible = paged;
        if (paged) {
            this.pagerLabel.setMessage(Component.translatable("espectral.suite.page",
                    this.page + 1, this.lastPages));
        }

        for (int i = 0; i < this.categoryButtons.size(); i++) {
            this.categoryButtons.get(i).active = !this.categoryButtonIds.get(i).equals(this.category);
        }
    }

    private MutableComponent toggleNarration(int slot) {
        if (slot >= this.slotFeatures.size()) {
            return Component.translatable("espectral.suite.title").copy();
        }
        FeatureRegistry.Feature feature = this.slotFeatures.get(slot);
        boolean on = ClientConfig.getInstance().isFeatureEnabledRaw(feature.id());
        return Component.translatable("espectral.suite.toggle.narration",
                Component.translatable(FeatureRegistry.nameKey(feature.id())),
                Component.translatable(on
                        ? "espectral.suite.state.on" : "espectral.suite.state.off")).copy();
    }

    private static MutableComponent rowLabel(FeatureRegistry.Feature feature, boolean on, boolean master) {
        MutableComponent label = Component.translatable(FeatureRegistry.nameKey(feature.id()))
                .append(Component.literal(" "));
        label.append(Component.translatable(on
                ? "espectral.suite.state.on" : "espectral.suite.state.off")
                .withStyle(on ? ChatFormatting.GREEN : ChatFormatting.GRAY));
        if (!master) {
            label.append(Component.literal(" "));
            label.append(Component.translatable("espectral.suite.state.suppressed")
                    .withStyle(ChatFormatting.GRAY));
        }
        return label;
    }

    private List<FeatureRegistry.Feature> filtered() {
        List<FeatureRegistry.Feature> base = CATEGORY_ALL.equals(this.category)
                ? FeatureRegistry.all()
                : FeatureRegistry.byCategory(this.category);
        String needle = fold(this.search);
        if (needle.isEmpty()) {
            return new ArrayList<>(base);
        }
        List<FeatureRegistry.Feature> out = new ArrayList<>();
        for (FeatureRegistry.Feature feature : base) {
            String hay = fold(feature.id() + " "
                    + Component.translatable(FeatureRegistry.nameKey(feature.id())).getString() + " "
                    + Component.translatable(FeatureRegistry.descriptionKey(feature.id())).getString());
            if (hay.contains(needle)) {
                out.add(feature);
            }
        }
        return out;
    }

    /** Case- and accent-insensitive folding for the live search. */
    private static String fold(String value) {
        String lower = value == null ? "" : value.toLowerCase(Locale.ROOT);
        return Normalizer.normalize(lower, Normalizer.Form.NFD).replaceAll("\\p{M}", "");
    }
}
