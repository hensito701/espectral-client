package es.spectral.menu.bench;

import java.util.ArrayList;
import java.util.List;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/**
 * Parsed form of {@code espectral-bench.json}.
 *
 * <p>Schema: {@code {"world": "<save folder>", "seed": <long, optional>,
 * "scenes": [{"id": ..., "duration_s": 30, "steps": [...]}}}.
 * Step verbs: {@code {"tp": [x,y,z]}}, {@code {"look": [yaw,pitch]}},
 * {@code {"summon": {"entity": ..., "count": N, "pos": [x,y,z]}}},
 * {@code {"wait_s": N}}. Unknown verbs fail the parse so a typoed harness
 * file never silently benchmarks the wrong scene.
 */
public final class BenchSpec {

    /** Fixed seed used when the bench file carries no {@code "seed"}. */
    public static final long DEFAULT_SEED = 1337L;

    public final String world;
    public final long seed;
    public final List<Scene> scenes;

    private BenchSpec(String world, long seed, List<Scene> scenes) {
        this.world = world;
        this.seed = seed;
        this.scenes = scenes;
    }

    public static BenchSpec parse(JsonObject root) {
        if (!root.has("world") || !root.get("world").isJsonPrimitive()) {
            throw new IllegalArgumentException("espectral-bench.json: missing string \"world\"");
        }
        String world = root.get("world").getAsString().trim();
        if (world.isEmpty()) {
            throw new IllegalArgumentException("espectral-bench.json: empty \"world\"");
        }
        long seed = root.has("seed") ? root.get("seed").getAsLong() : DEFAULT_SEED;
        if (!root.has("scenes") || !root.get("scenes").isJsonArray()) {
            throw new IllegalArgumentException("espectral-bench.json: missing array \"scenes\"");
        }
        List<Scene> scenes = new ArrayList<>();
        for (JsonElement el : root.getAsJsonArray("scenes")) {
            JsonObject o = el.getAsJsonObject();
            String id = o.has("id") ? o.get("id").getAsString() : "scene" + scenes.size();
            int durationS = o.has("duration_s") ? o.get("duration_s").getAsInt() : 30;
            if (durationS <= 0) {
                throw new IllegalArgumentException("espectral-bench.json: scene \"" + id + "\" needs duration_s > 0");
            }
            List<Step> steps = new ArrayList<>();
            if (o.has("steps")) {
                JsonArray arr = o.getAsJsonArray("steps");
                for (JsonElement s : arr) {
                    steps.add(Step.parse(s.getAsJsonObject()));
                }
            }
            scenes.add(new Scene(id, durationS, steps));
        }
        if (scenes.isEmpty()) {
            throw new IllegalArgumentException("espectral-bench.json: no scenes");
        }
        return new BenchSpec(world, seed, scenes);
    }

    public static final class Scene {
        public final String id;
        public final int durationS;
        public final List<Step> steps;

        Scene(String id, int durationS, List<Step> steps) {
            this.id = id;
            this.durationS = durationS;
            this.steps = steps;
        }
    }

    public static final class Step {
        public enum Kind { TP, LOOK, SUMMON, WAIT }

        public final Kind kind;
        public final double[] vec;
        public final String entity;
        public final int count;
        public final double waitS;

        private Step(Kind kind, double[] vec, String entity, int count, double waitS) {
            this.kind = kind;
            this.vec = vec;
            this.entity = entity;
            this.count = count;
            this.waitS = waitS;
        }

        static Step parse(JsonObject o) {
            if (o.has("tp")) {
                return new Step(Kind.TP, vec(o.getAsJsonArray("tp"), 3, "tp"), null, 0, 0);
            }
            if (o.has("look")) {
                return new Step(Kind.LOOK, vec(o.getAsJsonArray("look"), 2, "look"), null, 0, 0);
            }
            if (o.has("summon")) {
                JsonObject s = o.getAsJsonObject("summon");
                String entity = s.get("entity").getAsString();
                int count = s.has("count") ? s.get("count").getAsInt() : 1;
                if (count <= 0) {
                    throw new IllegalArgumentException("espectral-bench.json: summon count must be > 0");
                }
                double[] pos = s.has("pos") ? vec(s.getAsJsonArray("pos"), 3, "summon.pos") : new double[] {0, 80, 0};
                return new Step(Kind.SUMMON, pos, entity, count, 0);
            }
            if (o.has("wait_s")) {
                double waitS = o.get("wait_s").getAsDouble();
                if (waitS < 0) {
                    throw new IllegalArgumentException("espectral-bench.json: wait_s must be >= 0");
                }
                return new Step(Kind.WAIT, null, null, 0, waitS);
            }
            throw new IllegalArgumentException("espectral-bench.json: unknown step verb in " + o);
        }

        private static double[] vec(JsonArray arr, int n, String what) {
            if (arr.size() != n) {
                throw new IllegalArgumentException("espectral-bench.json: " + what + " needs " + n + " numbers");
            }
            double[] out = new double[n];
            for (int i = 0; i < n; i++) {
                out[i] = arr.get(i).getAsDouble();
            }
            return out;
        }
    }
}
