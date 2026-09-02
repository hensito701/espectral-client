package es.spectral.menu;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

/**
 * In-game GUI for toggling Espectral Client features (Contract A).
 * Opened via Right Shift keybind or the Title Screen "Espectral Client" button.
 */
public final class EspectralClientScreen extends Screen {

    private final Screen parent;

    public EspectralClientScreen(Screen parent) {
        super(Component.literal("Espectral Client"));
        this.parent = parent;
    }

    @Override
    protected void init() {
        super.init();
        ClientConfig.getInstance().load();

        int centerX = this.width / 2;
        int startY = Math.max(30, this.height / 6);

        // Title
        StringWidget titleWidget = new StringWidget(0, startY - 24, this.width, 16,
                Component.literal("§6§lEspectral Client"), this.font);
        this.addRenderableWidget(titleWidget);

        // Feature rows
        int y = startY;
        for (FeatureRegistry.Feature feature : FeatureRegistry.ALL) {
            final String featureId = feature.id();
            final boolean isManaged = feature.isManaged();
            final boolean currentEnabled = ClientConfig.getInstance().isFeatureEnabled(featureId);

            String labelText = "§f" + feature.name() + (isManaged ? " §7(reinicia al cambiar)" : " §a(en vivo)");
            StringWidget labelWidget = new StringWidget(centerX - 180, y + 4, 200, 14,
                    Component.literal(labelText), this.font);
            this.addRenderableWidget(labelWidget);

            String btnText = currentEnabled ? "§a[ACTIVADO]" : "§c[DESACTIVADO]";
            Button toggleBtn = Button.builder(Component.literal(btnText), b -> {
                boolean next = !ClientConfig.getInstance().isFeatureEnabled(featureId);
                ClientConfig.getInstance().setFeatureEnabled(featureId, next);
                ClientConfig.getInstance().save();
                b.setMessage(Component.literal(next ? "§a[ACTIVADO]" : "§c[DESACTIVADO]"));
            }).bounds(centerX + 60, y, 120, 20).build();

            this.addRenderableWidget(toggleBtn);
            y += 28;
        }

        // Bottom action buttons: Done / Reload
        int bottomY = Math.min(this.height - 35, y + 16);
        this.addRenderableWidget(Button.builder(Component.literal("Listo"), b -> {
            ClientConfig.getInstance().save();
            Compat.open(this.minecraft != null ? this.minecraft : Minecraft.getInstance(), this.parent);
        }).bounds(centerX - 100, bottomY, 200, 20).build());
    }

    @Override
    public void onClose() {
        ClientConfig.getInstance().save();
        Compat.open(this.minecraft != null ? this.minecraft : Minecraft.getInstance(), this.parent);
    }
}
