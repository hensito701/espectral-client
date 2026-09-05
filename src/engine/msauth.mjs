/**
 * Microsoft authentication (MSA) for real (online) Minecraft accounts.
 *
 * Device-code flow:
 *   1. POST /devicecode  -> { device_code, user_code, verification_uri, interval }
 *      (UI shows the URL + code; engine polls the token endpoint)
 *   2. POST /token (grant_type=device_code) until access_token (authorization_pending
 *      keeps polling; declined/expired abort)
 *   3. XBL: user.auth.xboxlive.com/user/authenticate   (RPS ticket)
 *   4. XSTS: xsts.auth.xboxlive.com/xsts/authorize     (RETAIL sandbox)
 *   5. MC: api.minecraftservices.com/authentication/login_with_xbox
 *   6. Profile: api.minecraftservices.com/minecraft/profile -> { id, name }
 *
 * CLIENT ID: Microsoft retired the old shared launcher ID (00000000402b5328).
 * A working login needs an Azure application (portal.azure.com -> App
 * registrations) with a public client / mobile-desktop platform, no secret,
 * the "XboxLive.signin offline_access" scopes, and — for new apps — approval
 * from Mojang for the Minecraft API (form: aka.ms/mce-reviewappid). The ID is
 * read from ESPECTRAL_MSA_CLIENT_ID, falling back to this constant.
 *
 * Token model:
 *   - The MSA refresh_token is persisted per account in data/config.json (needed to
 *     re-login without a browser).
 *   - Accounts imported from Lunar Client carry `microsoft.lunar: true`; their
 *     refresh token is bound to the legacy launcher client id + login.live.com
 *     endpoint (see refreshLunarMsaToken below).
 *   - The Minecraft access token is kept in MEMORY ONLY (never written to disk) and
 *     refreshed transparently on launch when expired (expires_in ~24h).
 *   - Account shape: { username, uuid, token_kind: 'msa', microsoft: {
 *       refresh_token, expires_at, xuid?, lunar? } } — token_kind stays 'msa' so
 *       the UI and launch can distinguish online vs offline accounts.
 *
 * All HTTP via global fetch (Node 18+); no deps. Errors are mapped to httpError
 * codes so the UI can show friendly messages.
 */
import fs from 'node:fs';
import path from 'node:path';
import { getActiveAccount } from './accounts.mjs';
import { httpError } from './error.mjs';
import { loadConfig, saveConfig, dataDir } from './config.mjs';

// Espectral Client's Azure app registration (client ID 071ffa4b-eafb-4b7a-aa71-d8beef4f4f2e,
// "Personal Microsoft accounts", public client flows enabled). An org/user can still override
// with msa_client_id in config.json or ESPECTRAL_MSA_CLIENT_ID.
const DEFAULT_CLIENT_ID = '071ffa4b-eafb-4b7a-aa71-d8beef4f4f2e';
/**
 * Client ID resolution order:
 *   1. `msa_client_id` in data/config.json (works in the installed desktop app —
 *      the Tauri shell does not forward arbitrary env vars to the engine)
 *   2. ESPECTRAL_MSA_CLIENT_ID env var (browser/dev flow)
 *   3. the constant above
 */
export function clientId() {
  try {
    const cfg = loadConfig();
    if (typeof cfg.msa_client_id === 'string' && cfg.msa_client_id.length > 0) {
      return cfg.msa_client_id;
    }
  } catch {
    /* config unavailable — fall through */
  }
  return process.env.ESPECTRAL_MSA_CLIENT_ID ?? DEFAULT_CLIENT_ID;
}

const AUTH_BASE = 'https://login.microsoftonline.com/consumers/oauth2/v2.0';
const XBL_AUTH = 'https://user.auth.xboxlive.com/user/authenticate';
const XSTS_AUTH = 'https://xsts.auth.xboxlive.com/xsts/authorize';
const MC_LOGIN = 'https://api.minecraftservices.com/authentication/login_with_xbox';
const MC_PROFILE = 'https://api.minecraftservices.com/minecraft/profile';

/** Time a Minecraft access token is considered valid before refresh (safety margin). */
const TOKEN_SKEW_MS = 5 * 60 * 1000;

const FETCH_TIMEOUT_MS = 15_000;

/** Disk cache for Minecraft access tokens — lets a launch survive a transient
 *  MSA/XBL outage by reusing a still-valid token from the previous successful
 *  launch, mirroring Lunar's accessTokenExpiresAt reuse. File is
 *  `<dataDir>/mc-token-cache.json` (gitignored, per-user). Tokens are ~24 h;
 *  the file is tiny (<1 KiB per account). */
function mcTokenCachePath() {
  try { return path.join(dataDir(), 'mc-token-cache.json'); } catch { return null; }
}
function readMcTokenCache() {
  try {
    const p = mcTokenCachePath();
    if (!p || !fs.existsSync(p)) return {};
    return JSON.parse(fs.readFileSync(p, 'utf8'));
  } catch { return {}; }
}
function writeMcTokenCache(cache) {
  try {
    const p = mcTokenCachePath();
    if (!p) return;
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, JSON.stringify(cache, null, 2), 'utf8');
  } catch { /* best-effort */ }
}
function getCachedMcToken(uuid) {
  if (!uuid) return null;
  const cache = readMcTokenCache();
  const entry = cache[uuid] ?? cache[uuid.replace(/-/g, '')] ?? null;
  if (!entry?.accessToken || !entry?.expiresAt) return null;
  return entry;
}
function setCachedMcToken(uuid, accessToken, expiresAt) {
  if (!uuid || !accessToken || !expiresAt) return;
  const cache = readMcTokenCache();
  cache[uuid] = { accessToken, expiresAt };
  // Also store dashless form for lookup tolerance
  cache[uuid.replace(/-/g, '')] = { accessToken, expiresAt };
  writeMcTokenCache(cache);
}

async function fetchJson(url, options = {}, timeoutMs = FETCH_TIMEOUT_MS) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const res = await fetch(url, { ...options, signal: ctrl.signal });
    const text = await res.text();
    let data = {};
    try { data = text ? JSON.parse(text) : {}; } catch { /* non-JSON */ }
    return { ok: res.ok, status: res.status, data, retryAfter: retryAfterSeconds(res) };
  } catch (err) {
    const message = err?.name === 'AbortError' ? 'request timed out' : (err?.message || 'network error');
    throw httpError(502, 'MSA_NETWORK', `Microsoft request failed: ${message}`);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Parse a Retry-After header into seconds (RFC 7231: delta-seconds or an
 * HTTP-date). Returns null when absent/unparsable so callers can fall back to
 * their own backoff. Mojang's Minecraft auth endpoints rate-limit bursts of
 * login_with_xbox calls with 429 + this header.
 */
function retryAfterSeconds(res) {
  const raw = res?.headers?.get?.('retry-after') ?? res?.headers?.get?.('Retry-After');
  if (raw == null) return null;
  const trimmed = String(raw).trim();
  if (trimmed === '') return null;
  const secs = Number(trimmed);
  if (Number.isFinite(secs) && secs >= 0) return secs;
  const at = Date.parse(trimmed);
  if (!Number.isNaN(at)) return Math.max(0, Math.ceil((at - Date.now()) / 1000));
  return null;
}

/**
 * Rate-limit / transient-failure retry for the Minecraft auth endpoints.
 *
 * Mojang rate-limits bursts of `login_with_xbox` with HTTP 429 (+ optional
 * Retry-After). Without a retry the caller — `resolveAccount()` — swallowed the
 * error and fell back to an OFFLINE token, so the game launched looking normal
 * and every online server rejected it with "Invalid session". Retrying here
 * turns a transient blip into a real session.
 *
 * Deliberately narrow:
 *  - only 429 and 5xx (a 4xx other than 429 is a real rejection: no Game Pass,
 *    underage, bad token — retrying those just delays an honest error);
 *  - at most 2 extra attempts (3 total), capped at ~4s of waiting so a launch
 *    never stalls on auth;
 *  - honours Retry-After when the server sends a sane value, else 1s then 3s.
 */
const RETRYABLE_STATUS = new Set([429, 500, 502, 503, 504]);
const RETRY_FALLBACK_DELAYS_MS = [1000, 3000];
/** Ignore absurd Retry-After values rather than hanging a launch on them. */
const MAX_RETRY_AFTER_MS = 4000;

/** Indirection so tests can mock waiting (mirrors launch.mjs's childProcess). */
export const timers = {
  sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
};

async function fetchJsonRetry(url, options = {}) {
  let last = null;
  for (let attempt = 0; attempt <= RETRY_FALLBACK_DELAYS_MS.length; attempt++) {
    last = await fetchJson(url, options);
    if (!RETRYABLE_STATUS.has(last.status)) return last;
    if (attempt === RETRY_FALLBACK_DELAYS_MS.length) return last;
    const hinted = last.retryAfter != null ? last.retryAfter * 1000 : null;
    const waitMs = hinted != null && hinted >= 0 && hinted <= MAX_RETRY_AFTER_MS
      ? hinted
      : RETRY_FALLBACK_DELAYS_MS[attempt];
    await timers.sleep(waitMs);
  }
  return last;
}

/**
 * Short, token-free diagnostic of a failed auth-API response body: the
 * errorMessage/error/detail/path fields Mojang and Microsoft actually return.
 * errorMessage comes first — Mojang's Minecraft Services errors carry the
 * diagnosable reason there (error is just the generic short code). raw covers
 * non-JSON rejection bodies (e.g. a bare 405 page) via parseSkinBody. Never
 * includes raw tokens — bodies at these endpoints carry error descriptors,
 * not credentials.
 */
function bodySnippet(data) {
  if (!data || typeof data !== 'object') return '';
  const interesting = data.errorMessage ?? data.error ?? data.detail ?? data.path ?? data.raw;
  return interesting ? `: ${String(interesting).slice(0, 140)}` : '';
}

/** Start a device-code login. Returns the user-facing code + URL + the flow id. */
export async function startDeviceLogin() {
  const form = new URLSearchParams({
    client_id: clientId(),
    scope: 'XboxLive.signin offline_access',
  });
  const { ok, data, status } = await fetchJson(`${AUTH_BASE}/devicecode`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: form.toString(),
  });
  if (!ok || !data.device_code) {
    // The retired shared ID (and a missing config) fails with AADSTS700016.
    // Anything else is a real Microsoft rejection — surface it so a typo'd or
    // unapproved client ID is diagnosable instead of hidden behind a generic
    // "not configured".
    const usingDefault = clientId() === DEFAULT_CLIENT_ID;
    const isAppNotFound =
      data.error === 'unauthorized_client' &&
      typeof data.error_description === 'string' &&
      data.error_description.includes('700016');
    const message = usingDefault && isAppNotFound
      ? 'Microsoft login is not configured yet. Create an Azure app (see the docs) and set msa_client_id in config.json, or set ESPECTRAL_MSA_CLIENT_ID.'
      : `could not start device login (HTTP ${status}${data.error ? `: ${data.error}${data.error_description ? ` — ${data.error_description}` : ''}` : ''})`;
    throw httpError(502, 'MSA_DEVICE_FAILED', message);
  }
  return {
    flow_id: data.device_code,
    user_code: data.user_code,
    verification_uri: data.verification_uri,
    expires_in: data.expires_in,
    interval: data.interval ?? 5,
  };
}

/**
 * Single-shot device-code token check (the UI drives the retry cadence by
 * polling /poll every few seconds). Resolves with { access_token,
 * refresh_token } when the user has signed in; throws a friendly error
 * otherwise (MSA_PENDING keeps the flow alive; MSA_SLOW_DOWN carries
 * err.retry_after — Microsoft's suggested wait — and MSA_TRANSIENT covers
 * RFC 8628 temporarily_unavailable/server_error, both of which the route
 * turns into back-off-and-continue instead of aborting the sign-in).
 */
export async function checkDeviceToken(deviceCode) {
  const form = new URLSearchParams({
    grant_type: 'urn:ietf:params:oauth:grant-type:device_code',
    device_code: deviceCode,
    client_id: clientId(),
  });
  const { ok, data } = await fetchJson(`${AUTH_BASE}/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: form.toString(),
  });
  if (ok && data.access_token) {
    return { access_token: data.access_token, refresh_token: data.refresh_token };
  }
  if (data.error === 'authorization_pending') {
    throw httpError(202, 'MSA_PENDING', 'waiting for you to sign in');
  }
  if (data.error === 'authorization_declined') {
    throw httpError(400, 'MSA_DECLINED', 'sign-in was declined in the browser');
  }
  if (data.error === 'expired_token') {
    throw httpError(408, 'MSA_EXPIRED', 'device code expired — start over');
  }
  if (data.error === 'slow_down') {
    // Microsoft asks us to slow down and tells us how long to wait. Carry the
    // suggested wait on the error so the route/UI can back off and keep the
    // flow alive instead of aborting the sign-in.
    const retryAfter = Number.isFinite(data.retry_after) && data.retry_after > 0 ? data.retry_after : 5;
    const err = httpError(429, 'MSA_SLOW_DOWN', `polling too fast — retry in ${retryAfter}s`);
    err.retry_after = retryAfter;
    throw err;
  }
  if (data.error === 'temporarily_unavailable' || data.error === 'server_error') {
    // RFC 8628 transient errors: Microsoft is temporarily unavailable. The
    // route backs off and keeps polling rather than killing the sign-in.
    throw httpError(503, 'MSA_TRANSIENT', 'Microsoft is temporarily unavailable — will retry');
  }
  throw httpError(502, 'MSA_POLL_FAILED', `token poll failed: ${data.error ?? 'unknown'}`);
}

/** Legacy: loop until the user signs in (used by CLI/direct callers). */
export async function pollDeviceToken(deviceCode, { interval = 5, onStatus } = {}) {
  for (;;) {
    if (onStatus) onStatus({ status: 'poll' });
    try {
      return await checkDeviceToken(deviceCode);
    } catch (err) {
      if (err?.code === 'MSA_PENDING') {
        await new Promise((r) => setTimeout(r, (interval || 5) * 1000));
        continue;
      }
      if (err?.code === 'MSA_SLOW_DOWN') {
        // Prefer Microsoft's suggested wait when present; otherwise keep
        // growing the interval like before.
        interval = err.retry_after ?? (interval || 5) + 5;
        await new Promise((r) => setTimeout(r, interval * 1000));
        continue;
      }
      if (err?.code === 'MSA_TRANSIENT') {
        interval = (interval || 5) + 5;
        await new Promise((r) => setTimeout(r, interval * 1000));
        continue;
      }
      throw err;
    }
  }
}

/** Exchange an MSA access token for a Minecraft access token + profile. */
export async function exchangeForMinecraft(msaAccessToken) {
  // 1. XBL
  const xbl = await fetchJson(XBL_AUTH, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({
      Properties: { AuthMethod: 'RPS', SiteName: 'user.auth.xboxlive.com', RpsTicket: 'd=' + msaAccessToken },
      RelyingParty: 'http://auth.xboxlive.com',
      TokenType: 'JWT',
    }),
  });
  if (!xbl.ok || !xbl.data.Token) {
    throw httpError(502, 'MSA_XBL_FAILED', 'Xbox Live authentication failed');
  }

  // 2. XSTS
  const xsts = await fetchJson(XSTS_AUTH, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({
      Properties: { SandboxId: 'RETAIL', UserTokens: [xbl.data.Token] },
      RelyingParty: 'rp://api.minecraftservices.com/',
      TokenType: 'JWT',
    }),
  });
  if (!xsts.ok || !xsts.data.Token) {
    const err = xsts.data?.XErr;
    if (err === 2148916233) throw httpError(403, 'MSA_NO_GAMEPASS', 'this Microsoft account does not own Minecraft');
    if (err === 2148916235) throw httpError(403, 'MSA_UNDERAGE', 'Xbox profile is under 18 — parental consent required');
    throw httpError(502, 'MSA_XSTS_FAILED', `Xbox security token failed (XErr ${err ?? 'unknown'})`);
  }
  const uhs = xsts.data.DisplayClaims?.xui?.[0]?.uhs;
  if (!uhs) throw httpError(502, 'MSA_XSTS_FAILED', 'XSTS response missing user hash');

  // 3. Minecraft login — the API returns SNAKE_CASE (access_token/expires_in);
  // reading camelCase here made every device-flow login fail with a generic
  // 'Minecraft login failed' (found via first-publish user reports).
  // fetchJsonRetry: this endpoint rate-limits bursts with 429, which would
  // otherwise surface as a launch with an offline token ("Invalid session").
  const mc = await fetchJsonRetry(MC_LOGIN, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({ identityToken: `XBL3.0 x=${uhs};${xsts.data.Token}` }),
  });
  if (!mc.ok || !mc.data.access_token) {
    throw httpError(
      502,
      'MSA_MC_LOGIN_FAILED',
      `Minecraft login failed (HTTP ${mc.status}${bodySnippet(mc.data)})`,
    );
  }

  // 4. Profile
  const pf = await fetchJson(MC_PROFILE, {
    headers: { Authorization: 'Bearer ' + mc.data.access_token },
  });
  if (!pf.ok || !pf.data.id || !pf.data.name) {
    throw httpError(502, 'MSA_PROFILE_FAILED', 'could not fetch the Minecraft profile');
  }

  return {
    accessToken: mc.data.access_token,
    refreshToken: null, // set by caller (device flow) or kept from stored account
    uuid: pf.data.id,
    username: pf.data.name,
    expiresIn: mc.data.expires_in ?? 86400,
  };
}

/* ---------------------------------------------------------------------------
 * Lunar-sourced account refresh
 *
 * Lunar Client stores a durable MSA OAuth refresh token per account in
 * `.lunarclient/settings/game/accounts.json`. The token is bound to the
 * legacy launcher client id (00000000402b5328) and the legacy
 * login.live.com token endpoint — NOT to Espectral's own Azure app — so
 * refreshing it requires the same client/endpoint/scope pair Lunar uses:
 *
 *   1. login.live.com/oauth20_token.srf
 *        client_id=00000000402b5328, scope=service::user.auth.xboxlive.com::MBI_SSL
 *        grant_type=refresh_token  -> fresh MSA access token (+ rotated refresh)
 *   2. user/authenticate  RpsTicket: "t=<msa>"   (compact ticket — "d=" is rejected)
 *   3. xsts/authorize     -> XSTS (uhs from DisplayClaims, token is a JWE)
 *   4. login_with_xbox    -> Minecraft access token
 *   5. minecraft/profile  -> { id, name }
 *
 * Imported accounts carry token_kind 'msa' with `microsoft.lunar: true` so
 * refreshMinecraftToken picks this path; the UI and launch treat them as
 * normal online accounts.
 * ------------------------------------------------------------------------- */

const LUNAR_REFRESH_URL = 'https://login.live.com/oauth20_token.srf';
const LUNAR_CLIENT_ID = '00000000402b5328'; // legacy Minecraft launcher client id
const LUNAR_SCOPE = 'service::user.auth.xboxlive.com::MBI_SSL';

/** POST to an XBL auth endpoint (contract version 1, JSON). */
async function xblPost(url, body) {
  return fetchJson(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json', 'x-xbl-contract-version': '1' },
    body: JSON.stringify(body),
  });
}

/**
 * Refresh the MSA access token for a Lunar-imported account.
 * Returns { msaAccessToken, newRefreshToken } (Microsoft rotates the refresh
 * token on every call).
 */
export async function refreshLunarMsaToken(refreshToken) {
  const form = new URLSearchParams({
    client_id: LUNAR_CLIENT_ID,
    grant_type: 'refresh_token',
    scope: LUNAR_SCOPE,
    refresh_token: refreshToken,
  });
  const { ok, data } = await fetchJson(LUNAR_REFRESH_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: form.toString(),
  });
  if (!ok || !data.access_token) {
    throw httpError(401, 'MSA_REFRESH_FAILED', 'session expired — re-import the account from Lunar Client');
  }
  return { msaAccessToken: data.access_token, newRefreshToken: data.refresh_token ?? refreshToken };
}

/**
 * Exchange a Lunar-refreshed MSA token for a Minecraft session.
 * Uses the compact-ticket ("t=") form Lunar's own flow uses.
 */
export async function exchangeLunarForMinecraft(msaAccessToken) {
  // 1. XBL — compact ticket (the "d=" form is rejected for these tokens).
  const xbl = await xblPost(XBL_AUTH, {
    Properties: { AuthMethod: 'RPS', SiteName: 'user.auth.xboxlive.com', RpsTicket: `t=${msaAccessToken}` },
    RelyingParty: 'http://auth.xboxlive.com', TokenType: 'JWT',
  });
  if (!xbl.ok || !xbl.data.Token) throw httpError(502, 'MSA_XBL_FAILED', 'Xbox Live authentication failed');

  // 2. XSTS — uhs comes from DisplayClaims (the Token itself is a JWE).
  const xsts = await xblPost(XSTS_AUTH, {
    Properties: { SandboxId: 'RETAIL', UserTokens: [xbl.data.Token] },
    RelyingParty: 'rp://api.minecraftservices.com/', TokenType: 'JWT',
  });
  if (!xsts.ok || !xsts.data.Token) {
    const err = xsts.data?.XErr;
    if (err === 2148916233) throw httpError(403, 'MSA_NO_GAMEPASS', 'this Microsoft account does not own Minecraft');
    if (err === 2148916235) throw httpError(403, 'MSA_UNDERAGE', 'Xbox profile is under 18 — parental consent required');
    throw httpError(502, 'MSA_XSTS_FAILED', `Xbox security token failed (XErr ${err ?? 'unknown'})`);
  }
  const uhs = xsts.data.DisplayClaims?.xui?.[0]?.uhs;
  if (!uhs) throw httpError(502, 'MSA_XSTS_FAILED', 'XSTS response missing user hash');

  // 3. Minecraft login. Retried: Mojang rate-limits bursts of this endpoint
  // with HTTP 429, and a swallowed failure here used to downgrade the launch
  // to an offline token (servers then reject with "Invalid session").
  const mc = await fetchJsonRetry(MC_LOGIN, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({ identityToken: `XBL3.0 x=${uhs};${xsts.data.Token}` }),
  });
  if (!mc.ok || !mc.data.access_token) {
    throw httpError(
      502,
      'MSA_MC_LOGIN_FAILED',
      `Minecraft login failed (HTTP ${mc.status}${bodySnippet(mc.data)})`,
    );
  }

  // 4. Profile.
  const pf = await fetchJson(MC_PROFILE, { headers: { Authorization: 'Bearer ' + mc.data.access_token } });
  if (!pf.ok || !pf.data.id || !pf.data.name) throw httpError(502, 'MSA_PROFILE_FAILED', 'could not fetch the Minecraft profile');

  return {
    accessToken: mc.data.access_token,
    username: pf.data.name,
    uuid: pf.data.id,
    expiresIn: mc.data.expires_in ?? 86400,
  };
}

/**
 * Refresh a stored Microsoft account's Minecraft token using its refresh_token.
 * Returns the fresh { accessToken, expiresAt } and updates the stored refresh
 * token if Microsoft rotates it.
 */
export async function refreshMinecraftToken(account) {
  // Lunar-imported account: refresh via the legacy login.live.com endpoint the
  // token is bound to, then the compact-ticket XBL chain.
  if (account?.microsoft?.lunar && account?.microsoft?.refresh_token) {
    const { msaAccessToken, newRefreshToken } = await refreshLunarMsaToken(account.microsoft.refresh_token);
    const mc = await exchangeLunarForMinecraft(msaAccessToken);
    if (newRefreshToken !== account.microsoft.refresh_token) {
      const cfg = loadConfig();
      const stored = cfg.accounts.find((a) => a.username === account.username);
      if (stored?.microsoft) {
        stored.microsoft.refresh_token = newRefreshToken;
        saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
      }
    }
    return {
      accessToken: mc.accessToken,
      expiresAt: Date.now() + (mc.expiresIn ?? 86400) * 1000,
    };
  }
  const refresh = account?.microsoft?.refresh_token;
  if (!refresh) throw httpError(400, 'MSA_NO_REFRESH', 'account has no refresh token');
  const form = new URLSearchParams({
    grant_type: 'refresh_token',
    client_id: clientId(),
    scope: 'XboxLive.signin offline_access',
    refresh_token: refresh,
  });
  const { ok, data } = await fetchJson(`${AUTH_BASE}/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: form.toString(),
  });
  if (!ok || !data.access_token) {
    // Refresh token invalid/revoked — the user must sign in again.
    throw httpError(401, 'MSA_REFRESH_FAILED', 'session expired — sign in with Microsoft again');
  }
  const msaAccess = data.access_token;
  const newRefresh = data.refresh_token ?? refresh;
  const mc = await exchangeForMinecraft(msaAccess);

  // Persist any rotated refresh token.
  if (newRefresh !== refresh) {
    const cfg = loadConfig();
    const stored = cfg.accounts.find((a) => a.username === account.username);
    if (stored?.microsoft) {
      stored.microsoft.refresh_token = newRefresh;
      saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
    }
  }
  return {
    accessToken: mc.accessToken,
    expiresAt: Date.now() + (mc.expiresIn ?? 86400) * 1000,
  };
}

/**
 * Get a usable Minecraft access token for an MSA account, refreshing it when
 * stale or expired. Used at launch time.
 *
 * The fresh access token is cached in the account object held by the in-memory
 * config cache (loadConfig returns a live object) so a second launch in the
 * same session doesn't re-refresh. It is ALSO persisted to
 * `<dataDir>/mc-token-cache.json` so a transient MSA/XBL outage at the next
 * launch can still reuse a still-valid token from the previous successful
 * launch — mirroring Lunar's accessToken reuse (its `accessTokenExpiresAt`
 * lets it launch without network until expiry). The cache is tiny and
 * gitignored; see `mcTokenCachePath()`.
 */
export async function ensureMinecraftToken(account) {
  console.log(`[msauth] ensureMinecraftToken called for ${account?.username} uuid=${account?.uuid} kind=${account?.token_kind} lunar=${account?.microsoft?.lunar} exp=${account?.microsoft?.expires_at} hasAccess=${!!account?.microsoft?.access_token}`);
  if (!account || account.token_kind !== 'msa') {
    console.log(`[msauth] -> not msa, returning null`);
    return null;
  }
  const exp = account.microsoft?.expires_at;
  if (exp && Date.now() < exp - TOKEN_SKEW_MS && account.microsoft?.access_token) {
    console.log(`[msauth] -> memory cache hit`);
    return { accessToken: account.microsoft.access_token, expiresAt: exp };
  }
  // Disk cache check before hitting the network — lets a cold engine start
  // reuse a still-valid token from the previous launch without any HTTP.
  const uuid = account.uuid;
  const diskCached = getCachedMcToken(uuid);
  console.log(`[msauth] diskCached for ${uuid}: ${diskCached ? `hit len=${diskCached.accessToken.length} exp=${new Date(diskCached.expiresAt).toISOString()} valid=${Date.now() < diskCached.expiresAt - TOKEN_SKEW_MS}` : 'miss'} cachePath=${mcTokenCachePath()}`);
  if (diskCached && Date.now() < diskCached.expiresAt - TOKEN_SKEW_MS) {
    // Populate the in-memory cache so the next call in this session is instant.
    const cfg = loadConfig();
    const stored = cfg.accounts.find((a) => a.username === account.username);
    if (stored?.microsoft) {
      stored.microsoft.access_token = diskCached.accessToken;
      stored.microsoft.expires_at = diskCached.expiresAt;
    }
    console.log(`[msauth] -> disk cache hit, returning`);
    return { accessToken: diskCached.accessToken, expiresAt: diskCached.expiresAt };
  }
  let fresh;
  try {
    console.log(`[msauth] -> attempting refresh`);
    fresh = await refreshMinecraftToken(account);
    console.log(`[msauth] -> refresh success len=${fresh.accessToken.length}`);
  } catch (e) {
    // Transient network / 5xx / 429 at launch time should not silently
    // downgrade to an offline token (which the server rejects as "Invalid
    // session"). If we have a still-valid disk-cached MC token, reuse it
    // instead.
    const fallback = getCachedMcToken(uuid);
    console.log(`[msauth] refresh failed ${e?.code} ${e?.message} fallback=${fallback ? `hit valid=${Date.now() < fallback.expiresAt - TOKEN_SKEW_MS}` : 'miss'}`);
    if (fallback && Date.now() < fallback.expiresAt - TOKEN_SKEW_MS) {
      console.warn(`[msauth] refresh failed (${e?.code ?? 'UNKNOWN'}: ${e?.message ?? e}) — using cached Minecraft token for '${account.username}' (expires ${new Date(fallback.expiresAt).toISOString()})`);
      const cfg2 = loadConfig();
      const stored2 = cfg2.accounts.find((a) => a.username === account.username);
      if (stored2?.microsoft) {
        stored2.microsoft.access_token = fallback.accessToken;
        stored2.microsoft.expires_at = fallback.expiresAt;
      }
      return { accessToken: fallback.accessToken, expiresAt: fallback.expiresAt };
    }
    throw e;
  }
  const cfg = loadConfig();
  const stored = cfg.accounts.find((a) => a.username === account.username);
  if (stored?.microsoft) {
    stored.microsoft.access_token = fresh.accessToken;
    stored.microsoft.expires_at = fresh.expiresAt;
  }
  // Persist to disk for the next cold start.
  setCachedMcToken(uuid, fresh.accessToken, fresh.expiresAt);
  console.log(`[msauth] -> persisted to disk, returning fresh`);
  return fresh;
}

/** Window before expiry that triggers a background refresh at engine start. */
const PREWARM_WINDOW_MS = 10 * 60 * 1000;

/** In-flight prewarm promises deduped per username. */
const _prewarmInflight = new Map();

/**
 * Pre-warm the Minecraft token for the active MSA account if it expires
 * within 10 minutes. Swallows all errors (console.warn at most) and dedupes
 * concurrent calls per username.
 */
export async function prewarmMsaToken() {
  try {
    const account = getActiveAccount();
    if (!account) return;
    if (account.token_kind !== 'msa') return;
    // Determine the effective cached expiry — memory first, then disk cache.
    let expiresAt = account.microsoft?.expires_at ?? null;
    let hasToken = !!account.microsoft?.access_token;
    if (!hasToken) {
      const cached = getCachedMcToken(account.uuid);
      if (cached) {
        expiresAt = cached.expiresAt;
        hasToken = true;
      }
    }
    if (hasToken && expiresAt && expiresAt - Date.now() > PREWARM_WINDOW_MS) {
      return;
    }
    const key = account.username;
    if (_prewarmInflight.has(key)) {
      return _prewarmInflight.get(key);
    }
    const p = (async () => {
      try {
        await ensureMinecraftToken(account);
      } catch (e) {
        console.warn(`[msauth] prewarm failed for '${account.username}': ${e?.message ?? e}`);
      }
    })().finally(() => {
      _prewarmInflight.delete(key);
    });
    _prewarmInflight.set(key, p);
    return p;
  } catch (e) {
    console.warn(`[msauth] prewarm error: ${e?.message ?? e}`);
  }
}

/* ---------------------------------------------------------------------------
 * Minecraft profile skins (Skin Atelier — vanilla parity)
 *
 * Pure helpers over the Mojang write API, authenticated with the Minecraft
 * access token (Bearer), NOT the MSA token:
 *   read   GET    /minecraft/profile -> { id, name, skins[], capes[] }
 *   upload POST   /minecraft/profile/skins (multipart: variant + file; single PUT retry on 405)
 *   reset  DELETE /minecraft/profile/skins/active
 * Every helper takes an optional fetchFn (= global fetch) so tests can stub
 * the wire without touching the network. Error mapping: 401 ->
 * MSA_REFRESH_FAILED (the MC token is dead — caller re-runs
 * ensureMinecraftToken, which surfaces the same code on dead refresh),
 * 429 -> SKIN_RATE_LIMITED (carries retry_after when the server sends
 * Retry-After), any other non-2xx -> SKIN_FETCH_FAILED (reads) or
 * SKIN_UPLOAD_FAILED (writes) with an HTTP-status + body snippet. Messages
 * never include the access token.
 * ------------------------------------------------------------------------- */

const MC_SKINS = `${MC_PROFILE}/skins`;
const MC_SKINS_ACTIVE = `${MC_PROFILE}/skins/active`;

/** Read a stub-or-real fetch response into a plain object (never throws). */
async function parseSkinBody(res) {
  try {
    if (res && typeof res.json === 'function') {
      const v = await res.json();
      if (v && typeof v === 'object') return v;
      return {};
    }
  } catch {
    /* fall through to text */
  }
  try {
    const text = res && typeof res.text === 'function' ? await res.text() : '';
    if (!text) return {};
    try {
      const v = JSON.parse(text);
      return v && typeof v === 'object' ? v : {};
    } catch {
      return { raw: String(text).slice(0, 140) };
    }
  } catch {
    return {};
  }
}

function skinOk(res) {
  if (res && typeof res.ok === 'boolean') return res.ok;
  const status = res?.status;
  return Number.isInteger(status) && status >= 200 && status < 300;
}

/**
 * Map a failed skin-API response to an httpError. op is 'fetch' (reads),
 * 'upload', or 'reset' (writes).
 */
function skinErrorForStatus(op, res, data) {
  const status = Number.isInteger(res?.status) ? res.status : 500;
  if (status === 401) {
    return httpError(401, 'MSA_REFRESH_FAILED', 'session expired — sign in with Microsoft again');
  }
  if (status === 429) {
    const retryAfter = retryAfterSeconds(res);
    const err = httpError(
      429,
      'SKIN_RATE_LIMITED',
      retryAfter != null
        ? `skin rate limited — retry in ${retryAfter}s`
        : 'skin rate limited — try again later',
    );
    if (retryAfter != null) err.retry_after = retryAfter;
    return err;
  }
  const action = op === 'fetch' ? 'fetch profile skin' : op === 'upload' ? 'upload profile skin' : 'reset profile skin';
  const code = op === 'fetch' ? 'SKIN_FETCH_FAILED' : 'SKIN_UPLOAD_FAILED';
  return httpError(status, code, `could not ${action} (HTTP ${status}${bodySnippet(data)})`);
}

function skinAuthHeaders(accessToken) {
  return { Authorization: `Bearer ${accessToken}` };
}

/**
 * Read the Mojang profile for skin/cape state.
 * Returns { variant: 'classic'|'slim', has_skin, cape, skinUrl, uuid, name }.
 */
export async function getProfileSkin(accessToken, fetchFn = fetch) {
  let res;
  try {
    res = await fetchFn(MC_PROFILE, { headers: skinAuthHeaders(accessToken) });
  } catch (err) {
    throw httpError(502, 'SKIN_FETCH_FAILED', `could not fetch profile skin: ${err?.message ?? err}`);
  }
  const data = await parseSkinBody(res);
  if (!skinOk(res)) throw skinErrorForStatus('fetch', res, data);
  const skins = Array.isArray(data?.skins) ? data.skins : [];
  const active = skins.find((s) => s && s.state === 'ACTIVE' && typeof s.url === 'string' && s.url.length > 0) ?? null;
  const capes = Array.isArray(data?.capes) ? data.capes : [];
  return {
    variant: active && String(active.variant ?? '').toUpperCase() === 'SLIM' ? 'slim' : 'classic',
    has_skin: !!active,
    cape: capes.some((c) => c && c.state === 'ACTIVE'),
    skinUrl: active ? active.url : null,
    uuid: typeof data?.id === 'string' ? data.id : null,
    name: typeof data?.name === 'string' ? data.name : null,
  };
}

/**
 * Upload a new skin: multipart { variant: CLASSIC|SLIM, file }.
 * pngBuffer is validated PNG bytes (the route checks dims first).
 * Mojang's gateway rings disagree on the method: historically PUT, but at
 * least one ring answers an authenticated PUT with 405 METHOD_NOT_ALLOWED
 * while POST succeeds (observed 2026-09-05, same token + bytes). So POST
 * first, with a single PUT retry on 405 only — any other failure surfaces
 * immediately. Returns the Mojang response body.
 */
async function sendSkinUpload(fetchFn, accessToken, bytes, mapped, method) {
  const form = new FormData();
  form.append('variant', mapped);
  form.append('file', new Blob([bytes], { type: 'image/png' }), 'skin.png');
  try {
    return await fetchFn(MC_SKINS, {
      method,
      headers: skinAuthHeaders(accessToken),
      body: form,
    });
  } catch (err) {
    throw httpError(502, 'SKIN_UPLOAD_FAILED', `could not upload profile skin: ${err?.message ?? err}`);
  }
}
export async function uploadProfileSkin(accessToken, pngBuffer, variant, fetchFn = fetch) {
  const mapped = variant === 'classic' ? 'CLASSIC' : variant === 'slim' ? 'SLIM' : null;
  if (!mapped) throw httpError(400, 'BAD_SKIN_VARIANT', 'variant must be classic|slim');
  const bytes = Buffer.isBuffer(pngBuffer) ? pngBuffer : pngBuffer ? Buffer.from(pngBuffer) : Buffer.alloc(0);
  if (bytes.length === 0) throw httpError(400, 'BAD_IMAGE', 'skin image is empty');
  let res = await sendSkinUpload(fetchFn, accessToken, bytes, mapped, 'POST');
  let data = await parseSkinBody(res);
  if (!skinOk(res) && res?.status === 405) {
    console.error('[msauth] uploadProfileSkin POST -> 405, retrying once with PUT');
    res = await sendSkinUpload(fetchFn, accessToken, bytes, mapped, 'PUT');
    data = await parseSkinBody(res);
  }
  if (!skinOk(res)) {
    // server.mjs only logs >=500, so a Mojang rejection (e.g. HTTP 405)
    // would otherwise vanish without a trace — log status + snippet here.
    // bodySnippet carries error fields only, never the Bearer token.
    const status = Number.isInteger(res?.status) ? res.status : 500;
    console.error(`[msauth] uploadProfileSkin failed (HTTP ${status}${bodySnippet(data)})`);
    throw skinErrorForStatus('upload', res, data);
  }
  return data;
}

/**
 * Reset to the Steve/Alex default: DELETE .../skins/active.
 * Returns the Mojang response body (or { reset: true } when empty).
 */
export async function resetProfileSkin(accessToken, fetchFn = fetch) {
  let res;
  try {
    res = await fetchFn(MC_SKINS_ACTIVE, {
      method: 'DELETE',
      headers: skinAuthHeaders(accessToken),
    });
  } catch (err) {
    throw httpError(502, 'SKIN_UPLOAD_FAILED', `could not reset profile skin: ${err?.message ?? err}`);
  }
  const data = await parseSkinBody(res);
  if (!skinOk(res)) throw skinErrorForStatus('reset', res, data);
  return data && typeof data === 'object' && Object.keys(data).length > 0 ? data : { reset: true };
}
