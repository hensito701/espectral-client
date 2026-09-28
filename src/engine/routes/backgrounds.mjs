/**
 * routes/backgrounds.mjs — menu background packs + personalization write
 * (Workstream 4 free features).
 *
 *   GET    /api/backgrounds                     -> { packs: [meta…] }
 *   POST   /api/backgrounds                     -> { pack }   image pack
 *           { name, image_base64? | source_path? }
 *   POST   /api/backgrounds/frames              -> { pack }   frame pack
 *           { name, fps?, frames[]? | frame_paths[]? | source_path? }
 *   DELETE /api/backgrounds/:id                 -> { ok }
 *   GET    /api/backgrounds/:id/image           -> image/png preview bytes
 *   PATCH  /api/instances/:name/personalization -> ClientInfo
 *           { theme?, hud_theme?, background?, reduced_motion? }
 *           (write goes through patchClientConfig — the THEMES slice owns
 *           the personalization whitelist; this route adds pack existence)
 */
import {
  listPacks,
  importImagePack,
  importFramesPack,
  deletePack,
  packPreviewPath,
  writePersonalization,
} from '../backgrounds.mjs';
import { httpError } from '../error.mjs';
import fs from 'node:fs';

export async function register(app) {
  app.get('/api/backgrounds', async () => ({ packs: listPacks() }));

  app.post('/api/backgrounds', async (req, res, params, body) => {
    return { pack: await importImagePack(body ?? {}) };
  });

  app.post('/api/backgrounds/frames', async (req, res, params, body) => {
    return { pack: await importFramesPack(body ?? {}) };
  });

  app.delete('/api/backgrounds/:id', async (req, res, params) => {
    return deletePack(params.id);
  });

  // GET serves raw image/png bytes (404 when absent) — same shape as the
  // instance icon route.
  app.get('/api/backgrounds/:id/image', async (req, res, params) => {
    const file = packPreviewPath(params.id);
    const buf = fs.readFileSync(file);
    res.writeHead(200, { 'Content-Type': 'image/png', 'Content-Length': buf.length });
    res.end(buf);
    return undefined;
  });

  // PATCH /api/instances/:name/personalization { theme?, hud_theme?,
  // background?, reduced_motion? } -> ClientInfo. `background` additionally
  // requires the pack to exist in the store.
  app.patch('/api/instances/:name/personalization', async (req, res, params, body) => {
    if (body === null || body === undefined || typeof body !== 'object' || Array.isArray(body)) {
      throw httpError(400, 'BAD_PERSONALIZATION', 'request body must be a personalization object');
    }
    return await writePersonalization(params.name, body);
  });
}
