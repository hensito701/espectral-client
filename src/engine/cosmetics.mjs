/**
 * Cosmetics state + loadout validation + the mod-facing file.
 *
 * Per-uuid state lives at <dataDir>/cosmetics.json:
 *   { schema:1, accounts: { <uuid>: {
 *       share: bool,            // publish presence (⚛ + loadout) — default ON
 *       emotes: bool,           // allow local emotes — default ON
 *       loadout: {slot: itemId|null},
 *       hidden_players: [names] } } }
 *
 * On every launch the engine writes <gameDir>/config/espectral-cosmetics.json
 * (mirrored into the per-account profile gameDir like espectral-client.json)
 * and rewrites it on any PUT; the mod hot-reloads on mtime:
 *   { schema:1, uuid, grant|null, share, loadout, hidden_players, emotes }
 *
 * The loadout written to the file is SANITIZED against the currently valid
 * grant — a dead/forged grant writes null grant and an empty loadout. The
 * stored loadout (user intent) is never mutated by sanitization.
 */
import fs from 'node:fs';
import path from 'node:path';
import { dataDir } from './config.mjs';
import * as identity from './identity.mjs';
import { verifyGrant } from './grant.mjs';
import { httpError } from './error.mjs';

const STATE_SCHEMA = 1;
const MAX_HIDDEN = 200;
const PLAYER_NAME_RE = /^[A-Za-z0-9_]{1,16}$/;

let cachedCatalog = null;
let cachedState = null;

/** The catalog document (single source: src/engine/cosmetics-catalog.json). */
export function catalog() {
  if (cachedCatalog) return cachedCatalog;
  const doc = JSON.parse(
    fs.readFileSync(new URL('./cosmetics-catalog.json', import.meta.url), 'utf8'),
  );
  const items = new Map();
  for (const it of doc.items ?? []) {
    if (it && typeof it.id === 'string') items.set(it.id, it);
  }
  cachedCatalog = {
    doc,
    slots: new Set(doc.slots ?? []),
    items,
    exclusive: Array.isArray(doc.exclusive) ? doc.exclusive : [],
    emotes: new Map((doc.emotes ?? []).map((e) => [e.id, e])),
  };
  return cachedCatalog;
}

export function cosmeticsPath() {
  return path.join(dataDir(), 'cosmetics.json');
}

export function loadCosmeticsState() {
  if (cachedState) return cachedState;
  let raw = {};
  try {
    raw = JSON.parse(fs.readFileSync(cosmeticsPath(), 'utf8'));
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

function persist(state) {
  try {
    fs.mkdirSync(path.dirname(cosmeticsPath()), { recursive: true });
    const tmp = cosmeticsPath() + '.tmp';
    fs.writeFileSync(tmp, JSON.stringify(state, null, 2), 'utf8');
    fs.renameSync(tmp, cosmeticsPath());
  } catch (e) {
    console.warn('[cosmetics] could not persist cosmetics.json:', e.message);
  }
}

function emptyAccountState() {
  // Sharing defaults ON (product decision 2026-10-24). Presence still only
  // publishes under a verified identity session (presence.mjs token gate).
  return { share: true, emotes: true, loadout: {}, hidden_players: [] };
}

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

/**
 * Normalize a stored record into the public account state. Unknown keys are
 * tolerated on read but never returned.
 */
export function getCosmeticsState(uuid) {
  const flat = identity.normalizeUuid(uuid);
  if (!flat) return emptyAccountState();
  const raw = loadCosmeticsState().accounts[flat];
  const out = emptyAccountState();
  if (isPlainObject(raw)) {
    if (typeof raw.share === 'boolean') out.share = raw.share;
    if (typeof raw.emotes === 'boolean') out.emotes = raw.emotes;
    if (isPlainObject(raw.loadout)) {
      for (const slot of catalog().slots) {
        const v = raw.loadout[slot];
        if (typeof v === 'string' && v) out.loadout[slot] = v;
      }
    }
    if (Array.isArray(raw.hidden_players)) {
      out.hidden_players = raw.hidden_players.filter((n) => typeof n === 'string' && n).slice(0, MAX_HIDDEN);
    }
  }
  return out;
}

function putCosmeticsState(uuid, next) {
  const flat = identity.normalizeUuid(uuid);
  if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
  const state = loadCosmeticsState();
  state.accounts[flat] = next;
  persist(state);
}

// ---------------------------------------------------------------------------
// Loadout validation
// ---------------------------------------------------------------------------

function itemAllowed(item, flags) {
  const requires = item.requires ?? null;
  if (!requires) return true;
  return flags.includes(requires);
}

/**
 * Validate a candidate loadout ({slot: itemId|null}) against the catalog and
 * a flag set. Throws ApiError (400) on the first problem. Returns the
 * normalized loadout (only real slots; nulls kept).
 */
export function validateLoadout(loadout, flags = []) {
  const cat = catalog();
  if (!isPlainObject(loadout)) {
    throw httpError(400, 'BAD_LOADOUT', 'loadout must be an object {slot: itemId|null}');
  }
  const normalized = {};
  for (const [slot, value] of Object.entries(loadout)) {
    if (!cat.slots.has(slot)) {
      throw httpError(400, 'BAD_LOADOUT', `unknown slot '${slot}'`);
    }
    if (value === null || value === undefined) {
      normalized[slot] = null;
      continue;
    }
    if (typeof value !== 'string' || !value) {
      throw httpError(400, 'BAD_LOADOUT', `loadout.${slot} must be an item id or null`);
    }
    const item = cat.items.get(value);
    if (!item) {
      throw httpError(400, 'UNKNOWN_COSMETIC', `unknown cosmetic id '${value}'`);
    }
    if (item.slot !== slot) {
      throw httpError(400, 'WRONG_SLOT', `'${value}' belongs in slot '${item.slot}', not '${slot}'`);
    }
    if (!itemAllowed(item, flags)) {
      throw httpError(400, 'FLAG_REQUIRED', `'${value}' requires '${item.requires}'`);
    }
    normalized[slot] = value;
  }
  for (const slot of cat.slots) {
    if (!(slot in normalized)) normalized[slot] = null;
  }
  // Exclusivity groups: [slotName, [itemIds…]] — an equipped item in slotName
  // forbids any listed item (in whichever slot they live).
  for (const group of cat.exclusive) {
    if (!Array.isArray(group) || group.length !== 2) continue;
    const [slotName, blockedIds] = group;
    const equipped = normalized[slotName];
    if (!equipped || !Array.isArray(blockedIds)) continue;
    const blocked = new Set(blockedIds);
    for (const [otherSlot, itemId] of Object.entries(normalized)) {
      if (otherSlot === slotName || !itemId) continue;
      if (blocked.has(itemId)) {
        throw httpError(
          400,
          'EXCLUSIVE_CONFLICT',
          `'${itemId}' cannot be equipped together with '${equipped}'`,
        );
      }
    }
  }
  return normalized;
}

/**
 * Drop everything `flags` does not allow (and anything unknown) — for the
 * file the mod reads. Never throws; never mutates the input.
 */
export function sanitizeLoadout(loadout, flags = []) {
  const cat = catalog();
  const out = {};
  for (const slot of cat.slots) out[slot] = null;
  if (!isPlainObject(loadout)) return out;
  const granted = { ...out };
  for (const [slot, itemId] of Object.entries(loadout)) {
    if (!cat.slots.has(slot)) continue;
    const item = cat.items.get(itemId);
    if (!item || item.slot !== slot) continue;
    if (!itemAllowed(item, flags)) continue;
    granted[slot] = itemId;
  }
  // Re-apply exclusivity by trying to keep already-granted combos legal:
  // validate would throw — instead drop the conflicting non-slot member.
  for (const group of cat.exclusive) {
    if (!Array.isArray(group) || group.length !== 2) continue;
    const [slotName, blockedIds] = group;
    if (!granted[slotName] || !Array.isArray(blockedIds)) continue;
    const blocked = new Set(blockedIds);
    for (const [otherSlot, itemId] of Object.entries(granted)) {
      if (otherSlot === slotName || !itemId) continue;
      if (blocked.has(itemId)) granted[otherSlot] = null;
    }
  }
  return granted;
}

/** Rewrite the file for every running launch owned by `uuid`. */
export async function rewriteForRunning(uuid) {
  const flat = identity.normalizeUuid(uuid);
  if (!flat) return;
  try {
    const b = await bridge();
    for (const launch of b.launchesFor(flat)) {
      writeCosmeticsFile(flat, launch.gameDir);
    }
  } catch (e) {
    console.warn('[cosmetics] rewrite for running instance failed:', e?.message ?? e);
  }
}

// Lazy indirection: presence.mjs imports cosmetics.mjs, so cosmetics cannot
// import the bridge registry statically (module-eval order). Dynamic import
// resolves at call time, once every module is fully loaded.
let _bridgePromise = null;
function bridge() {
  if (!_bridgePromise) _bridgePromise = import('./bridge.mjs');
  return _bridgePromise;
}
/**
 * Build the document the mod reads for a uuid. The grant is included only
 * when it still verifies against THIS uuid — an expired/tampered grant means
 * grant:null and an empty sanitized loadout (contract: no ★, no cosmetics).
 */
export function cosmeticsDoc(uuid) {
  const flat = identity.normalizeUuid(uuid);
  const state = getCosmeticsState(flat ?? '');
  const { grant, grantValid, flags } = flat ? identity.flagsFor(flat) : { grant: null, grantValid: false, flags: [] };
  const grantFlags = grantValid ? flags : [];
  return {
    schema: 1,
    uuid: flat ?? String(uuid ?? ''),
    grant: grantValid ? grant : null,
    share: state.share,
    loadout: grantValid ? sanitizeLoadout(state.loadout, grantFlags) : Object.fromEntries([...catalog().slots].map((s) => [s, null])),
    hidden_players: [...state.hidden_players],
    emotes: state.emotes,
  };
}

function writeFileAtomic(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = file + '.tmp';
  fs.writeFileSync(tmp, text, 'utf8');
  fs.renameSync(tmp, file);
}

/**
 * Write espectral-cosmetics.json into a gameDir's config dir. When
 * canonicalGameDir differs (per-account profile launch, Contract C) the
 * canonical dir gets the same bytes — mirroring the espectral-client.json
 * launch mirror, but for a per-account document. Never throws.
 */
export function writeCosmeticsFile(uuid, gameDir, canonicalGameDir = null) {
  try {
    const doc = JSON.stringify(cosmeticsDoc(uuid), null, 2);
    const dirs = new Set();
    if (gameDir) dirs.add(path.resolve(gameDir));
    if (canonicalGameDir) dirs.add(path.resolve(canonicalGameDir));
    for (const dir of dirs) {
      writeFileAtomic(path.join(dir, 'config', 'espectral-cosmetics.json'), doc);
    }
  } catch (e) {
    console.warn('[cosmetics] could not write espectral-cosmetics.json:', e.message);
  }
}


/**
 * Validate + apply a PUT /api/cosmetics/:uuid patch. `flags` are the caller's
 * verified flags (identity.flagsFor). Side effects: persist state, republish
 * presence (when sharing), rewrite the file in every running gameDir of the
 * uuid. Returns the stored account state.
 */
export async function patchCosmetics(uuid, patch, { flags = [] } = {}) {
  const flat = identity.normalizeUuid(uuid);
  if (!flat) throw httpError(400, 'BAD_UUID', 'uuid must be a 32-char hex string');
  if (!isPlainObject(patch)) throw httpError(400, 'BAD_PATCH', 'body must be a JSON object');
  const { share, loadout, hidden_players, emotes, ...rest } = patch;
  if (Object.keys(rest).length > 0) {
    throw httpError(400, 'BAD_PATCH', `unknown fields: ${Object.keys(rest).join(', ')}`);
  }
  const current = getCosmeticsState(flat);
  if (share !== undefined) {
    if (typeof share !== 'boolean') throw httpError(400, 'BAD_SHARE', 'share must be a boolean');
    current.share = share;
  }
  if (emotes !== undefined) {
    if (typeof emotes !== 'boolean') throw httpError(400, 'BAD_EMOTES', 'emotes must be a boolean');
    current.emotes = emotes;
  }
  if (hidden_players !== undefined) {
    if (!Array.isArray(hidden_players) || hidden_players.length > MAX_HIDDEN) {
      throw httpError(400, 'BAD_HIDDEN', `hidden_players must be an array of at most ${MAX_HIDDEN} names`);
    }
    for (const n of hidden_players) {
      if (typeof n !== 'string' || !PLAYER_NAME_RE.test(n)) {
        throw httpError(400, 'BAD_HIDDEN', `hidden player names must match ${PLAYER_NAME_RE}`);
      }
    }
    current.hidden_players = [...new Set(hidden_players)];
  }
  if (loadout !== undefined) {
    current.loadout = validateLoadout(loadout, flags);
  }
  putCosmeticsState(flat, current);

  const b = await bridge();
  if (share !== undefined || loadout !== undefined) {
    const { republishNow } = await import('./presence.mjs');
    republishNow(flat);
  }
  for (const launch of b.launchesFor(flat)) {
    writeCosmeticsFile(flat, launch.gameDir);
  }
  return current;
}

/** Public view of one uuid for GET /api/cosmetics. */
export function cosmeticsAccountView(uuid, flags = []) {
  const state = getCosmeticsState(uuid);
  return {
    share: state.share,
    emotes: state.emotes,
    loadout: { ...state.loadout },
    hidden_players: [...state.hidden_players],
    flags: [...flags],
  };
}
