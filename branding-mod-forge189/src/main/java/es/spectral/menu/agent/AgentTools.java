package es.spectral.menu.agent;

import java.util.Locale;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiScreen;

import es.spectral.menu.Keys;
import es.spectral.menu.MacroEngine;
import es.spectral.menu.ui.EspectralTitleScreen;

/**
 * The in-client tool table (same names, tiers and JSON shapes as the Fabric
 * mod). Every tool runs on the client thread from {@link ClientTasks} (never
 * inside {@code addScheduledTask}: network actions would deadlock there).
 * Tier checks happen here, not in the engine. Recording, replays, clips and
 * shader packs do not exist on 1.8.9 yet and answer {@code UNSUPPORTED}.
 */
public final class AgentTools {

    private static final Logger LOGGER = LogManager.getLogger("espectral-client");

    private AgentTools() {}

    public static CompletableFuture<JsonObject> dispatch(final Minecraft minecraft, String tool,
            final JsonObject args, long timeoutMs) {
        final CompletableFuture<JsonObject> future = new CompletableFuture<JsonObject>();
        if (minecraft == null) {
            future.completeExceptionally(new AgentException("FAILED", "no client"));
            return future;
        }
        final String name = tool == null ? "" : tool.trim();
        ClientTasks.submit(new Runnable() {
            @Override
            public void run() {
                try {
                    if ("screenshot".equals(name)) {
                        AgentArgs parsed = new AgentArgs(args);
                        boolean capture = parsed.has("width") || !parsed.bool("hud", true);
                        requireTier(minecraft, capture ? AgentTier.CAPTURE : AgentTier.OBSERVE, name);
                        Capture.request(parsed, future);
                        return;
                    }
                    future.complete(execute(minecraft, name, new AgentArgs(args)));
                } catch (AgentException e) {
                    future.completeExceptionally(e);
                } catch (Throwable t) {
                    LOGGER.warn("agent: tool '{}' failed", name, t);
                    future.completeExceptionally(new AgentException("FAILED", name + ": " + t));
                }
            }
        });
        return future;
    }

    private static JsonObject execute(Minecraft mc, String name, AgentArgs args) throws Exception {
        switch (name) {
            case "screen_tree":
                return AgentScreens.tree(mc);
            case "player":
                return player(mc);
            case "click":
                requireTier(mc, AgentTier.ACT, name);
                return click(mc, args);
            case "key":
                requireTier(mc, AgentTier.ACT, name);
                return key(mc, args);
            case "type":
                requireTier(mc, AgentTier.ACT, name);
                return type(mc, args);
            case "chat":
                requireTier(mc, AgentTier.ACT, name);
                return chat(mc, args, false);
            case "command":
                requireTier(mc, AgentTier.ACT, name);
                return chat(mc, args, true);
            case "disconnect":
                return disconnect(mc);
            case "hud":
                requireTier(mc, AgentTier.CAPTURE, name);
                return hud(mc, args);
            case "shaders":
            case "record_start":
            case "record_stop":
            case "replays":
            case "clip_render":
            case "clip_status":
                throw new AgentException("UNSUPPORTED", "'" + name + "' is not available on Minecraft 1.8.9 yet");
            default:
                throw new AgentException("UNSUPPORTED", "unknown tool '" + name + "'");
        }
    }

    private static void requireTier(Minecraft mc, AgentTier tier, String tool) throws AgentException {
        AgentPolicy.Context context;
        try {
            context = AgentPolicy.classify(mc);
        } catch (Throwable t) {
            context = AgentPolicy.Context.ESPECTRAL;
        }
        if (!tier.allows(context)) {
            throw new AgentException("TIER_REFUSED", "'" + tool + "' is " + tier.name().toLowerCase(Locale.ROOT)
                    + "-tier and the current context is '" + context.wireName()
                    + "' (allowed: singleplayer, replay, localhost, allowlisted server, menu)");
        }
        if (tier == AgentTier.ACT && AgentPause.paused()) {
            throw new AgentException("PAUSED", "the agent is paused (F8); press F8 in the game to resume");
        }
    }

    /**
     * Where the player is (same JSON as the Fabric mod): position, block,
     * rotation, dimension and game mode. A server that turned on reduced debug
     * info hides coordinates from F3, so this tool refuses them too.
     */
    private static JsonObject player(Minecraft mc) throws AgentException {
        net.minecraft.client.entity.EntityPlayerSP player = mc.thePlayer;
        if (player == null || mc.theWorld == null) {
            throw new AgentException("NOT_IN_WORLD", "the player is not in a world");
        }
        if (player.hasReducedDebug()) {
            throw new AgentException("REDUCED_DEBUG_INFO", "this server hides coordinates (reduced debug info)");
        }
        JsonObject json = new JsonObject();
        json.addProperty("x", round(player.posX));
        json.addProperty("y", round(player.posY));
        json.addProperty("z", round(player.posZ));
        net.minecraft.util.BlockPos block = new net.minecraft.util.BlockPos(player);
        com.google.gson.JsonArray blockPos = new com.google.gson.JsonArray();
        blockPos.add(new com.google.gson.JsonPrimitive(block.getX()));
        blockPos.add(new com.google.gson.JsonPrimitive(block.getY()));
        blockPos.add(new com.google.gson.JsonPrimitive(block.getZ()));
        json.add("block", blockPos);
        json.addProperty("yaw", round(net.minecraft.util.MathHelper.wrapAngleTo180_float(player.rotationYaw)));
        json.addProperty("pitch", round(player.rotationPitch));
        json.addProperty("dimension", dimensionId(player.worldObj.provider.getDimensionId()));
        if (mc.playerController != null) {
            json.addProperty("game_mode", mc.playerController.getCurrentGameType().getName());
        }
        return json;
    }

    /** Modern ids for the vanilla dimensions, so both mods answer alike. */
    private static String dimensionId(int id) {
        switch (id) {
            case 0: return "minecraft:overworld";
            case -1: return "minecraft:the_nether";
            case 1: return "minecraft:the_end";
            default: return "dim:" + id;
        }
    }

    private static double round(double v) {
        return Math.round(v * 1000.0) / 1000.0;
    }

    private static JsonObject hud(Minecraft mc, AgentArgs args) throws AgentException {
        if (!args.has("hidden")) throw new AgentException("BAD_ARGS", "hidden (boolean) is required");
        mc.gameSettings.hideGUI = args.bool("hidden", false);
        JsonObject json = new JsonObject();
        json.addProperty("hidden", mc.gameSettings.hideGUI);
        return json;
    }

    private static JsonObject click(Minecraft mc, AgentArgs args) throws Exception {
        GuiScreen screen = mc.currentScreen;
        if (screen == null) throw new AgentException("NO_SCREEN", "no screen is open to click in");
        int x;
        int y;
        if (args.has("id")) {
            String id = args.requiredString("id");
            AgentScreens.Bounds b = AgentScreens.boundsFor(mc, id);
            if (b == null) {
                throw new AgentException("UNKNOWN_WIDGET",
                        "no widget '" + id + "' on " + screen.getClass().getSimpleName() + " (call screen_tree again)");
            }
            x = b.x + b.w / 2;
            y = b.y + b.h / 2;
        } else if (args.has("x") && args.has("y")) {
            x = args.optInt("x", 0);
            y = args.optInt("y", 0);
        } else {
            throw new AgentException("BAD_ARGS", "click needs either {id} or {x, y}");
        }
        String buttonName = args.string("button", "left").toLowerCase(Locale.ROOT);
        int button;
        if ("left".equals(buttonName)) {
            button = 0;
        } else if ("right".equals(buttonName)) {
            button = 1;
        } else if ("middle".equals(buttonName)) {
            button = 2;
        } else {
            throw new AgentException("BAD_ARGS", "'button' must be left, right or middle");
        }
        AgentInput.click(screen, x, y, button, args.bool("double", false));
        JsonObject out = new JsonObject();
        out.addProperty("x", x);
        out.addProperty("y", y);
        out.addProperty("button", buttonName);
        return out;
    }

    private static JsonObject key(Minecraft mc, AgentArgs args) throws Exception {
        String raw = args.requiredString("key");
        int code = AgentInput.resolveKey(raw);
        if (code <= 0) {
            throw new AgentException("BAD_ARGS", "unknown key '" + raw
                    + "' (use a key.keyboard.* name or a short form like e, escape, f3)");
        }
        if (Keys.AGENT_PAUSE != null && Keys.AGENT_PAUSE.getKeyCode() == code) {
            throw new AgentException("TIER_REFUSED", "'" + raw + "' is bound to the agent pause key; only the human can press it");
        }
        String action = args.string("action", "press").toLowerCase(Locale.ROOT);
        if ("press".equals(action)) {
            AgentInput.pressKey(mc, code);
        } else if ("down".equals(action)) {
            AgentInput.keyDown(mc, code);
        } else if ("up".equals(action)) {
            AgentInput.keyUp(code);
        } else {
            throw new AgentException("BAD_ARGS", "'action' must be press, down or up");
        }
        JsonObject out = new JsonObject();
        out.addProperty("key", raw);
        out.addProperty("action", action);
        return out;
    }

    private static JsonObject type(Minecraft mc, AgentArgs args) throws Exception {
        String text = args.string("text", null);
        if (text == null) throw new AgentException("BAD_ARGS", "'text' is required");
        if (mc.currentScreen == null) {
            throw new AgentException("NO_SCREEN",
                    "nothing has focus to type into; open a screen (e.g. press t for chat)");
        }
        AgentInput.typeText(mc.currentScreen, text);
        JsonObject out = new JsonObject();
        out.addProperty("chars", text.codePointCount(0, text.length()));
        return out;
    }

    private static JsonObject chat(Minecraft mc, AgentArgs args, boolean command) throws AgentException {
        String text = args.requiredString("text");
        if (!command && text.startsWith("/")) {
            throw new AgentException("BAD_ARGS", "chat text must not start with '/'; use the command tool for that");
        }
        if (mc.thePlayer == null || mc.getNetHandler() == null) {
            throw new AgentException("NOT_IN_WORLD", "not in a world/server; chat needs a live connection");
        }
        String payload = command && text.startsWith("/") ? text.substring(1) : text;
        if (payload.isEmpty()) throw new AgentException("BAD_ARGS", "text is empty");
        MacroEngine.sendChat(mc, command ? "/" + payload : payload);
        LOGGER.info("agent: {} sent ({} chars)", command ? "command" : "chat", payload.length());
        JsonObject out = new JsonObject();
        out.addProperty("sent", command ? "command" : "chat");
        out.addProperty("length", payload.length());
        return out;
    }

    private static JsonObject disconnect(Minecraft mc) {
        AgentInput.releaseAll();
        JsonObject out = new JsonObject();
        if (mc.theWorld == null) {
            out.addProperty("disconnected", false);
            out.addProperty("note", "not in a world");
            return out;
        }
        // GuiIngameMenu's Disconnect sequence; the trailing screen is required
        // (without it the renderer keeps drawing a torn-down world).
        mc.theWorld.sendQuittingDisconnectingPacket();
        mc.loadWorld(null);
        mc.displayGuiScreen(new EspectralTitleScreen());
        out.addProperty("disconnected", true);
        return out;
    }

    /** Awaits a future for at most {@code timeoutMs} (relay helper). */
    public static JsonObject await(CompletableFuture<JsonObject> future, long timeoutMs) throws AgentException {
        try {
            return future.get(Math.max(1000L, timeoutMs), TimeUnit.MILLISECONDS);
        } catch (TimeoutException e) {
            throw new AgentException("BUSY", "the client did not run the tool in time");
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AgentException("FAILED", "interrupted");
        } catch (ExecutionException e) {
            Throwable cause = e.getCause();
            if (cause instanceof AgentException) throw (AgentException) cause;
            throw new AgentException("FAILED", String.valueOf(cause));
        }
    }
}
