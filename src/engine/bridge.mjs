/**
 * Mod bridge — the loopback surface the in-game mod talks to.
 *
 * The launcher hands every launched game a random per-launch token via the
 * JVM property -Despectral.bridge.token (url via -Despectral.bridge.url).
 * The mod then calls `POST /api/bridge/visible` with header
 * `x-espectral-bridge: <token>` and a {uuids:[…]} body of players currently
 * visible through the vanilla player list.
 *
 * Answers come from an in-memory cache ONLY — the request path never does
 * network I/O. Uncached uuids are queued and resolved in the background by
 * POST /v1/presence/query on espectral-id: at most 1 request / 3 s, batches
 * of ≤100, positive cache TTL 60 s, negative cache TTL 60 s.
 */
import crypto from 'node:crypto';
import * as identity from './identity.mjs';
import { httpError } from './error.mjs';

export const BRIDGE_HEADER = 'x-espectral-bridge';
export const CACHE_TTL_MS = 60_000;
export const NEGATIVE_TTL_MS = 60_000;
export const QUERY_BATCH = 100;
export const QUERY_INTERVAL_MS = 3_000;
export const MAX_VISIBLE_INPUT = 1000;

/**
 * Running launches registered for the bridge.
 * Map<string instanceName, { uuid, gameDir, token, startedAt }>
 */
const launches = new Map();

/** Presence cache: uuid -> { grant, loadout, expiresAt } | { absent: true, expiresAt } */
const cache = new Map();

/** Queued uuid -> Set<ownerUuid> needing a background query. */
const pending = new Map();
let flushing = false;
let lastFlushAt = 0;
let flushTimer = null;

// ---------------------------------------------------------------------------
// Launch registry
// ---------------------------------------------------------------------------

/** Mint a per-launch bridge token (random 32 bytes hex). */
export function mintToken() {
  return crypto.randomBytes(32).toString('hex');
}

/**
 * Register a running game. Called by the launch route when the child spawns.
 * A re-register (same instance relaunching) swaps the token and refreshes
 * the entry — a stale token can never authorize after reattach.
 */
export function attach({ instance, uuid, gameDir, token }) {
  const entry = {
    uuid: identity.normalizeUuid(uuid) ?? String(uuid ?? ''),
    gameDir: String(gameDir ?? ''),
    token: String(token),
    startedAt: Date.now(),
  };
  launches.set(instance, entry);
  return entry;
}

/**
 * Unregister a launch (game exit / failed spawn / stop). Returns the removed
 * entry or null. `expectedToken`, when given, scopes the delete to the launch
 * that owns that token — a stale teardown (e.g. a trainer) must not evict a
 * newer registration for the same instance name.
 */
export function detach(instance, expectedToken = undefined) {
  const entry = launches.get(instance);
  if (!entry) return null;
  if (expectedToken !== undefined && entry.token !== expectedToken) return null;
  launches.delete(instance);
  return entry;
}

/** All live launches owned by a uuid (0..n — concurrent instances). */
export function launchesFor(uuid) {
  const flat = identity.normalizeUuid(uuid);
  const out = [];
  for (const [instance, entry] of launches) {
    if (flat && entry.uuid === flat) out.push({ instance, ...entry });
  }
  return out;
}

/** All live launches (test/introspection helper). */
export function allLaunches() {
  return [...launches.entries()].map(([instance, entry]) => ({ instance, ...entry }));
}

/** True when `token` belongs to a live launch; returns the launch entry. */
export function authenticate(token) {
  if (typeof token !== 'string' || !/^[0-9a-f]{64}$/i.test(token)) return null;
  const probe = Buffer.from(token.toLowerCase(), 'utf8');
  for (const [instance, entry] of launches) {
    const stored = Buffer.from(entry.token.toLowerCase(), 'utf8');
    // timingSafeEqual throws RangeError on different lengths — launches
    // registered without a minted token (unverified/offline accounts carry '')
    // must be skipped, not crash every bridge request.
    if (stored.length !== probe.length) continue;
    if (crypto.timingSafeEqual(stored, probe)) {
      return { instance, ...entry };
    }
  }
  return null;
}
/** Test hook: direct cache injection. */
export function _seedCache(uuid, entry) {
  const flat = identity.normalizeUuid(uuid);
  if (flat) cache.set(flat, { ...entry });
}

/** Test hook: wipe module state between tests. */
export function _reset() {
  launches.clear();
  cache.clear();
  pending.clear();
  flushing = false;
  lastFlushAt = 0;
  if (flushTimer) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
}

function cacheGet(uuid) {
  const entry = cache.get(uuid);
  if (!entry) return null;
  if (entry.expiresAt <= Date.now()) {
    cache.delete(uuid);
    return null;
  }
  return entry;
}

/**
 * Answer a visible-uuid query from cache. `owner` is the launcher entry of
 * the calling game. Returns { players } synchronously-shaped; uncached uuids
 * are queued for a background fetch (fire and forget).
 */
export function visible(owner, uuids) {
  const players = [];
  const now = Date.now();
  const seen = new Set();
  for (const raw of Array.isArray(uuids) ? uuids.slice(0, MAX_VISIBLE_INPUT) : []) {
    const flat = identity.normalizeUuid(typeof raw === 'string' ? raw : '');
    if (!flat || seen.has(flat)) continue;
    seen.add(flat);
    const hit = cacheGet(flat);
    if (hit) {
      if (!hit.absent) players.push({ uuid: flat, grant: hit.grant ?? null, loadout: hit.loadout ?? null });
      continue; // negative entries are also "cached" — don't requeue
    }
    queueQuery(flat, owner.uuid);
  }
  return { players };
}

/** Queue `uuid` for a background presence/query, billed to `ownerUuid`. */
function queueQuery(uuid, ownerUuid) {
  if (!ownerUuid) return;
  let owners = pending.get(uuid);
  if (!owners) {
    owners = new Set();
    pending.set(uuid, owners);
  }
  owners.add(ownerUuid);
  scheduleFlush();
}

function scheduleFlush() {
  if (flushing || flushTimer || pending.size === 0) return;
  const wait = Math.max(0, QUERY_INTERVAL_MS - (Date.now() - lastFlushAt));
  flushTimer = setTimeout(() => {
    flushTimer = null;
    void flush();
  }, wait);
  if (flushTimer.unref) flushTimer.unref();
}

/**
 * Drain the pending queue in ≤100-uuid batches, at most QUERY_INTERVAL_MS
 * between requests. Groups by owner uuid (each presence/query call is Bearer-
 * authed with the owner's session token).
 */
async function flush() {
  if (flushing) return;
  flushing = true;
  try {
    while (pending.size > 0) {
      // First owner with queued uuids, first 100 of them.
      const byOwner = new Map();
      for (const [uuid, owners] of pending) {
        for (const owner of owners) {
          if (!byOwner.has(owner)) byOwner.set(owner, []);
          byOwner.get(owner).push(uuid);
        }
      }
      let didWork = false;
      for (const [owner, uuids] of byOwner) {
        const batch = uuids.slice(0, QUERY_BATCH);
        for (const u of batch) {
          const owners = pending.get(u);
          owners?.delete(owner);
          if (owners && owners.size === 0) pending.delete(u);
        }
        const token = identity.tokenFor(owner);
        if (!token) continue; // owner unverified — uuids dropped, not queued
        didWork = true;
        await queryOnce(owner, token, batch);
        if (pending.size > 0) {
          await new Promise((r) => setTimeout(r, QUERY_INTERVAL_MS));
        }
      }
      if (!didWork && pending.size === 0) break;
      if (!didWork) break; // every queued uuid had an unverified owner — drop
    }
  } finally {
    flushing = false;
    lastFlushAt = Date.now();
    if (pending.size > 0) scheduleFlush();
  }
}

async function queryOnce(ownerUuid, token, uuids) {
  try {
    const data = await identity.serviceFetch('POST', '/v1/presence/query', {
      token,
      body: { uuids },
      timeoutMs: 8000,
    });
    const seen = new Set();
    const expiresAt = Date.now() + CACHE_TTL_MS;
    for (const p of Array.isArray(data?.players) ? data.players : []) {
      const flat = identity.normalizeUuid(p?.uuid ?? '');
      if (!flat) continue;
      seen.add(flat);
      cache.set(flat, {
        grant: typeof p.grant === 'string' ? p.grant : null,
        loadout: p.loadout && typeof p.loadout === 'object' ? p.loadout : null,
        expiresAt,
      });
    }
    // Uuids asked for but absent from the response have no live presence —
    // negative-cache them so the mod's next visible sweep doesn't refetch.
    const negExpires = Date.now() + NEGATIVE_TTL_MS;
    for (const u of uuids) {
      if (!seen.has(u)) cache.set(u, { absent: true, expiresAt: negExpires });
    }
  } catch (e) {
    console.warn(`[bridge] presence/query failed for ${ownerUuid.slice(0, 8)}…: ${e?.code ?? e?.message ?? e}`);
    // Transport/service failure: negatively cache the batch briefly so a
    // failing service isn't hammered every poll tick.
    const negExpires = Date.now() + NEGATIVE_TTL_MS;
    for (const u of uuids) cache.set(u, { absent: true, expiresAt: negExpires });
  }
}
