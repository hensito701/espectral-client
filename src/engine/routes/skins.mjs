/**
 * Skin library routes: gallery CRUD + apply-to-Mojang + vanilla import.
 * Token handling lives in ../skins.mjs (resolveSkinAccount); the UI never
 * sees a Minecraft access token.
 */
import * as skins from '../skins.mjs';
import { httpError } from '../error.mjs';

/**
 * The `:id.png` pattern compiles to a param holding the full segment
 * ('<id>.png'). Strip the suffix to get the library id.
 */
function skinId(params) {
  const raw = params.id ?? params['id.png'] ?? '';
  return String(raw).replace(/\.png$/i, '');
}

export async function register(app) {
  // GET /api/skins -> { skins: [{ id, name, variant, created_at, source }] }
  app.get('/api/skins', async () => {
    const index = await skins.loadLibraryIndex();
    return { skins: index.map(skins.publicSkinEntry) };
  });

  // GET /api/skins/:id.png serves raw image/png bytes (404 when absent).
  app.get('/api/skins/:id.png', async (req, res, params) => {
    const buf = await skins.readLibraryPng(skinId(params));
    res.writeHead(200, { 'Content-Type': 'image/png', 'Content-Length': buf.length });
    res.end(buf);
    return undefined;
  });

  // POST /api/skins { image_base64, name, variant } -> the saved entry.
  app.post('/api/skins', async (req, res, params, body) => {
    const png = skins.decodeSkinPng(body ? body.image_base64 : undefined);
    return skins.saveLibrarySkin({
      name: body ? body.name : undefined,
      variant: body ? body.variant : undefined,
      png,
      source: 'upload',
    });
  });

  // PATCH /api/skins/:id { name?, variant? } -> the updated entry.
  app.patch('/api/skins/:id', async (req, res, params, body) => {
    const patch = {};
    if (body && body.name !== undefined) patch.name = body.name;
    if (body && body.variant !== undefined) patch.variant = body.variant;
    if (Object.keys(patch).length === 0) {
      throw httpError(400, 'BAD_SKIN_PATCH', 'nothing to update (name?, variant?)');
    }
    return skins.updateLibrarySkin(params.id, patch);
  });

  // DELETE /api/skins/:id -> { removed: true }
  app.delete('/api/skins/:id', async (req, res, params) => {
    return skins.deleteLibrarySkin(params.id);
  });

  // POST /api/skins/:id/apply { username } -> uploads the saved art to the
  // Mojang profile (MSA only) and refreshes the texture cache.
  app.post('/api/skins/:id/apply', async (req, res, params, body) => {
    const username = body && typeof body.username === 'string' ? body.username : '';
    const entry = await skins.getLibraryEntry(params.id);
    const png = await skins.readLibraryPng(params.id);
    const { account, accessToken, msauth } = await skins.resolveSkinAccount(username);
    await msauth.uploadProfileSkin(accessToken, png, entry.variant);
    await skins.writeSkinCache(account.uuid, png);
    // Gallery apply -> refresh the derived avatar unless custom. Best-effort.
    try {
      if (skins.shouldSyncAvatarFromSkin(account.username)) {
        await skins.syncAccountAvatarFromSkin(account.username, png);
      }
    } catch {
      /* avatar stays as-is */
    }
    return { ok: true, variant: entry.variant };
  });

  // POST /api/skins/import-vanilla -> { imported, skipped, total }
  app.post('/api/skins/import-vanilla', async () => {
    return skins.importVanillaSkins();
  });
}
