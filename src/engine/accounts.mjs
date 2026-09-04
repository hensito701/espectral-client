/**
 * Offline accounts (v0.1.0: offline mode only, no MSA).
 *
 * Offline UUID derivation (Java-compatible):
 *   md5 = MD5('OfflinePlayer:' + username)            (UTF-8)
 *   md5[6] = (md5[6] & 0x0f) | 0x30                   -> version 3 (name-based)
 *   md5[8] = (md5[8] & 0x3f) | 0x80                   -> RFC 4122 variant
 *   uuid  = canonical 8-4-4-4-12 hex of the munged bytes
 *
 * Accounts are persisted inside data/config.json (engine-owned section),
 * alongside the AppConfig document.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { dataDir, loadConfig, saveConfig } from './config.mjs';
import { httpError } from './error.mjs';

const USERNAME_RE = /^[A-Za-z0-9_]{1,16}$/;

/** Avatar color is an integer on the 0-359 wheel, or null when unset. */
export function isValidAvatarColor(c) {
  return Number.isInteger(c) && c >= 0 && c <= 359;
}

/** Circular distance between two hues on the 0-359 wheel. */
function hueDistance(a, b) {
  const d = Math.abs(a - b) % 360;
  return Math.min(d, 360 - d);
}

/**
 * Random avatar color >= 30 degrees (circular) from every color in use.
 * 50 tries, then accept anything.
 */
function pickAvatarColor(accounts) {
  const taken = (Array.isArray(accounts) ? accounts : [])
    .map((a) => a?.avatar_color)
    .filter(isValidAvatarColor);
  for (let i = 0; i < 50; i += 1) {
    const h = Math.floor(Math.random() * 360);
    if (taken.every((t) => hueDistance(h, t) >= 30)) return h;
  }
  return Math.floor(Math.random() * 360);
}

/** Avatar images live at <dataDir>/avatars/<uuid>.png. */
export function avatarsDir() {
  return path.join(dataDir(), 'avatars');
}

export function avatarPathForUuid(uuid) {
  return path.join(avatarsDir(), `${uuid}.png`);
}

/** True when the account has an avatar image on disk. Never throws. */
export function hasAvatar(a) {
  try {
    const uuid = typeof a === 'string' ? a : a?.uuid;
    if (typeof uuid !== 'string' || uuid.length === 0) return false;
    return fs.existsSync(avatarPathForUuid(uuid));
  } catch {
    return false;
  }
}

export function isValidUsername(username) {
  return typeof username === 'string' && USERNAME_RE.test(username);
}

/** Java-compatible offline UUID for a username. */
export function offlineUuid(username) {
  const bytes = crypto.createHash('md5').update('OfflinePlayer:' + username, 'utf8').digest();
  bytes[6] = (bytes[6] & 0x0f) | 0x30;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = bytes.toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export function listAccounts() {
  return [...(loadConfig().accounts ?? [])];
}

/**
 * Public API shape for an account (H2): strips the durable refresh token and
 * any in-memory access/XBL material; keeps identity fields (username, uuid,
 * token_kind, last_used, created_at) plus the lunar badge for the UI.
 */
export function publicAccount(a) {
  const base =
    a.token_kind !== 'msa' || !a.microsoft || typeof a.microsoft !== 'object'
      ? (() => {
          const { microsoft, ...rest } = a;
          return rest;
        })()
      : { ...a, microsoft: { lunar: a.microsoft.lunar === true } };
  return { ...base, avatar_color: a.avatar_color ?? null, has_avatar: hasAvatar(a) };
}

export function getAccount(username) {
  return listAccounts().find((a) => a.username === username) ?? null;
}

export function getActiveAccount() {
  const username = loadConfig().active_username;
  return username ? getAccount(username) : null;
}

export function createAccount(username) {
  if (!isValidUsername(username)) {
    throw httpError(400, 'INVALID_USERNAME', 'username must be 1-16 characters of [A-Za-z0-9_]');
  }
  const cfg = loadConfig();
  if (cfg.accounts.some((a) => a.username === username)) {
    throw httpError(409, 'ALREADY_EXISTS', `account '${username}' already exists`);
  }
  const now = new Date().toISOString();
  const account = {
    username,
    uuid: offlineUuid(username),
    token_kind: 'offline',
    avatar_color: pickAvatarColor(cfg.accounts),
    created_at: now,
    last_used: now,
  };
  cfg.accounts.push(account);
  saveConfig({ accounts: cfg.accounts, active_username: username });
  return account;
}

/**
 * Add (or replace) a Microsoft account from a completed device login.
 * `ms` = { username, uuid, refreshToken, accessToken, expiresIn }.
 * Re-logging into the same Microsoft account updates the existing entry.
 */
export function upsertMsaAccount(ms) {
  const cfg = loadConfig();
  const now = new Date().toISOString();
  const existing = cfg.accounts.find((a) => a.token_kind === 'msa' && a.uuid === ms.uuid);
  const account = {
    username: ms.username,
    uuid: ms.uuid,
    token_kind: 'msa',
    avatar_color: isValidAvatarColor(existing?.avatar_color)
      ? existing.avatar_color
      : pickAvatarColor(cfg.accounts),
    microsoft: {
      refresh_token: ms.refreshToken,
      // access token cached in memory only — NOT persisted (see msauth.mjs)
      expires_at: Date.now() + (ms.expiresIn ?? 86400) * 1000,
    },
    created_at: existing?.created_at ?? now,
    updated_at: now,
    // last_used is set so the UI's active derivation (max last_used) matches
    // the engine's active_username — see createAccount/setActiveAccount.
    last_used: now,
  };
  if (existing) {
    const idx = cfg.accounts.indexOf(existing);
    cfg.accounts[idx] = account;
  } else {
    cfg.accounts.push(account);
  }
  cfg.active_username = account.username;
  saveConfig({ accounts: cfg.accounts, active_username: account.username });
  return account;
}

/**
 * Add (or replace) a Lunar-sourced Microsoft account — imported from Lunar
 * Client's `accounts.json` (the durable MSA OAuth refresh token it stores per
 * account). `lr` = { username, uuid, refreshToken }.
 * token_kind stays 'msa' so the UI badge, logout, and launch path treat it
 * like a normal online account; msauth.mjs refreshes it via the legacy
 * login.live.com endpoint the token is bound to (microsoft.lunar = true).
 */
export function upsertLunarAccount(lr) {
  const cfg = loadConfig();
  const now = new Date().toISOString();
  const existing = cfg.accounts.find((a) => a.token_kind === 'msa' && a.uuid === lr.uuid);
  const account = {
    username: lr.username,
    uuid: lr.uuid,
    token_kind: 'msa',
    avatar_color: isValidAvatarColor(existing?.avatar_color)
      ? existing.avatar_color
      : pickAvatarColor(cfg.accounts),
    microsoft: {
      refresh_token: lr.refreshToken,
      lunar: true,
      // access token cached in memory only — NOT persisted (see msauth.mjs)
      expires_at: 0,
    },
    created_at: existing?.created_at ?? now,
    updated_at: now,
    // Same as upsertMsaAccount: mark as used so the UI's active highlight
    // follows active_username (the launch will use this identity).
    last_used: now,
  };
  if (existing) {
    const idx = cfg.accounts.indexOf(existing);
    cfg.accounts[idx] = account;
  } else {
    cfg.accounts.push(account);
  }
  cfg.active_username = account.username;
  saveConfig({ accounts: cfg.accounts, active_username: account.username });
  return account;
}

/** Remove a Microsoft account (and any stored refresh token). */
export function deleteMsaAccount(username) {
  const cfg = loadConfig();
  const idx = cfg.accounts.findIndex((a) => a.username === username && a.token_kind === 'msa');
  if (idx === -1) {
    throw httpError(404, 'NOT_FOUND', `Microsoft account '${username}' does not exist`);
  }
  const [removed] = cfg.accounts.splice(idx, 1);
  if (cfg.active_username === username) cfg.active_username = null;
  saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
  if (removed?.uuid) removeAvatarFile(removed.uuid);
  return { deleted: username };
}

export function setActiveAccount(username) {
  if (!getAccount(username)) {
    throw httpError(404, 'NOT_FOUND', `account '${username}' does not exist`);
  }
  const cfg = loadConfig();
  const account = cfg.accounts.find((a) => a.username === username);
  account.last_used = new Date().toISOString();
  cfg.active_username = username;
  saveConfig({ accounts: cfg.accounts, active_username: username });
  return { active: username };
}

export function deleteAccount(username) {
  const cfg = loadConfig();
  const idx = cfg.accounts.findIndex((a) => a.username === username);
  if (idx === -1) {
    throw httpError(404, 'NOT_FOUND', `account '${username}' does not exist`);
  }

  const [removed] = cfg.accounts.splice(idx, 1);
  if (cfg.active_username === username) cfg.active_username = null;
  saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
  if (removed?.uuid) removeAvatarFile(removed.uuid);
  return { deleted: username };
}

// ---------------------------------------------------------------------------
// Avatar color + image (<dataDir>/avatars/<uuid>.png)
// ---------------------------------------------------------------------------

/** Set an account's avatar color (int 0-359 | null to clear). 404 when missing. */
export function setAvatarColor(username, color) {
  if (color !== null && !isValidAvatarColor(color)) {
    throw httpError(400, 'BAD_AVATAR_COLOR', 'avatar_color must be an integer between 0 and 359 or null');
  }
  const cfg = loadConfig();
  const account = cfg.accounts.find((a) => a.username === username);
  if (!account) {
    throw httpError(404, 'NOT_FOUND', `account '${username}' does not exist`);
  }
  account.avatar_color = color;
  saveConfig({ accounts: cfg.accounts });
  return publicAccount(account);
}

/** Avatar image path for an account (by username). 404 when missing. */
export function avatarPathFor(username) {
  const account = getAccount(username);
  if (!account) {
    throw httpError(404, 'NOT_FOUND', `account '${username}' does not exist`);
  }
  return avatarPathForUuid(account.uuid);
}

/** Read an account avatar (PNG bytes). 404 when the account or image is missing. */
export async function readAccountAvatar(username) {
  const p = avatarPathFor(username); // 404 when missing
  try {
    return await fs.promises.readFile(p);
  } catch {
    throw httpError(404, 'NOT_FOUND', `account '${username}' has no avatar`);
  }
}

/** Store an account avatar (validated PNG bytes; mkdir -p the avatars dir). */
export async function writeAccountAvatar(username, buffer) {
  const p = avatarPathFor(username); // 404 when missing
  await fs.promises.mkdir(avatarsDir(), { recursive: true });
  await fs.promises.writeFile(p, buffer);
  return { ok: true, has_avatar: true };
}

/** Remove an account avatar (missing file is a no-op). */
export async function removeAccountAvatar(username) {
  const p = avatarPathFor(username); // 404 when missing
  try {
    await fs.promises.unlink(p);
  } catch {
    /* already absent */
  }
  return { ok: true, has_avatar: false };
}

/** Best-effort avatar file cleanup when an account goes away. */
function removeAvatarFile(uuid) {
  try {
    fs.rmSync(avatarPathForUuid(uuid), { force: true });
  } catch {
    /* ignore */
  }
}
