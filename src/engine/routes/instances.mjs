/**
 * Instance routes — the full instance API owned by B1:
 * list/create/get(detail)/delete/patch + servers.dat GET/PUT + options GET.
 * (Mods endpoints live in routes/mods.mjs (B2), import in routes/import.mjs (B3),
 * AOT train/launch in routes/launch.mjs (B4).)
 */
import * as instances from '../instances.mjs';
import { httpError } from '../error.mjs';
import { ensureBrandingSeeded } from '../mods.mjs';
import { activeInstances } from './launch.mjs';
import { applyVersionChange, planVersionChange } from '../version-change.mjs';

export async function register(app) {
  app.get('/api/instances', async () => {
    const list = await instances.listInstances();
    return list.map((inst) => ({
      ...inst,
      running: activeInstances.has(inst.name),
    }));
  });
  app.post('/api/instances', async (req, res, params, body) => {
    return instances.createInstance(body ?? {});
  });

  app.get('/api/instances/:name', async (req, res, params) => {
    const detail = await instances.getInstanceDetail(params.name);
    // Retro-seed branding for old fabric instances that predate the preset
    try { ensureBrandingSeeded(detail.summary).catch(() => {}); } catch { /* ignore */ }
    return {
      ...detail,
      summary: {
        ...detail.summary,
        running: activeInstances.has(params.name),
      },
    };
  });

  app.delete('/api/instances/:name', async (req, res, params) => {
    return instances.deleteInstance(params.name);
  });

  app.patch('/api/instances/:name', async (req, res, params, body) => {
    const patch = body ?? {};
    const rawRename = patch.new_name !== undefined ? patch.new_name : patch.name;
    const wantsRename =
      typeof rawRename === 'string' && rawRename.trim() !== '' && rawRename.trim() !== params.name;
    if (wantsRename && activeInstances.has(params.name)) {
      throw httpError(409, 'ALREADY_RUNNING', `instance '${params.name}' is running; stop it before renaming`);
    }
    return instances.patchInstance(params.name, patch);
  });

  // Version / platform change: preview, then apply. Never while the game runs
  // (it holds the mod jars), and the instance is marked busy while applying
  // so a launch cannot start against half-swapped mods.
  const versionRequest = (body) => ({
    version: body && typeof body.version === 'string' ? body.version : undefined,
    loader: body && typeof body.loader === 'string' ? body.loader : undefined,
  });
  app.post('/api/instances/:name/version-change/plan', async (req, res, params, body) => {
    return planVersionChange(params.name, versionRequest(body));
  });
  app.post('/api/instances/:name/version-change', async (req, res, params, body) => {
    if (activeInstances.has(params.name)) {
      throw httpError(409, 'ALREADY_RUNNING', `instance '${params.name}' is running; close the game first`);
    }
    activeInstances.add(params.name);
    try {
      return await applyVersionChange(params.name, versionRequest(body));
    } finally {
      activeInstances.delete(params.name);
    }
  });

  app.get('/api/instances/:name/servers', async (req, res, params) => {
    return { servers: await instances.readServersDat(params.name) };
  });

  app.put('/api/instances/:name/servers', async (req, res, params, body) => {
    return instances.writeServers(params.name, body && body.servers);
  });

  app.get('/api/instances/:name/options', async (req, res, params) => {
    return { options: await instances.readOptions(params.name) };
  });

  app.post('/api/instances/:name/options/import', async (req, res, params, body) => {
    const source = body && typeof body.source === 'string' ? body.source.trim() : '';
    if (!source) throw httpError(400, 'BAD_REQUEST', 'source is required');
    return instances.importOptionsFromInstance(params.name, source);
  });
}
