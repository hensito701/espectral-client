package es.spectral.menu.agent;

import java.lang.reflect.InvocationTargetException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.stream.Stream;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;

/**
 * {@code shaders {enabled?, pack?}} through Iris, as a soft dependency: every
 * call goes through reflection so the mod loads without Iris (→ {@code NO_IRIS}).
 *
 * <p>Iris's public API v0 only toggles shaders; switching packs uses the same
 * internals Iris's own pack screen uses ({@code Iris.getIrisConfig()} setters,
 * {@code save()}, {@code Iris.reload()}). The reply is sent after a few frames
 * so it — and the next screenshot — reflect the applied pipeline.
 */
public final class AgentShaders {

    private static final String IRIS = "net.irisshaders.iris.Iris";
    /** Frames to render with the new pipeline before replying. */
    private static final int SETTLE_FRAMES = 3;

    private AgentShaders() {}

    public static CompletableFuture<JsonObject> apply(Minecraft minecraft, AgentArgs args) {
        CompletableFuture<JsonObject> result = new CompletableFuture<>();
        if (minecraft == null) {
            result.completeExceptionally(new AgentException("FAILED", "no client"));
            return result;
        }
        minecraft.execute(() -> {
            try {
                if (applyNow(args)) {
                    AgentFrames.after(SETTLE_FRAMES, () -> complete(result));
                } else {
                    complete(result);
                }
            } catch (AgentException e) {
                result.completeExceptionally(e);
            } catch (Throwable t) {
                result.completeExceptionally(new AgentException("FAILED", "shaders: " + rootMessage(t)));
            }
        });
        return result;
    }

    /** Applies the request; true when Iris was reloaded. Render thread. */
    private static boolean applyNow(AgentArgs args) throws Exception {
        Class<?> iris = irisClass();
        Object config = iris.getMethod("getIrisConfig").invoke(null);
        boolean changed = false;
        if (args.has("pack")) {
            String pack = args.requiredString("pack");
            if (pack.contains("/") || pack.contains("\\") || pack.startsWith(".")) {
                throw new AgentException("BAD_ARGS", "pack must be a file name from 'available'");
            }
            List<String> available = available(iris);
            if (!available.contains(pack)) {
                throw new AgentException("BAD_ARGS", "no shader pack '" + pack + "'; available: " + available);
            }
            if (!pack.equals(currentPack(config))) {
                config.getClass().getMethod("setShaderPackName", String.class).invoke(config, pack);
                changed = true;
            }
            if (!args.has("enabled") && !enabled(config)) {
                // Picking a pack means "show me this pack".
                config.getClass().getMethod("setShadersEnabled", boolean.class).invoke(config, true);
                changed = true;
            }
        }
        if (args.has("enabled")) {
            boolean wanted = args.bool("enabled", true);
            if (wanted != enabled(config)) {
                if (wanted && currentPack(config) == null) {
                    throw new AgentException("BAD_ARGS", "no shader pack selected; pass pack as well");
                }
                config.getClass().getMethod("setShadersEnabled", boolean.class).invoke(config, wanted);
                changed = true;
            }
        }
        if (changed) {
            config.getClass().getMethod("save").invoke(config);
            iris.getMethod("reload").invoke(null);
        }
        return changed;
    }

    private static void complete(CompletableFuture<JsonObject> result) {
        try {
            Class<?> iris = irisClass();
            Object config = iris.getMethod("getIrisConfig").invoke(null);
            JsonObject json = new JsonObject();
            json.addProperty("enabled", enabled(config));
            String pack = currentPack(config);
            if (pack != null) json.addProperty("pack", pack);
            JsonArray list = new JsonArray();
            for (String name : available(iris)) list.add(name);
            json.add("available", list);
            result.complete(json);
        } catch (AgentException e) {
            result.completeExceptionally(e);
        } catch (Throwable t) {
            result.completeExceptionally(new AgentException("FAILED", "shaders: " + rootMessage(t)));
        }
    }

    private static Class<?> irisClass() throws AgentException {
        try {
            return Class.forName(IRIS);
        } catch (ClassNotFoundException e) {
            throw new AgentException("NO_IRIS", "Iris is not installed in this instance "
                    + "(enable capture tools for it in the launcher)");
        }
    }

    private static boolean enabled(Object config) throws Exception {
        return (Boolean) config.getClass().getMethod("areShadersEnabled").invoke(config);
    }

    private static String currentPack(Object config) throws Exception {
        Object name = config.getClass().getMethod("getShaderPackName").invoke(config);
        return name instanceof Optional<?> optional ? (String) optional.orElse(null) : null;
    }

    /** Pack file/folder names in Iris's shaderpacks dir (option .txt files excluded). */
    private static List<String> available(Class<?> iris) throws Exception {
        Path dir = (Path) iris.getMethod("getShaderpacksDirectory").invoke(null);
        List<String> names = new ArrayList<>();
        if (dir == null || !Files.isDirectory(dir)) return names;
        try (Stream<Path> entries = Files.list(dir)) {
            entries.filter(p -> Files.isDirectory(p) || p.getFileName().toString().toLowerCase().endsWith(".zip"))
                    .map(p -> p.getFileName().toString())
                    .sorted()
                    .forEach(names::add);
        }
        return names;
    }

    private static String rootMessage(Throwable t) {
        Throwable cause = t;
        while ((cause instanceof InvocationTargetException || cause.getCause() != null)
                && cause.getCause() != null && cause.getCause() != cause) {
            cause = cause.getCause();
        }
        return cause.toString();
    }
}
