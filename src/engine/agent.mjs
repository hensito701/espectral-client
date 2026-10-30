/**
 * Agent access — launch-option validation, token/config, the per-launch
 * command queue and the relay protocol state.
 *
 * Two peers talk here:
 *  - the MCP shim (src/engine/mcp.mjs) over `POST /api/agent/*`, Bearer-authed
 *    with the rotatable `<data_dir>/agent-token`;
 *  - the in-game mod (espectral-menu) over the existing bridge channel,
 *    `x-espectral-bridge: <per-launch token>`, long-polling for commands.
 *
 * The engine is the only component with a stable address, so it owns
 * multi-client addressing (by launch key), auth and the lifetime of an
 * in-flight call: a waiter is released with GAME_GONE the moment the game
 * exits, never left hanging on a dead socket.
 *
 * Everything is inert unless `agent.enabled` is set in the config (the routes
 * answer 403 AGENT_DISABLED otherwise) and a launch was tagged `agent: true`.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { httpError } from './error.mjs';
import { dataDir, loadConfig, saveConfig } from './config.mjs';

// ---------------------------------------------------------------------------
// Policy constants
// ---------------------------------------------------------------------------

/**
 * Public Espectral hosts where agent *control* must never work, whatever the
 * allowlist says. Domain suffixes — the mod mirrors this list and additionally
 * blocks the IPs they resolve to (DNS answers are not known to the engine).
 */
export const BLOCKED_HOSTS = Object.freeze(['uhce.top', 'espectral.es']);

/** Long-poll budget for the mod's command channel (contract: ≤25 s). */
export const POLL_TIMEOUT_MS = 25_000;
/** Default `call` timeout when the caller does not pass one (contract: 30 s). */
export const CALL_TIMEOUT_S = 30;
export const MAX_CALL_TIMEOUT_S = 300;
/** Commands queued for a game that stopped polling — bounded so a dead launch
 *  cannot grow a queue forever. */
export const MAX_QUEUE = 100;

// ---------------------------------------------------------------------------
// Config (agent.enabled + agent.allowed_servers)
// ---------------------------------------------------------------------------

export const DEFAULT_AGENT_CONFIG = Object.freeze({
  enabled: false,
  allowed_servers: [],
});

/** Set config path — one place so the shim and routes agree. */
export function tokenPath() {
  return path.join(dataDir(), 'agent-token');
}

const TOKEN_RE = /^[0-9a-f]{64}$/;

/** Current config view (never throws; defaults on an unreadable config). */
export function agentConfig() {
  let cfg = {};
  try {
    cfg = loadConfig();
  } catch {
    cfg = {};
  }
  const raw = cfg.agent && typeof cfg.agent === 'object' ? cfg.agent : {};
  return {
    enabled: raw.enabled === true,
    allowed_servers: normalizeAllowedServers(raw.allowed_servers),
  };
}

/**
 * `host[:port]` entries, lowercased, deduped, order preserved. Invalid entries
 * are dropped rather than failing the whole save: a typo in one row must not
 * silently discard the rest of the user's allowlist.
 */
export function normalizeAllowedServers(list) {
  if (!Array.isArray(list)) return [];
  const out = [];
  const seen = new Set();
  for (const entry of list) {
    if (typeof entry !== 'string') continue;
    const v = entry.trim().toLowerCase().replace(/\.$/, '');
    if (!v || v.length > 255) continue;
    if (!/^(\[[0-9a-f:.]+\]|[a-z0-9._-]+)(:\d{1,5})?$/.test(v)) continue;
    if (seen.has(v)) continue;
    seen.add(v);
    out.push(v);
  }
  return out;
}

/** Merge an agent patch into the config document and persist it. */
export function saveAgentConfig(patch = {}) {
  const current = loadConfig();
  const agent = { ...DEFAULT_AGENT_CONFIG, ...(current.agent ?? {}) };
  if (patch.enabled !== undefined) agent.enabled = patch.enabled === true;
  if (patch.allowed_servers !== undefined) {
    agent.allowed_servers = normalizeAllowedServers(patch.allowed_servers);
  }
  saveConfig({ agent });
  return agentConfig();
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

/** Read the token file; null when missing/blank. Never logged, never returned. */
export function readToken() {
  try {
    const raw = fs.readFileSync(tokenPath(), 'utf8').trim();
    return TOKEN_RE.test(raw) ? raw : null;
  } catch {
    return null;
  }
}

/**
 * Write a fresh token (0600). Called when agent access is enabled and on every
 * rotate — rotating while a client is connected is exactly how a user revokes
 * access, so the old file is replaced, never appended.
 */
export function writeToken() {
  const token = crypto.randomBytes(32).toString('hex');
  const file = tokenPath();
  fs.mkdirSync(path.dirname(file), { recursive: true });
  // Atomic replace: the shim re-reads this file on EVERY call, so a rotate that
  // truncated in place could be observed as a blank/invalid token (spurious 401
  // until the next rotate). Write a sibling tmp with 0600 and rename over it —
  // rename is atomic on both POSIX and NTFS. `mode` only applies on creation and
  // a pre-existing file may carry looser bits, so chmod the tmp before the swap.
  const tmp = `${file}.${process.pid}.${crypto.randomBytes(4).toString('hex')}.tmp`;
  try {
    fs.writeFileSync(tmp, token + '\n', { mode: 0o600 });
    try {
      fs.chmodSync(tmp, 0o600);
    } catch {
      /* chmod is a no-op on some filesystems (FAT/exFAT); the token still works */
    }
    fs.renameSync(tmp, file);
  } catch (e) {
    try {
      fs.rmSync(tmp, { force: true });
    } catch {
      /* best effort */
    }
    throw e;
  }
  return token;
}

/** Token, creating it on first use (enabling agent access creates it eagerly). */
export function ensureToken() {
  return readToken() ?? writeToken();
}

/**
 * Constant-time compare of a presented Bearer token against the stored one.
 * Length is compared first because timingSafeEqual throws on a mismatch; the
 * length itself is a fixed 64-hex contract, so nothing secret leaks.
 */
export function tokenMatches(token) {
  if (typeof token !== 'string' || !TOKEN_RE.test(token)) return false;
  const stored = readToken();
  if (!stored) return false;
  const a = Buffer.from(stored, 'utf8');
  const b = Buffer.from(token, 'utf8');
  if (a.length !== b.length) return false;
  return crypto.timingSafeEqual(a, b);
}

/**
 * Bearer auth for every `/api/agent/*` route. `AGENT_DISABLED` (403) comes
 * before auth so a disabled engine does not confirm a token's validity.
 */
export function authenticate(req) {
  if (!agentConfig().enabled) {
    throw httpError(403, 'AGENT_DISABLED', 'agent access is disabled (enable it in Settings → Agent access)');
  }
  const header = req?.headers?.authorization ?? '';
  const m = /^Bearer\s+(.+)$/i.exec(String(header));
  if (!m || !tokenMatches(m[1].trim())) {
    throw httpError(401, 'AGENT_AUTH', 'missing or invalid Authorization: Bearer <agent-token>');
  }
}

// ---------------------------------------------------------------------------
// Launch options (agent / join / window)
// ---------------------------------------------------------------------------
// Validation lives here (not in the launch route) so the UI route and the
// agent route share one implementation and one set of error codes.

export const MIN_WINDOW = 64;
export const MAX_WINDOW_W = 7680;
export const MAX_WINDOW_H = 4320;

/** Save-folder names: a single path component under `saves/`. */
const WORLD_RE = /^[^/\\:*?"<>|\u0000-\u001f]{1,255}$/;
/**
 * host[:port] with optional [IPv6]. Deliberately strict: the value reaches a
 * JVM argument, and a stray space/quote would split the command line.
 */
const SERVER_RE = /^(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9]([A-Za-z0-9.-]{0,253}))(:\d{1,5})?$/;

/**
 * Normalize `join` to `{world}` or `{server}`. Both/neither → 400.
 * Returns null when absent (the common case).
 */
export function normalizeJoin(join) {
  if (join === undefined || join === null) return null;
  if (typeof join !== 'object' || Array.isArray(join)) {
    throw httpError(400, 'BAD_JOIN', 'join must be an object: {world} or {server}');
  }
  const hasWorld = join.world !== undefined && join.world !== null;
  const hasServer = join.server !== undefined && join.server !== null;
  if (hasWorld && hasServer) {
    throw httpError(400, 'BAD_JOIN', 'join takes either {world} or {server}, not both');
  }
  if (hasWorld) {
    if (typeof join.world !== 'string' || !WORLD_RE.test(join.world) || join.world === '.' || join.world === '..') {
      throw httpError(400, 'BAD_JOIN', 'join.world must be an existing save folder name (no slashes or "..")');
    }
    // A value starting with '-' would be read as the NEXT OPTION by the game's
    // arg parser instead of as the --quickPlaySingleplayer value (same reason
    // the server form is regex-anchored on an alphanumeric).
    if (join.world.includes('..') || join.world.startsWith('-')) {
      throw httpError(400, 'BAD_JOIN', 'join.world may not contain ".." or start with "-"');
    }
    return { world: join.world };
  }
  if (hasServer) {
    const server = typeof join.server === 'string' ? join.server.trim() : '';
    if (!SERVER_RE.test(server)) {
      throw httpError(400, 'BAD_JOIN', 'join.server must be host[:port] (hostname, IPv4 or [IPv6])');
    }
    if (server.includes('..')) {
      throw httpError(400, 'BAD_JOIN', 'join.server is not a valid host');
    }
    return { server };
  }
  throw httpError(400, 'BAD_JOIN', 'join must carry {world} or {server}');
}

/**
 * Normalize `window` to `{width, height, hidden}`. `hidden` is agent-only —
 * a plain UI launch with hidden:true is a 400 rather than a silent no-op.
 */
export function normalizeWindow(win, { agent = false } = {}) {
  if (win === undefined || win === null) return null;
  if (typeof win !== 'object' || Array.isArray(win)) {
    throw httpError(400, 'BAD_WINDOW', 'window must be an object: {width, height, hidden?}');
  }
  const width = Number(win.width);
  const height = Number(win.height);
  if (!Number.isInteger(width) || width < MIN_WINDOW || width > MAX_WINDOW_W) {
    throw httpError(400, 'BAD_WINDOW', `window.width must be an integer ${MIN_WINDOW}…${MAX_WINDOW_W}`);
  }
  if (!Number.isInteger(height) || height < MIN_WINDOW || height > MAX_WINDOW_H) {
    throw httpError(400, 'BAD_WINDOW', `window.height must be an integer ${MIN_WINDOW}…${MAX_WINDOW_H}`);
  }
  const hidden = win.hidden === true;
  if (hidden && !agent) {
    throw httpError(400, 'BAD_WINDOW', 'window.hidden is only available on agent launches');
  }
  return { width, height, hidden };
}

/** `agent` must be a boolean when present. */
export function normalizeAgent(value) {
  if (value === undefined || value === null) return false;
  if (typeof value !== 'boolean') {
    throw httpError(400, 'BAD_AGENT', 'agent must be true or false');
  }
  return value;
}

/**
 * Validate the whole launch-option bundle at once. `agent` in the body is also
 * honored for agent launches (the agent route forces it).
 */
export function normalizeLaunchOptions(body, { forceAgent = false } = {}) {
  const b = body && typeof body === 'object' ? body : {};
  const agent = forceAgent || normalizeAgent(b.agent);
  const join = normalizeJoin(b.join);
  const window = normalizeWindow(b.window, { agent });
  return { agent, join, window };
}

/**
 * Game-arg fragment for the validated options (order asserted by tests):
 *   --quickPlaySingleplayer <world> | --quickPlayMultiplayer <server>
 *   --width W --height H
 * Placeholders (${quickPlaySingleplayer}, ${resolution_width}, …) carry the
 * same values on the NeoForge feature path (see launch.buildArgv).
 */
export function gameArgFragment(options) {
  const out = [];
  if (options?.join?.world) out.push('--quickPlaySingleplayer', options.join.world);
  else if (options?.join?.server) out.push('--quickPlayMultiplayer', options.join.server);
  if (options?.window) out.push('--width', String(options.window.width), '--height', String(options.window.height));
  return out;
}

// ---------------------------------------------------------------------------
// Launch registry + command queue
// ---------------------------------------------------------------------------

/**
 * Per launch key:
 *   { key, instance, agent, gameDir, running,
 *     queue: [{id, tool, args, timeout_ms, enqueuedAt}],
 *     waiters: Set<pollWaiter>,      // parked mod polls
 *     inflight: Map<id, callWaiter>, // calls awaiting a result
 *     state: null | {...mod report}, stateAt: number|0 }
 */
const launches = new Map();
/** key -> array of released waiter resolvers (never reused across launches). */
let seqCounter = 0;

/** Fresh command id (unique per process; the mod correlates by it). */
function nextCommandId() {
  seqCounter += 1;
  return `c${Date.now().toString(36)}${seqCounter.toString(36)}`;
}

/**
 * Register an agent-capable launch. Called by the launch route when the child
 * spawns (agent launches only — a plain launch never enters this registry and
 * every agent route answers 409 NOT_AGENT_LAUNCH for it).
 */
export function registerLaunch({ key, instance, gameDir = null }) {
  // Called twice per launch by design: once when the key is minted (so a call
  // that races the spawn still finds the key) and again on spawn with the
  // resolved gameDir (per-account profile dirs are only known then). MERGE
  // into the existing entry: replacing it would orphan everything queued in
  // between — the mod resolves the new entry, the parked poll and the caller
  // wait on the old one, and every call issued before spawn times out with
  // its command lost.
  const existing = launches.get(key);
  if (existing) {
    existing.instance = instance;
    if (gameDir) existing.gameDir = gameDir;
    existing.running = true;
    return existing;
  }
  const entry = {
    key,
    instance,
    agent: true,
    gameDir,
    running: true,
    queue: [],
    waiters: new Set(),
    inflight: new Map(),
    state: null,
    stateAt: 0,
  };
  launches.set(key, entry);
  return entry;
}

/**
 * Mark a launch gone: fail every pending call (GAME_GONE), release parked
 * polls with an empty reply (so the mod's thread unblocks instead of polling a
 * dead key), and drop the queue. Idempotent.
 */
export function releaseLaunch(key, { code = 'GAME_GONE', message = 'game exited' } = {}) {
  const entry = launches.get(key);
  if (!entry) return false;
  entry.running = false;
  const err = { code, message };
  for (const [, waiter] of entry.inflight) {
    try {
      waiter.reject(httpError(502, err.code, err.message));
    } catch {
      /* a waiter that already settled is ignored */
    }
  }
  entry.inflight.clear();
  entry.queue.length = 0;
  for (const waiter of [...entry.waiters]) {
    try {
      waiter.reply([]);
    } catch {
      /* ignore */
    }
  }
  entry.waiters.clear();
  launches.delete(key);
  return true;
}

/** True when `key` is a live agent launch. */
export function isAgentLaunch(key) {
  const entry = launches.get(key);
  return !!(entry && entry.agent && entry.running);
}

/** The agent registry entry for a key (or null). */
export function getLaunch(key) {
  return launches.get(key) ?? null;
}

/** All live agent launches (GET /api/agent/launches / test helper). */
export function allLaunches() {
  return [...launches.values()].map((e) => ({
    key: e.key,
    instance: e.instance,
    running: e.running,
    agent: e.agent,
    state: e.state,
    state_at: e.stateAt || null,
  }));
}

/**
 * Resolve a bridge-authenticated launch to its agent registry entry.
 * `bridgeEntry` is what bridge.authenticate returned ({instance, token, …}).
 * Non-agent launches → 409 NOT_AGENT_LAUNCH: the mod token is valid, but that
 * game was never started in agent mode.
 */
export function launchForBridge(bridgeEntry, key) {
  if (!bridgeEntry) throw httpError(403, 'BRIDGE_AUTH', 'missing or invalid x-espectral-bridge token');
  const entry = key ? launches.get(key) : findByBridge(bridgeEntry);
  if (!entry) {
    // Distinguish "bridge token valid but this launch is not an agent launch"
    // from "unknown launch": the former is the contract's 409.
    throw httpError(409, 'NOT_AGENT_LAUNCH', 'this launch was not started with agent: true');
  }
  // A key names a launch, and the bridge token names a game: they must agree.
  // Without this check a mod running under ANY launch (a plain player game
  // included, it holds a valid bridge token too) could poll another agent
  // launch's commands or post forged results for it — keys are not secret
  // (they come back from /api/launches).
  if (key && bridgeEntry.instance !== entry.instance) {
    throw httpError(403, 'BRIDGE_AUTH', 'this bridge token does not own that launch key');
  }
  if (!entry.agent) throw httpError(409, 'NOT_AGENT_LAUNCH', 'this launch was not started with agent: true');
  if (!entry.running) throw httpError(409, 'NOT_AGENT_LAUNCH', 'this launch is no longer running');
  return entry;
}

/**
 * Find the registry entry matching a bridge identity. The mod does not know
 * its launch key (it only holds the bridge token), so the mapping is by
 * instance + gameDir: the launch route registers both.
 */
function findByBridge(bridgeEntry) {
  for (const entry of launches.values()) {
    if (entry.instance === bridgeEntry.instance) return entry;
  }
  return null;
}

/**
 * Enqueue a command for a launch and return `{command, promise}`. The promise
 * settles when the mod posts the result (resolve) or the call times out / the
 * game exits (reject with AGENT_TIMEOUT / GAME_GONE).
 */
export function callTool(key, tool, args = {}, { timeoutMs = CALL_TIMEOUT_S * 1000 } = {}) {
  const entry = launches.get(key);
  if (!entry) throw httpError(404, 'NO_LAUNCH', `unknown launch key: ${key}`);
  if (!entry.agent) throw httpError(409, 'NOT_AGENT_LAUNCH', 'this launch was not started with agent: true');
  if (!entry.running) throw httpError(502, 'GAME_GONE', 'the game is no longer running');
  if (typeof tool !== 'string' || tool.length === 0) {
    throw httpError(400, 'BAD_REQUEST', 'tool is required');
  }
  if (entry.queue.length >= MAX_QUEUE) {
    throw httpError(429, 'QUEUE_FULL', 'too many commands queued for this launch (is the mod polling?)');
  }
  const id = nextCommandId();
  const command = {
    id,
    tool,
    args: args && typeof args === 'object' && !Array.isArray(args) ? args : {},
    timeout_ms: Math.min(Math.max(1000, timeoutMs), MAX_CALL_TIMEOUT_S * 1000),
  };
  const promise = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      entry.inflight.delete(id);
      // The caller has given up, so the command must not sit in the queue: the
      // mod would pick it up on its next poll (a `record_start` minutes late)
      // and post a result nobody is waiting for.
      const queued = entry.queue.indexOf(command);
      if (queued !== -1) entry.queue.splice(queued, 1);
      reject(httpError(504, 'AGENT_TIMEOUT', `no result for '${tool}' within ${Math.round(command.timeout_ms / 1000)} s`));
    }, command.timeout_ms);
    if (timer.unref) timer.unref();
    entry.inflight.set(id, {
      tool,
      resolve: (data) => {
        clearTimeout(timer);
        entry.inflight.delete(id);
        resolve(data);
      },
      reject: (err) => {
        clearTimeout(timer);
        entry.inflight.delete(id);
        reject(err);
      },
    });
  });
  entry.queue.push(command);
  releasePollers(entry);
  return { command, promise };
}

/**
 * Hand queued commands to the parked polls. With NO waiter the queue must
 * stay put: draining it there would silently discard every command issued
 * between two polls (the mod polls again within 25 s, the command must wait
 * for it, not vanish).
 */
function releasePollers(entry) {
  if (entry.queue.length === 0 || entry.waiters.size === 0) return;
  // Exactly ONE waiter gets the batch. A launch has one mod relay thread, so a
  // second parked poll is a duplicate/rogue socket; handing it the same
  // commands would execute them twice (two clicks, two record_starts) and two
  // results would race for one call.
  const waiter = entry.waiters.values().next().value;
  const commands = entry.queue.splice(0, entry.queue.length);
  try {
    waiter.reply(commands);
  } catch {
    /* ignore: the waiter settles itself and re-queues nothing */
  }
}

/**
 * Long-poll: resolves with the queued commands, or with `[]` after
 * `timeoutMs` / when the game exits. Returns a promise the route awaits.
 */
export function awaitCommands(entry, { timeoutMs = POLL_TIMEOUT_MS, signal = null } = {}) {
  if (!entry) return Promise.resolve([]);
  if (signal?.aborted) {
    // The socket is already gone: take nothing out of the queue, or the batch
    // would be dropped instead of waiting for the next poll.
    return Promise.resolve([]);
  }
  if (entry.queue.length === 0) {
    return new Promise((resolve) => {
      const waiter = {
        settled: false,
        reply: (commands) => {
          if (waiter.settled) return;
          waiter.settled = true;
          clearTimeout(timer);
          entry.waiters.delete(waiter);
          resolve(commands);
        },
      };
      const timer = setTimeout(() => waiter.reply([]), Math.max(0, timeoutMs));
      if (timer.unref) timer.unref();
      entry.waiters.add(waiter);
      // A client that hangs up (window closed, engine shutting down) must not
      // leave a parked waiter consuming a queue slot until the timer fires.
      if (signal) {
        const onAbort = () => waiter.reply([]);
        if (signal.aborted) onAbort();
        else signal.addEventListener('abort', onAbort, { once: true });
      }
    });
  }
  const commands = entry.queue.splice(0, entry.queue.length);
  return Promise.resolve(commands);
}

/** Resolve an in-flight call with the mod's result payload. */
export function completeCommand(entry, { id, ok, data, error }) {
  const waiter = entry?.inflight?.get(id);
  if (!waiter) return false; // late/duplicate result or a timed-out call
  if (ok === true) waiter.resolve(data ?? {});
  else {
    const code = typeof error?.code === 'string' && /^[A-Z][A-Z0-9_]*$/.test(error.code) ? error.code : 'FAILED';
    const message = typeof error?.message === 'string' ? error.message : `'${waiter.tool}' failed`;
    waiter.reject(httpError(502, code, message));
  }
  return true;
}

/** Store the mod's latest state report (bridged to SSE as `agent-state`). */
export function setState(entry, state) {
  if (!entry) return null;
  const next = {
    phase: typeof state?.phase === 'string' ? state.phase : null,
    screen: typeof state?.screen === 'string' ? state.screen : null,
    server: typeof state?.server === 'string' ? state.server : null,
    world: typeof state?.world === 'string' ? state.world : null,
    context: typeof state?.context === 'string' ? state.context : null,
    paused: state?.paused === true,
    recording: state?.recording === true,
  };
  entry.state = next;
  entry.stateAt = Date.now();
  return { ...next, updated_at: entry.stateAt };
}

/** `state` view for a launch: the last report plus `updated_at`, or null. */
export function stateOf(entry) {
  if (!entry || !entry.state) return null;
  return { ...entry.state, updated_at: entry.stateAt };
}

/** Policy block handed to the mod on every poll (tiers enforced mod-side). */
export function policy() {
  return {
    allowed_servers: agentConfig().allowed_servers,
    blocked_hosts: [...BLOCKED_HOSTS],
  };
}

// ---------------------------------------------------------------------------
// File results
// ---------------------------------------------------------------------------

export const CAPTURES_DIR = 'espectral-captures';

/**
 * Rewrite every `*_path` field of a mod result from gameDir-relative to
 * absolute, refusing anything that escapes `<gameDir>/espectral-captures/`.
 *
 * Containment is checked on the RESOLVED path (symlinks/`..` cannot win) and
 * must be inside the captures directory — the mod writes there, and a result
 * naming `../options.txt` is either a bug or a compromised game process; both
 * must not hand the agent a launcher file. `preview_path` additionally gets
 * an inline `preview: {mime, base64}` so MCP image content needs no second
 * request.
 */
export function rewriteFileResults(data, gameDir) {
  if (!data || typeof data !== 'object' || !gameDir) return data;
  const root = path.resolve(gameDir, CAPTURES_DIR);
  const out = Array.isArray(data) ? [] : {};
  let preview = null;
  for (const [k, v] of Object.entries(data)) {
    if (typeof v === 'string' && k.endsWith('_path')) {
      // The mod reports paths relative to the gameDir ("espectral-captures/x.png");
      // absolute paths are accepted too (path.resolve keeps them).
      const abs = resolveCapturePath(v, gameDir, root);
      out[k] = abs;
      if (k === 'preview_path' && abs) preview = readPreview(abs);
    } else {
      out[k] = v;
    }
  }
  if (preview) out.preview = preview;
  return out;
}

/**
 * Absolute path of `rel` (relative to `gameDir`) when it stays inside
 * `allowedRoot`; null otherwise. A returned null (rather than a throw) keeps
 * one bad field from failing the whole result — the agent sees `path: null`
 * instead of a launcher file, and a `../` escape can never resolve.
 */
export function resolveCapturePath(rel, gameDir, allowedRoot = null) {
  if (typeof rel !== 'string' || rel.length === 0) return null;
  const base = path.resolve(allowedRoot ?? path.join(gameDir, CAPTURES_DIR));
  const abs = path.resolve(gameDir, rel);
  const cmp = process.platform === 'win32';
  const a = cmp ? abs.toLowerCase() : abs;
  const b0 = cmp ? base.toLowerCase() : base;
  const withSep = cmp ? `${b0}${path.sep}` : `${base}${path.sep}`;
  if (a !== b0 && !a.startsWith(withSep)) return null;
  // Lexical containment is not enough: a SYMLINK inside the captures dir (or a
  // captured path that traverses one) resolves to a real file elsewhere. When
  // the real paths can be resolved, the containment must hold there too.
  const realBase = realpathOrNull(base);
  if (realBase) {
    const realAbs = realpathOrNull(abs);
    if (realAbs) {
      const ra = cmp ? realAbs.toLowerCase() : realAbs;
      const rb = cmp ? realBase.toLowerCase() : realBase;
      const rSep = `${rb}${path.sep}`;
      if (ra !== rb && !ra.startsWith(rSep)) return null;
    }
  }
  return abs;
}

/** realpath that returns null instead of throwing (missing file / no perms). */
function realpathOrNull(p) {
  try {
    return fs.realpathSync.native ? fs.realpathSync.native(p) : fs.realpathSync(p);
  } catch {
    return null;
  }
}

/** Inline preview for an image file (bounded so a 4K PNG cannot bloat a reply). */
export const MAX_PREVIEW_BYTES = 4 * 1024 * 1024;

function readPreview(file) {
  let fd = null;
  try {
    fd = fs.openSync(file, 'r');
    const st = fs.fstatSync(fd);
    if (!st.isFile() || st.size === 0 || st.size > MAX_PREVIEW_BYTES) return null;
    // Read at most MAX_PREVIEW_BYTES + 1: the stat above is a hint and the file
    // can grow between it and the read (the game writes these), so a bounded
    // read never pulls more than the cap into memory.
    const buf = Buffer.alloc(Math.min(st.size, MAX_PREVIEW_BYTES) + 1);
    const read = fs.readSync(fd, buf, 0, buf.length, 0);
    if (read === 0 || read > MAX_PREVIEW_BYTES) return null;
    const ext = path.extname(file).toLowerCase();
    const mime = ext === '.jpg' || ext === '.jpeg' ? 'image/jpeg' : ext === '.webp' ? 'image/webp' : 'image/png';
    return { mime, base64: buf.subarray(0, read).toString('base64') };
  } catch {
    return null;
  } finally {
    if (fd !== null) {
      try {
        fs.closeSync(fd);
      } catch {
        /* already closed */
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Test hooks
// ---------------------------------------------------------------------------

/** Test hook: wipe module state. */
export function _reset() {
  launches.clear();
  seqCounter = 0;
}
