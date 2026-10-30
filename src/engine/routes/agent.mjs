/**
 * Agent access routes.
 *
 * Two surfaces share this module:
 *
 *  - UI (`/api/agent-access`, normal CSRF header, no token): enable/disable
 *    agent access, edit the act-tier allowlist, rotate the token, and read the
 *    ready-to-paste MCP server entry pointing at `src/engine/mcp.mjs`.
 *
 *  - Agent (`/api/agent/*`, `Authorization: Bearer <agent-token>`): list
 *    instances/launches, launch/stop/observe a game, and relay tool calls to
 *    the in-game mod. `GET /api/agent/instances` reports `capture_tools`.
 *
 *  - Mod relay (`/api/bridge/agent/{poll,result,state}`): authenticated with
 *    the per-launch bridge token (`x-espectral-bridge`), served only for
 *    launches started with `agent: true`.
 *
 * Everything is inert unless `agent.enabled` is set.
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as agent from '../agent.mjs';
import * as bridge from '../bridge.mjs';
import * as instances from '../instances.mjs';
import { httpError } from '../error.mjs';
import { emit } from '../events.mjs';
import { dataDir } from '../config.mjs';
import { DEFAULT_PORT } from '../server.mjs';
import {
  activeInstances,
  bufferView,
  launchSummaries,
  startLaunch,
  stopLaunch,
} from './launch.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
/** Absolute path of the stdio shim the MCP config points at. */
export const MCP_SHIM_PATH = path.resolve(__dirname, '..', 'mcp.mjs');

/**
 * Ready-to-paste MCP server entry. `command` is the absolute node binary the
 * launcher itself runs on (process.execPath — the bundled one in a packaged
 * app), `args` the absolute shim, `env` the data dir + port the shim must talk
 * to. A relative path would be resolved against the MCP client's cwd, which is
 * the agent's project dir, not ours.
 */
export function mcpEntry() {
  return {
    command: process.execPath,
    args: [MCP_SHIM_PATH],
    env: {
      ESPECTRAL_DATA_DIR: dataDir(),
      ESPECTRAL_PORT: String(process.env.ESPECTRAL_PORT || DEFAULT_PORT),
    },
  };
}

/** `{enabled, allowed_servers, token_path, mcp}` — the token itself is never returned. */
export function accessStatus() {
  const cfg = agent.agentConfig();
  return {
    enabled: cfg.enabled,
    allowed_servers: cfg.allowed_servers,
    token_path: agent.tokenPath(),
    mcp: mcpEntry(),
  };
}

function queryParam(req, name, fallback = '') {
  try {
    const v = new URL(req.url, 'http://localhost').searchParams.get(name);
    return v === null ? fallback : v;
  } catch {
    return fallback;
  }
}

/** Throw 403 AGENT_DISABLED when agent access is off (relay included). */
function requireEnabled() {
  if (!agent.agentConfig().enabled) {
    throw httpError(403, 'AGENT_DISABLED', 'agent access is disabled (enable it in Settings → Agent access)');
  }
}

export async function register(app) {
  // -------------------------------------------------------------------------
  // UI: agent access config + token
  // -------------------------------------------------------------------------

  app.get('/api/agent-access', async () => accessStatus());

  app.put('/api/agent-access', async (req, res, params, body) => {
    const patch = body && typeof body === 'object' ? body : {};
    if (patch.enabled !== undefined && typeof patch.enabled !== 'boolean') {
      throw httpError(400, 'BAD_AGENT_ACCESS', 'enabled must be a boolean');
    }
    if (patch.allowed_servers !== undefined && !Array.isArray(patch.allowed_servers)) {
      throw httpError(400, 'BAD_AGENT_ACCESS', 'allowed_servers must be an array of host[:port] strings');
    }
    if (Array.isArray(patch.allowed_servers) && patch.allowed_servers.some((s) => typeof s !== 'string')) {
      throw httpError(400, 'BAD_AGENT_ACCESS', 'allowed_servers entries must be strings');
    }
    const cfg = agent.saveAgentConfig(patch);
    // Enabling is what creates the token; the UI copies the MCP config right
    // after, so it must exist by the time this response is read.
    if (cfg.enabled) agent.ensureToken();
    return accessStatus();
  });

  app.post('/api/agent-access/rotate', async () => {
    agent.writeToken();
    return accessStatus();
  });

  // -------------------------------------------------------------------------
  // Agent API (Bearer token)
  // -------------------------------------------------------------------------

  app.get('/api/agent/instances', async (req) => {
    agent.authenticate(req);
    const list = await instances.listInstances();
    return {
      instances: list.map((inst) => ({
        name: inst.name,
        version: inst.version,
        loader: inst.loader,
        capture_tools: inst.capture_tools === true,
        running: activeInstances.has(inst.name),
      })),
    };
  });

  app.get('/api/agent/launches', async (req) => {
    agent.authenticate(req);
    const rows = launchSummaries();
    const out = rows.map((row) => {
      const entry = agent.getLaunch(row.key);
      return {
        key: row.key,
        instance: row.instance,
        running: row.running,
        agent: row.agent === true,
        state: agent.stateOf(entry),
      };
    });
    return { launches: out };
  });

  app.post('/api/agent/launch', async (req, res, params, body) => {
    agent.authenticate(req);
    const b = body && typeof body === 'object' ? body : {};
    const name = typeof b.instance === 'string' ? b.instance : '';
    if (!name) throw httpError(400, 'BAD_REQUEST', 'instance is required');
    const options = agent.normalizeLaunchOptions(b, { forceAgent: true });
    const waitFor = b.wait_for ?? null;
    if (waitFor !== null && waitFor !== 'title' && waitFor !== 'in_world') {
      throw httpError(400, 'BAD_REQUEST', "wait_for must be 'title' or 'in_world'");
    }
    let timeoutS = Number(b.timeout_s ?? 180);
    if (!Number.isFinite(timeoutS) || timeoutS <= 0) timeoutS = 180;
    timeoutS = Math.min(Math.max(5, timeoutS), 600);

    const reply = await startLaunch(name, {
      account: b.account,
      agent: true,
      join: options.join,
      window: options.window,
    });
    if (!waitFor) {
      const entry = agent.getLaunch(reply.key);
      return { key: reply.key, state: agent.stateOf(entry) };
    }
    const entry = await waitForState(reply.key, waitFor, timeoutS * 1000);
    return { key: reply.key, state: entry };
  });

  app.post('/api/agent/stop', async (req, res, params, body) => {
    agent.authenticate(req);
    const key = body?.key;
    if (typeof key !== 'string' || key.length === 0) {
      throw httpError(400, 'BAD_REQUEST', 'key is required');
    }
    const row = await requireLaunch(key);
    // A key addresses ONE launch. Once it has exited, the same instance may
    // already have been relaunched under a new key — stopping by the old key
    // must not kill the new game.
    if (!row.running) return { ok: true, already_stopped: true };
    await stopLaunch(row.instance);
    return { ok: true };
  });

  app.get('/api/agent/state', async (req) => {
    agent.authenticate(req);
    const key = queryParam(req, 'key');
    const row = await requireLaunch(key);
    const entry = agent.getLaunch(key);
    return { key, instance: row.instance, running: row.running, state: agent.stateOf(entry) };
  });

  app.get('/api/agent/logs', async (req) => {
    agent.authenticate(req);
    const key = queryParam(req, 'key');
    const row = await requireLaunch(key);
    let cursor = Number(queryParam(req, 'cursor', '0'));
    if (!Number.isInteger(cursor) || cursor < 0) cursor = 0;
    const view = bufferView(key, cursor);
    // bufferView answers running:false for a key whose log ring was already
    // pruned (an adopted agent game outlives its buffer). The registry row is
    // the authority on whether the game is still up — otherwise the agent
    // reads "exited" while /api/agent/state says it is running.
    if (!view.running && row.running) return { ...view, running: true };
    return view;
  });

  app.post('/api/agent/call', async (req, res, params, body) => {
    agent.authenticate(req);
    const b = body && typeof body === 'object' ? body : {};
    const key = typeof b.key === 'string' ? b.key : '';
    if (!key) throw httpError(400, 'BAD_REQUEST', 'key is required');
    const entry = agent.getLaunch(key);
    if (!entry) throw httpError(404, 'NO_LAUNCH', `unknown launch key: ${key}`);
    if (!entry.agent) throw httpError(409, 'NOT_AGENT_LAUNCH', 'this launch was not started with agent: true');
    if (!entry.running) throw httpError(502, 'GAME_GONE', 'the game is no longer running');
    let timeoutS = Number(b.timeout_s ?? agent.CALL_TIMEOUT_S);
    if (!Number.isFinite(timeoutS) || timeoutS <= 0) timeoutS = agent.CALL_TIMEOUT_S;
    // Floor as well as ceiling: a sub-second timeout_s would abort a call before
    // the mod could answer (callTool itself clamps to >=1 s; keep both in sync).
    timeoutS = Math.min(Math.max(timeoutS, 1), agent.MAX_CALL_TIMEOUT_S);
    const { promise } = agent.callTool(key, b.tool, b.args ?? {}, { timeoutMs: Math.round(timeoutS * 1000) });
    const data = await promise;
    return { ok: true, data: agent.rewriteFileResults(data, entry.gameDir) };
  });

  // -------------------------------------------------------------------------
  // Mod relay (per-launch bridge token)
  // -------------------------------------------------------------------------

  app.post('/api/bridge/agent/poll', async (req, res, params, body) => {
    const owner = bridge.authenticate(req.headers[bridge.BRIDGE_HEADER] ?? '');
    if (!owner) throw httpError(403, 'BRIDGE_AUTH', 'missing or invalid x-espectral-bridge token');
    requireEnabled();
    const entry = agent.launchForBridge(owner, body?.key);
    const commands = await agent.awaitCommands(entry, {
      timeoutMs: agent.POLL_TIMEOUT_MS,
      signal: abortSignalOf(res),
    });
    return { commands, policy: agent.policy() };
  });

  app.post('/api/bridge/agent/result', async (req, res, params, body) => {
    const owner = bridge.authenticate(req.headers[bridge.BRIDGE_HEADER] ?? '');
    if (!owner) throw httpError(403, 'BRIDGE_AUTH', 'missing or invalid x-espectral-bridge token');
    requireEnabled();
    const entry = agent.launchForBridge(owner, body?.key);
    const id = typeof body?.id === 'string' ? body.id : '';
    if (!id) throw httpError(400, 'BAD_REQUEST', 'id is required');
    const accepted = agent.completeCommand(entry, {
      id,
      ok: body?.ok === true,
      data: body?.data,
      error: body?.error,
    });
    return { ok: true, accepted };
  });

  app.post('/api/bridge/agent/state', async (req, res, params, body) => {
    const owner = bridge.authenticate(req.headers[bridge.BRIDGE_HEADER] ?? '');
    if (!owner) throw httpError(403, 'BRIDGE_AUTH', 'missing or invalid x-espectral-bridge token');
    requireEnabled();
    const entry = agent.launchForBridge(owner, body?.key);
    const state = agent.setState(entry, body ?? {});
    try {
      emit('agent-state', { key: entry.key, instance: entry.instance, ...state });
    } catch {
      /* the SSE registry must never break the relay */
    }
    return { ok: true };
  });
}

/**
 * Resolve a launch key to its `/api/launches` row, 404 when unknown. The agent
 * routes address launches by key only (an instance name is ambiguous: a
 * relaunch creates a new key while the old buffer lingers for backfill).
 */
async function requireLaunch(key) {
  if (!key) throw httpError(400, 'BAD_REQUEST', 'key is required');
  const row = launchSummaries().find((l) => l.key === key);
  if (row) return row;
  // An agent launch outlives its log buffer (the buffer is LRU-pruned 10 min
  // after exit, while the game may still be relayed/adopted). The registry is
  // the authoritative "this key is a live agent launch" source.
  const entry = agent.getLaunch(key);
  if (entry) {
    return { key, instance: entry.instance, running: entry.running, agent: true, started_at: null };
  }
  throw httpError(404, 'NO_LAUNCH', `unknown launch key: ${key}`);
}

/**
 * Wait for the mod's state to satisfy `want` ('title' → phase title/in_world,
 * 'in_world' → phase in_world). Polls the registry (the mod reports over HTTP;
 * there is no in-process event to subscribe to) and gives up after `timeoutMs`
 * with AGENT_TIMEOUT, failing fast when the launch disappears.
 */
async function waitForState(key, want, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  const wanted = want === 'in_world' ? ['in_world'] : ['title', 'in_world'];
  for (;;) {
    const entry = agent.getLaunch(key);
    if (!entry) throw httpError(502, 'GAME_GONE', 'the game exited before reaching the requested state');
    const state = agent.stateOf(entry);
    if (state && wanted.includes(state.phase)) return state;
    if (Date.now() >= deadline) {
      throw httpError(
        504,
        'AGENT_TIMEOUT',
        `game did not reach '${want}' within ${Math.round(timeoutMs / 1000)} s (last phase: ${state?.phase ?? 'none'})`
      );
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

/**
 * Abort signal for a parked long-poll, bound to the RESPONSE socket.
 *
 * Deliberately not `req`'s 'close': since Node 16 that event fires as soon as
 * the request body has been consumed, i.e. immediately for our empty POST —
 * every poll would answer [] at once and the queue would starve. `res` 'close'
 * fires when the response finished or the connection dropped; while the
 * handler is parked the response cannot have finished, so it means hang-up.
 */
function abortSignalOf(res) {
  const ac = new AbortController();
  if (!res || typeof res.once !== 'function') return ac.signal;
  // A response that already finished or dropped cannot park a poll; treat it
  // as an immediate hang-up so the waiter never enters the set at all.
  if (res.writableEnded || res.destroyed) {
    ac.abort();
    return ac.signal;
  }
  res.once('close', () => ac.abort());
  return ac.signal;
}
