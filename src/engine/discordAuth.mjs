/**
 * Discord OAuth2 authorization & session management core.
 * Implements Discord OAuth2 Authorization Code flow (RFC 6749) for the client app.
 *
 * Security guarantees:
 * - Tokens (access_token, refresh_token) are NEVER persisted to disk.
 * - Sensitive values are kept in transient memory only.
 * - Sessions use random opaque UUIDs.
 * - Identity upsert stores id, name, avatar, mc_accounts to <data>/discord-users.json.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { loadConfig, dataDir } from './config.mjs';
import { httpError } from './error.mjs';

const REDIRECT_URI = 'http://127.0.0.1:4199/api/discord/callback';
const DISCORD_OAUTH_TOKEN_URL = 'https://discord.com/api/v10/oauth2/token';
const DISCORD_USER_ME_URL = 'https://discord.com/api/v10/users/@me';
const STATE_TTL_MS = 15 * 60 * 1000; // 15 minutes
const SESSION_TTL_MS = 30 * 24 * 60 * 60 * 1000; // 30 days

/** In-memory map of active OAuth state parameters: state -> { createdAt, sessionId? } */
const states = new Map();

/** In-memory map of active authenticated sessions: sessionId -> { user: { id, name, avatar }, createdAt: number } */
const sessions = new Map();

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
 * Read Discord OAuth credentials from config document or environment overrides.
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

  const clientSecret =
    cfg.discord?.client_secret ||
    cfg['discord.client_secret'] ||
    cfg.discord_client_secret ||
    process.env.DISCORD_CLIENT_SECRET ||
    process.env.ESPECTRAL_DISCORD_CLIENT_SECRET ||
    null;

  return { clientId, clientSecret };
}

/**
 * Generate a Discord OAuth2 authorization URL with a secure random state.
 * Returns { url: string }.
 */
export function getAuthStartUrl() {
  sweepExpiredStates();
  const { clientId } = getDiscordCredentials();
  const isMock = process.env.ESPECTRAL_DISCORD_MOCK === '1' || process.env.ESPECTRAL_DISCORD_MOCK === 'true';

  const state = crypto.randomBytes(16).toString('hex');
  states.set(state, { createdAt: Date.now() });

  if (!clientId) {
    if (isMock) {
      return {
        url: `http://127.0.0.1:4199/api/discord/callback?code=mock_code&state=${state}`,
      };
    }
    throw httpError(
      400,
      'DISCORD_NOT_CONFIGURED',
      'Discord client credentials are not configured in config.json (discord.client_id / discord.client_secret)',
    );
  }

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
 * Claim the session produced by an OAuth flow the APP started (state comes
 * from getAuthStartUrl). The authorize URL normally completes in the SYSTEM
 * browser (the Tauri webview blocks window.open), whose cookie jar is not the
 * webview's — so /api/discord/me polling can't see the session cookie. The
 * callback re-attaches the sessionId to the state entry and the app claims it
 * here. One-time: a successful claim deletes the state.
 */
export function claimSessionUser(state) {
  sweepExpiredStates();
  if (!state || !states.has(state)) return { status: 'invalid' };
  const entry = states.get(state);
  if (!entry.sessionId) return { status: 'pending' };
  states.delete(state);
  const session = sessions.get(entry.sessionId);
  if (!session) return { status: 'invalid' };
  return { status: 'ok', sessionId: entry.sessionId, user: session.user };
}

/**
 * Handle the OAuth2 callback from Discord:
 * 1. Validate the state parameter.
 * 2. Exchange authorization code for an in-memory access token.
 * 3. Fetch user profile from Discord API.
 * 4. Upsert user identity in <data>/discord-users.json.
 * 5. Create an in-memory session and return { sessionId, user }.
 */
export async function handleCallback(query = {}) {
  const isMock = process.env.ESPECTRAL_DISCORD_MOCK === '1' || process.env.ESPECTRAL_DISCORD_MOCK === 'true';
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

  if (!state || !states.has(state)) {
    if (!isMock) {
      throw httpError(400, 'INVALID_STATE', 'OAuth state is invalid or expired. Please try logging in again.');
    }
  }
  states.delete(state);

  let user;
  if (isMock && (code.startsWith('mock_') || !getDiscordCredentials().clientId)) {
    user = {
      id: '718293847561829304',
      name: 'TestPlayer',
      avatar: 'https://cdn.discordapp.com/embed/avatars/0.png',
    };
  } else {
    const { clientId, clientSecret } = getDiscordCredentials();
    if (!clientId || !clientSecret) {
      throw httpError(
        400,
        'DISCORD_NOT_CONFIGURED',
        'Discord credentials missing (discord.client_id / discord.client_secret) in config.json',
      );
    }

    // Exchange code for access token (never persist this token)
    const tokenRes = await fetch(DISCORD_OAUTH_TOKEN_URL, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/x-www-form-urlencoded',
      },
      body: new URLSearchParams({
        client_id: clientId,
        client_secret: clientSecret,
        grant_type: 'authorization_code',
        code,
        redirect_uri: REDIRECT_URI,
      }),
    });

    if (!tokenRes.ok) {
      const errText = await tokenRes.text();
      throw httpError(400, 'TOKEN_EXCHANGE_FAILED', `Failed to exchange Discord authorization code: ${errText}`);
    }

    const tokenData = await tokenRes.json();
    const accessToken = tokenData.access_token;
    if (!accessToken) {
      throw httpError(400, 'NO_ACCESS_TOKEN', 'Discord response did not contain an access token');
    }

    // Fetch user profile with in-memory access token
    const userRes = await fetch(DISCORD_USER_ME_URL, {
      headers: {
        Authorization: `Bearer ${accessToken}`,
      },
    });

    if (!userRes.ok) {
      throw httpError(400, 'USER_FETCH_FAILED', 'Failed to fetch Discord user profile (/users/@me)');
    }

    const rawUser = await userRes.json();
    let avatarUrl = null;
    if (rawUser.avatar) {
      avatarUrl = `https://cdn.discordapp.com/avatars/${rawUser.id}/${rawUser.avatar}.png?size=128`;
    } else {
      const defaultIndex = (BigInt(rawUser.id || '0') >> 22n) % 6n;
      avatarUrl = `https://cdn.discordapp.com/embed/avatars/${defaultIndex}.png`;
    }

    user = {
      id: rawUser.id,
      name: rawUser.global_name || rawUser.username || `User#${rawUser.discriminator || '0000'}`,
      avatar: avatarUrl,
    };
  }

  // Upsert user identity in discord-users.json (no tokens saved)
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
  if (state) {
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
  const isMockEnv = process.env.ESPECTRAL_DISCORD_MOCK === '1' || process.env.ESPECTRAL_DISCORD_MOCK === 'true';
  if (mockRequested && isMockEnv) {
    return {
      id: '718293847561829304',
      name: 'TestPlayer',
      avatar: 'https://cdn.discordapp.com/embed/avatars/0.png',
    };
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
