import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-agent-api-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { start } = await import('../src/engine/server.mjs');
const agent = await import('../src/engine/agent.mjs');
const bridge = await import('../src/engine/bridge.mjs');

let server;
let base;

test.before(async () => {
  server = await start(0);
  base = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  agent._reset();
  bridge._reset();
  if (server) await new Promise((r) => server.close(r));
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const CSRF = { 'Content-Type': 'application/json', 'x-espectral-client': '1' };

async function put(body) {
  const res = await fetch(`${base}/api/agent-access`, { method: 'PUT', headers: CSRF, body: JSON.stringify(body) });
  return { status: res.status, json: await res.json() };
}

async function agentGet(route, token) {
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  const res = await fetch(base + route, { headers });
  return { status: res.status, json: await res.json() };
}

async function bridgePost(route, token, body) {
  const res = await fetch(base + route, {
    method: 'POST',
    headers: { ...CSRF, 'x-espectral-bridge': token },
    body: JSON.stringify(body ?? {}),
  });
  return { status: res.status, json: await res.json() };
}

/** A fake registered agent launch + its bridge token. */
function attachAgentLaunch(key, instance = 'inst-agent') {
  const gameDir = path.join(dataDir, 'instances', instance);
  fs.mkdirSync(path.join(gameDir, 'espectral-captures'), { recursive: true });
  const token = bridge.mintToken();
  bridge.attach({ instance, uuid: 'a'.repeat(32), gameDir, token });
  agent.registerLaunch({ key, instance, gameDir });
  return { token, gameDir };
}

test('agent access: disabled by default, PUT validates, token file is 0600', async () => {
  const before = await fetch(`${base}/api/agent-access`).then((r) => r.json());
  assert.equal(before.enabled, false);
  assert.deepEqual(before.allowed_servers, []);
  assert.ok(path.isAbsolute(before.token_path), 'token_path is absolute');
  assert.ok(path.isAbsolute(before.mcp.command), 'mcp command is absolute');
  assert.ok(before.mcp.args[0].endsWith(path.join('engine', 'mcp.mjs')), 'mcp args point at mcp.mjs');
  assert.equal(before.mcp.env.ESPECTRAL_DATA_DIR, dataDir);
  assert.ok(!('token' in before), 'the token is never returned');

  let r = await fetch(`${base}/api/agent-access`, {
    method: 'PUT',
    headers: CSRF,
    body: JSON.stringify({ enabled: 'yes' }),
  });
  assert.equal(r.status, 400);
  r = await put({ allowed_servers: 'localhost' });
  assert.equal(r.status, 400);

  // Non-string entries are a hard 400 (a typo must be visible, not dropped).
  r = await put({ enabled: true, allowed_servers: ['localhost', 7] });
  assert.equal(r.status, 400);

  const on = await put({ enabled: true, allowed_servers: ['Play.Example.com.', 'localhost:25565', 'bad host'] });
  assert.equal(on.status, 200);
  assert.equal(on.json.enabled, true);
  // Normalized: lowercased, trailing dot stripped, invalid entries dropped.
  assert.deepEqual(on.json.allowed_servers, ['play.example.com', 'localhost:25565']);

  const mode = fs.statSync(on.json.token_path).mode & 0o777;
  assert.equal(mode, 0o600, 'token file is 0600');
  assert.match(fs.readFileSync(on.json.token_path, 'utf8'), /^[0-9a-f]{64}\n$/);
});

test('agent auth: missing, wrong and right bearer token; disabled beats auth', async () => {
  const token = fs.readFileSync(agent.tokenPath(), 'utf8').trim();

  let r = await agentGet('/api/agent/instances');
  assert.equal(r.status, 401);
  assert.equal(r.json.error.code, 'AGENT_AUTH');

  r = await agentGet('/api/agent/instances', 'f'.repeat(64));
  assert.equal(r.status, 401);

  r = await agentGet('/api/agent/instances', token);
  assert.equal(r.status, 200);
  assert.ok(Array.isArray(r.json.instances));

  // Disabled wins over a valid token (a disabled engine never confirms a token).
  await put({ enabled: false });
  r = await agentGet('/api/agent/instances', token);
  assert.equal(r.status, 403);
  assert.equal(r.json.error.code, 'AGENT_DISABLED');
  await put({ enabled: true });
});

test('rotate rewrites the token and invalidates the old one', async () => {
  const oldToken = fs.readFileSync(agent.tokenPath(), 'utf8').trim();
  const res = await fetch(`${base}/api/agent-access/rotate`, { method: 'POST', headers: CSRF });
  assert.equal(res.status, 200);
  const fresh = fs.readFileSync(agent.tokenPath(), 'utf8').trim();
  assert.notEqual(fresh, oldToken);

  assert.equal((await agentGet('/api/agent/instances', oldToken)).status, 401);
  assert.equal((await agentGet('/api/agent/instances', fresh)).status, 200);
});

test('bridge agent poll: refused for a non-agent launch (409) and a bad token (403)', async () => {
  // A bridge-authenticated launch that was never registered as an agent launch.
  const plainToken = bridge.mintToken();
  bridge.attach({ instance: 'plain-inst', uuid: 'b'.repeat(32), gameDir: dataDir, token: plainToken });

  let r = await bridgePost('/api/bridge/agent/poll', plainToken, {});
  assert.equal(r.status, 409);
  assert.equal(r.json.error.code, 'NOT_AGENT_LAUNCH');

  r = await bridgePost('/api/bridge/agent/poll', 'c'.repeat(64), {});
  assert.equal(r.status, 403);
  assert.equal(r.json.error.code, 'BRIDGE_AUTH');
});

test('relay: a queued call reaches the mod poll and its result resolves the call', async () => {
  const key = 'lrelay1';
  const { token } = attachAgentLaunch(key, 'relay-inst');
  const apiToken = fs.readFileSync(agent.tokenPath(), 'utf8').trim();

  // The mod parks a poll; the call is answered with the command.
  const poll = bridgePost('/api/bridge/agent/poll', token, {});
  const callPromise = fetch(`${base}/api/agent/call`, {
    method: 'POST',
    headers: { ...CSRF, Authorization: `Bearer ${apiToken}` },
    body: JSON.stringify({ key, tool: 'screen_tree', args: {} }),
  }).then(async (r) => ({ status: r.status, json: await r.json() }));

  const polled = await poll;
  assert.equal(polled.status, 200);
  assert.equal(polled.json.commands.length, 1);
  const command = polled.json.commands[0];
  assert.equal(command.tool, 'screen_tree');
  // Command shape is exactly the contract's: {id, tool, args, timeout_ms}.
  assert.deepEqual(Object.keys(command).sort(), ['args', 'id', 'timeout_ms', 'tool']);
  // Policy rides along on every poll.
  assert.deepEqual(polled.json.policy.blocked_hosts, ['uhce.top', 'espectral.es']);
  assert.ok(Array.isArray(polled.json.policy.allowed_servers));

  // The call is still pending until the mod posts its result.
  const resultPost = await bridgePost('/api/bridge/agent/result', token, {
    id: command.id,
    ok: true,
    data: { screen: 'TitleScreen', widgets: [{ id: 'w0', text: 'Singleplayer' }] },
  });
  assert.equal(resultPost.status, 200);
  assert.equal(resultPost.json.accepted, true);

  const call = await callPromise;
  assert.equal(call.status, 200);
  assert.equal(call.json.ok, true);
  assert.equal(call.json.data.screen, 'TitleScreen');
});

test('relay: an error result becomes the call error code', async () => {
  const key = 'lrelay-err';
  const { token } = attachAgentLaunch(key, 'relay-err');
  const apiToken = fs.readFileSync(agent.tokenPath(), 'utf8').trim();

  const poll = bridgePost('/api/bridge/agent/poll', token, {});
  const callPromise = fetch(`${base}/api/agent/call`, {
    method: 'POST',
    headers: { ...CSRF, Authorization: `Bearer ${apiToken}` },
    body: JSON.stringify({ key, tool: 'chat', args: { text: 'hi' } }),
  }).then(async (r) => ({ status: r.status, json: await r.json() }));

  const { json } = await poll;
  await bridgePost('/api/bridge/agent/result', token, {
    id: json.commands[0].id,
    ok: false,
    error: { code: 'TIER_REFUSED', message: 'act tier refused on an Espectral server' },
  });

  const call = await callPromise;
  assert.equal(call.status, 502);
  assert.equal(call.json.error.code, 'TIER_REFUSED');
  assert.match(call.json.error.message, /TIER_REFUSED|refused/);
});

test('relay: a call with no mod polling times out with AGENT_TIMEOUT', async () => {
  const key = 'lrelay-timeout';
  attachAgentLaunch(key, 'relay-timeout');
  const apiToken = fs.readFileSync(agent.tokenPath(), 'utf8').trim();

  const res = await fetch(`${base}/api/agent/call`, {
    method: 'POST',
    headers: { ...CSRF, Authorization: `Bearer ${apiToken}` },
    body: JSON.stringify({ key, tool: 'screenshot', args: {}, timeout_s: 1 }),
  });
  assert.equal(res.status, 504);
  const json = await res.json();
  assert.equal(json.error.code, 'AGENT_TIMEOUT');
});

test('relay: game exit fails pending calls (GAME_GONE) and releases parked polls', async () => {
  const key = 'lrelay-gone';
  const { token } = attachAgentLaunch(key, 'relay-gone');
  const apiToken = fs.readFileSync(agent.tokenPath(), 'utf8').trim();

  // A parked poll with nothing queued must NOT be left hanging on exit: the
  // mod's relay thread would keep the socket open against a dead key.
  const parkedPromise = bridgePost('/api/bridge/agent/poll', token, {});
  await new Promise((r) => setTimeout(r, 80));

  const callPromise = fetch(`${base}/api/agent/call`, {
    method: 'POST',
    headers: { ...CSRF, Authorization: `Bearer ${apiToken}` },
    body: JSON.stringify({ key, tool: 'screenshot', args: {}, timeout_s: 30 }),
  }).then(async (r) => ({ status: r.status, json: await r.json() }));
  await new Promise((r) => setTimeout(r, 80));

  // The queued call is handed to the parked poll (the mod is polling), so the
  // waiter that must fail is the NEXT one: park a second poll, then exit.
  const firstPoll = await parkedPromise;
  assert.equal(firstPoll.json.commands.length, 1, 'the queued call reached the parked poll');
  const parked2 = bridgePost('/api/bridge/agent/poll', token, {});
  await new Promise((r) => setTimeout(r, 80));

  agent.releaseLaunch(key);

  const call = await callPromise;
  assert.equal(call.status, 502);
  assert.equal(call.json.error.code, 'GAME_GONE');

  const released = await parked2;
  assert.equal(released.status, 200);
  assert.deepEqual(released.json.commands, [], 'the parked poll is released empty');

  // The key is gone: further calls are refused, not queued into the void.
  const after = await fetch(`${base}/api/agent/call`, {
    method: 'POST',
    headers: { ...CSRF, Authorization: `Bearer ${apiToken}` },
    body: JSON.stringify({ key, tool: 'screenshot', args: {} }),
  });
  assert.equal(after.status, 404);
});

test('relay: calls to a non-agent or unknown launch are refused', async () => {
  const apiToken = fs.readFileSync(agent.tokenPath(), 'utf8').trim();
  let res = await fetch(`${base}/api/agent/call`, {
    method: 'POST',
    headers: { ...CSRF, Authorization: `Bearer ${apiToken}` },
    body: JSON.stringify({ key: 'nope', tool: 'screenshot' }),
  });
  assert.equal(res.status, 404);
  assert.equal((await res.json()).error.code, 'NO_LAUNCH');
});

test('relay: state is stored per launch and echoed by the agent state route', async () => {
  const key = 'lrelay-state';
  const { token } = attachAgentLaunch(key, 'relay-state');
  const apiToken = fs.readFileSync(agent.tokenPath(), 'utf8').trim();

  let r = await agentGet(`/api/agent/state?key=${key}`, apiToken);
  assert.equal(r.status, 200);
  assert.equal(r.json.state, null, 'no state before the first mod report');

  const post = await bridgePost('/api/bridge/agent/state', token, {
    phase: 'in_world',
    screen: 'GameScreen',
    server: null,
    world: 'Mundo 1',
    context: 'singleplayer',
    paused: false,
    recording: true,
  });
  assert.equal(post.status, 200);

  r = await agentGet(`/api/agent/state?key=${key}`, apiToken);
  assert.equal(r.json.state.phase, 'in_world');
  assert.equal(r.json.state.world, 'Mundo 1');
  assert.equal(r.json.state.recording, true);
  assert.ok(Number.isFinite(r.json.state.updated_at));
});

test('queue: bounded, and a stalled launch cannot grow it forever', () => {
  const key = 'lqueue';
  attachAgentLaunch(key, 'queue-inst');
  const pending = [];
  for (let i = 0; i < agent.MAX_QUEUE; i++) {
    const { promise } = agent.callTool(key, 'screenshot', {});
    promise.catch(() => {}); // never awaited: the queue never drains here
    pending.push(promise);
  }
  assert.throws(
    () => agent.callTool(key, 'screenshot', {}),
    (e) => e.status === 429 && e.code === 'QUEUE_FULL'
  );
  agent.releaseLaunch(key);
});

test('bridge relay goes inert when agent access is switched off', async () => {
  const key = 'lgate';
  const { token } = attachAgentLaunch(key, 'gate-inst');
  await put({ enabled: false });
  try {
    const r = await bridgePost('/api/bridge/agent/poll', token, {});
    assert.equal(r.status, 403);
    assert.equal(r.json.error.code, 'AGENT_DISABLED');
  } finally {
    await put({ enabled: true });
  }
});

test('relay auth: a bridge token cannot act on another launch key', async () => {
  const victimKey = 'lvictim';
  const attacker = attachAgentLaunch('lattacker', 'attacker-inst');
  attachAgentLaunch(victimKey, 'victim-inst');

  // Keys are not secret (they come back from /api/launches), so a token must
  // only ever address its own launch.
  for (const route of ['/api/bridge/agent/poll', '/api/bridge/agent/result', '/api/bridge/agent/state']) {
    const r = await bridgePost(route, attacker.token, { key: victimKey, id: 'c1', ok: true });
    assert.equal(r.status, 403, `${route} refuses a foreign key`);
    assert.equal(r.json.error.code, 'BRIDGE_AUTH');
  }

  // A victim call must not be handed to the attacker's poll: queue it, then
  // answer the attacker's poll with one command queued FOR the attacker so the
  // poll returns at once and we can see exactly what it received.
  const victimCall = agent.callTool(victimKey, 'screenshot', {}, { timeoutMs: 400 });
  victimCall.promise.catch(() => {});
  const ownCall = agent.callTool('lattacker', 'screen_tree', {}, { timeoutMs: 5000 });
  ownCall.promise.catch(() => {});
  const poll = await bridgePost('/api/bridge/agent/poll', attacker.token, {});
  assert.equal(poll.status, 200);
  assert.equal(poll.json.commands.length, 1);
  assert.equal(poll.json.commands[0].id, ownCall.command.id, 'only its own launch commands are delivered');

  const err = await victimCall.promise.catch((e) => e);
  assert.equal(err.code, 'AGENT_TIMEOUT');

  agent.releaseLaunch(victimKey);
  agent.releaseLaunch('lattacker');
});

test('instance PATCH: capture_tools is refused while the instance is running', async () => {
  const routes = await import('../src/engine/routes/launch.mjs');
  const created = await fetch(`${base}/api/instances`, {
    method: 'POST',
    headers: CSRF,
    body: JSON.stringify({ name: 'capture-guard', version: '26.3', loader: 'fabric' }),
  });
  assert.equal(created.status, 200);

  routes.activeInstances.add('capture-guard');
  try {
    const res = await fetch(`${base}/api/instances/capture-guard`, {
      method: 'PATCH',
      headers: CSRF,
      body: JSON.stringify({ capture_tools: true }),
    });
    assert.equal(res.status, 409);
    assert.equal((await res.json()).error.code, 'ALREADY_RUNNING');

    // Unrelated fields stay patchable while the game runs.
    const hue = await fetch(`${base}/api/instances/capture-guard`, {
      method: 'PATCH',
      headers: CSRF,
      body: JSON.stringify({ memory_mb: 2048 }),
    });
    assert.equal(hue.status, 200);
  } finally {
    routes.activeInstances.delete('capture-guard');
  }
});
