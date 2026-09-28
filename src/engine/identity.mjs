/**
 * Espectral identity — engine half of the espectral-id service contract.
 *
 * State lives at <dataDir>/identity.json (like mc-token-cache.json), keyed by
 * the account's 32-hex uuid:
 *   { schema:1, accounts: { <uuid>: {
 *       token: <session token>, token_expires_at: <iso>,
 *       discord: { id, username, avatar } | null,
 *       flags: [..], grant: <jws>, grant_exp: <unix s> } } }
 *
 * The service base URL comes from config.identity_url (default
 * https://id.espectral.es, env ESPECTRAL_ID_URL override) so tests point at a
 * local stub. The Mojang session server can be stubbed the same way via
 * ESPECTRAL_SESSION_SERVER (tests only; the real endpoint is the default).
 *
 * NOTHING here may throw into a launch path — callers that need soft failure
 * use refreshGrantSoft().
 */
import fs from 'node:fs';
import path from 'node:path';
import { dataDir, loadConfig } from './config.mjs';
import { verifyGrant } from './grant.mjs';
import { httpError } from './error.mjs';

export const DEFAULT_IDENTITY_URL = 'https://id.espectral.es';

const STATE_SCHEMA = 1;
let cachedState = null;

/** Service base URL (no trailing slash). Env override wins. */
export function identityUrl() {
  const env = process.env.ESPECTRAL_ID_URL;
  if (env && env.trim()) return env.trim().replace(/\/+$/, '');
  const cfg = loadConfig().identity_url;
  return (typeof cfg === 'string' && cfg.trim() ? cfg.trim() : DEFAULT_IDENTITY_URL).replace(/\/+$/, '');
}

/** Mojang session server base (env override for stubbed tests). */
function sessionServerUrl() {
  const env = process.env.ESPECTRAL_SESSION_SERVER;
  return (env && env.trim() ? env.trim() : 'https://sessionserver.mojang.com').replace(/\/+$/, '');
}

export function identityPath() {
  return path.join(dataDir(), 'identity.json');
}

/** Load (and lazily create) the identity state document. Live object. */
export function loadIdentityState() {
  if (cachedState) return cachedState;
  let raw = {};
  try {
    raw = JSON.parse(fs.readFileSync(identityPath(), 'utf8'));
  } catch {
    raw = {};
  }
  cachedState = {
    schema: STATE_SCHEMA,
    accounts:
      raw && typeof raw.accounts === 'object' && raw.accounts !== null && !Array.isArray(raw.accounts)
        ? raw.accounts
        : {},
  };
  return cachedState;
}

function persistIdentity(state) {
  try {
    fs.mkdirSync(path.dirname(identityPath()), { recursive: true });
    const tmp = identityPath() + '.tmp';
    fs.writeFileSync(tmp, JSON.stringify(state, null, 2), 'utf8');
    fs.renameSync(tmp, identityPath());
  } catch (e) {
    console.warn('[identity] could not persist identity.json:', e.message);
  }
}

/** Normalize any uuid form ("dashes ok") to 32 lowercase hex; null when bad. */
export function normalizeUuid(uuid) {
  if (typeof uuid !== 'string') return null;
  const flat = uuid.replace(/-/g, '').toLowerCase();
  return /^[0-9a-f]{32}$/.test(flat) ? flat : null;
}

/** The stored identity entry for a uuid, or null. */
export function getEntry(uuid) {
  const flat = normalizeUuid(uuid);
  if (!flat) return null;
  const entry = loadIdentityState().accounts[flat];
  return entry && typeof entry === 'object' ? entry : null;
}

/** Shallow-merge fields into the uuid's identity entry and persist. */
export function updateEntry(uuid, patch) {
  const flat = normalizeUuid(uuid);
  if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
  const state = loadIdentityState();
  const prev = state.accounts[flat] && typeof state.accounts[flat] === 'object' ? state.accounts[flat] : {};
  state.accounts[flat] = { ...prev, ...patch };
  persistIdentity(state);
  return state.accounts[flat];
}

/** Bearer token for service calls, or null when absent/expired. */
export function tokenFor(uuid, { now = Date.now() } = {}) {
  const entry = getEntry(uuid);
  if (!entry || typeof entry.token !== 'string' || !entry.token) return null;
  const exp = Date.parse(entry.token_expires_at ?? '');
  if (Number.isFinite(exp) && exp <= now) return null;
  return entry.token;
}

/**
 * The flags a uuid may act on: a currently-valid grant's flags win; when the
 * grant is absent/expired but a live session token remains, fall back to the
 * flags the service returned with the last grant (server-verified at fetch
 * time). Never trust a stored grant's flags once the grant itself is dead.
 */
export function flagsFor(uuid) {
  const entry = getEntry(uuid);
  if (!entry) return { flags: [], grant: null, grantValid: false };
  if (typeof entry.grant === 'string' && entry.grant) {
    try {
      const payload = verifyGrant(entry.grant, { sub: normalizeUuid(uuid) });
      return { flags: [...(payload.flags ?? [])], grant: entry.grant, grantValid: true };
    } catch {
      /* invalid/expired grant -> no grant-derived flags */
    }
  }
  if (tokenFor(uuid)) {
    const flags = Array.isArray(entry.flags) ? entry.flags.filter((f) => typeof f === 'string') : [];
    return { flags, grant: null, grantValid: false };
  }
  return { flags: [], grant: null, grantValid: false };
}

// ---------------------------------------------------------------------------
// Service HTTP client
// ---------------------------------------------------------------------------

/**
 * fetch() against espectral-id with a timeout. `token` adds the Bearer
 * header. Service error envelopes ({error:{code,message}}) map to ApiError
 * with the service's status/code; transport failures map to 502
 * IDENTITY_UNREACHABLE so callers can distinguish "service down" from
 * "service said no".
 */
export async function serviceFetch(method, p, { body = undefined, token = null, timeoutMs = 5000 } = {}) {
  const headers = { Accept: 'application/json' };
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  if (token) headers.Authorization = `Bearer ${token}`;
  let res;
  try {
    res = await fetch(identityUrl() + p, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(timeoutMs),
    });
  } catch (e) {
    throw httpError(502, 'IDENTITY_UNREACHABLE', `identity service unreachable: ${e?.message ?? e}`);
  }
  let data = null;
  try {
    data = await res.json();
  } catch {
    data = null;
  }
  if (!res.ok) {
    const code = data && typeof data.error?.code === 'string' ? data.error.code : `HTTP_${res.status}`;
    const msg = data && typeof data.error?.message === 'string' ? data.error.message : `identity service returned ${res.status}`;
    const err = httpError(res.status >= 400 && res.status < 500 ? res.status : 502, code, msg);
    throw err;
  }
  return data;
}

/**
 * Normalize a service-side discord identity ({id, username, avatar}) into the
 * engine shape ({id, name, avatar-url}). `avatar` may be a full URL or a bare
 * Discord avatar hash.
 */
export function normalizeDiscordIdentity(d) {
  if (!d || typeof d !== 'object' || d.id == null) return null;
  let avatar = typeof d.avatar === 'string' ? d.avatar : null;
  if (avatar && !/^https?:\/\//.test(avatar)) {
    avatar = `https://cdn.discordapp.com/avatars/${d.id}/${avatar}.png?size=128`;
  }
  if (!avatar) {
    let defaultIndex = 0;
    try {
      defaultIndex = Number((BigInt(String(d.id)) >> 22n) % 6n);
    } catch {
      defaultIndex = 0;
    }
    avatar = `https://cdn.discordapp.com/embed/avatars/${defaultIndex}.png`;
  }
  return {
    id: String(d.id),
    name: String(d.username ?? d.global_name ?? `User ${d.id}`),
    avatar,
  };
}

/**
 * Fetch a fresh grant for a verified uuid (GET /v1/grant). Stores the grant +
 * flags; returns {grant, flags, exp}. Throws ApiError (502 when unreachable,
 * or the service's own 4xx).
 */
export async function refreshGrant(uuid, { timeoutMs = 5000 } = {}) {
  const token = tokenFor(uuid);
  if (!token) throw httpError(401, 'NOT_VERIFIED', 'uuid has no live identity session');
  const data = await serviceFetch('GET', '/v1/grant', { token, timeoutMs });
  const grant = typeof data?.grant === 'string' ? data.grant : null;
  const flags = Array.isArray(data?.flags) ? data.flags.filter((f) => typeof f === 'string') : [];
  const exp = Number.isFinite(data?.exp) ? data.exp : null;
  updateEntry(uuid, { grant, flags, grant_exp: exp });
  return { grant, flags, exp };
}

/**
 * Grant refresh for launch/put paths: bounded by `budgetMs`, returns
 * {grant,flags}|null and NEVER throws — identity must not fail or delay a
 * launch beyond its small budget.
 */
export async function refreshGrantSoft(uuid, budgetMs = 2000) {
  try {
    return await refreshGrant(uuid, { timeoutMs: Math.max(250, Math.floor(budgetMs)) });
  } catch {
    return null;
  }
}

/**
 * Prove uuid ownership: Mojang join with the account's live MC token, then
 * the service challenge/verify handshake. Returns the stored account entry.
 */
export async function verifyAccount(account) {
  const flat = normalizeUuid(account?.uuid);
  if (!flat) throw httpError(400, 'BAD_UUID', 'account uuid is missing or malformed');
  if (account.token_kind !== 'msa') {
    throw httpError(400, 'ACCOUNT_NOT_MSA', 'only Microsoft accounts can prove uuid ownership');
  }
  const msauth = await import('./msauth.mjs');
  const mc = await msauth.ensureMinecraftToken(account);
  if (!mc?.accessToken) {
    throw httpError(401, 'MSA_REFRESH_FAILED', 'could not obtain a Minecraft session token — sign in again');
  }

  // 1. challenge
  const challenge = await serviceFetch('POST', '/v1/auth/challenge', { body: {} });
  const challengeId = challenge?.challenge_id;
  const serverId = challenge?.server_id;
  if (typeof challengeId !== 'string' || typeof serverId !== 'string' || !challengeId || !serverId) {
    throw httpError(502, 'BAD_CHALLENGE', 'identity service returned a malformed challenge');
  }

  // 2. Mojang join (proof the launcher holds the session for `username`)
  let joinRes;
  try {
    joinRes = await fetch(`${sessionServerUrl()}/session/minecraft/join`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        accessToken: mc.accessToken,
        selectedProfile: flat,
        serverId,
      }),
      signal: AbortSignal.timeout(8000),
    });
  } catch (e) {
    throw httpError(502, 'MOJANG_UNREACHABLE', `Mojang session server unreachable: ${e?.message ?? e}`);
  }
  if (joinRes.status !== 204 && !joinRes.ok) {
    throw httpError(401, 'MOJANG_JOIN_REJECTED', `Mojang rejected the join (HTTP ${joinRes.status}) — session may be stale`);
  }

  // 3. verify -> session token
  const verified = await serviceFetch('POST', '/v1/auth/verify', {
    body: { challenge_id: challengeId, username: account.username },
  });
  if (typeof verified?.token !== 'string' || !verified.token) {
    throw httpError(502, 'BAD_VERIFY', 'identity service did not return a session token');
  }
  if (verified.uuid && normalizeUuid(verified.uuid) && normalizeUuid(verified.uuid) !== flat) {
    throw httpError(409, 'UUID_MISMATCH', 'identity service verified a different uuid than the account');
  }
  const entry = updateEntry(flat, {
    token: verified.token,
    token_expires_at: typeof verified.expires_at === 'string' ? verified.expires_at : null,
  });
  // Grant straight away so the account starts out with live flags.
  await refreshGrant(flat).catch((e) => {
    console.warn(`[identity] initial grant fetch for ${flat.slice(0, 8)}… failed: ${e?.code ?? e?.message}`);
  });
  return entry;
}

/** GET /v1/discord/identify — plain launcher Discord login (no link). */
export async function discordIdentify(code, redirectUri) {
  const data = await serviceFetch('POST', '/v1/discord/identify', {
    body: { code, redirect_uri: redirectUri },
  });
  const user = normalizeDiscordIdentity(data?.discord);
  if (!user) throw httpError(502, 'DISCORD_IDENTIFY_FAILED', 'identity service did not return a discord identity');
  return user;
}
/** POST /v1/discord/link — bind a discord identity to a verified uuid. */
export async function discordLink(uuid, code, redirectUri) {
  const token = tokenFor(uuid);
  if (!token) throw httpError(401, 'NOT_VERIFIED', 'uuid has no live identity session');
  const data = await serviceFetch('POST', '/v1/discord/link', {
    token,
    body: { code, redirect_uri: redirectUri },
  });
  const user = normalizeDiscordIdentity(data?.discord);
  if (!user) throw httpError(502, 'DISCORD_LINK_FAILED', 'identity service did not return a discord identity');
  const flags = Array.isArray(data?.flags) ? data.flags.filter((f) => typeof f === 'string') : null;
  updateEntry(uuid, { discord: user, ...(flags ? { flags } : {}) });
  await refreshGrant(uuid).catch(() => {});
  return user;
}


/** Public account view for GET /api/identity. */
export function accountIdentityEntry(account) {
  const flat = normalizeUuid(account?.uuid);
  const entry = flat ? getEntry(flat) : null;
  const token = flat ? tokenFor(flat) : null;
  let grantExp = Number.isFinite(entry?.grant_exp) ? entry.grant_exp : null;
  const flags = flat ? flagsFor(flat) : { flags: [] };
  if (flags.grantValid) {
    // Grant itself is the authority on expiry when it still verifies.
    try {
      grantExp = verifyGrant(entry.grant, { sub: flat }).exp;
    } catch {
      /* keep stored exp */
    }
  }
  // Contract shape: {id, username, avatar} (internal records use `name`).
  const discord = entry?.discord
    ? { id: String(entry.discord.id), username: entry.discord.name ?? entry.discord.username ?? null, avatar: entry.discord.avatar ?? null }
    : null;
  return {
    uuid: flat ?? String(account?.uuid ?? ''),
    username: account?.username ?? null,
    verified: Boolean(token),
    discord,
    flags: flags.flags,
    grant_exp: grantExp,
  };
}

/** DELETE /v1/discord/link — drop the discord binding for a uuid. */
export async function discordUnlink(uuid) {
  const token = tokenFor(uuid);
  if (!token) throw httpError(401, 'NOT_VERIFIED', 'uuid has no live identity session');
  await serviceFetch('DELETE', '/v1/discord/link', { token });
  updateEntry(uuid, { discord: null });
  await refreshGrant(uuid).catch(() => {});
  return { ok: true };
}

