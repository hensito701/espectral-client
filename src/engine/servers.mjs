/**
 * Server status for the three Espectral hosts via mcsrvstat v3.
 *
 * - User-Agent header is REQUIRED (the API returns 403 without it).
 * - 150s in-memory cache per host. A cache shorter than 2x the UI poll
 *   interval (60s, stores.ts) locks into an alternating hit/miss — the old
 *   60s TTL matched the poll exactly and never served a hit. 150s (> 2x poll)
 *   keeps ~2/3 of polls as cache hits, so the game hosts aren't re-pinged
 *   every cycle (upstream mcsrvstat itself caches 5min).
 * - A host that responds `online:false` maps to online:false WITHOUT an error;
 *   only transport-level failures (HTTP status, network, timeout) set `error`.
 */
import { USER_AGENT } from './download.mjs';

export const HOSTS = ['uhc.espectral.es', 'uhc2.espectral.es', '24h.espectral.es'];
const CACHE_TTL_MS = 150_000;
const FETCH_TIMEOUT_MS = 10_000;

const cache = new Map(); // host -> { fetched_at, status }

/** Status for one host (cached CACHE_TTL_MS). Never rejects: always returns a shape. */
export async function getServerStatus(host) {
  const hit = cache.get(host);
  if (hit && Date.now() - hit.fetched_at < CACHE_TTL_MS) return hit.status;
  const status = await fetchStatus(host);
  cache.set(host, { fetched_at: Date.now(), status });
  return status;
}

/** Status for all three hosts, in HOSTS order. */
export async function getAllServerStatuses() {
  return Promise.all(HOSTS.map((h) => getServerStatus(h)));
}

async function fetchStatus(host) {
  const url = `https://api.mcsrvstat.us/3/${encodeURIComponent(host)}`;
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), FETCH_TIMEOUT_MS);
  try {
    const res = await fetch(url, {
      headers: { 'User-Agent': USER_AGENT },
      signal: ctrl.signal,
    });
    if (!res.ok) return offlineStatus(host, `mcsrvstat HTTP ${res.status}`);
    const data = await res.json();
    return mapStatus(host, data);
  } catch (err) {
    const message = err?.name === 'AbortError' ? 'request timed out' : (err?.message || 'network error');
    return offlineStatus(host, message);
  } finally {
    clearTimeout(timer);
  }
}

function mapStatus(host, data) {
  const online = data.online === true;
  const motd = data.motd ?? {};
  const players = data.players ?? {};
  const status = {
    host,
    online,
    hostname: typeof data.hostname === 'string' ? data.hostname : host,
    ip: typeof data.ip === 'string' ? data.ip : null,
    port: Number.isInteger(data.port) ? data.port : 25565,
    version: typeof data.version === 'string' ? data.version : null,
    software: typeof data.software === 'string' ? data.software : null,
    motd_raw: Array.isArray(motd.raw) ? motd.raw : [],
    motd_clean: Array.isArray(motd.clean) ? motd.clean : [],
    players_online: Number.isInteger(players.online) ? players.online : 0,
    players_max: Number.isInteger(players.max) ? players.max : 0,
    fetched_at: Date.now(),
  };
  if (typeof data.icon === 'string' && data.icon.length > 0) status.icon = data.icon;
  return status;
}

function offlineStatus(host, error) {
  const status = {
    host,
    online: false,
    hostname: host,
    ip: null,
    port: 25565,
    version: null,
    software: null,
    motd_raw: [],
    motd_clean: [],
    players_online: 0,
    players_max: 0,
    fetched_at: Date.now(),
  };
  if (error) status.error = error;
  return status;
}
