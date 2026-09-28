/**
 * Identity routes — engine half of the espectral-id contract.
 *
 * - GET    /api/identity                      -> { service_url, accounts:[…], error? }
 * - POST   /api/identity/:uuid/verify         -> Mojang join + service verify -> the account entry
 * - GET    /api/identity/:uuid/discord/start  -> { url } (link-state OAuth start)
 * - DELETE /api/identity/:uuid/discord        -> unlink
 * - POST   /api/identity/:uuid/refresh        -> fresh grant for a verified uuid
 *
 * :uuid params accept dashed or flat hex; all stored state is flat 32-hex.
 * Non-GET routes need `x-espectral-client: 1` (global guard in server.mjs).
 */
import { listAccounts, getActiveAccount } from '../accounts.mjs';
import * as discordAuth from '../discordAuth.mjs';
import * as identity from '../identity.mjs';
import * as cosmetics from '../cosmetics.mjs';
import { httpError } from '../error.mjs';

function findAccountByUuid(uuidParam) {
  const flat = identity.normalizeUuid(uuidParam);
  if (!flat) return { flat: null, account: null };
  const account = listAccounts().find((a) => identity.normalizeUuid(a?.uuid ?? '') === flat) ?? null;
  return { flat, account };
}

export async function register(app) {
  // GET /api/identity — every configured account with its identity overlay.
  app.get('/api/identity', async () => {
    let error = null;
    let accounts = [];
    try {
      accounts = listAccounts().map((a) => identity.accountIdentityEntry(a));
    } catch (e) {
      error = String(e?.message ?? e);
    }
    const out = { service_url: identity.identityUrl(), accounts };
    if (error) out.error = error;
    return out;
  });

  // POST /api/identity/:uuid/verify — uuid ownership proof handshake.
  app.post('/api/identity/:uuid/verify', async (req, res, params) => {
    const { flat, account } = findAccountByUuid(params.uuid);
    if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
    if (!account) throw httpError(404, 'NOT_FOUND', `no account with uuid ${params.uuid}`);
    await identity.verifyAccount(account);
    return identity.accountIdentityEntry(account);
  });

  // GET /api/identity/:uuid/discord/start — OAuth URL whose state is bound to
  // the uuid; /api/discord/callback forwards to /v1/discord/link for it.
  app.get('/api/identity/:uuid/discord/start', async (req, res, params) => {
    const { flat, account } = findAccountByUuid(params.uuid);
    if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
    if (!account) throw httpError(404, 'NOT_FOUND', `no account with uuid ${params.uuid}`);
    if (!identity.tokenFor(flat)) {
      throw httpError(409, 'NOT_VERIFIED', 'verify this account before linking Discord');
    }
    const { url } = discordAuth.getAuthStartUrl({ linkUuid: flat });
    return { url };
  });

  // DELETE /api/identity/:uuid/discord — unlink.
  app.delete('/api/identity/:uuid/discord', async (req, res, params) => {
    const { flat, account } = findAccountByUuid(params.uuid);
    if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
    if (!account) throw httpError(404, 'NOT_FOUND', `no account with uuid ${params.uuid}`);
    await identity.discordUnlink(flat);
    return identity.accountIdentityEntry(account);
  });

  // POST /api/identity/:uuid/refresh — pull a fresh grant now.
  app.post('/api/identity/:uuid/refresh', async (req, res, params) => {
    const { flat, account } = findAccountByUuid(params.uuid);
    if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
    if (!account) throw httpError(404, 'NOT_FOUND', `no account with uuid ${params.uuid}`);
    await identity.refreshGrant(flat);
    await cosmetics.rewriteForRunning(flat);
    return identity.accountIdentityEntry(account);
  });
}
