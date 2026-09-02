/**
 * Launch / train / AOT-status / launch-log / stats routes (B4 contract).
 *   POST /api/instances/:name/launch { mode:'normal'|'aot', dry_run:boolean }
 *       -> LaunchReply { key, pid? } | DryRunResult (when dry_run)
 *   POST /api/instances/:name/stop   -> { ok: true, instance: string }
 *   POST /api/instances/:name/train -> { key }       (background; train-done)
 *   GET  /api/instances/:name/aot    -> AotStatus
 *   GET  /api/launch/:key/log?cursor=N -> { lines, cursor, running }
 *   GET  /api/stats/launches?limit=N -> { launches } (newest first)
 *
 * Launch log streaming: a ring buffer per launch key (capacity 2000 lines),
 * SSE `launch-log` events as lines arrive, `launch-exit` on process end, and
 * cursor-based backfill through the GET endpoint. `launch-exit` carries the
 * exit fields plus launch timing (menu_ms / played_ms). Timing records are
 * appended to the launch-stats JSONL via ../stats.mjs and served back through
 * GET /api/stats/launches. Auto-train: when aot_auto_train is on
 * (instance-level, else global config) and the AOT cache for the launch key
 * is missing, a background train is kicked off once the game reaches the menu.
 */
import crypto from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import * as instances from '../instances.mjs';
import * as accounts from '../accounts.mjs';
import * as aot from '../aot.mjs';
import * as launch from '../launch.mjs';
import * as discord from '../discord.mjs';
import * as stats from '../stats.mjs';
import { emit } from '../events.mjs';
import { loadConfig } from '../config.mjs';
import { httpError } from '../error.mjs';

const execFileAsync = promisify(execFile);
const RING_CAPACITY = 2000;

/** One launch's log buffer: { instance, version, account, seq, entries: [{seq, text}], running, started_at, menu_at, ended_at } */
const buffers = new Map();
// M7: buffers were never evicted — each launch left a 2000-line ring in the
// Map forever. Prune finalized buffers after a backfill window, and cap the
// Map with an LRU so a run of short launches can't grow it unboundedly.
const BUFFER_TTL_MS = 10 * 60 * 1000; // finalized buffer kept 10 min (UI backfill window)
const BUFFER_LRU_CAP = 8; // max retained buffers (running ones are never evicted)

/** Drop stale finalized buffers (TTL) and LRU-evict the least-recently-read
 *  finished ones when the map exceeds the cap. Never evicts a running buffer.
 *  Cheap (one scan), called from createBuffer — the only growth point, so
 *  every allocation is preceded by cleanup. Finalized buffers that are never
 *  read again still linger until the next launch (bounded by BUFFER_LRU_CAP). */
function pruneBuffers() {
  const now = Date.now();
  for (const [key, buf] of buffers) {
    if (!buf.running && buf.ended_at !== null && now - buf.ended_at >= BUFFER_TTL_MS) {
      buffers.delete(key);
    }
  }
  if (buffers.size <= BUFFER_LRU_CAP) return;
  const finished = [];
  for (const [key, buf] of buffers) {
    if (!buf.running) finished.push([buf.last_read ?? buf.started_at ?? 0, key]);
  }
  finished.sort((a, b) => a[0] - b[0]); // oldest last_read first
  for (const [, key] of finished) {
    if (buffers.size <= BUFFER_LRU_CAP) break;
    buffers.delete(key);
  }
}

/** Pending stop requests that arrived while a launch was still in STARTING (pre-spawn). */
export const pendingStops = new Set();

/** Test hooks — when set, route uses these instead of the real implementations (ESM namespace is non-configurable, so t.mock.method fails). */
export const __testHooks = {
  resolveLaunch: null,
  launchInstance: null,
  resolveAccountFor: null,
  emit: null,
};
function routeEmit(event, payload) {
  const fn = __testHooks.emit ?? emit;
  return fn(event, payload);
}

function makeKey(name) {
  return `l${Date.now().toString(36)}${crypto.randomBytes(3).toString('hex')}`;
}

function createBuffer(key, instanceName, version, account = null) {
  const buf = {
    instance: instanceName,
    version,
    account,
    seq: 0,
    entries: [],
    running: true,
    started_at: Date.now(),
    menu_at: null,
    ended_at: null,
    last_read: Date.now(),
    spawned_at: null,
    phases: {},
  };
  pruneBuffers(); // piggyback cleanup on each new launch (no extra timer)
  buffers.set(key, buf);
  return buf;
}

/** Contract C: keys of launches that are still preparing/running. Presence is
 *  cleared only when this set becomes empty, so concurrent launches don't
 *  wipe each other's Rich Presence on exit. */
const liveLaunches = new Set();

/** Instances with a launch currently preparing or running. In-memory only —
 *  an engine restart clears it. Guards against launching the same instance
 *  twice (two games would share one gameDir/latest.log/natives). */
export const activeInstances = new Set();

/** Active running child processes: Map<string, { child, pid, key }> keyed by instance name. */
const runningProcesses = new Map();
/** Force-kill a running game process tree (win32 taskkill /T /F, elsewhere SIGKILL). */
async function forceKillProcess(child, pid) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  try {
    if (process.platform === 'win32') {
      await execFileAsync('taskkill', ['/pid', String(pid), '/T', '/F'], {
        windowsHide: true,
      });
    } else {
      child.kill('SIGKILL');
    }
  } catch {
    /* process may have already exited */
  }
  if (child.exitCode === null && child.signalCode === null) {
    await new Promise((resolve) => {
      const timer = setTimeout(resolve, 5000);
      child.once('exit', () => {
        clearTimeout(timer);
        resolve();
      });
      child.once('error', () => {
        clearTimeout(timer);
        resolve();
      });
    });
  }
}
/** Public summary of one launch buffer (GET /api/launches). */
function launchSummary(key, buf) {
  return {
    key,
    instance: buf.instance,
    version: buf.version,
    account: buf.account,
    running: buf.running,
    started_at: buf.started_at,
    menu_at: buf.menu_at,
    ended_at: buf.ended_at,
  };
}

/** Append raw output; splits on newlines, keeps a capped ring. Returns seqs. */
function appendOutput(key, chunk) {
  const buf = buffers.get(key);
  if (!buf) return [];
  const lines = chunk
    .replace(/\r\n/g, '\n')
    .replace(/\r/g, '\n')
    .split('\n');
  const out = [];
  for (const line of lines) {
    if (line.length === 0) continue;
    buf.seq += 1;
    const seq = buf.seq;
    buf.entries.push({ seq, text: line });
    if (buf.entries.length > RING_CAPACITY) buf.entries.splice(0, buf.entries.length - RING_CAPACITY);
    out.push({ seq, line });
  }
  return out;
}

function appendSystem(key, line) {
  const buf = buffers.get(key);
  if (!buf) return [];
  buf.seq += 1;
  const seq = buf.seq;
  buf.entries.push({ seq, text: line });
  if (buf.entries.length > RING_CAPACITY) buf.entries.splice(0, buf.entries.length - RING_CAPACITY);
  return [{ seq, line }];
}

function bufferView(key, cursor) {
  const buf = buffers.get(key);
  if (!buf) return { lines: [], cursor, running: false };
  buf.last_read = Date.now(); // touch for the LRU eviction order
  const entries = buf.entries.filter((e) => e.seq > cursor);
  return {
    lines: entries.map((e) => e.text),
    cursor: entries.length > 0 ? entries[entries.length - 1].seq : cursor,
    running: buf.running,
  };
}

/** Background auto-train (fire-and-forget; emits its own train-* events). */
function autoTrain(instance, estimateKey, force = false) {
  aot
    .queueTrainInstance(instance, { key: estimateKey, force })
    .catch((e) => {
      try {
        routeEmit('train-done', { instance: instance.name, key: estimateKey, ok: false, error: e.message });
      } catch {
        /* ignore */
      }
    });
}


/* ---------------------------------------------------------------------------
 * Discord Rich Presence
 * ------------------------------------------------------------------------- */

/** Known Espectral hosts → the presence "state" line shown while playing. */
const SERVER_STATE = new Map([
  ['uhc2', 'Jugando en uhc2.espectral.es'],
  ['uhc', 'Jugando en uhc.espectral.es'],
  ['24h', 'Jugando en 24h.espectral.es'],
]);

/** True when the global `discord_enabled` config flag is set (default on). */
function discordEnabled() {
  try {
    return loadConfig().discord_enabled !== false;
  } catch {
    return true;
  }
}

/**
 * Map a launched instance to the presence text. A known Espectral host in the
 * instance name wins; otherwise a single-player-friendly instance (snapshot
 * build, e.g. 25w11a — a version with no server focus) gets the singleplayer
 * line; everything else is treated as multiplayer (Fabric/vanilla installs
 * without a server prefix in the name). All strings stay below Discord's
 * 128-char per-field limit.
 */
export function presenceForInstance(instance) {
  if (!instance || !instance.name) return { state: 'En el launcher' };
  // A modpack instance (mrpack import) shows its pack name — it is the ground
  // truth of what is being played, so it wins over server-key heuristics.
  // Capped well under Discord's 128-char field limit.
  if (instance.modpack) {
    const state = ('Jugando ' + String(instance.modpack)).slice(0, 128);
    return { state, image: 'espectral_logo' };
  }
  const name = String(instance.name).toLowerCase();
  for (const [key, state] of SERVER_STATE) {
    if (name.includes(key)) {
      // Espectral server logos uploaded as Rich Presence assets with these keys.
      const image = key === '24h' ? '24h' : 'uhc';
      return { state, image };
    }
  }
  // A modpack instance (mrpack import) gets its pack name in the presence
  // line; known Espectral hosts above still win.
  if (instance.modpack) return { state: 'Jugando ' + instance.modpack, image: 'espectral_logo' };
  const version = String(instance.version ?? '');
  const isSnapshot = /^\d{2}w\d{1,2}[a-z]$/.test(version); // e.g. 25w11a
  // M2: singleplayer is detected EXPLICITLY (snapshots only), never by
  // negating the 3-group release regex. The old rule mislabeled every launch
  // whose version is not exactly `\d+.\d+.\d+` — modern naming (26.2, 25.1,
  // 21.0) and 2-group classic naming (1.21) are all multiplayer-capable
  // releases and showed "Jugando singleplayer". Every release in the
  // launcher's supported range (1.15.2 – 26.2) has a server focus; only a
  // snapshot build (no server focus) gets the singleplayer line.
  const isSingleplayer = isSnapshot;
  return isSingleplayer
    ? { state: 'Jugando singleplayer', image: 'espectral_logo' }
    : { state: 'Jugando servidor multijugador', image: 'espectral_logo' };
}

/**
 * Best-effort presence update; never throws into the launch flow.
 * `startTimestamp` is the moment the game was launched, so Discord's native
 * elapsed clock counts up from the same instant on every refresh.
 */
async function pushPresence(instance, startTimestamp = Date.now()) {
  if (__testHooks.emit !== null) return;
  if (!discordEnabled()) return;
  try {
    await discord.setPresence({
      ...presenceForInstance(instance),
      startTimestamp,
    });
  } catch {
    /* presence is cosmetic — ignore */
  }
}

/** Best-effort presence clear (game closed); never throws. */
async function pushClear() {
  if (__testHooks.emit !== null) return;
  if (!discordEnabled()) return;
  try {
    await discord.clearPresence();
  } catch {
    /* ignore */
  }
}


export async function register(app) {
  // POST /api/instances/:name/launch
  app.post('/api/instances/:name/launch', async (req, res, params, body) => {
    const instance = await instances.getInstance(params.name);
    const mode = body?.mode ?? 'normal';
    if (!['normal', 'aot'].includes(mode)) {
      throw httpError(400, 'BAD_MODE', "mode must be 'normal' or 'aot'");
    }
    // Contract C: optional account override — launch under a non-active
    // account (its own per-account run dir is resolved in resolveLaunch).
    let accountOverride = null;
    if (body?.account != null) {
      if (typeof body.account !== 'string' || body.account.length === 0) {
        throw httpError(400, 'BAD_ACCOUNT', 'account must be a non-empty string');
      }
      accountOverride = accounts.getAccount(body.account);
      if (!accountOverride) {
        throw httpError(404, 'NOT_FOUND', `unknown account: ${body.account}`);
      }
    }
    const dryRun = body?.dry_run === true;
    if (dryRun) {
      const resolved = await launch.resolveLaunch(instance, { mode, dryRun: true, account: accountOverride });
      return launch.dryRunResult(instance, resolved);
    }
    if (activeInstances.has(params.name)) {
      throw httpError(409, 'ALREADY_RUNNING', `instance is already launching or running: ${params.name}`);
    }
    activeInstances.add(params.name);

    const key = makeKey(params.name);
    createBuffer(key, params.name, instance.version, accountOverride?.username ?? null);

    // Stream the resolve/prepare phase (version, JDK provisioning, libs,
    // assets — the first launch of an instance can take minutes) as
    // launch-log lines so the UI shows progress instead of a dead button,
    // and the buffer exists even if resolveLaunch later throws.
    const progress = (msg) => {
      console.log(msg);
      for (const { seq, line } of appendSystem(key, msg)) {
        try {
          routeEmit('launch-log', { key, seq, line });
        } catch {
          /* SSE registry unavailable */
        }
      }
    };
    progress(`[espectral] preparing ${params.name}…`);

    // Presence shows the target game line with Discord's native elapsed clock
    // counting from this exact moment (the click on Play).
    const presenceStart = Date.now();

    // Resolve + spawn in the BACKGROUND: the prepare phase (JDK provisioning,
    // libraries, assets — anything the instance's first launch still needs)
    // can take minutes on a slow connection. The POST answers immediately with
    // the key and the progress above streams as launch-log lines; a failure
    // here is reported via launch-exit instead of a hung request.
    void (async () => {
      let resolved;
      try {
        let accountForResolve = accountOverride;
        const _resolveAccountFor = __testHooks.resolveAccountFor ?? launch.resolveAccountFor;
        if (accountOverride && typeof _resolveAccountFor === 'function') {
          const warn = (msg) => progress(msg.startsWith('[espectral]') ? msg : `[espectral] ${msg}`);
          try {
            accountForResolve = await _resolveAccountFor(accountOverride, warn);
          } catch (e) {
            const fallbackMsg =
              `Microsoft session for '${accountOverride.username}' failed (${e?.code ?? 'UNKNOWN'}: ${e?.message ?? e}) — ` +
              'launching OFFLINE; online servers will reject this session.';
            warn(fallbackMsg);
            accountForResolve = { username: accountOverride.username, uuid: accountOverride.uuid, accessToken: '0', userType: 'mojang' };
          }
        }
        const _resolveLaunch = __testHooks.resolveLaunch ?? launch.resolveLaunch;
        resolved = await _resolveLaunch(instance, { mode, onProgress: progress, account: accountForResolve });
      } catch (e) {
        const msg = `[espectral] launch error: ${e?.message ?? e}`;
        activeInstances.delete(params.name);
        pendingStops.delete(params.name);
        console.warn(msg);
        for (const { seq, line } of appendSystem(key, msg)) {
          try {
            routeEmit('launch-log', { key, seq, line });
          } catch {
            /* SSE registry unavailable */
          }
        }
        const buf = buffers.get(key);
        if (buf) {
          buf.running = false;
          buf.ended_at = Date.now();
        }
        if (liveLaunches.size === 0) void pushClear();
        try {
          const buf2 = buffers.get(key);
          const spawnMs = buf2?.spawned_at ? buf2.spawned_at - (buf2?.started_at ?? Date.now()) : null;
          const bootMs = buf2?.menu_at && buf2?.spawned_at ? buf2.menu_at - buf2.spawned_at : null;
          routeEmit('launch-exit', {
            key,
            instance: params.name,
            account: accountOverride?.username ?? null,
            code: null,
            signal: null,
            marker: null,
            error: String(e?.message ?? e),
            menu_ms: null,
            played_ms: null,
            spawn_ms: spawnMs,
            boot_ms: bootMs,
            phases: buf2?.phases ?? {},
          });
        } catch {
          /* ignore */
        }
        return;
      }

      // C6: cancelled during prepare — stop arrived while resolve was in flight
      if (pendingStops.delete(params.name)) {
        activeInstances.delete(params.name);
        const buf = buffers.get(key);
        if (buf) {
          buf.running = false;
          buf.ended_at = Date.now();
        }
        if (liveLaunches.size === 0) void pushClear();
        try {
          const phases = buffers.get(key)?.phases ?? {};
          routeEmit('launch-exit', {
            key,
            instance: params.name,
            account: accountOverride?.username ?? null,
            code: null,
            signal: null,
            marker: null,
            error: 'cancelled during prepare',
            menu_ms: null,
            played_ms: null,
            spawn_ms: null,
            boot_ms: null,
            phases,
          });
        } catch {
          /* ignore */
        }
        return;
      }

      liveLaunches.add(key);
      void pushPresence(instance, presenceStart);

      try {
        const _launchInstance = __testHooks.launchInstance ?? launch.launchInstance;
        const spawned = _launchInstance(instance, resolved, {
          onLog: (chunk) => {
            for (const { seq, line } of appendOutput(key, chunk)) {
              try {
                routeEmit('launch-log', { key, seq, line });
              } catch {
                /* SSE registry unavailable */
              }
            }
          },
          onSpawned: (at) => {
            const buf = buffers.get(key);
            if (buf) buf.spawned_at = at;
          },
          onPhase: (name, at) => {
            const buf = buffers.get(key);
            if (!buf) return;
            if (buf.phases[name] !== undefined) return;
            const ts = typeof at === 'number' ? at : Date.now();
            buf.phases[name] = ts;
          },
          onMarker: (at) => {
            const buf = buffers.get(key);
            if (buf) buf.menu_at = at;
            const line = `[espectral] Sound engine started (${new Date(at).toISOString()})`;
            for (const { seq, line: text } of appendSystem(key, line)) {
              try {
                routeEmit('launch-log', { key, seq, line: text });
              } catch {
                /* ignore */
              }
            }
            // Presence refreshes at the main menu with the same elapsed clock.
            void pushPresence(instance, presenceStart);
            // AOT auto-train on first launch — and on a cache the JVM refused
            // (classpath identity drifted): without this a stale cache is never
            // replaced and every boot silently loses the AOT win.
            const autoTrainOn = instance.aot_auto_train ?? loadConfig().aot_auto_train ?? false;
            const needsCache = !resolved.aotCacheExists || resolved.aotCacheStale === true;
            if (autoTrainOn && needsCache && resolved.aotAvailable) {
              autoTrain(instance, resolved.aotKey);
            }
          },
          onExit: ({ code, signal, marker, error }) => {
            runningProcesses.delete(params.name);
            pendingStops.delete(params.name);
            const buf = buffers.get(key);
            const endedAt = Date.now();
            if (buf) {
              buf.running = false;
              buf.ended_at = endedAt;
            }
            const startedAt = buf?.started_at ?? endedAt;
            const menuMs = buf?.menu_at ? buf.menu_at - buf.started_at : null;
            const playedMs = endedAt - startedAt;
            const spawnAt = buf?.spawned_at ?? null;
            const spawnMs = spawnAt ? spawnAt - startedAt : null;
            const bootMs = buf?.menu_at && spawnAt ? buf.menu_at - spawnAt : null;
            const phases = buf?.phases ?? {};
            // Presence reverts only when no launch is still live (Contract C
            // refcount) — concurrent launches keep the presence line.
            liveLaunches.delete(key);
            activeInstances.delete(params.name);
            if (liveLaunches.size === 0) void pushClear();
            const summary = error
              ? `[espectral] launch error: ${error}`
              : `[espectral] game exited (code ${code ?? 'null'}${signal ? `, signal ${signal}` : ''})`;
            for (const { seq, line } of appendSystem(key, summary)) {
              try {
                routeEmit('launch-log', { key, seq, line });
              } catch {
                /* ignore */
              }
            }
            try {
              routeEmit('launch-exit', {
                key,
                instance: buf?.instance ?? params.name,
                account: buf?.account ?? null,
                code,
                signal,
                marker,
                error: error ?? null,
                menu_ms: menuMs,
                played_ms: playedMs,
                spawn_ms: spawnMs,
                boot_ms: bootMs,
                phases,
              });
            } catch {
              /* ignore */
            }
            stats.recordLaunchStat({
              key,
              instance: buf?.instance ?? params.name,
              version: buf?.version ?? null,
              started_at: startedAt,
              menu_at: buf?.menu_at ?? null,
              menu_ms: menuMs,
              played_ms: playedMs,
              spawn_ms: spawnMs,
              boot_ms: bootMs,
              phases,
            });
          },
        });
        if (spawned?.child && spawned.child.exitCode === null && spawned.child.signalCode === null) {
          runningProcesses.set(params.name, { child: spawned.child, pid: spawned.pid, key });
        }
      } catch (spawnErr) {
        runningProcesses.delete(params.name);
        pendingStops.delete(params.name);
        const errSummary = String(spawnErr?.message ?? spawnErr);
        console.warn(`[espectral] spawn error: ${errSummary}`);
        const buf = buffers.get(key);
        if (buf) {
          buf.running = false;
          buf.ended_at = Date.now();
        }
        liveLaunches.delete(key);
        activeInstances.delete(params.name);
        if (liveLaunches.size === 0) void pushClear();
        try {
          const buf2 = buffers.get(key);
          const spawnMs = buf2?.spawned_at ? buf2.spawned_at - (buf2?.started_at ?? Date.now()) : null;
          const bootMs = buf2?.menu_at && buf2?.spawned_at ? buf2.menu_at - buf2.spawned_at : null;
          routeEmit('launch-exit', {
            key,
            instance: params.name,
            account: accountOverride?.username ?? null,
            code: null,
            signal: null,
            marker: null,
            error: errSummary,
            menu_ms: null,
            played_ms: null,
            spawn_ms: spawnMs,
            boot_ms: bootMs,
            phases: buf2?.phases ?? {},
          });
        } catch {
          /* ignore */
        }
      }
    })();

    // The game spawns asynchronously; pid becomes known through launch-log /
    // launch-exit. preparing:true tells the UI the resolve phase is streaming.
    return {
      key,
      pid: null,
      preparing: true,
      mode,
      instance: params.name,
      version: instance.version,
      account: accountOverride?.username ?? null,
    };
  });

  // POST /api/instances/:name/stop -> { ok: true, instance: string, cancelled?: string }
  app.post('/api/instances/:name/stop', async (req, res, params) => {
    const name = params.name;
    if (!activeInstances.has(name)) {
      throw httpError(404, 'NOT_RUNNING', `instance is not running: ${name}`);
    }
    const proc = runningProcesses.get(name);
    if (!proc || !proc.child) {
      pendingStops.add(name);
      return { ok: true, instance: name, cancelled: 'starting' };
    }
    await forceKillProcess(proc.child, proc.pid);
    return { ok: true, instance: name };
  });

  // POST /api/instances/:name/train -> { key } (background)
  app.post('/api/instances/:name/train', async (req, res, params) => {
    const instance = await instances.getInstance(params.name);
    if (instance.loader === 'neoforge') {
      throw httpError(400, 'AOT_UNAVAILABLE', 'AOT unavailable on NeoForge (requires JDK 25)');
    }
    const key = await aot.instanceKey(instance);
    // Background — the response carries the key estimate; train-done carries
    // the authoritative key (same in the common case).
    autoTrain(instance, key, true); // explicit user action — always trains
    return { key };
  });

  // GET /api/instances/:name/aot -> AotStatus (read-side owned by instances.mjs)
  app.get('/api/instances/:name/aot', async (req, res, params) => {
    const instance = await instances.getInstance(params.name);
    return instances.aotStatus(instance);
  });

  // GET /api/launch/:key/log?cursor=N -> { lines, cursor, running }
  app.get('/api/launch/:key/log', async (req, res, params) => {
    const key = params.key;
    if (!buffers.has(key)) {
      throw httpError(404, 'NOT_FOUND', `unknown launch key: ${key}`);
    }
    let cursor = 0;
    try {
      const q = new URL(req.url, 'http://localhost').searchParams;
      cursor = Number(q.get('cursor') ?? 0);
      if (!Number.isInteger(cursor) || cursor < 0) cursor = 0;
    } catch {
      cursor = 0;
    }
    return bufferView(key, cursor);
  });

  // GET /api/launches -> { launches } — live launch buffers, newest first (Contract C).
  app.get('/api/launches', async () => {
    const rows = [...buffers.entries()].map(([key, buf]) => launchSummary(key, buf));
    rows.sort((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0));
    return { launches: rows };
  });

  // GET /api/stats/launches?limit=N -> { launches } (newest first)
  app.get('/api/stats/launches', async (req) => {
    let limit = 10;
    try {
      const q = new URL(req.url, 'http://localhost').searchParams;
      limit = Number(q.get('limit') ?? 10);
    } catch {
      limit = 10;
    }
    if (!Number.isInteger(limit) || limit < 1) limit = 10;
    if (limit > 100) limit = 100;
    return { launches: await stats.readLaunchStats(limit) };
  });
}
