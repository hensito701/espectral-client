package es.spectral.menu.agent;

import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.List;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiOptionSlider;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.gui.GuiTextField;
import net.minecraft.client.gui.ScaledResolution;
import net.minecraft.client.gui.inventory.GuiContainer;
import net.minecraft.inventory.Slot;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.nbt.NBTTagCompound;
import net.minecraft.nbt.NBTTagList;
import net.minecraft.util.EnumChatFormatting;
import net.minecraftforge.fml.client.config.GuiSlider;

import es.spectral.menu.mixin.agent.GuiContainerAccessor;
import es.spectral.menu.mixin.agent.GuiScreenInvoker;
import es.spectral.menu.ui.SuiteConfirmScreen;

/**
 * {@code screen_tree}: the open screen's widgets as JSON (same node shape as
 * the Fabric mod): every {@link GuiButton} in {@code buttonList} as
 * {@code w<index>}, every {@link GuiTextField} the screen holds (found by
 * field type, so vanilla, Forge and Espectral screens all work) as
 * {@code t<index>} with its value, plus container slots ({@code s<index>}).
 * Coordinates are GUI-scaled; {@code click} resolves the same ids.
 */
public final class AgentScreens {

    private AgentScreens() {}

    /** Widget bounds in GUI-scaled coordinates. */
    public static final class Bounds {
        public final int x;
        public final int y;
        public final int w;
        public final int h;

        Bounds(int x, int y, int w, int h) {
            this.x = x;
            this.y = y;
            this.w = w;
            this.h = h;
        }
    }

    public static JsonObject tree(Minecraft mc) throws AgentException {
        GuiScreen screen = mc.currentScreen;
        if (screen == null) throw new AgentException("NO_SCREEN", "no screen is open (the game is showing the world)");
        JsonObject root = new JsonObject();
        root.addProperty("screen", screen.getClass().getSimpleName());
        root.addProperty("title", title(screen));
        ScaledResolution res = new ScaledResolution(mc);
        root.addProperty("gui_width", res.getScaledWidth());
        root.addProperty("gui_height", res.getScaledHeight());
        root.addProperty("gui_scale", res.getScaleFactor());
        JsonArray widgets = new JsonArray();
        List<GuiButton> buttons = buttons(screen);
        for (int i = 0; i < buttons.size(); i++) {
            GuiButton b = buttons.get(i);
            JsonObject json = node("w" + i, typeOf(b), b.getClass().getSimpleName(), label(screen, b),
                    b.xPosition, b.yPosition, b.width, b.height, b.enabled, b.visible, false);
            widgets.add(json);
        }
        List<GuiTextField> fields = textFields(screen);
        for (int i = 0; i < fields.size(); i++) {
            GuiTextField f = fields.get(i);
            JsonObject json = node("t" + i, "edit_box", "GuiTextField", "", f.xPosition, f.yPosition, f.width, f.height,
                    f.getVisible(), f.getVisible(), f.isFocused());
            json.addProperty("value", f.getText());
            widgets.add(json);
        }
        root.add("widgets", widgets);
        JsonArray slots = slots(screen);
        if (slots != null) root.add("slots", slots);
        return root;
    }

    /** Bounds of a {@code w<i>}/{@code t<i>}/{@code s<i>} id, or null. */
    public static Bounds boundsFor(Minecraft mc, String id) {
        GuiScreen screen = mc.currentScreen;
        if (screen == null || id == null || id.length() < 2) return null;
        int index;
        try {
            index = Integer.parseInt(id.substring(1));
        } catch (NumberFormatException e) {
            return null;
        }
        char kind = id.charAt(0);
        if (kind == 'w') {
            List<GuiButton> buttons = buttons(screen);
            if (index < 0 || index >= buttons.size()) return null;
            GuiButton b = buttons.get(index);
            return new Bounds(b.xPosition, b.yPosition, b.width, b.height);
        }
        if (kind == 't') {
            List<GuiTextField> fields = textFields(screen);
            if (index < 0 || index >= fields.size()) return null;
            GuiTextField f = fields.get(index);
            return new Bounds(f.xPosition, f.yPosition, f.width, f.height);
        }
        if (kind == 's' && screen instanceof GuiContainer) {
            GuiContainer container = (GuiContainer) screen;
            if (index < 0 || index >= container.inventorySlots.inventorySlots.size()) return null;
            Slot slot = container.inventorySlots.inventorySlots.get(index);
            GuiContainerAccessor access = (GuiContainerAccessor) container;
            return new Bounds(access.espectral$guiLeft() + slot.xDisplayPosition,
                    access.espectral$guiTop() + slot.yDisplayPosition, 16, 16);
        }
        return null;
    }

    private static JsonObject node(String id, String type, String cls, String text, int x, int y, int w, int h,
            boolean enabled, boolean visible, boolean focused) {
        JsonObject json = new JsonObject();
        json.addProperty("id", id);
        json.addProperty("type", type);
        json.addProperty("class", cls);
        json.addProperty("text", text);
        json.addProperty("x", x);
        json.addProperty("y", y);
        json.addProperty("w", w);
        json.addProperty("h", h);
        json.addProperty("enabled", enabled);
        json.addProperty("visible", visible);
        json.addProperty("focused", focused);
        return json;
    }

    private static String title(GuiScreen screen) {
        if (screen instanceof SuiteConfirmScreen) return ((SuiteConfirmScreen) screen).titleText();
        return "";
    }

    private static String label(GuiScreen screen, GuiButton b) {
        String custom = screen instanceof AgentLabeled ? ((AgentLabeled) screen).agentLabel(b) : null;
        String raw = custom != null ? custom : b.displayString;
        return raw == null ? "" : EnumChatFormatting.getTextWithoutFormattingCodes(raw);
    }

    private static String typeOf(GuiButton b) {
        if (b instanceof GuiOptionSlider || b instanceof GuiSlider) return "slider";
        return "button";
    }

    static List<GuiButton> buttons(GuiScreen screen) {
        return new ArrayList<GuiButton>(((GuiScreenInvoker) screen).espectral$buttons());
    }

    /** Every non-null GuiTextField held by the screen (declared fields, class hierarchy). */
    static List<GuiTextField> textFields(GuiScreen screen) {
        List<GuiTextField> out = new ArrayList<GuiTextField>();
        for (Class<?> c = screen.getClass(); c != null && c != Object.class; c = c.getSuperclass()) {
            for (Field f : c.getDeclaredFields()) {
                if (!GuiTextField.class.isAssignableFrom(f.getType())) continue;
                try {
                    f.setAccessible(true);
                    Object v = f.get(screen);
                    if (v != null && !out.contains(v)) out.add((GuiTextField) v);
                } catch (ReflectiveOperationException ignored) {
                    // inaccessible field: skip it
                }
            }
        }
        return out;
    }

    private static JsonArray slots(GuiScreen screen) {
        if (!(screen instanceof GuiContainer)) return null;
        GuiContainer container = (GuiContainer) screen;
        GuiContainerAccessor access = (GuiContainerAccessor) container;
        JsonArray out = new JsonArray();
        List<Slot> list = container.inventorySlots.inventorySlots;
        for (int i = 0; i < list.size(); i++) {
            Slot slot = list.get(i);
            JsonObject json = new JsonObject();
            json.addProperty("id", "s" + i);
            json.addProperty("index", i);
            json.addProperty("x", access.espectral$guiLeft() + slot.xDisplayPosition);
            json.addProperty("y", access.espectral$guiTop() + slot.yDisplayPosition);
            json.addProperty("w", 16);
            json.addProperty("h", 16);
            ItemStack stack = slot.getStack();
            if (stack != null) {
                Object key = Item.itemRegistry.getNameForObject(stack.getItem());
                json.addProperty("item", key != null ? key.toString() : "unknown");
                json.addProperty("count", stack.stackSize);
                json.addProperty("name", EnumChatFormatting.getTextWithoutFormattingCodes(stack.getDisplayName()));
                NBTTagCompound display = stack.getSubCompound("display", false);
                if (display != null && display.hasKey("Lore", 9)) {
                    NBTTagList lore = display.getTagList("Lore", 8);
                    JsonArray lines = new JsonArray();
                    for (int l = 0; l < lore.tagCount(); l++) {
                        lines.add(new com.google.gson.JsonPrimitive(
                                EnumChatFormatting.getTextWithoutFormattingCodes(lore.getStringTagAt(l))));
                    }
                    json.add("lore", lines);
                }
            }
            out.add(json);
        }
        return out;
    }
}
