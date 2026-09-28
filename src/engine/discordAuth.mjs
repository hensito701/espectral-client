/**
 * Discord OAuth2 authorization & session management core.
 * Implements Discord OAuth2 Authorization Code flow (RFC 6749) for the client app.
 *
 * The authorization-code exchange happens SERVER-SIDE on espectral-id since
 * Workstream 4 — the launcher holds only the public client_id (the client
 * secret never ships in the app). Plain logins forward the code to
 * /v1/discord/identify; callbacks whose state was minted by
 * /api/identity/:uuid/discord/start forward to /v1/discord/link and bind the
 * Discord identity to that verified Minecraft uuid.
 *
 * Security guarantees:
 * - OAuth artifacts are NEVER persisted to disk.
 * - Sessions use random opaque UUIDs.
 * - Identity upsert stores id, name, avatar, mc_accounts to <data>/discord-users.json.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { loadConfig, dataDir } from './config.mjs';
import * as identity from './identity.mjs';
import { httpError } from './error.mjs';

export const REDIRECT_URI = 'http://127.0.0.1:4199/api/discord/callback';
const STATE_TTL_MS = 15 * 60 * 1000; // 15 minutes
const SESSION_TTL_MS = 30 * 24 * 60 * 60 * 1000; // 30 days

/** In-memory map of active OAuth state parameters: state -> { createdAt, linkUuid?, sessionId?, linked?, user?, processing? } */
const states = new Map();

/** In-memory map of active authenticated sessions: sessionId -> { user: { id, name, avatar }, createdAt: number } */
const sessions = new Map();

const MOCK_USER = {
  id: '718293847561829304',
  name: 'TestPlayer',
  avatar: 'https://cdn.discordapp.com/embed/avatars/0.png',
};

function isMockMode() {
  return process.env.ESPECTRAL_DISCORD_MOCK === '1' || process.env.ESPECTRAL_DISCORD_MOCK === 'true';
}

/**
 * Sweep expired OAuth states from memory.
 */
function sweepExpiredStates() {
  const now = Date.now();
  for (const [state, meta] of states.entries()) {
    if (now - meta.createdAt > STATE_TTL_MS) {
      states.delete(state);
    }
  }
}

/**
 * Read the Discord OAuth client id from config or environment overrides.
 * The client secret is intentionally absent — the code exchange lives on
 * espectral-id; the launcher only knows the public app id.
 */
export function getDiscordCredentials() {
  const cfg = loadConfig();
  const clientId =
    cfg.discord?.client_id ||
    cfg['discord.client_id'] ||
    cfg.discord_client_id ||
    process.env.DISCORD_CLIENT_ID ||
    process.env.ESPECTRAL_DISCORD_CLIENT_ID ||
    null;
  return { clientId };
}

/**
 * Generate a Discord OAuth2 authorization URL with a secure random state.
 * `opts.linkUuid` (flat 32-hex) marks the state as an identity link: the
 * callback then binds Discord to that uuid via the service instead of
 * creating a plain launcher session.
 * Returns { url, state }.
 */
export function getAuthStartUrl({ linkUuid = null } = {}) {
  sweepExpiredStates();
  const { clientId } = getDiscordCredentials();

  const state = crypto.randomBytes(16).toString('hex');

  if (!clientId) {
    if (isMockMode()) {
      states.set(state, { createdAt: Date.now(), linkUuid: linkUuid ?? null });
      return {
        url: `http://127.0.0.1:4199/api/discord/callback?code=mock_code&state=${state}`,
        state,
      };
    }
    throw httpError(
      400,
      'DISCORD_NOT_CONFIGURED',
      'Discord client id is not configured in config.json (discord.client_id)',
    );
  }

  // Register the state only on the success path — a failed start must not
  // leave a dangling claimable entry behind.
  states.set(state, { createdAt: Date.now(), linkUuid: linkUuid ?? null });

  const params = new URLSearchParams({
    client_id: clientId,
    response_type: 'code',
    redirect_uri: REDIRECT_URI,
    scope: 'identify',
    state,
    prompt: 'consent',
  });

  return {
    url: `https://discord.com/oauth2/authorize?${params.toString()}`,
    state,
  };
}

/**
 * Claim the result of an OAuth flow the APP started (state comes
 * from getAuthStartUrl). The authorize URL normally completes in the SYSTEM
 * browser (the Tauri webview blocks window.open), whose cookie jar is not the
 * webview's — so /api/discord/me polling can't see the session cookie. The
 * callback re-attaches the outcome to the state entry and the app claims it
 * here. One-time: a successful claim deletes the state.
 *
 * Plain login  -> { status:'ok', sessionId, user }
 * Identity link -> { status:'ok', linked:true, uuid, user }
 */
export function claimSessionUser(state) {
  sweepExpiredStates();
  if (!state || !states.has(state)) return { status: 'invalid' };
  const entry = states.get(state);
  if (entry.sessionId) {
    states.delete(state);
    const session = sessions.get(entry.sessionId);
    if (!session) return { status: 'invalid' };
    return { status: 'ok', sessionId: entry.sessionId, user: session.user };
  }
  if (entry.linked) {
    states.delete(state);
    return { status: 'ok', linked: true, uuid: entry.linkUuid, user: entry.user ?? null };
  }
  return { status: 'pending' };
}

/**
 * Handle the OAuth2 callback from Discord:
 * 1. Validate the state parameter.
 * 2. Forward the code to espectral-id — /v1/discord/link when the state is an
 *    identity link for a uuid (binds discord id + recomputes flags), else
 *    /v1/discord/identify for a plain launcher login.
 * 3. Plain logins additionally upsert discord-users.json and create an
 *    in-memory launcher session; links store the identity on the uuid entry.
 *
 * Returns { sessionId|null, user, linked? } — sessionId is null for links.
 */
export async function handleCallback(query = {}) {
  const isMock = isMockMode();
  const { code, state, error, error_description } = query;

  if (error) {
    throw httpError(
      400,
      'DISCORD_OAUTH_FAILED',
      error_description || error || 'Discord authorization was cancelled or denied',
    );
  }

  if (!code) {
    throw httpError(400, 'MISSING_CODE', 'No authorization code received from Discord');
  }

  const stateEntry = state ? states.get(state) : null;
  if (!state || !stateEntry) {
    if (!isMock) {
      throw httpError(400, 'INVALID_STATE', 'OAuth state is invalid or expired. Please try logging in again.');
    }
  } else if (stateEntry.processing || stateEntry.sessionId || stateEntry.linked) {
    // Replay protection: a state already consumed (result attached) or with an
    // exchange in flight must not run a second code exchange — a duplicate
    // callback would otherwise wipe the completed entry before the app claims
    // it (or leak two Discord-side exchanges for one consent).
    throw httpError(400, 'INVALID_STATE', 'OAuth state was already used. Please try logging in again.');
  } else {
    // Keep the entry while the async exchange runs: a /api/discord/claim poll
    // mid-flight must answer 'pending', not 'invalid' (deleting here produced
    // exactly that race). Processing states read as pending in claimSessionUser.
    stateEntry.processing = true;
  }

  const linkUuid = stateEntry?.linkUuid ? identity.normalizeUuid(stateEntry.linkUuid) : null;
  let user;
  try {
    if (isMock && (String(code).startsWith('mock_') || !getDiscordCredentials().clientId)) {
      user = { ...MOCK_USER };
      if (linkUuid) {
        identity.updateEntry(linkUuid, { discord: user });
        await identity.refreshGrant(linkUuid).catch(() => {});
      }
    } else if (linkUuid) {
      // Identity link: the service exchanges the code AND binds the discord id
      // to the session uuid, recomputing flags server-side.
      user = await identity.discordLink(linkUuid, code, REDIRECT_URI);
    } else {
      user = await identity.discordIdentify(code, REDIRECT_URI);
    }
  } catch (err) {
    // The flow is dead — the user restarts from /auth/start. Drop the state so
    // the app's next claim sees 'invalid' instead of 'pending' forever.
    states.delete(state);
    throw err;
  }

  if (linkUuid) {
    // Re-attach the link result to the state so the waiting app can claim it.
    if (state && (stateEntry || isMock)) {
      states.set(state, { createdAt: Date.now(), linkUuid, linked: true, user });
    }
    return { sessionId: null, user, linked: true };
  }

  // Plain login path: upsert identity in discord-users.json (no tokens saved)
  upsertDiscordUser(user);

  // Generate an opaque session id
  const sessionId = crypto.randomUUID();
  sessions.set(sessionId, {
    user,
    createdAt: Date.now(),
  });

  // Re-attach the session to the OAuth state so the waiting app can claim it
  // (see claimSessionUser) — the callback usually completes in the system
  // browser, whose cookie jar is invisible to the Tauri webview.
  if (state && (stateEntry || isMock)) {
    states.set(state, { createdAt: Date.now(), sessionId });
  }

  return { sessionId, user };
}

/**
 * Upsert user profile to <data>/discord-users.json
 * Preserves existing mc_accounts mappings.
 */
export function upsertDiscordUser(user) {
  try {
    const usersPath = path.join(dataDir(), 'discord-users.json');
    let list = [];
    try {
      if (fs.existsSync(usersPath)) {
        const raw = JSON.parse(fs.readFileSync(usersPath, 'utf8'));
        if (Array.isArray(raw)) {
          list = raw;
        } else if (typeof raw === 'object' && raw !== null) {
          list = Object.values(raw);
        }
      }
    } catch {
      list = [];
    }

    const idx = list.findIndex((u) => u && u.id === user.id);
    if (idx >= 0) {
      list[idx] = {
        id: user.id,
        name: user.name,
        avatar: user.avatar,
        mc_accounts: Array.isArray(list[idx].mc_accounts) ? list[idx].mc_accounts : [],
      };
    } else {
      list.push({
        id: user.id,
        name: user.name,
        avatar: user.avatar,
        mc_accounts: [],
      });
    }

    fs.mkdirSync(path.dirname(usersPath), { recursive: true });
    const tmp = usersPath + '.tmp';
    fs.writeFileSync(tmp, JSON.stringify(list, null, 2), 'utf8');
    fs.renameSync(tmp, usersPath);
  } catch (err) {
    console.warn('[discordAuth] could not upsert discord-users.json:', err.message);
  }
}

/**
 * Get active session user from sessionId, or mock user if ?mock=1 is passed and mock mode is active.
 * Returns { id, name, avatar } or null.
 */
export function getSessionUser(sessionId, mockRequested = false) {
  if (mockRequested && isMockMode()) {
    return { ...MOCK_USER };
  }

  if (!sessionId) return null;
  const sess = sessions.get(sessionId);
  if (!sess) return null;

  if (Date.now() - sess.createdAt > SESSION_TTL_MS) {
    sessions.delete(sessionId);
    return null;
  }

  return sess.user;
}

/**
 * Invalidate/destroy a session in memory.
 */
export function destroySession(sessionId) {
  if (sessionId) {
    sessions.delete(sessionId);
  }
}

/**
 * Extract session ID from cookie header or Authorization Bearer header.
 */
export function extractSessionId(cookieHeader, authHeader = null, customHeader = null) {
  if (customHeader && typeof customHeader === 'string' && customHeader.trim()) {
    return customHeader.trim();
  }
  if (authHeader && typeof authHeader === 'string' && authHeader.startsWith('Bearer ')) {
    return authHeader.slice(7).trim();
  }
  if (!cookieHeader || typeof cookieHeader !== 'string') return null;

  const parts = cookieHeader.split(';');
  for (const p of parts) {
    const trimmed = p.trim();
    const eqIdx = trimmed.indexOf('=');
    if (eqIdx > 0) {
      const key = trimmed.slice(0, eqIdx).trim();
      const val = trimmed.slice(eqIdx + 1).trim();
      if (key === 'espectral_session' || key === 'session') {
        try {
          return decodeURIComponent(val);
        } catch {
          return val;
        }
      }
    }
  }
  return null;
}

/**
 * Create a Set-Cookie header string for an opaque session ID.
 */
export function createSessionCookie(sessionId) {
  return `espectral_session=${encodeURIComponent(sessionId)}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000`;
}

/**
 * Create a Set-Cookie header string to expire/clear the session cookie.
 */
export function clearSessionCookie() {
  return `espectral_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT`;
}
