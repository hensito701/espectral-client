import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { engineFetch, handleMessage } from '../src/engine/mcp.mjs';

// Protocol round-trip of the stdio shim against a STUB engine: this test owns
// the wire format (JSON-RPC framing, tools/list, tools/call), so it must not
// depend on the real engine's routes.
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SHIM = path.resolve(__dirname, '..', 'src', 'engine', 'mcp.mjs');
const TOKEN = 'a'.repeat(64);

const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-mcp-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;
fs.writeFileSync(path.join(dataDir, 'agent-token'), TOKEN + '\n', { mode: 0o600 });

/** Requests the stub saw: {method, url, auth, body}. */
const seen = [];
let reply = null;
/** Stub engine: /api/agent/* */
const stub = http.createServer((req, res) => {
  let raw = '';
  req.on('data', (d) => {
    raw += d;
  });
  req.on('end', () => {
    const url = new URL(req.url, 'http://127.0.0.1');
    const body = raw ? JSON.parse(raw) : null;
    seen.push({ method: req.method, path: url.pathname, query: url.search, auth: req.headers.authorization, body });
    const send = (status, obj) => {
      res.writeHead(status, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(obj));
    };
    if (req.headers.authorization !== `Bearer ${TOKEN}`) {
      return send(401, { error: { code: 'AGENT_AUTH', message: 'bad token' } });
    }
    if (reply) return reply(req, url, body, send);
    if (url.pathname === '/api/agent/instances') {
      return send(200, { instances: [{ name: 'agent-spike', version: '26.3', loader: 'fabric', capture_tools: true }] });
    }
    if (url.pathname === '/api/agent/launches') {
      return send(200, { launches: [{ key: 'l1', instance: 'agent-spike', running: true, agent: true, state: null }] });
    }
    if (url.pathname === '/api/agent/call') {
      return send(200, {
        ok: true,
        data: { image_path: '/tmp/shot.png', preview_path: '/tmp/shot.png', width: 4, height: 2, preview: { mime: 'image/png', base64: 'iVBORw0KGgo=' } },
      });
    }
    if (url.pathname === '/api/agent/state') {
      return send(200, { key: 'l1', instance: 'agent-spike', running: true, state: { phase: 'in_world' } });
    }
    if (url.pathname === '/api/agent/launch') {
      return send(200, { key: 'l2', state: { phase: 'in_world' } });
    }
    return send(404, { error: { code: 'NOT_FOUND', message: 'nope' } });
  });
});

/** Every shim spawned by a test (closed in `after` even if a test failed). */
const shims = [];
let server;
let base;
let port;

test.before(async () => {
  await new Promise((resolve) => stub.listen(0, '127.0.0.1', resolve));
  port = stub.address().port;
  base = `http://127.0.0.1:${port}`;
});

after(async () => {
  for (const s of shims) await s.close();
  // keep-alive sockets from the stub's fetch clients would otherwise hold the
  // server open and keep the test process alive after the last assertion.
  stub.closeAllConnections?.();
  await new Promise((r) => stub.close(r));
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

/** Spawn the shim and speak JSON-RPC over stdio. */
function startShim({ dataDirPath = dataDir, enginePort = port } = {}) {
  const child = spawn(process.execPath, [SHIM], {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, ESPECTRAL_DATA_DIR: dataDirPath, ESPECTRAL_PORT: String(enginePort) },
  });
  let buffer = '';
  const pending = new Map();
  const frames = [];
  let idc = 0;
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', (d) => {
    stderr += d;
  });
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (d) => {
    buffer += d;
    let i;
    while ((i = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, i).trim();
      buffer = buffer.slice(i + 1);
      if (!line) continue;
      const msg = JSON.parse(line); // throws if a non-protocol byte leaked
      frames.push(msg);
      const w = pending.get(msg.id);
      if (w) {
        pending.delete(msg.id);
        w(msg);
      }
    }
  });
  const rpc = (method, params) => {
    const id = ++idc;
    const p = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`no reply for ${method} (stderr: ${stderr})`)), 15_000);
      pending.set(id, (msg) => {
        clearTimeout(timer);
        resolve(msg);
      });
    });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    return p;
  };
  const notify = (method) => {
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method }) + '\n');
  };
  let closed = false;
  const close = async () => {
    if (closed) return;
    closed = true;
    if (child.exitCode === null) child.stdin.end();
    await new Promise((r) => {
      if (child.exitCode !== null) return r();
      child.once('exit', r);
      const t = setTimeout(() => {
        child.kill('SIGKILL');
        r();
      }, 3000);
      t.unref?.();
    });
  };
  const handle = { child, rpc, notify, frames, close, stderr: () => stderr };
  shims.push(handle);
  return handle;
}

test('mcp shim: initialize / tools/list / tools/call round-trip over stdio', async () => {
  server = startShim();
  const init = await server.rpc('initialize', {
    protocolVersion: '2025-06-18',
    capabilities: {},
    clientInfo: { name: 'test', version: '1' },
  });
  assert.equal(init.jsonrpc, '2.0');
  assert.equal(init.result.protocolVersion, '2025-06-18', 'echoes a supported protocol version');
  assert.deepEqual(init.result.capabilities.tools, { listChanged: false });
  assert.equal(init.result.serverInfo.name, 'espectral-client');
  assert.match(init.result.instructions, /untrusted/i, 'instructions warn about untrusted game text');
  server.notify('notifications/initialized');

  const ping = await server.rpc('ping', {});
  assert.deepEqual(ping.result, {});

  const list = await server.rpc('tools/list', {});
  const names = list.result.tools.map((t) => t.name);
  for (const expected of [
    'list_instances',
    'list_launches',
    'launch',
    'stop',
    'state',
    'logs',
    'screenshot',
    'screen_tree',
    'click',
    'key',
    'type',
    'chat',
    'command',
    'disconnect',
    'hud',
    'shaders',
    'record_start',
    'record_stop',
    'replays',
    'clip_render',
    'clip_status',
  ]) {
    assert.ok(names.includes(expected), `tool ${expected} is listed`);
  }
  for (const tool of list.result.tools) {
    assert.ok(tool.description.length > 20, `${tool.name} has a real description`);
    assert.equal(tool.inputSchema.type, 'object', `${tool.name} has an object schema`);
  }
  // Every description names the tier vocabulary where it matters.
  assert.match(names.length && list.result.tools.find((t) => t.name === 'screenshot').description, /TIER/);

  // Engine-level tool.
  const instances = await server.rpc('tools/call', { name: 'list_instances', arguments: {} });
  assert.equal(instances.result.isError, false);
  const payload = JSON.parse(instances.result.content[0].text);
  assert.equal(payload.instances[0].capture_tools, true);
  const seek = seen.find((s) => s.path === '/api/agent/instances');
  assert.equal(seek.auth, `Bearer ${TOKEN}`);
  assert.equal(seek.method, 'GET');

  // Engine-level launch with join/window: the body must be forwarded verbatim.
  const launched = await server.rpc('tools/call', {
    name: 'launch',
    arguments: { instance: 'agent-spike', join: { world: 'Mundo' }, window: { width: 1080, height: 1920 }, wait_for: 'in_world' },
  });
  assert.equal(launched.result.isError, false);
  const launchSeen = seen.find((s) => s.path === '/api/agent/launch');
  assert.deepEqual(launchSeen.body, {
    instance: 'agent-spike',
    join: { world: 'Mundo' },
    window: { width: 1080, height: 1920 },
    wait_for: 'in_world',
  });

  // Mod tool: args minus `key` go to the engine as call.args.
  const shot = await server.rpc('tools/call', {
    name: 'screenshot',
    arguments: { key: 'l1', width: 1920, height: 1080, hud: false },
  });
  assert.equal(shot.result.isError, false);
  assert.deepEqual(
    shot.result.content.map((c) => c.type),
    ['image', 'text'],
    'image content then the JSON text content'
  );
  assert.equal(shot.result.content[0].mimeType, 'image/png');
  assert.equal(shot.result.content[0].data, 'iVBORw0KGgo=');
  const text = JSON.parse(shot.result.content[1].text);
  assert.equal(text.image_path, '/tmp/shot.png');
  assert.equal(text.preview, undefined, 'the inline base64 is not repeated in the text');
  const callSeen = seen.find((s) => s.path === '/api/agent/call');
  assert.deepEqual(callSeen.body, {
    key: 'l1',
    tool: 'screenshot',
    args: { width: 1920, height: 1080, hud: false },
    timeout_s: 60,
  });

  // `key` tool maps the MCP `keyName` argument to the mod's `key` arg.
  await server.rpc('tools/call', { name: 'key', arguments: { key: 'l1', keyName: 'e', action: 'press' } });
  const keyCall = seen.filter((s) => s.path === '/api/agent/call').pop();
  assert.deepEqual(keyCall.body.args, { key: 'e', action: 'press' });

  await server.close();
});

test('mcp shim: engine errors come back as actionable isError content', async () => {
  server = startShim();
  await server.rpc('initialize', { protocolVersion: '2024-11-05' });

  reply = (req, url, body, send) => send(403, { error: { code: 'AGENT_DISABLED', message: 'off' } });
  let res = await server.rpc('tools/call', { name: 'list_instances', arguments: {} });
  assert.equal(res.result.isError, true);
  assert.match(res.result.content[0].text, /AGENT_DISABLED/);
  assert.match(res.result.content[0].text, /disabled/i);
  reply = null;

  // A tool the engine refuses (tier) surfaces the engine's code.
  reply = (req, url, body, send) => send(502, { error: { code: 'TIER_REFUSED', message: 'act tier refused here' } });
  res = await server.rpc('tools/call', { name: 'chat', arguments: { key: 'l1', text: 'hi' } });
  assert.equal(res.result.isError, true);
  assert.match(res.result.content[0].text, /TIER_REFUSED/);
  reply = null;
  await server.close();
});

test('mcp shim: no token and no engine produce distinct clear errors', async () => {
  const emptyDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-mcp-empty-'));
  server = startShim({ dataDirPath: emptyDir });
  await server.rpc('initialize', {});
  let res = await server.rpc('tools/call', { name: 'list_instances', arguments: {} });
  assert.equal(res.result.isError, true);
  assert.match(res.result.content[0].text, /NO_TOKEN/);
  assert.match(res.result.content[0].text, /Agent access/);
  await server.close();
  fs.rmSync(emptyDir, { recursive: true, force: true });

  // A token file that exists but is garbage is a different problem (rotate),
  // not "agent access is off" — and the token itself is never printed.
  const badDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-mcp-badtoken-'));
  fs.writeFileSync(path.join(badDir, 'agent-token'), 'not-a-token\n', { mode: 0o600 });
  server = startShim({ dataDirPath: badDir });
  await server.rpc('initialize', {});
  res = await server.rpc('tools/call', { name: 'list_instances', arguments: {} });
  assert.equal(res.result.isError, true);
  assert.match(res.result.content[0].text, /malformed/);
  assert.match(res.result.content[0].text, /rotate/);
  assert.ok(!res.result.content[0].text.includes('not-a-token'), 'the file contents are not echoed');
  await server.close();
  fs.rmSync(badDir, { recursive: true, force: true });

  // Token present, engine down.
  server = startShim({ enginePort: 1 });
  await server.rpc('initialize', {});
  res = await server.rpc('tools/call', { name: 'list_instances', arguments: {} });
  assert.equal(res.result.isError, true);
  assert.match(res.result.content[0].text, /ENGINE_DOWN/);
  assert.match(res.result.content[0].text, /Is Espectral Client running/);
  await server.close();
});

test('mcp shim: unknown method and malformed frames do not kill the stream', async () => {
  server = startShim();
  await server.rpc('initialize', {});
  server.child.stdin.write('{not json}\n');
  const bad = await server.rpc('resources/list', {});
  assert.equal(bad.error.code, -32601);
  // Still alive and speaking protocol after the garbage frame.
  const list = await server.rpc('tools/list', {});
  assert.ok(list.result.tools.length > 0);
  await server.close();
});

test('mcp shim: token is re-read per call, so rotation takes effect immediately', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-mcp-rotate-'));
  fs.writeFileSync(path.join(dir, 'agent-token'), TOKEN + '\n', { mode: 0o600 });
  const shim = startShim({ dataDirPath: dir });
  await shim.rpc('initialize', {});
  let res = await shim.rpc('tools/call', { name: 'list_instances', arguments: {} });
  assert.equal(res.result.isError, false);

  fs.writeFileSync(path.join(dir, 'agent-token'), 'b'.repeat(64) + '\n', { mode: 0o600 });
  res = await shim.rpc('tools/call', { name: 'list_instances', arguments: {} });
  assert.equal(res.result.isError, true);
  assert.match(res.result.content[0].text, /AGENT_AUTH/);
  await shim.close();
  fs.rmSync(dir, { recursive: true, force: true });
});

test('mcp shim: JSON-RPC 2.0 semantics (id echo, notifications, unknown methods/tools)', async () => {
  // Ids are echoed verbatim: strings, 0 and integers beyond 32 bits included.
  assert.deepEqual(await handleMessage({ jsonrpc: '2.0', id: 'abc', method: 'ping' }), {
    jsonrpc: '2.0',
    id: 'abc',
    result: {},
  });
  assert.equal((await handleMessage({ jsonrpc: '2.0', id: 0, method: 'ping' })).id, 0);
  assert.equal((await handleMessage({ jsonrpc: '2.0', id: 9007199254740991, method: 'ping' })).id, 9007199254740991);

  // Notifications (no `id`) are NEVER answered — not even with an error, and
  // not even when they name a request method.
  assert.equal(await handleMessage({ jsonrpc: '2.0', method: 'notifications/initialized' }), null);
  assert.equal(await handleMessage({ jsonrpc: '2.0', method: 'tools/list' }), null);
  assert.equal(await handleMessage({ jsonrpc: '2.0', method: 'tools/call', params: { name: 'list_instances' } }), null);
  assert.equal(await handleMessage({ jsonrpc: '2.0', method: 'no/such/notification' }), null);

  // Unknown tool = protocol error (-32602), never a successful tool result.
  const unknown = await handleMessage({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'nope' } });
  assert.equal(unknown.error.code, -32602);
  assert.equal(unknown.result, undefined);
  assert.equal(unknown.id, 1);

  // Unknown method and malformed request.
  assert.equal((await handleMessage({ jsonrpc: '2.0', id: 2, method: 'resources/list' })).error.code, -32601);
  const noMethod = await handleMessage({ jsonrpc: '2.0', id: 3 });
  assert.equal(noMethod.error.code, -32600);
  assert.equal(noMethod.id, 3);

  // initialize result shape.
  const init = await handleMessage({
    jsonrpc: '2.0',
    id: 4,
    method: 'initialize',
    params: { protocolVersion: '2025-03-26' },
  });
  assert.equal(init.id, 4);
  assert.equal(init.result.protocolVersion, '2025-03-26');
  assert.deepEqual(init.result.capabilities, { tools: { listChanged: false } });
  assert.equal(init.result.serverInfo.name, 'espectral-client');
  assert.equal(typeof init.result.serverInfo.version, 'string');
});

test('mcp shim: a 200 {ok:false} call envelope is a refusal, not an empty success', async () => {
  server = startShim();
  await server.rpc('initialize', {});
  reply = (req, url, body, send) => send(200, { ok: false, error: { code: 'BUSY', message: 'renderer busy' } });
  try {
    const res = await server.rpc('tools/call', { name: 'screenshot', arguments: { key: 'l1' } });
    assert.equal(res.result.isError, true);
    assert.match(res.result.content[0].text, /BUSY: renderer busy/);
  } finally {
    reply = null;
  }
  await server.close();
});

test('mcp shim: CRLF frames and anonymous frames keep the stream healthy', async () => {
  const shim = startShim();
  await shim.rpc('initialize', {});
  const before = shim.frames.length;
  // CRLF line ending, then a notification (no id → no reply at all).
  shim.child.stdin.write(
    '{"jsonrpc":"2.0","id":"crlf","method":"ping"}\r\n{"jsonrpc":"2.0","method":"notifications/initialized"}\r\n'
  );
  const sentinel = await shim.rpc('ping', {});
  const got = shim.frames.slice(before).filter((f) => f.id !== sentinel.id);
  assert.deepEqual(
    got.map((f) => f.id),
    ['crlf'],
    'the CRLF request was answered and the notification produced no frame'
  );
  assert.deepEqual(got[0].result, {});
  await shim.close();
});

test('mcp shim: stdin close flushes the last frame and any in-flight reply', async () => {
  const shim = startShim();
  await shim.rpc('initialize', {});

  // A final frame with no trailing newline must still be answered...
  let before = shim.frames.length;
  shim.child.stdin.end('{"jsonrpc":"2.0","id":"last","method":"ping"}');
  await new Promise((r) => {
    if (shim.child.exitCode !== null) return r();
    shim.child.once('exit', r);
  });
  assert.equal(shim.child.exitCode, 0, 'stdin close exits cleanly');
  const flushed = shim.frames.slice(before);
  assert.deepEqual(flushed.map((f) => f.id), ['last']);
  assert.deepEqual(flushed[0].result, {});

  // ...and a tool call still waiting on the engine is not dropped by the exit.
  const slow = startShim();
  await slow.rpc('initialize', {});
  reply = (req, url, body, send) => setTimeout(() => send(200, { instances: [{ name: 'slow' }] }), 250);
  try {
    before = slow.frames.length;
    slow.child.stdin.end(
      JSON.stringify({ jsonrpc: '2.0', id: 'slow1', method: 'tools/call', params: { name: 'list_instances' } }) + '\n'
    );
    await new Promise((r) => {
      if (slow.child.exitCode !== null) return r();
      slow.child.once('exit', r);
    });
    const replyFrame = slow.frames.slice(before).find((f) => f.id === 'slow1');
    assert.ok(replyFrame, 'the in-flight reply was delivered before exit');
    assert.equal(replyFrame.result.isError, false);
    assert.equal(JSON.parse(replyFrame.result.content[0].text).instances[0].name, 'slow');
  } finally {
    reply = null;
  }
});

test('mcp shim: engineFetch deadlines and error text', async () => {
  const prev = process.env.ESPECTRAL_PORT;
  process.env.ESPECTRAL_PORT = String(port);
  const stalling = http.createServer((req, res) => {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.write('{"instances":'); // headers + a partial body, then never finishes
  });
  const emptyBody = http.createServer((req, res) => {
    res.writeHead(503, { 'Content-Type': 'text/plain' });
    res.end('');
  });
  const leak = http.createServer((req, res) => {
    res.writeHead(401, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ error: { code: 'AGENT_AUTH', message: `bad token ${req.headers.authorization}` } }));
  });
  const listen = (s) => new Promise((r) => s.listen(0, '127.0.0.1', r));
  await Promise.all([listen(stalling), listen(emptyBody), listen(leak)]);
  try {
    process.env.ESPECTRAL_PORT = String(stalling.address().port);
    const t0 = Date.now();
    await assert.rejects(
      () => engineFetch('GET', '/api/agent/instances', null, { timeoutMs: 150 }),
      (e) => e.code === 'TIMEOUT' || e.code === 'ENGINE_DOWN'
    );
    assert.ok(Date.now() - t0 < 5000, 'a stalled body does not hang the call');

    process.env.ESPECTRAL_PORT = String(emptyBody.address().port);
    await assert.rejects(
      () => engineFetch('GET', '/api/agent/instances'),
      (e) => /HTTP 503/.test(e.message)
    );

    process.env.ESPECTRAL_PORT = String(leak.address().port);
    await assert.rejects(
      () => engineFetch('GET', '/api/agent/instances'),
      (e) => {
        assert.equal(e.code, 'AGENT_AUTH');
        assert.ok(!e.message.includes(TOKEN), 'the token is never echoed back in an error message');
        return true;
      }
    );
  } finally {
    if (prev === undefined) delete process.env.ESPECTRAL_PORT;
    else process.env.ESPECTRAL_PORT = prev;
    for (const s of [stalling, emptyBody, leak]) {
      s.closeAllConnections?.();
      await new Promise((r) => s.close(r));
    }
  }
});

// The stub server + shim children hold sockets; without an explicit
// force-exit the node:test process can linger after the last assertion (a
// keep-alive connection keeps the event loop alive and the runner waits for
// it). Only the resources this file owns are torn down above; this is the
// final guarantee that a finished run terminates.
test.after(() => {
  setTimeout(() => process.exit(process.exitCode ?? 0), 1500).unref();
});