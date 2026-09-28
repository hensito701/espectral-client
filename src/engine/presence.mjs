/**
 * Presence publisher — ties PUT/DELETE /v1/presence to running launches.
 *
 * While an instance launched under account X is running AND X's cosmetics
 * `share` flag is on, the engine PUTs the sanitized loadout now and every
 * REPUBLISH_MS, and DELETEs on exit or when share flips off. Presence is
 * memory-only on the service (TTL 600 s), so a missed DELETE expires anyway.
 *
 * The launch route calls launchStarted() when the child process spawns and
 * launchExited() in the exit finalizer — the same bookkeeping points that own
 * Discord rich presence, so a crash in reconcile/stop paths still unwinds.
 */
import * as identity from './identity.mjs';
import * as cosmetics from './cosmetics.mjs';
import * as bridge from './bridge.mjs';
import { loadConfig } from './config.mjs';

export const REPUBLISH_MS = 240_000; // service TTL is 600 s; republish at 240 s

/** uuid -> { timer, inflight } — one publisher per account uuid. */
const publishers = new Map();

/**
 * Register a spawned game. `token` is the per-launch bridge token already
 * placed on the JVM command line; `gameDir` is the resolved run dir. Returns
 * the token actually registered (same value).
 */
export function launchStarted({ instance, uuid, gameDir, token }) {
  const flat = identity.normalizeUuid(uuid) ?? String(uuid ?? '');
  bridge.attach({ instance, uuid: flat, gameDir, token });
  if (cosmetics.getCosmeticsState(flat).share) startPublisher(flat);
  return token;
}

/**
 * Unregister a finished launch; stops + DELETEs presence when it was the last
 * launch for that account. `token` is the bridge token this launch was
 * registered with — teardown for a stale registration must not evict a newer
 * one for the same instance name.
 */
export function launchExited(instance, token = undefined) {
  const entry = bridge.detach(instance, token);
  if (entry && bridge.launchesFor(entry.uuid).length === 0) {
    stopPublisher(entry.uuid, { publish: true });
  }
}

/**
 * Flip a publisher on/off to match the current share flag — called by the
 * cosmetics PUT path after `share` changes.
 */
export function syncShare(uuid) {
  const flat = identity.normalizeUuid(uuid);
  if (!flat) return;
  const running = bridge.launchesFor(flat).length > 0;
  const sharing = cosmetics.getCosmeticsState(flat).share;
  if (running && sharing) startPublisher(flat);
  else stopPublisher(flat, { publish: true });
}

/** Push the current loadout immediately when it changes while sharing. */
export function republishNow(uuid) {
  const flat = identity.normalizeUuid(uuid);
  if (!flat) return;
  const running = bridge.launchesFor(flat).length > 0;
  const sharing = cosmetics.getCosmeticsState(flat).share;
  if (running && sharing) {
    startPublisher(flat); // ensures a timer exists
    void publishOnce(flat);
  } else {
    syncShare(flat);
  }
}

function startPublisher(uuid) {
  if (publishers.has(uuid)) return;
  const pub = { timer: null, inflight: false };
  publishers.set(uuid, pub);
  void publishOnce(uuid);
  pub.timer = setInterval(() => void publishOnce(uuid), REPUBLISH_MS);
  if (pub.timer.unref) pub.timer.unref();
}

function stopPublisher(uuid, { publish = false } = {}) {
  const pub = publishers.get(uuid);
  if (!pub) {
    if (publish) void unpublish(uuid);
    return;
  }
  clearInterval(pub.timer);
  publishers.delete(uuid);
  if (publish) void unpublish(uuid);
}

/** Test hook: wipe publisher state without touching the bridge registry. */
export function _reset() {
  for (const [, pub] of publishers) {
    clearInterval(pub.timer);
  }
  publishers.clear();
}

async function publishOnce(uuid) {
  const pub = publishers.get(uuid);
  if (!pub || pub.inflight) return;
  pub.inflight = true;
  try {
    const token = identity.tokenFor(uuid);
    if (!token) return; // unverified — nothing to publish under
    const state = cosmetics.getCosmeticsState(uuid);
    const { flags } = identity.flagsFor(uuid);
    const loadout = cosmetics.sanitizeLoadout(state.loadout, flags);
    await identity.serviceFetch('PUT', '/v1/presence', { token, body: { loadout }, timeoutMs: 8000 });
  } catch (e) {
    console.warn(`[presence] publish for ${uuid.slice(0, 8)}… failed: ${e?.code ?? e?.message ?? e}`);
  } finally {
    const cur = publishers.get(uuid);
    if (cur) cur.inflight = false;
  }
}

async function unpublish(uuid) {
  try {
    const token = identity.tokenFor(uuid);
    if (!token) return;
    await identity.serviceFetch('DELETE', '/v1/presence', { token, timeoutMs: 8000 });
  } catch (e) {
    console.warn(`[presence] unpublish for ${uuid.slice(0, 8)}… failed: ${e?.code ?? e?.message ?? e}`);
  }
}
