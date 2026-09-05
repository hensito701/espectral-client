/**
 * routes/mods.mjs — instance mod management endpoints:
 *   GET  /api/instances/:name/mods                      -> ModEntry[]
 *   GET  /api/instances/:name/mods/preset               -> { supported, note }
 *   POST /api/instances/:name/mods/:filename/enable     -> { filename, enabled }
 *   POST /api/instances/:name/mods/:filename/disable    -> { filename, enabled }
 *   POST /api/instances/:name/mods/install { preset }   -> { queued: true }
 *   GET  /api/modrinth/search?q=&version=&loader=&instance=&offset= -> { results, total, offset, limit }
 *   POST /api/instances/:name/mods/install-modrinth { project_id } -> { queued: true }
 * The install endpoints validate + queue immediately and run the
 * sha1-verified install in the background, streaming mod-progress SSE events
 * (B1's events.mjs registry).
 */
import {
  listMods,
  setModEnabled,
  installPreset,
  supportsPerformanceBundle,
  supportsBranding,
  brandingVersions,
  pinNoteForVersion,
  searchModrinth,
  installModrinthMod,
} from '../mods.mjs';
import { loadInstanceMeta } from '../resolver.mjs';
import { effectiveModsDir, getInstance } from '../instances.mjs';
import { mkdir } from 'node:fs/promises';

function sendError(app, res, err, fallbackStatus = 500, fallbackCode = 'internal_error') {
  const status = Number.isInteger(err?.status) ? err.status : fallbackStatus;
  return app.sendJson(res, status, {
    error: { code: err?.code ?? fallbackCode, message: err?.message ?? 'internal error' },
  });
}

export async function register(app) {
  // GET /api/instances/:name/mods -> ModEntry[]
  app.get('/api/instances/:name/mods', async (req, res, params) => {
    try {
      return await listMods(params.name);
    } catch (err) {
      return sendError(app, res, err, 500, 'mods_list_failed');
    }
  });

  // GET /api/instances/:name/mods-dir -> { path } — the EFFECTIVE mods dir
  // (<game_dir>/mods when a custom folder is set). Created when missing so
  // POST /api/open-folder (which 404s on absent paths) can open it directly.
  app.get('/api/instances/:name/mods-dir', async (req, res, params) => {
    try {
      const inst = await getInstance(params.name); // 404 when missing
      const dir = effectiveModsDir(inst);
      await mkdir(dir, { recursive: true });
      return { path: dir };
    } catch (err) {
      return sendError(app, res, err, 500, 'mods_dir_failed');
    }
  });

  // POST /api/instances/:name/mods/:filename/enable
  app.post('/api/instances/:name/mods/:filename/enable', async (req, res, params) => {
    try {
      return await setModEnabled(params.name, params.filename, true);
    } catch (err) {
      return sendError(app, res, err, 500, 'mod_enable_failed');
    }
  });

  // POST /api/instances/:name/mods/:filename/disable
  app.post('/api/instances/:name/mods/:filename/disable', async (req, res, params) => {
    try {
      return await setModEnabled(params.name, params.filename, false);
    } catch (err) {
      return sendError(app, res, err, 500, 'mod_disable_failed');
    }
  });

  // POST /api/instances/:name/mods/install { preset: 'performance' | 'branding' | 'qol' } -> { queued: true }
  app.post('/api/instances/:name/mods/install', async (req, res, params, body) => {
    try {
      const name = params.name;
      const preset = body?.preset ?? 'performance';
      if (preset !== 'performance' && preset !== 'branding' && preset !== 'qol') {
        return app.sendJson(res, 400, {
          error: { code: 'unknown_preset', message: `unknown preset: ${preset}` },
        });
      }
      await loadInstanceMeta(name); // 404 before queuing background work
      installPreset(name, preset).catch((err) => {
        console.error(`[mods] install failed for ${name}: ${err?.message ?? err}`);
      });
      return { queued: true };
    } catch (err) {
      return sendError(app, res, err, 500, 'mod_install_failed');
    }
  });

  // GET /api/instances/:name/mods/preset -> { supported, note, branding: { supported, note } }
  app.get('/api/instances/:name/mods/preset', async (req, res, params) => {
    try {
      const inst = await loadInstanceMeta(params.name);
      // The mod-pin presets (performance/QoL) are Fabric-only — vanilla and
      // NeoForge instances can't load them (for vanilla, integrated
      // no-fog/fullbright live in Settings → Client). Branding is
      // version-keyed (bundled Espectral Menu), so it's reported
      // independently of the loader.
      const note = inst.loader === 'neoforge'
        ? 'Conjunto de rendimiento solo disponible en Fabric'
        : inst.loader === 'vanilla'
          ? 'vanilla instances use integrated no-fog/fullbright (Settings → Client)'
          : pinNoteForVersion(inst.version);
      return {
        supported: inst.loader === 'fabric' ? supportsPerformanceBundle(inst.version) : false,
        loader: inst.loader,
        note,
        branding: {
          supported: supportsBranding(inst.version),
          note: supportsBranding(inst.version)
            ? null
            : `no bundled Espectral Menu for ${inst.version}; branding covers ${brandingVersions().join(', ') || 'nothing'}`,
        },
      };
    } catch (err) {
      return sendError(app, res, err, 404, 'instance_not_found');
    }
  });

  // GET /api/modrinth/search?q=&version=&loader=&instance=&offset= -> { results, total, offset, limit }
  // Empty q returns the most-downloaded mods (explore mode), always sorted by
  // downloads with a fixed page size of 12. version/loader scope the facets
  // only when provided.
  app.get('/api/modrinth/search', async (req, res) => {
    try {
      const q = new URL(req.url, 'http://localhost').searchParams;
      const query = (q.get('q') ?? '').trim();
      const version = q.get('version') || null;
      const loader = q.get('loader') || null;
      const instance = q.get('instance');
      const offset = Math.max(0, Number.parseInt(q.get('offset') ?? '0', 10) || 0);
      return await searchModrinth(query, version, loader, instance, { offset });
    } catch (err) {
      return sendError(app, res, err, 502, 'modrinth_search_failed');
    }
  });

  // POST /api/instances/:name/mods/install-modrinth { project_id } -> { queued: true }
  app.post('/api/instances/:name/mods/install-modrinth', async (req, res, params, body) => {
    try {
      const name = params.name;
      const projectId = body?.project_id;
      if (typeof projectId !== 'string' || projectId.length === 0) {
        return app.sendJson(res, 400, {
          error: { code: 'modrinth_project_id_required', message: 'missing project_id' },
        });
      }
      const inst = await loadInstanceMeta(name); // 404 before queuing background work
      const loader = inst.loader === 'neoforge' ? 'neoforge' : 'fabric';
      installModrinthMod(name, projectId, loader).catch((err) => {
        console.error(`[mods] install-modrinth failed for ${name}: ${err?.message ?? err}`);
      });
      return { queued: true };
    } catch (err) {
      return sendError(app, res, err, 500, 'mod_install_modrinth_failed');
    }
  });
}
