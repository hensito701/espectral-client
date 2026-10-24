package es.spectral.menu.ui;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;

/**
 * Cosmetics viewer settings, opened from the "Configurar" button on the Suite's
 * Cosméticos row. Everything here is local and viewer-side: nothing is sent to
 * the server and {@code config/espectral-cosmetics.json} (launcher-owned) is
 * never written.
 *
 * <p>State lives under {@code features.cosmetics.*} in
 * {@code config/espectral-client.json} — the same object the registry's
 * {@code state} block seeds — so the launcher round-trips it and the render
 * paths read it through {@link ClientConfig#cosmeticsState()} (one volatile
 * field, no I/O).
 *
 * <p>Geometry is recomputed in {@code init()} from width/height only and the
 * widget list is cleared first, so a resize can never stack widgets. Esc or
 * Volver returns to the Suite.
 */
public class SuiteCosmeticsScreen extends Screen implements SuiteSheet {

    /** Compact row pitch for the toggle stack (keeps 240 px logical height). */
    private static final int ROW = 20;
    /** Slot buttons per line (espalda, cuello, cabeza, elitros). */
    private static final String[] SLOTS = {"espalda", "cuello", "cabeza", "elitros"};

    private final Screen parent;

    private StringWidget headerLabel;
    private Button ownButton;
    private Button othersButton;
    private Button emotesButton;
    private Button physicsButton;
    private final List<Button> slotButtons = new ArrayList<>();
    private StringWidget noteLabel;
    private StringWidget physicsHint;
    private StringWidget emotesHint;

    public SuiteCosmeticsScreen(Screen parent) {
        super(Component.translatable("espectral.cosmetics.title"));
        this.parent = parent;
    }

    /** Parent screen, so a nested confirm dialog can return here. */
    public Screen parentScreen() {
        return parent;
    }

    @Override
    protected void init() {
        super.init();
        ClientConfig.getInstance().load();
        this.clearWidgets();
        this.slotButtons.clear();

        int innerX = SuiteTheme.panelX(this.width) + SuiteTheme.MARGIN;
        int innerW = SuiteTheme.panelWidth(this.width) - 2 * SuiteTheme.MARGIN;
        int y = SuiteTheme.PANEL_TOP + SuiteTheme.PAD;

        this.headerLabel = new StringWidget(innerX, y, innerW, SuiteTheme.HEADER_H,
                Component.empty(), this.font);
        this.addRenderableWidget(this.headerLabel);
        y += SuiteTheme.HEADER_H + 2;

        this.ownButton = toggleButton(innerX, y, innerW,
                "espectral.cosmetics.own", ClientConfig.COSMETICS_OWN);
        y += ROW + 2;
        this.othersButton = toggleButton(innerX, y, innerW,
                "espectral.cosmetics.others", ClientConfig.COSMETICS_OTHERS);
        y += ROW + 4;

        this.addRenderableWidget(new StringWidget(innerX, y, innerW, 9,
                Component.translatable("espectral.cosmetics.slots"), this.font));
        y += 11;
        int gap = SuiteTheme.GAP;
        int slotW = (innerW - gap * (SLOTS.length - 1)) / SLOTS.length;
        for (int i = 0; i < SLOTS.length; i++) {
            final String slot = SLOTS[i];
            String key = ClientConfig.cosmeticsSlotKey(slot);
            Button button = Button.builder(Component.empty(), b -> {
                ClientConfig config = ClientConfig.getInstance();
                config.setCosmeticsState(key, !config.getCosmeticsState(key));
                refresh();
            }).bounds(innerX + i * (slotW + gap), y, slotW, ROW).build();
            this.slotButtons.add(button);
            this.addRenderableWidget(button);
        }
        y += ROW + 4;

        this.emotesButton = toggleButton(innerX, y, innerW,
                "espectral.cosmetics.emotes", ClientConfig.COSMETICS_EMOTES);
        y += ROW + 1;
        this.emotesHint = new StringWidget(innerX + 2, y, innerW - 4, 9,
                Component.empty(), this.font);
        this.addRenderableWidget(this.emotesHint);
        y += 11;

        this.physicsButton = toggleButton(innerX, y, innerW,
                "espectral.cosmetics.physics", ClientConfig.COSMETICS_PHYSICS);
        y += ROW + 1;
        this.physicsHint = new StringWidget(innerX + 2, y, innerW - 4, 9,
                Component.empty(), this.font);
        this.addRenderableWidget(this.physicsHint);

        int footerY = SuiteTheme.footerY(this.height);
        this.noteLabel = new StringWidget(innerX, footerY - 12, innerW, 9,
                Component.translatable("espectral.cosmetics.note"), this.font);
        this.addRenderableWidget(this.noteLabel);

        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.suite.back"),
                b -> this.onClose())
                .bounds(innerX, footerY, innerW, SuiteTheme.FOOTER_H).build());

        this.refresh();
    }

    private Button toggleButton(int x, int y, int w, String labelKey, String stateKey) {
        Button button = Button.builder(Component.empty(), b -> {
            ClientConfig config = ClientConfig.getInstance();
            config.setCosmeticsState(stateKey, !config.getCosmeticsState(stateKey));
            refresh();
        }).bounds(x, y, w, ROW)
                .tooltip(Tooltip.create(Component.translatable(labelKey)))
                .createNarration(supplier -> Component.empty()
                        .append(Component.translatable(labelKey))
                        .append(Component.literal(": "))
                        .append(Component.translatable(
                                ClientConfig.getInstance().getCosmeticsState(stateKey)
                                        ? "espectral.suite.state.on"
                                        : "espectral.suite.state.off")).copy())
                .build();
        this.addRenderableWidget(button);
        return button;
    }

    /** Rewrites every widget caption from the stored state (no widgets added). */
    private void refresh() {
        ClientConfig config = ClientConfig.getInstance();
        this.headerLabel.setMessage(Component.translatable("espectral.cosmetics.title")
                .withStyle(SuiteTheme.accentFormat()));

        this.ownButton.setMessage(labeled("espectral.cosmetics.own", ClientConfig.COSMETICS_OWN));
        this.othersButton.setMessage(labeled("espectral.cosmetics.others", ClientConfig.COSMETICS_OTHERS));
        this.emotesButton.setMessage(labeled("espectral.cosmetics.emotes", ClientConfig.COSMETICS_EMOTES));
        this.physicsButton.setMessage(labeled("espectral.cosmetics.physics", ClientConfig.COSMETICS_PHYSICS));

        for (int i = 0; i < this.slotButtons.size(); i++) {
            String slot = SLOTS[i];
            boolean on = config.getCosmeticsState(ClientConfig.cosmeticsSlotKey(slot));
            Button button = this.slotButtons.get(i);
            button.setMessage(Component.empty()
                    .append(Component.translatable("espectral.slot." + slot))
                    .append(Component.literal(on ? " ✓" : " ✗"))
                    .withStyle(on ? SuiteTheme.accentFormat() : net.minecraft.ChatFormatting.GRAY));
            button.setTooltip(Tooltip.create(Component.translatable("espectral.slot." + slot)));
        }

        this.physicsHint.setMessage(Component.translatable("espectral.cosmetics.physics.hint")
                .withStyle(net.minecraft.ChatFormatting.GRAY));
        // The launcher-owned flag can switch emotes off too; say so instead of
        // letting the switch look broken.
        boolean launcherEmotes = es.spectral.menu.cosmetics.CosmeticsService.own().emotes();
        this.emotesHint.setMessage(launcherEmotes
                ? Component.empty()
                : Component.translatable("espectral.cosmetics.emotes.launcherOff")
                        .withStyle(net.minecraft.ChatFormatting.GRAY));
    }

    private static Component labeled(String labelKey, String stateKey) {
        boolean on = ClientConfig.getInstance().getCosmeticsState(stateKey);
        return Component.empty()
                .append(Component.translatable(labelKey))
                .append(Component.literal("  ·  "))
                .append(Component.translatable(on
                        ? "espectral.suite.state.on" : "espectral.suite.state.off")
                        .withStyle(on ? net.minecraft.ChatFormatting.GREEN
                                : net.minecraft.ChatFormatting.GRAY));
    }

    @Override
    public void onClose() {
        ClientConfig.getInstance().save();
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
