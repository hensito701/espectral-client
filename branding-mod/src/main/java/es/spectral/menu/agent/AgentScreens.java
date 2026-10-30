package es.spectral.menu.agent;

import java.util.List;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.AbstractSelectionList;
import net.minecraft.client.gui.components.AbstractSliderButton;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.Checkbox;
import net.minecraft.client.gui.components.CycleButton;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.MultiLineEditBox;
import net.minecraft.client.gui.components.TabButton;
import net.minecraft.client.gui.components.events.ContainerEventHandler;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.core.component.DataComponents;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.ItemLore;

import es.spectral.menu.mixin.ContainerScreenAccess;
import es.spectral.menu.mixin.SliderValueAccess;

/**
 * {@code screen_tree}: the current screen's widgets as JSON, plus its container
 * slots when it is an inventory screen.
 *
 * <p>Ids are {@code w<index>} paths into the (recursive) widget tree —
 * {@code w3}, {@code w3.1} — so they stay stable across one layout and the
 * {@code click} tool can resolve them without a second lookup. Espectral's own
 * screens either add vanilla widgets (Suite, cosmetics, macros) or implement
 * {@link AgentScreenExtras} for hand-painted regions (the main menu).
 */
public final class AgentScreens {

    private static final int MAX_DEPTH = 6;
    private static final int MAX_NODES = 400;

    private AgentScreens() {}

    /** Widget bounds in GUI-scaled coordinates, resolved from an id. */
    public record Bounds(int x, int y, int w, int h) {}

    /** Builds the {@code screen_tree} payload for the open screen. */
    public static JsonObject tree(Minecraft minecraft) throws AgentException {
        if (minecraft == null) throw new AgentException("NO_SCREEN", "no client");
        Screen screen = openScreen(minecraft);
        if (screen == null) {
            throw new AgentException("NO_SCREEN", "no screen is open (the game is showing the world)");
        }
        JsonObject root = new JsonObject();
        root.addProperty("screen", screen.getClass().getSimpleName());
        root.addProperty("title", text(screen.getTitle()));
        int guiWidth = screen.width;
        int guiHeight = screen.height;
        if (minecraft.getWindow() != null) {
            guiWidth = minecraft.getWindow().getGuiScaledWidth();
            guiHeight = minecraft.getWindow().getGuiScaledHeight();
        }
        root.addProperty("gui_width", guiWidth);
        root.addProperty("gui_height", guiHeight);
        root.addProperty("gui_scale", minecraft.getWindow() != null ? minecraft.getWindow().getGuiScale() : 1);

        JsonArray widgets = new JsonArray();
        Counter counter = new Counter();
        walk(screen, "w", 1, widgets, counter);
        if (screen instanceof AgentScreenExtras extras) {
            try {
                for (AgentWidget widget : extras.agentWidgets()) {
                    JsonObject json = new JsonObject();
                    json.addProperty("id", widget.id());
                    json.addProperty("type", widget.type());
                    json.addProperty("class", "custom:" + screen.getClass().getSimpleName());
                    json.addProperty("text", widget.text());
                    json.addProperty("x", widget.x());
                    json.addProperty("y", widget.y());
                    json.addProperty("w", widget.w());
                    json.addProperty("h", widget.h());
                    json.addProperty("enabled", true);
                    json.addProperty("visible", true);
                    json.addProperty("focused", false);
                    widgets.add(json);
                }
            } catch (Throwable ignored) {
                // a custom screen must never break the tool
            }
        }
        root.add("widgets", widgets);

        JsonArray slots = slots(screen);
        if (slots != null) root.add("slots", slots);
        return root;
    }

    /**
     * Resolves a widget id to its bounds (vanilla widget walk, then custom
     * regions). Returns null when the id matches nothing.
     */
    public static Bounds boundsFor(Minecraft minecraft, String id) {
        Screen screen = minecraft != null ? openScreen(minecraft) : null;
        if (screen == null || id == null) return null;
        Counter counter = new Counter();
        Bounds found = find(screen, "w", 1, id, counter);
        if (found != null) return found;
        if (screen instanceof AgentScreenExtras extras) {
            try {
                for (AgentWidget widget : extras.agentWidgets()) {
                    if (id.equals(widget.id())) {
                        return new Bounds(widget.x(), widget.y(), widget.w(), widget.h());
                    }
                }
            } catch (Throwable ignored) {
                return null;
            }
        }
        return null;
    }

    private static Screen openScreen(Minecraft minecraft) {
        try {
            return es.spectral.menu.Compat.getScreen(minecraft);
        } catch (Throwable t) {
            return null;
        }
    }

    private static final class Counter {
        int nodes;
    }

    private static void walk(GuiEventListener element, String prefix, int depth,
            JsonArray out, Counter counter) {
        if (element == null || depth > MAX_DEPTH || counter.nodes >= MAX_NODES) return;
        List<? extends GuiEventListener> children = childrenOf(element);
        for (int i = 0; i < children.size(); i++) {
            if (counter.nodes >= MAX_NODES) return;
            GuiEventListener child = children.get(i);
            if (child == null) continue;
            String id = prefix + (prefix.isEmpty() ? "" : ".") + i;
            if (child instanceof AbstractWidget widget) {
                counter.nodes++;
                JsonObject json = widgetJson(id, widget);
                if (json != null) out.add(json);
                // Widgets can still contain widgets (scroll lists, tab bars).
                walk(widget, id, depth + 1, out, counter);
            } else {
                walk(child, id, depth + 1, out, counter);
            }
        }
    }

    private static Bounds find(GuiEventListener element, String prefix, int depth,
            String wanted, Counter counter) {
        if (element == null || depth > MAX_DEPTH || counter.nodes >= MAX_NODES) return null;
        List<? extends GuiEventListener> children = childrenOf(element);
        for (int i = 0; i < children.size(); i++) {
            if (counter.nodes >= MAX_NODES) return null;
            GuiEventListener child = children.get(i);
            if (child == null) continue;
            String id = prefix + (prefix.isEmpty() ? "" : ".") + i;
            if (child instanceof AbstractWidget widget) {
                counter.nodes++;
                if (id.equals(wanted)) {
                    return new Bounds(widget.getX(), widget.getY(), widget.getWidth(), widget.getHeight());
                }
                Bounds nested = find(widget, id, depth + 1, wanted, counter);
                if (nested != null) return nested;
            } else {
                Bounds nested = find(child, id, depth + 1, wanted, counter);
                if (nested != null) return nested;
            }
        }
        return null;
    }

    private static List<? extends GuiEventListener> childrenOf(GuiEventListener element) {
        if (element instanceof ContainerEventHandler container) {
            try {
                List<? extends GuiEventListener> children = container.children();
                return children != null ? children : List.of();
            } catch (Throwable t) {
                return List.of();
            }
        }
        return List.of();
    }

    /** One widget's JSON node, or null for entries too abstract to describe. */
    private static JsonObject widgetJson(String id, AbstractWidget widget) {
        JsonObject json = new JsonObject();
        json.addProperty("id", id);
        json.addProperty("type", typeOf(widget));
        json.addProperty("class", widget.getClass().getSimpleName());
        json.addProperty("text", text(safeMessage(widget)));
        json.addProperty("x", widget.getX());
        json.addProperty("y", widget.getY());
        json.addProperty("w", widget.getWidth());
        json.addProperty("h", widget.getHeight());
        json.addProperty("enabled", safeActive(widget));
        json.addProperty("visible", widget.visible);
        json.addProperty("focused", widget.isFocused());
        String value = valueOf(widget);
        if (value != null) json.addProperty("value", value);
        return json;
    }

    private static Component safeMessage(AbstractWidget widget) {
        try {
            return widget.getMessage();
        } catch (Throwable t) {
            return null;
        }
    }

    private static boolean safeActive(AbstractWidget widget) {
        try {
            return widget.isActive();
        } catch (Throwable t) {
            return widget.active;
        }
    }

    private static String typeOf(AbstractWidget widget) {
        if (widget instanceof Checkbox) return "checkbox";
        if (widget instanceof CycleButton<?>) return "button";
        if (widget instanceof TabButton) return "tab";
        if (widget instanceof AbstractSliderButton) return "slider";
        if (widget instanceof EditBox || widget instanceof MultiLineEditBox) return "edit_box";
        if (widget instanceof AbstractSelectionList<?>) return "list";
        if (widget instanceof net.minecraft.client.gui.components.Button) return "button";
        return "other";
    }

    private static String valueOf(AbstractWidget widget) {
        try {
            if (widget instanceof Checkbox checkbox) {
                return String.valueOf(checkbox.selected());
            }
            if (widget instanceof AbstractSliderButton slider) {
                return String.valueOf(((SliderValueAccess) slider).espectral$value());
            }
            if (widget instanceof EditBox box) {
                return box.getValue();
            }
            if (widget instanceof MultiLineEditBox box) {
                return box.getValue();
            }
            if (widget instanceof CycleButton<?> cycle) {
                Object value = cycle.getValue();
                return value != null ? String.valueOf(value) : null;
            }
        } catch (Throwable ignored) {
            return null;
        }
        return null;
    }

    // ------------------------------------------------------------------
    // Container slots
    // ------------------------------------------------------------------

    private static JsonArray slots(Screen screen) {
        if (!(screen instanceof AbstractContainerScreen<?> container)) return null;
        AbstractContainerMenu menu;
        try {
            menu = container.getMenu();
        } catch (Throwable t) {
            return null;
        }
        if (menu == null) return null;
        JsonArray out = new JsonArray();
        int left;
        int top;
        try {
            ContainerScreenAccess access = (ContainerScreenAccess) container;
            left = access.espectral$leftPos();
            top = access.espectral$topPos();
        } catch (Throwable t) {
            return null;
        }
        List<Slot> list = menu.slots;
        for (int i = 0; i < list.size(); i++) {
            Slot slot = list.get(i);
            if (slot == null) continue;
            JsonObject json = new JsonObject();
            json.addProperty("id", "s" + i);
            json.addProperty("index", i);
            json.addProperty("x", left + slot.x);
            json.addProperty("y", top + slot.y);
            json.addProperty("w", 16);
            json.addProperty("h", 16);
            ItemStack stack = slot.getItem();
            if (stack != null && !stack.isEmpty()) {
                json.addProperty("item", itemId(stack));
                json.addProperty("count", stack.getCount());
                json.addProperty("name", text(stack.getHoverName()));
                ItemLore lore = stack.get(DataComponents.LORE);
                if (lore != null) {
                    JsonArray lines = new JsonArray();
                    for (Component line : lore.lines()) {
                        lines.add(text(line));
                    }
                    json.add("lore", lines);
                }
            }
            out.add(json);
        }
        return out;
    }

    private static String itemId(ItemStack stack) {
        try {
            return String.valueOf(net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(stack.getItem()));
        } catch (Throwable t) {
            return "unknown";
        }
    }

    // ------------------------------------------------------------------
    // Text helpers
    // ------------------------------------------------------------------

    /** Plain text of a component (null-safe, never throws). */
    public static String text(Component component) {
        if (component == null) return "";
        try {
            return component.getString();
        } catch (Throwable t) {
            return "";
        }
    }

}
