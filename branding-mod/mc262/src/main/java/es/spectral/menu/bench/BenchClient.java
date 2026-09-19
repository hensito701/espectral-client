package es.spectral.menu.bench;

import java.io.File;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.loader.api.FabricLoader;

/**
 * Lane-local bench entrypoint (26.2 only). Checks once for
 * {@code <gameDir>/espectral-bench.json} and arms {@link BenchEngine};
 * absent file means the engine stays fully inert.
 */
public final class BenchClient implements ClientModInitializer {

    @Override
    public void onInitializeClient() {
        File gameDir = FabricLoader.getInstance().getGameDir().toFile();
        BenchEngine.maybeArm(new File(gameDir, "espectral-bench.json"),
                new File(gameDir, "bench-metrics.jsonl"));
    }
}
