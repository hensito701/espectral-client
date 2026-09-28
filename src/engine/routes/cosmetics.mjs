/**
 * Cosmetics + mod-bridge routes.
 *
 * - GET /api/cosmetics          -> { catalog, accounts:{<uuid>:{share, loadout, hidden_players, flags, emotes}} }
 * - PUT /api/cosmetics/:uuid    -> validate + apply {share?, loadout?, hidden_players?, emotes?}
 * - POST /api/bridge/visible    -> mod-facing: {uuids:[…]} -> {players:[{uuid, grant, loadout}]}
 *
 * The bridge route requires both the global `x-espectral-client: 1` header and
 * the per-launch `x-espectral-bridge` token minted at launch time.
 */
import { listAccounts } from '../accounts.mjs';
import * as identity from '../identity.mjs';
import * as cosmetics from '../cosmetics.mjs';
import * as bridge from '../bridge.mjs';
import { httpError } from '../error.mjs';

export async function register(app) {
  // GET /api/cosmetics — catalog plus every account's cosmetics state and its
  // verified flags (grant-valid flags; last-known flags while still verified).
  app.get('/api/cosmetics', async () => {
    const accounts = {};
    for (const a of listAccounts()) {
      const flat = identity.normalizeUuid(a?.uuid ?? '');
      if (!flat) continue;
      const { flags } = identity.flagsFor(flat);
      accounts[flat] = cosmetics.cosmeticsAccountView(flat, flags);
    }
    return { catalog: cosmetics.catalog().doc, accounts };
  });

  // PUT /api/cosmetics/:uuid — validated against catalog + verified flags +
  // exclusivity. Rewrites the mod file in every running gameDir of the uuid
  // and republishes presence when the uuid is sharing.
  app.put('/api/cosmetics/:uuid', async (req, res, params, body) => {
    const flat = identity.normalizeUuid(params.uuid);
    if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
    const known = listAccounts().some((a) => identity.normalizeUuid(a?.uuid ?? '') === flat);
    if (!known) throw httpError(404, 'NOT_FOUND', `no account with uuid ${params.uuid}`);
    const { flags } = identity.flagsFor(flat);
    await cosmetics.patchCosmetics(flat, body ?? {}, { flags });
    return cosmetics.cosmeticsAccountView(flat, identity.flagsFor(flat).flags);
  });

  // POST /api/bridge/visible — the in-game mod reports the uuids currently
  // visible on the vanilla player list; answers from the presence cache
  // immediately and queues uncached uuids for batched background queries.
  app.post('/api/bridge/visible', async (req, res, params, body) => {
    const owner = bridge.authenticate(req.headers[bridge.BRIDGE_HEADER] ?? '');
    if (!owner) {
      throw httpError(403, 'BRIDGE_AUTH', 'missing or invalid x-espectral-bridge token');
    }
    return bridge.visible(owner, body?.uuids);
  });
}
