/**
 * First-launch onboarding: create the ready-to-play default instance and
 * import settings from the most recently active detected Minecraft launcher.
 *
 * This runs before the HTTP server starts, so the first UI request sees the
 * instance. The performance and branding presets are installed and AOT
 * training is queued immediately after so the AOT cache is warm before the
 * first launch.
 */
import fs from 'node:fs';
import path from 'node:path';
import { loadConfig, saveConfig } from './config.mjs';
import { detectImportSources } from './import.mjs';
import { createInstance, listInstances } from './instances.mjs';

const DEFAULT_INSTANCE = Object.freeze({
  name: 'Espectral default',
  version: '26.2',
  loader: 'fabric',
  memory_mb: 3072,
});

function sourceActivity(source) {
  let latest = 0;
  for (const file of ['options.txt', 'optionsLC.txt', 'servers.dat']) {
    try {
      latest = Math.max(latest, fs.statSync(path.join(source.path, file)).mtimeMs);
    } catch {
      /* Missing files are normal for a detected but unused source. */
    }
  }
  return latest;
}

/** Choose the source whose settings were changed most recently. */
function chooseActiveSource(sources) {
  const usable = sources.filter((source) => source.options_exists || source.servers_exists);
  const otherLaunchers = usable.filter((source) => source.kind !== 'vanilla');
  const candidates = otherLaunchers.length ? otherLaunchers : usable;
  return candidates
    .map((source) => ({ source, activity: sourceActivity(source) }))
    .sort((a, b) => b.activity - a.activity)[0]?.source ?? null;
}

/**
 * Perform the one-time first-launch setup. The config marker prevents a later
 * manual deletion of all instances from silently recreating the default.
 */
export async function ensureFirstLaunch() {
  const config = loadConfig();
  if (config.first_launch_setup_complete === true) return { created: false, reason: 'already-complete' };

  const existing = await listInstances();
  if (existing.length > 0) {
    saveConfig({ first_launch_setup_complete: true });
    return { created: false, reason: 'instances-exist' };
  }

  let source = null;
  try {
    source = chooseActiveSource(await detectImportSources());
  } catch (error) {
    console.warn('[onboarding] import source detection failed:', error.message);
  }

  const importedFrom = source?.id ?? undefined;
  const memory = Number.isInteger(source?.lunar?.allocated_memory) && source.lunar.allocated_memory >= 512
    ? source.lunar.allocated_memory
    : DEFAULT_INSTANCE.memory_mb;

  try {
    await createInstance({
      ...DEFAULT_INSTANCE,
      memory_mb: memory,
      import_from: importedFrom,
      merge_optionslc: source?.kind === 'lunar',
      defer_auto_train: true,
    });
    // Ship the default instance with the performance mod preset + the bundled
    // Espectral Menu, then kick off AOT training once presets are installed
    // so the AOT cache is warm before first play (fire-and-forget).
    void (async () => {
      try {
        const mods = await import('./mods.mjs');
        try {
          await mods.installPreset(DEFAULT_INSTANCE.name, mods.PERFORMANCE_PRESET);
        } catch (e) {
          console.warn('[onboarding] performance preset install failed:', e.message);
        }
        try {
          await mods.installPreset(DEFAULT_INSTANCE.name, mods.BRANDING_PRESET);
        } catch (e) {
          console.warn('[onboarding] branding install failed:', e.message);
        }
        const [aot, instances] = await Promise.all([
          import('./aot.mjs'),
          import('./instances.mjs'),
        ]);
        const instance = await instances.getInstance(DEFAULT_INSTANCE.name);
        if (instance && aot && typeof aot.queueTrainInstance === 'function') {
          await aot.queueTrainInstance(instance);
        }
      } catch (e) {
        console.warn('[onboarding] preset install / AOT training failed:', e.message);
      }
    })();
    saveConfig({
      first_launch_setup_complete: true,
      first_launch_import_source: source?.id ?? null,
    });
    console.log(`[onboarding] created '${DEFAULT_INSTANCE.name}'${source ? ` from ${source.label}` : ''}`);
    return { created: true, source: source?.id ?? null };
  } catch (error) {
    console.warn('[onboarding] default instance creation failed:', error.message);
    // Do not permanently mark a failed setup complete; the next launch retries.
    return { created: false, reason: 'creation-failed', error: error.message };
  }
}
