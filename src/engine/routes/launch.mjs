/**
 * Launch / train / AOT-status / launch-log / stats routes (B4 contract).
 *   POST /api/instances/:name/launch { dry_run:boolean, account?:string }
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
import * as presence from '../presence.mjs';
import { emit } from '../events.mjs';
import { loadConfig } from '../config.mjs';
import { httpError } from '../error.mjs';
import * as sessions from '../game-sessions.mjs';
import * as agentRuntime from '../agent.mjs';

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
  /** Set to an array to record presence transitions instead of calling Discord. */
  presence: null,
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

/** Active running child processes: Map<string, { child, pid, key }> keyed by
 *  instance name. `child` is null for a game adopted from an earlier launcher
 *  session (pid only). */
const runningProcesses = new Map();
/** Force-kill a running game process tree (win32 taskkill /T /F, elsewhere SIGKILL). */
async function forceKillProcess(child, pid) {
  if (child && (child.exitCode !== null || child.signalCode !== null)) return;
  try {
    if (process.platform === 'win32') {
      await execFileAsync('taskkill', ['/pid', String(pid), '/T', '/F'], {
        windowsHide: true,
      });
    } else if (child) {
      child.kill('SIGKILL');
    } else {
      process.kill(pid, 'SIGKILL');
    }
  } catch {
    /* process may have already exited */
  }
  if (!child) {
    // Adopted game: no exit event to wait for — poll the pid instead.
    for (let i = 0; i < 50 && isPidAlive(pid); i++) await new Promise((r) => setTimeout(r, 100));
    return;
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
const isPidAlive = sessions.isPidAlive;

/** Reconcile spawned processes against the OS. A missed exit event (or a kill
 *  from outside the launcher) would otherwise leave a stale activeInstances
 *  entry plus a forever-running buffer: the UI keeps showing the game as
 *  running and Terminate keeps targeting a dead process. Dead entries are
 *  finalized through the same per-launch exit path (single launch-exit emit,
 *  stats, presence), which is idempotent against a late real event.
 *  Preparing launches (no spawned process yet) are never touched — the
 *  pendingStops path owns those. */
function reconcileRunning() {
  for (const [name, proc] of runningProcesses) {
    const exited = proc.child != null && (proc.child.exitCode !== null || proc.child.signalCode !== null);
    if (!exited && isPidAlive(proc.pid)) continue;
    runningProcesses.delete(name);
    if (typeof proc.finalize === 'function') {
      const buf = buffers.get(proc.key);
      try {
        proc.finalize({
          code: proc.child?.exitCode ?? null,
          signal: proc.child?.signalCode ?? null,
          marker: buf?.menu_at != null,
          error: null,
        });
      } catch {
        /* finalizer is self-contained; never break status reads */
      }
    }
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
    agent: buf.agent === true,
    started_at: buf.started_at,
    menu_at: buf.menu_at,
    ended_at: buf.ended_at,
  };
}

/** All live launch rows, newest first (GET /api/launches + the agent API). */
export function launchSummaries() {
  reconcileRunning();
  const rows = [...buffers.entries()].map(([key, buf]) => launchSummary(key, buf));
  rows.sort((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0));
  return rows;
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

/** Backfill view of a launch log ring: {lines, cursor, running}. */
export function bufferView(key, cursor) {
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
    .queueTrainInstance(instance, {
      key: estimateKey,
      force,
      // The trainer shares the instance's gameDir: never spawn it under a
      // live game (relaunch during the resolve phase opens a duplicate
      // Minecraft window). The relaunch's own exit re-queues instead.
      isBlocked: () => activeInstances.has(instance.name),
    })
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

/**
 * Known Espectral instance names → the presence shown before the player has
 * joined anything. uhc/uhc2 names predate the swapper merge; both play MAIN.
 */
const SERVER_STATE = new Map([
  ['uhc2', 'Jugando en main.uhce.top'],
  ['uhc', 'Jugando en main.uhce.top'],
  ['24h', 'Jugando en 24h.uhce.top'],
]);

/**
 * Espectral servers by the host the game connects to (vanilla logs
 * `Connecting to <host>, <port>`). Legacy espectral.es names and the raw
 * node addresses still route to the same servers. Art assets on the Discord
 * app: `espectral_logo` and `24h` (there is no dedicated MAIN/Mini asset).
 */
const ESPECTRAL_SERVERS = [
  {
    hosts: ['main.uhce.top', 'uhc.espectral.es', 'uhc2.espectral.es', 'uhc3.espectral.es',
      'bingo.espectral.es', '2.27.233.26'],
    details: 'Espectral UHC · MAIN',
    state: 'main.uhce.top',
    image: 'espectral_logo',
    largeText: 'Espectral UHC',
  },
  {
    hosts: ['mini.uhce.top', '150.136.181.209'],
    details: 'Espectral UHC · Mini',
    state: 'mini.uhce.top',
    image: 'espectral_logo',
    largeText: 'Espectral UHC Mini',
  },
  {
    hosts: ['24h.uhce.top', '24h.espectral.es', '247.espectral.es'],
    details: 'Espectral Meetups 24/7',
    state: '24h.uhce.top',
    image: '24h',
    largeText: 'Espectral Meetups 24/7',
  },
];

/**
 * Presence for a server the game is connected to. Espectral servers get
 * their own line; any other server stays generic (never leak a private
 * server's address into a public profile).
 */
export function presenceForServer(host) {
  const h = String(host ?? '').trim().toLowerCase().replace(/\.$/, '');
  const known = ESPECTRAL_SERVERS.find((s) => s.hosts.includes(h));
  if (known) {
    const { details, state, image, largeText } = known;
    return { details, state, image, largeText };
  }
  return { state: 'Jugando servidor multijugador', image: 'espectral_logo' };
}

/** Mod marker logged when the client leaves a server/world (MinecraftMixin). */
export const PRESENCE_MENU_MARKER = '[espectral] presence: menu';

/**
 * Map one game-log line to a presence transition, or null for no change:
 *   { server: host }        vanilla `Connecting to <host>, <port>`
 *   { singleplayer: true }  the integrated server starting (a world opened)
 *   { menu: true }          our mod's leave marker, or a failed connection
 */
export function presenceEventFromLog(line) {
  const text = String(line ?? '');
  const connecting = /\bConnecting to ([^\s,]+), \d+/.exec(text);
  if (connecting) return { server: connecting[1] };
  if (text.includes('Starting integrated minecraft server')) return { singleplayer: true };
  if (text.includes(PRESENCE_MENU_MARKER) || text.includes("Couldn't connect to server")) return { menu: true };
  return null;
}

/** True when the global `discord_enabled` config flag is set (default on). */
function discordEnabled() {
  try {
    return loadConfig().discord_enabled !== false;
  } catch {
    return true;
  }
}

/**
 * Map a launched instance to the presence shown before the game has joined
 * anything (launch click, main menu). A known Espectral host in the instance
 * name wins; a snapshot build (no server focus) gets the singleplayer line;
 * everything else is at the main menu — joins are tracked live from the game
 * log (`presenceEventFromLog`). All strings stay below Discord's 128-char
 * per-field limit.
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
      // `24h` is a Discord art asset; there is none for MAIN, so the logo.
      const image = key === '24h' ? '24h' : 'espectral_logo';
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
    : { state: 'En el menú principal', image: 'espectral_logo' };
}

/**
 * Best-effort presence update; never throws into the launch flow.
 * `startTimestamp` is the moment the game was launched, so Discord's native
 * elapsed clock counts up from the same instant on every refresh.
 */
async function pushPresence(instance, startTimestamp = Date.now(), override = null) {
  if (__testHooks.presence) {
    __testHooks.presence.push({ kind: 'set', instance: instance?.name, override: override ?? null });
    return;
  }
  if (__testHooks.emit !== null) return;
  if (!discordEnabled()) return;
  try {
    await discord.setPresence({
      ...(override ?? presenceForInstance(instance)),
      startTimestamp,
    });
  } catch {
    /* presence is cosmetic — ignore */
  }
}

/** Presence for a parsed log event; null = keep the instance's base line. */
function presenceForEvent(event) {
  if (event.server) return presenceForServer(event.server);
  if (event.singleplayer) return { state: 'Jugando singleplayer', image: 'espectral_logo' };
  return null;
}

/** Best-effort presence clear (game closed); never throws. */
async function pushClear() {
  if (__testHooks.presence) {
    __testHooks.presence.push({ kind: 'clear' });
    return;
  }
  if (__testHooks.emit !== null) return;
  if (!discordEnabled()) return;
  try {
    await discord.clearPresence();
  } catch {
    /* ignore */
  }
}


/**
 * Start a launch (the body of POST /api/instances/:name/launch, exported so
 * the agent route can create a launch with the same buffer/presence/exit
 * bookkeeping instead of duplicating it).
 *
 * `options.agent/join/window` come validated from agent.normalizeLaunchOptions
 * (the UI route validates them here; the agent route forces agent:true).
 * Agent launches additionally: skip Discord presence, register the launch in
 * the agent registry (so `/api/agent/*` and the mod relay can address it), and
 * tag the launch buffer + `/api/launches` row with `agent: true`.
 */
export async function startLaunch(name, body = {}) {
  const instance = await instances.getInstance(name);
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
  const options = agentRuntime.normalizeLaunchOptions(body);
  const dryRun = body?.dry_run === true;
  if (dryRun) {
    const resolved = await launch.resolveLaunch(instance, {
      dryRun: true,
      account: accountOverride,
      agent: options.agent,
      join: options.join,
      window: options.window,
    });
    return launch.dryRunResult(instance, resolved);
  }
  if (activeInstances.has(name)) {
    throw httpError(409, 'ALREADY_RUNNING', `instance is already launching or running: ${name}`);
  }
  activeInstances.add(name);

  const key = makeKey(name);
  const buf = createBuffer(key, name, instance.version, accountOverride?.username ?? null);
  // Agent launches are tagged on the buffer (→ /api/launches rows) so the UI
  // and the agent routes can tell a bot game from a player game.
  buf.agent = options.agent;
  // Register with the agent registry BEFORE the key is returned: the mod's
  // first poll races the spawn, and an agent call to a key the caller already
  // holds must never answer "unknown launch". gameDir is filled in on spawn.
  if (options.agent) {
    try {
      agentRuntime.registerLaunch({
        key,
        instance: name,
        gameDir: typeof instance.game_dir === 'string' && instance.game_dir ? instance.game_dir : null,
      });
    } catch {
      /* the agent registry must never fail a launch */
    }
  }

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
  progress(`[espectral] preparing ${name}…`);

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
      resolved = await _resolveLaunch(instance, {
        onProgress: progress,
        account: accountForResolve,
        agent: options.agent,
        join: options.join,
        window: options.window,
      });
    } catch (e) {
      const msg = `[espectral] launch error: ${e?.message ?? e}`;
      activeInstances.delete(name);
      pendingStops.delete(name);
      if (options.agent) {
        try {
          agentRuntime.releaseLaunch(key);
        } catch {
          /* ignore */
        }
      }
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
          instance: name,
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
    if (pendingStops.delete(name)) {
      activeInstances.delete(name);
      if (options.agent) {
        try {
          agentRuntime.releaseLaunch(key);
        } catch {
          /* ignore */
        }
      }
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
          instance: name,
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

    // Agent launches never touch Discord rich presence: a bot-driven game is
    // not what the player is playing. They are also NOT counted in the
    // presence refcount — otherwise a player game exiting would leave the
    // presence line stuck ("playing <server>") for as long as the bot game
    // runs.
    if (!options.agent) {
      liveLaunches.add(key);
      void pushPresence(instance, presenceStart);
    }
    // Single-flight exit finalizer (see reconcileRunning): the exit/error/
    // close events can race each other, and the reconcile sweep may finalize
    // a dead PID before a late real event arrives — only the first call
    // runs. Stored on the runningProcesses entry so the sweep finalizes
    // through this exact path (one launch-exit emit, one stats row).
    let exitSeen = false;
    let handleExit = null;
    // Live presence: what the game log says the player is doing right now
    // (null = at the menu → the instance's base line).
    let livePresence = null;

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
            if (options.agent) continue; // agent games are invisible to Discord
            const event = presenceEventFromLog(line);
            if (event) {
              livePresence = presenceForEvent(event);
              void pushPresence(instance, presenceStart, livePresence);
            }
          }
        },
        onSpawned: (at) => {
          const buf = buffers.get(key);
          if (buf) buf.spawned_at = at;
          // The agent entry was registered when the key was minted; the
          // resolved gameDir (per-account profile dirs) is only known now.
          if (options.agent) {
            try {
              agentRuntime.registerLaunch({ key, instance: name, gameDir: resolved.gameDir });
            } catch {
              /* the agent registry must never fail a launch */
            }
          }
          // Register the running game with the cosmetics bridge + presence
          // publisher (per-launch token was minted by resolveLaunch).
          try {
            presence.launchStarted({
              instance: name,
              uuid: resolved.account?.uuid,
              gameDir: resolved.gameDir,
              token: resolved.bridgeToken ?? '',
            });
          } catch {
            /* bridge registration must never fail a launch */
          }
        },
        onPhase: (phaseName, at) => {
          const buf = buffers.get(key);
          if (!buf) return;
          if (buf.phases[phaseName] !== undefined) return;
          const ts = typeof at === 'number' ? at : Date.now();
          buf.phases[phaseName] = ts;
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
          // Presence refreshes at the main menu with the same elapsed clock —
          // except for agent launches, which must never appear in Discord
          // presence (this is the only push left on the marker path).
          if (!options.agent) void pushPresence(instance, presenceStart, livePresence);
          sessions.recordSession(name, { menu_at: at });
          // The game is up: the UI may close the launcher now (setting
          // "close on game start") — the detached game keeps running. Never
          // for an agent launch: the launcher IS the relay (agent API + mod
          // long-poll), so closing it would cut the agent off from the game it
          // just started.
          if (!options.agent) {
            try {
              routeEmit('launch-ready', { key, instance: name });
            } catch {
              /* ignore */
            }
          }
          // AOT auto-train on first launch — and on a cache the JVM refused
          // (classpath identity drifted): without this a stale cache is never
          // replaced and every boot silently loses the AOT win.
          // Deferred until this game EXITS: the trainer is a second full game
          // boot in the SAME gameDir — spawning it here opens a duplicate
          // Minecraft window alongside the one being played (2× heap, both
          // writing latest.log/natives). Record the need; onExit retrains.
          const autoTrainOn = instance.aot_auto_train ?? loadConfig().aot_auto_train ?? false;
          const needsCache = !resolved.aotCacheExists || resolved.aotCacheStale === true;
          if (buf && autoTrainOn && needsCache && resolved.aotAvailable) {
            buf.needsAutoTrain = true;
          }
        },
        onExit: (handleExit = ({ code, signal, marker, error }) => {
          if (exitSeen) return;
          exitSeen = true;
          runningProcesses.delete(name);
          sessions.removeSession(name);
          pendingStops.delete(name);
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
          activeInstances.delete(name);
          // Any in-flight agent call is failed with GAME_GONE and parked polls
          // are released now — a waiter must never outlive the game process.
          if (options.agent) {
            try {
              agentRuntime.releaseLaunch(key);
            } catch {
              /* registry teardown must never break the exit path */
            }
          }
          try {
            presence.launchExited(name, resolved?.bridgeToken ?? '');
          } catch {
            /* presence teardown must never break the exit path */
          }
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
              instance: buf?.instance ?? name,
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
            instance: buf?.instance ?? name,
            version: buf?.version ?? null,
            started_at: startedAt,
            menu_at: buf?.menu_at ?? null,
            menu_ms: menuMs,
            played_ms: playedMs,
            spawn_ms: spawnMs,
            boot_ms: bootMs,
            phases,
          });
          // Deferred auto-train (see onMarker): the gameDir is free now, so
          // the trainer boots alone — one window at a time. queueTrainInstance
          // dedupes against an explicit POST /train running concurrently.
          if (buf?.needsAutoTrain === true) {
            buf.needsAutoTrain = false;
            autoTrain(instance, resolved.aotKey);
          }
        }),
      });
      if (spawned?.child && spawned.child.exitCode === null && spawned.child.signalCode === null) {
        runningProcesses.set(name, { child: spawned.child, pid: spawned.pid, key, finalize: handleExit });
        sessions.recordSession(name, {
          key,
          pid: spawned.pid,
          version: instance.version,
          account: buffers.get(key)?.account ?? null,
          uuid: resolved.account?.uuid ?? null,
          game_dir: resolved.gameDir,
          started_at: buffers.get(key)?.started_at ?? Date.now(),
          bridge_token: resolved.bridgeToken ?? '',
          // Survives a launcher restart: an adopted AGENT game must come back
          // as an agent launch (relay servable, no Discord presence).
          agent: options.agent === true,
        });
      }
    } catch (spawnErr) {
      runningProcesses.delete(name);
      pendingStops.delete(name);
      const errSummary = String(spawnErr?.message ?? spawnErr);
      console.warn(`[espectral] spawn error: ${errSummary}`);
      const buf = buffers.get(key);
      if (buf) {
        buf.running = false;
        buf.ended_at = Date.now();
      }
      liveLaunches.delete(key);
      activeInstances.delete(name);
      if (options.agent) {
        try {
          agentRuntime.releaseLaunch(key);
        } catch {
          /* ignore */
        }
      }
      try {
        presence.launchExited(name, resolved?.bridgeToken ?? '');
      } catch {
        /* ignore */
      }
      if (liveLaunches.size === 0) void pushClear();
      try {
        const buf2 = buffers.get(key);
        const spawnMs = buf2?.spawned_at ? buf2.spawned_at - (buf2?.started_at ?? Date.now()) : null;
        const bootMs = buf2?.menu_at && buf2?.spawned_at ? buf2.menu_at - buf2.spawned_at : null;
        routeEmit('launch-exit', {
          key,
          instance: name,
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
    instance: name,
    version: instance.version,
    account: accountOverride?.username ?? null,
  };
}

/**
 * Stop a launch (body of POST /api/instances/:name/stop). Idempotent: a
 * non-running instance succeeds quietly, and a stop that arrives while the
 * launch is still preparing (no spawn yet) is recorded as a pending stop the
 * resolve phase honours.
 *
 * Exported so the agent route can stop a launch by key without reimplementing
 * the pending-stop/kill/teardown chain.
 */
export async function stopLaunch(name) {
  reconcileRunning();
  if (!activeInstances.has(name)) {
    return { ok: true, instance: name, already_stopped: true };
  }
  const proc = runningProcesses.get(name);
  if (!proc) {
    pendingStops.add(name);
    return { ok: true, instance: name, cancelled: 'starting' };
  }
  await forceKillProcess(proc.child, proc.pid);
  reconcileRunning();
  return { ok: true, instance: name };
}

export async function register(app) {
  app.post('/api/instances/:name/launch', async (req, res, params, body) => {
    return startLaunch(params.name, body ?? {});
  });

  // POST /api/instances/:name/stop -> { ok: true, instance: string, cancelled?: string, already_stopped?: true }
  // Idempotent: stopping a non-running instance succeeds quietly (the UI can
  // offer Terminate off a stale poll row, or race the game exiting on its
  // own) instead of surfacing "instance is not running" as an error toast.
  app.post('/api/instances/:name/stop', async (req, res, params) => stopLaunch(params.name));

  // POST /api/instances/:name/train -> { key, deferred? } (background)
  app.post('/api/instances/:name/train', async (req, res, params) => {
    const instance = await instances.getInstance(params.name);
    if (instance.loader === 'neoforge') {
      throw httpError(400, 'AOT_UNAVAILABLE', 'AOT unavailable on NeoForge (requires JDK 25)');
    }
    const key = await aot.instanceKey(instance);
    if (activeInstances.has(params.name)) {
      // A trainer is a second full game in the same gameDir — never run it
      // under the live game (duplicate window). Defer: the launch's onExit
      // picks this flag up and trains once the gameDir is free.
      const proc = runningProcesses.get(params.name);
      const buf = proc ? buffers.get(proc.key) : null;
      if (buf) buf.needsAutoTrain = true;
      return { key, deferred: true };
    }
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
    return { launches: launchSummaries() };
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

  void adoptRunningGames();
}

/**
 * Re-attach games still running from an earlier launcher session (the game is
 * detached, so it survives the launcher closing). Each becomes a pid-only
 * runningProcesses entry: Play is blocked for that instance, Stop kills it,
 * the bridge + Discord presence come back, and reconcileRunning() finalizes
 * it when the process ends (no exit code is available for an adopted game).
 */
export async function adoptRunningGames() {
  let live;
  try {
    live = await sessions.liveSessions();
  } catch {
    return [];
  }
  const adopted = [];
  for (const s of live) {
    const name = s.instance;
    if (activeInstances.has(name)) continue;
    let instance = null;
    try {
      instance = await instances.getInstance(name);
    } catch {
      instance = { name, version: s.version };
    }
    const key = typeof s.key === 'string' ? s.key : makeKey(name);
    const adoptedAgent = s.agent === true;
    const buf = createBuffer(key, name, s.version ?? instance.version, s.account ?? null);
    buf.agent = adoptedAgent;
    buf.started_at = s.started_at ?? Date.now();
    buf.spawned_at = buf.started_at;
    buf.menu_at = s.menu_at ?? null;
    appendSystem(key, `[espectral] ${name} sigue abierto desde antes de reabrir el launcher (pid ${s.pid})`);
    activeInstances.add(name);
    if (!adoptedAgent) liveLaunches.add(key);
    try {
      presence.launchStarted({ instance: name, uuid: s.uuid, gameDir: s.game_dir, token: s.bridge_token ?? '' });
    } catch {
      /* bridge registration is best-effort */
    }
    if (adoptedAgent) {
      // The game is still the one an agent started: re-register it so
      // /api/agent/* and the mod relay keep working after a launcher restart.
      try {
        agentRuntime.registerLaunch({ key, instance: name, gameDir: s.game_dir ?? null });
      } catch {
        /* ignore */
      }
    }
    if (!adoptedAgent) void pushPresence(instance, buf.started_at);
    let done = false;
    runningProcesses.set(name, {
      child: null,
      pid: s.pid,
      key,
      finalize: () => {
        if (done) return;
        done = true;
        runningProcesses.delete(name);
        sessions.removeSession(name);
        const endedAt = Date.now();
        buf.running = false;
        buf.ended_at = endedAt;
        liveLaunches.delete(key);
        activeInstances.delete(name);
        if (adoptedAgent) {
          try {
            agentRuntime.releaseLaunch(key);
          } catch {
            /* ignore */
          }
        }
        try {
          presence.launchExited(name, s.bridge_token ?? '');
        } catch {
          /* ignore */
        }
        if (liveLaunches.size === 0) void pushClear();
        for (const { seq, line } of appendSystem(key, '[espectral] game exited')) {
          try {
            routeEmit('launch-log', { key, seq, line });
          } catch {
            /* ignore */
          }
        }
        try {
          routeEmit('launch-exit', {
            key,
            instance: name,
            account: buf.account,
            code: null,
            signal: null,
            marker: buf.menu_at != null,
            error: null,
            menu_ms: buf.menu_at ? buf.menu_at - buf.started_at : null,
            played_ms: endedAt - buf.started_at,
            spawn_ms: null,
            boot_ms: null,
            phases: {},
          });
        } catch {
          /* ignore */
        }
      },
    });
    console.log(`[launch] adopted running game ${name} (pid ${s.pid})`);
    adopted.push(name);
  }
  return adopted;
}

/** Test hook: forget in-memory launch state (a "fresh engine"). */
export function __resetLaunchState() {
  buffers.clear();
  liveLaunches.clear();
  activeInstances.clear();
  runningProcesses.clear();
  pendingStops.clear();
}
