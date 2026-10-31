package es.spectral.menu.agent;

import java.util.concurrent.CompletableFuture;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonObject;

import com.mojang.blaze3d.platform.InputConstants;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.multiplayer.ClientLevel;

import es.spectral.menu.Compat;

/**
 * The in-client tool table (Protocol v1 "Tools executed in the mod").
 *
 * <p>Every tool runs on the render thread — {@link AgentRelay} dispatches
 * through {@code Minecraft.execute}. Tier checks happen here, not in the
 * engine, so a policy bug in the launcher cannot grant control on an Espectral
 * server.
 *
 * <p>Tools that need a later frame ({@code screenshot}) return a future the
 * relay awaits; everything else answers synchronously.
 */
public final class AgentTools {

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    /** Contract error code for tools this build does not implement yet. */
    private static final String NOT_YET =
            "not implemented in this build (relay P2: observe + act only)";

    private AgentTools() {}

    /**
     * Entry point. Everything that touches game state runs on the render
     * thread; {@code screenshot} is the exception — it must NOT block that
     * thread, because the frame that fulfils it is produced there. It parks a
     * request and returns a future that completes on the PNG writer thread.
     */
    public static CompletableFuture<JsonObject> dispatch(Minecraft minecraft, String tool,
            JsonObject args, long timeoutMs) {
        if ("screenshot".equals(tool)) {
            return screenshotAsync(minecraft, new AgentArgs(args), timeoutMs);
        }
        CompletableFuture<JsonObject> future = new CompletableFuture<>();
        Runnable task = () -> {
            try {
                future.complete(run(minecraft, tool, new AgentArgs(args), timeoutMs));
            } catch (AgentException e) {
                future.completeExceptionally(e);
            } catch (Throwable t) {
                LOGGER.warn("agent: tool '{}' failed", tool, t);
                future.completeExceptionally(
                        new AgentException("FAILED", tool + ": " + t));
            }
        };
        if (minecraft == null) {
            task.run();
        } else {
            minecraft.execute(task);
        }
        return future;
    }

    private static JsonObject run(Minecraft minecraft, String tool, AgentArgs args, long timeoutMs)
            throws AgentException {
        if (minecraft == null) throw new AgentException("FAILED", "no client");
        String name = tool == null ? "" : tool.trim();
        switch (name) {
            case "screenshot":
                // Reached only through the async path above.
                throw new AgentException("FAILED", "screenshot must not run on the render thread");
            case "screen_tree":
                return AgentScreens.tree(minecraft);
            case "click":
                requireTier(minecraft, AgentTier.ACT, name);
                return click(minecraft, args);
            case "key":
                requireTier(minecraft, AgentTier.ACT, name);
                return key(minecraft, args);
            case "type":
                requireTier(minecraft, AgentTier.ACT, name);
                return type(minecraft, args);
            case "chat":
                requireTier(minecraft, AgentTier.ACT, name);
                return chat(minecraft, args, false);
            case "command":
                requireTier(minecraft, AgentTier.ACT, name);
                return chat(minecraft, args, true);
            case "disconnect":
                return disconnect(minecraft);
            case "hud":
            case "shaders":
            case "record_start":
            case "record_stop":
            case "replays":
            case "clip_render":
            case "clip_status":
                throw new AgentException("UNSUPPORTED",
                        "'" + name + "' is " + NOT_YET + "; use screenshot, screen_tree, "
                                + "click, key, type, chat, command or disconnect");
            default:
                throw new AgentException("UNSUPPORTED", "unknown tool '" + name + "'");
        }
    }

    // ------------------------------------------------------------------
    // Tiers
    // ------------------------------------------------------------------

    /**
     * Context for the tier gate. Derived from the same snapshot the state
     * report uses, so a kicked client on a public server stays {@code espectral}
     * /{@code server} instead of degrading to {@code menu} the moment the
     * socket drops (that degradation would re-open act tools).
     */
    private static AgentPolicy.Context context(Minecraft minecraft) {
        try {
            return AgentStates.snapshot(minecraft).context();
        } catch (Throwable t) {
            return AgentPolicy.Context.ESPECTRAL;
        }
    }

    private static void requireTier(Minecraft minecraft, AgentTier tier, String tool)
            throws AgentException {
        AgentPolicy.Context context = context(minecraft);
        if (!tier.allows(context)) {
            throw new AgentException("TIER_REFUSED",
                    "'" + tool + "' is " + tier.name().toLowerCase(java.util.Locale.ROOT)
                            + "-tier and the current context is '" + context.wireName()
                            + "' (allowed: singleplayer, replay, localhost, allowlisted server, menu)");
        }
        if (tier == AgentTier.ACT && AgentPause.paused()) {
            throw new AgentException("PAUSED",
                    "the agent is paused (F8); press F8 in the game to resume");
        }
    }

    // ------------------------------------------------------------------
    // Observe
    // ------------------------------------------------------------------

    /**
     * Requests an end-of-frame grab without blocking the render thread: the
     * validation runs here (relay thread), the frame hook does the grab, the
     * PNG encoder completes the future.
     */
    private static CompletableFuture<JsonObject> screenshotAsync(Minecraft minecraft,
            AgentArgs args, long timeoutMs) {
        CompletableFuture<JsonObject> result = new CompletableFuture<>();
        try {
            screenshotArgs(args);
        } catch (AgentException e) {
            result.completeExceptionally(e);
            return result;
        }
        // Tier: the sampled state is the render thread's own view, so reading it
        // here cannot race with the client's data structures.
        AgentState state = AgentStatePublisher.current();
        AgentPolicy.Context context = state != null ? state.context()
                : AgentPolicy.Context.MENU;
        if (!AgentTier.OBSERVE.allows(context)) {
            result.completeExceptionally(new AgentException("TIER_REFUSED",
                    "'screenshot' is refused in context '" + context.wireName() + "'"));
            return result;
        }
        boolean preview = false;
        try {
            preview = args.bool("preview", true);
        } catch (AgentException ignored) {
            // validated above; preview defaults to true
        }
        CompletableFuture<AgentCapture.Grabbed> grab = AgentCapture.request(preview);
        grab.whenComplete((shot, error) -> {
            if (error != null) {
                result.completeExceptionally(error);
            } else {
                result.complete(AgentCapture.toJson(shot));
            }
        });
        return result;
    }

    /** Argument validation for {@code screenshot} (no side effects). */
    private static void screenshotArgs(AgentArgs args) throws AgentException {
        if (args.contains("width") || args.contains("height")) {
            throw new AgentException("UNSUPPORTED",
                    "custom-size capture (width/height) is not implemented yet; "
                            + "capture at the window size instead");
        }
        if (!args.bool("hud", true)) {
            throw new AgentException("UNSUPPORTED",
                    "hud:false (HUD-less capture) is not implemented yet; only the full frame "
                            + "can be captured in this build");
        }
        args.bool("preview", true);
    }

    // ------------------------------------------------------------------
    // Act
    // ------------------------------------------------------------------

    private static JsonObject click(Minecraft minecraft, AgentArgs args) throws AgentException {
        Screen screen = Compat.getScreen(minecraft);
        if (screen == null) {
            throw new AgentException("NO_SCREEN", "no screen is open to click in");
        }
        int guiWidth = screen.width;
        int guiHeight = screen.height;
        double guiX;
        double guiY;
        if (args.has("id")) {
            String id = args.requiredString("id");
            AgentScreens.Bounds bounds = AgentScreens.boundsFor(minecraft, id);
            if (bounds == null) {
                throw new AgentException("UNKNOWN_WIDGET",
                        "no widget '" + id + "' on " + screen.getClass().getSimpleName()
                                + " (call screen_tree again)");
            }
            guiX = bounds.x() + bounds.w() / 2.0;
            guiY = bounds.y() + bounds.h() / 2.0;
        } else if (args.has("x") && args.has("y")) {
            guiX = args.requiredInt("x");
            guiY = args.requiredInt("y");
        } else {
            throw new AgentException("BAD_ARGS", "click needs either {id} or {x, y}");
        }
        String buttonName = args.string("button", "left");
        int button = switch (buttonName.toLowerCase(java.util.Locale.ROOT)) {
            case "left" -> InputConstants.MOUSE_BUTTON_LEFT;
            case "right" -> InputConstants.MOUSE_BUTTON_RIGHT;
            case "middle" -> InputConstants.MOUSE_BUTTON_MIDDLE;
            default -> throw new AgentException("BAD_ARGS",
                    "'button' must be left, right or middle");
        };
        boolean doubleClick = args.bool("double", false);
        double[] window = toWindow(minecraft, screen, guiX, guiY);
        AgentInput.click(minecraft, window[0], window[1], button, doubleClick);
        LOGGER.debug("agent: click {} {} at gui ({}, {}) -> window ({}, {})",
                args.string("id", ""), buttonName, guiX, guiY, window[0], window[1]);
        JsonObject out = new JsonObject();
        out.addProperty("x", Math.round(guiX));
        out.addProperty("y", Math.round(guiY));
        out.addProperty("button", buttonName);
        return out;
    }

    /** GUI-scaled → window pixels (inverse of MouseHandler.getScaledXPos). */
    private static double[] toWindow(Minecraft minecraft, Screen screen, double guiX, double guiY) {
        double windowWidth = minecraft.getWindow() != null
                ? minecraft.getWindow().getScreenWidth() : screen.width;
        double windowHeight = minecraft.getWindow() != null
                ? minecraft.getWindow().getScreenHeight() : screen.height;
        double scaleX = screen.width > 0 ? windowWidth / screen.width : 1.0;
        double scaleY = screen.height > 0 ? windowHeight / screen.height : 1.0;
        return new double[] {guiX * scaleX, guiY * scaleY};
    }

    private static JsonObject key(Minecraft minecraft, AgentArgs args) throws AgentException {
        String raw = args.requiredString("key");
        int key = AgentInput.resolveKey(raw);
        if (key < 0) {
            throw new AgentException("BAD_ARGS", "unknown key '" + raw
                    + "' (use a key.keyboard.* name or a short form like e, escape, f3)");
        }
        // The pause binding is user-only: an agent must never be able to
        // release its own leash. Refused outright (and AgentPause also ignores
        // injected samples).
        if (isPauseKey(key)) {
            throw new AgentException("TIER_REFUSED",
                    "'" + raw + "' is bound to the agent pause key; only the human can press it");
        }
        String action = args.string("action", "press").toLowerCase(java.util.Locale.ROOT);
        long now = System.currentTimeMillis();
        switch (action) {
            case "press" -> AgentInput.pressKey(minecraft, key, 0);
            case "down" -> {
                AgentInput.keyDown(minecraft, key, 0, now);
                LOGGER.info("agent: key {} held (lease {} ms)", raw, AgentInput.LEASE_MS);
            }
            case "up" -> AgentInput.keyUp(minecraft, key, 0);
            default -> throw new AgentException("BAD_ARGS",
                    "'action' must be press, down or up");
        }
        JsonObject out = new JsonObject();
        out.addProperty("key", raw);
        out.addProperty("action", action);
        return out;
    }

    private static boolean isPauseKey(int key) {
        var binding = es.spectral.menu.EspectralClient.AGENT_PAUSE_KEY;
        if (binding == null) return false;
        try {
            // KeyMapping.key is protected: read the bound value through the
            // public accessor the whole game uses for labels.
            return com.mojang.blaze3d.platform.InputConstants.getKey(
                    binding.saveString()).getValue() == key;
        } catch (Throwable t) {
            return false;
        }
    }

    private static JsonObject type(Minecraft minecraft, AgentArgs args) throws AgentException {
        String text = args.string("text", null);
        if (text == null) throw new AgentException("BAD_ARGS", "'text' is required");
        if (Compat.getScreen(minecraft) == null) {
            throw new AgentException("NO_SCREEN",
                    "nothing has focus to type into; open a screen (e.g. press E after clicking the world)");
        }
        AgentInput.typeText(minecraft, text);
        JsonObject out = new JsonObject();
        out.addProperty("chars", text.codePointCount(0, text.length()));
        return out;
    }

    private static JsonObject chat(Minecraft minecraft, AgentArgs args, boolean command)
            throws AgentException {
        String text = args.requiredString("text");
        if (!command && text.startsWith("/")) {
            // A chat message that starts with '/' is a command: refuse rather
            // than silently escalate privileges.
            throw new AgentException("BAD_ARGS",
                    "chat text must not start with '/'; use the command tool for that");
        }
        if (minecraft.player == null || minecraft.getConnection() == null) {
            throw new AgentException("NOT_IN_WORLD",
                    "not in a world/server; chat needs a live connection");
        }
        String payload = command && text.startsWith("/") ? text.substring(1) : text;
        if (payload.isEmpty()) throw new AgentException("BAD_ARGS", "text is empty");
        Compat.sendChat(minecraft, command ? "/" + payload : payload);
        LOGGER.info("agent: {} sent ({} chars)", command ? "command" : "chat", payload.length());
        JsonObject out = new JsonObject();
        out.addProperty("sent", command ? "command" : "chat");
        out.addProperty("length", payload.length());
        return out;
    }

    private static JsonObject disconnect(Minecraft minecraft) throws AgentException {
        AgentInput.releaseAll(minecraft);
        boolean inWorld = minecraft.getConnection() != null
                || minecraft.hasSingleplayerServer() || minecraft.level != null;
        if (!inWorld) {
            JsonObject out = new JsonObject();
            out.addProperty("disconnected", false);
            out.addProperty("note", "not in a world");
            return out;
        }
        // Same entry point the pause menu's Disconnect button uses.
        minecraft.disconnectFromWorld(ClientLevel.DEFAULT_QUIT_MESSAGE);
        JsonObject out = new JsonObject();
        out.addProperty("disconnected", true);
        return out;
    }
}
