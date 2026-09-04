import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir before importing so the live engine never touches real data.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-personalization-http-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { start } = await import('../src/engine/server.mjs');

let server;
let base;

test.before(async () => {
  server = await start(0); // ephemeral port
  base = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  if (server) await new Promise((r) => server.close(r));
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const H = { 'Content-Type': 'application/json', 'x-espectral-client': '1' };

async function api(method, p, body) {
  const res = await fetch(`${base}${p}`, {
    method,
    headers: H,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = null;
  }
  return { res, json, text };
}

// 1x1 transparent PNG.
const TINY_DATA_URL =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const TINY_BYTES = Buffer.from(TINY_DATA_URL.split(',')[1], 'base64');

test('instance icon endpoints: POST roundtrip, GET bytes, DELETE, error codes', async () => {
  const created = await api('POST', '/api/instances', { name: 'http-icon', version: '1.21.11' });
  assert.equal(created.res.status, 200);
  assert.ok(Number.isInteger(created.json.hue), 'summary carries the auto hue');
  assert.equal(created.json.has_icon, false);
  assert.equal(created.json.game_dir, null);

  // Missing icon reads 404.
  const missing = await fetch(`${base}/api/instances/http-icon/icon`);
  assert.equal(missing.status, 404);

  // Non-PNG and oversize uploads are 400.
  const bad = await api('POST', '/api/instances/http-icon/icon', { image_base64: 'aGVsbG8=' });
  assert.equal(bad.res.status, 400);
  const big = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    Buffer.alloc(500 * 1024 + 1 - 8),
  ]);
  const bigRes = await api('POST', '/api/instances/http-icon/icon', {
    image_base64: big.toString('base64'),
  });
  assert.equal(bigRes.res.status, 400);

  // Unknown instance is 404.
  const ghost = await api('POST', '/api/instances/nope/icon', { image_base64: TINY_DATA_URL });
  assert.equal(ghost.res.status, 404);

  // Roundtrip.
  const put = await api('POST', '/api/instances/http-icon/icon', { image_base64: TINY_DATA_URL });
  assert.equal(put.res.status, 200);
  assert.deepEqual(put.json, { ok: true, has_icon: true });

  const get = await fetch(`${base}/api/instances/http-icon/icon`);
  assert.equal(get.status, 200);
  assert.match(get.headers.get('content-type'), /image\/png/);
  assert.ok(Buffer.from(await get.arrayBuffer()).equals(TINY_BYTES));
  const detail = await (await fetch(`${base}/api/instances/http-icon`)).json();
  assert.equal(detail.summary.has_icon, true);


  const del = await api('DELETE', '/api/instances/http-icon/icon');
  assert.equal(del.res.status, 200);
  const gone = await fetch(`${base}/api/instances/http-icon/icon`);
  assert.equal(gone.status, 404);
});

test('account avatar + color endpoints roundtrip with validation', async () => {
  const created = await api('POST', '/api/accounts', { username: 'HttpAvatar' });
  assert.equal(created.res.status, 200);
  assert.ok(Number.isInteger(created.json.avatar_color), 'create assigns an avatar color');

  const list = await api('GET', '/api/accounts');
  assert.equal(list.res.status, 200);
  const me = list.json.find((a) => a.username === 'HttpAvatar');
  assert.ok(me, 'account listed');
  assert.equal(me.has_avatar, false);
  assert.equal(me.avatar_color, created.json.avatar_color);

  const missing = await fetch(`${base}/api/accounts/HttpAvatar/avatar`);
  assert.equal(missing.status, 404);

  const bad = await api('POST', '/api/accounts/HttpAvatar/avatar', { image_base64: 'aGVsbG8=' });
  assert.equal(bad.res.status, 400);
  assert.equal(bad.json.error.code, 'BAD_IMAGE');

  const put = await api('POST', '/api/accounts/HttpAvatar/avatar', { image_base64: TINY_DATA_URL });
  assert.equal(put.res.status, 200);
  assert.deepEqual(put.json, { ok: true, has_avatar: true });

  const get = await fetch(`${base}/api/accounts/HttpAvatar/avatar`);
  assert.equal(get.status, 200);
  assert.match(get.headers.get('content-type'), /image\/png/);
  assert.ok(Buffer.from(await get.arrayBuffer()).equals(TINY_BYTES));

  const color = await api('POST', '/api/accounts/HttpAvatar/avatar-color', { avatar_color: 77 });
  assert.equal(color.res.status, 200);
  assert.equal(color.json.avatar_color, 77);

  const colorBad = await api('POST', '/api/accounts/HttpAvatar/avatar-color', { avatar_color: 500 });
  assert.equal(colorBad.res.status, 400);
  assert.equal(colorBad.json.error.code, 'BAD_AVATAR_COLOR');

  const colorGhost = await api('POST', '/api/accounts/NobodyHere/avatar-color', { avatar_color: 5 });
  assert.equal(colorGhost.res.status, 404);

  const avatarGhost = await api('POST', '/api/accounts/NobodyHere/avatar', {
    image_base64: TINY_DATA_URL,
  });
  assert.equal(avatarGhost.res.status, 404);

  const del = await api('DELETE', '/api/accounts/HttpAvatar/avatar');
  assert.equal(del.res.status, 200);
  const gone = await fetch(`${base}/api/accounts/HttpAvatar/avatar`);
  assert.equal(gone.status, 404);
});

test('instance create/patch carry hue + game_dir over HTTP', async () => {
  const custom = path.join(dataDir, 'http-game-dir');
  const created = await api('POST', '/api/instances', {
    name: 'http-dirs',
    version: '1.21.11',
    hue: 300,
    game_dir: custom,
  });
  assert.equal(created.res.status, 200);
  assert.equal(created.json.hue, 300);
  assert.equal(created.json.game_dir, custom);
  assert.ok(fs.existsSync(custom));

  const rel = await api('POST', '/api/instances', {
    name: 'http-dirs-bad',
    version: '1.21.11',
    game_dir: 'relative/path',
  });
  assert.equal(rel.res.status, 400);

  const patched = await api('PATCH', '/api/instances/http-dirs', { hue: 10, game_dir: null });
  assert.equal(patched.res.status, 200);
  assert.equal(patched.json.hue, 10);
  assert.equal(patched.json.game_dir, null);
});
