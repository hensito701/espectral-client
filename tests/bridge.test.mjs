import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import http from 'node:http';

// Sandbox the data dir before importing the engine.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-bridge-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { start } = await import('../src/engine/server.mjs');
const bridge = await import('../src/engine/bridge.mjs');
const identity = await import('../src/engine/identity.mjs');

let server;
let base;
const OWNER_UUID = '11111111111111111111111111111111';
const PEER_UUID = '22222222222222222222222222222222';
const GHOST_UUID = '33333333333333333333333333333333';

test.before(async () => {
  server = await start(0);
  base = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  bridge._reset();
  if (server) await new Promise((r) => server.close(r));
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const CLIENT = { 'Content-Type': 'application/json', 'x-espectral-client': '1' };

async function postVisible(body, bridgeToken) {
  const headers = { ...CLIENT };
  if (bridgeToken !== undefined) headers['x-espectral-bridge'] = bridgeToken;
  const res = await fetch(`${base}/api/bridge/visible`, {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
  });
  return { status: res.status, json: await res.json().catch(() => null) };
}

test('bridge: missing or wrong token -> 403', async () => {
  // Missing bridge header entirely.
  let r = await postVisible({ uuids: [PEER_UUID] });
  assert.equal(r.status, 403);
  assert.equal(r.json.error.code, 'BRIDGE_AUTH');

  // Wrong token.
  bridge.attach({ instance: 'inst-a', uuid: OWNER_UUID, gameDir: dataDir, token: 'a'.repeat(64) });
  r = await postVisible({ uuids: [PEER_UUID] }, 'b'.repeat(64));
  assert.equal(r.status, 403);

  // Missing x-espectral-client (CSRF guard) beats bridge auth.
  const res = await fetch(`${base}/api/bridge/visible`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'x-espectral-bridge': 'a'.repeat(64) },
    body: '{}',
  });
  assert.equal(res.status, 403);
  assert.equal((await res.json()).error.code, 'FORBIDDEN');
});

test('bridge: correct token answers from cache only; uncached uuid absent', async () => {
  const token = bridge.mintToken();
  bridge.attach({ instance: 'inst-b', uuid: OWNER_UUID, gameDir: dataDir, token });

  // Seed one positive entry and one negative entry.
  bridge._seedCache(PEER_UUID, {
    grant: 'hdr.pay.sig',
    loadout: { espalda: 'alas_eco' },
    expiresAt: Date.now() + 60_000,
  });
  bridge._seedCache(GHOST_UUID, { absent: true, expiresAt: Date.now() + 60_000 });

  const r = await postVisible({ uuids: [PEER_UUID, GHOST_UUID, 'not-a-uuid'] }, token);
  assert.equal(r.status, 200);
  assert.equal(r.json.players.length, 1);
  assert.equal(r.json.players[0].uuid, PEER_UUID);
  assert.equal(r.json.players[0].grant, 'hdr.pay.sig');
  assert.deepEqual(r.json.players[0].loadout, { espalda: 'alas_eco' });
});

test('bridge: expired cache entries are re-queued and not answered', async () => {
  const token = bridge.mintToken();
  bridge.attach({ instance: 'inst-c', uuid: OWNER_UUID, gameDir: dataDir, token });
  bridge._seedCache(PEER_UUID, { grant: 'g', loadout: {}, expiresAt: Date.now() - 1 });
  const r = await postVisible({ uuids: [PEER_UUID] }, token);
  assert.equal(r.status, 200);
  assert.deepEqual(r.json.players, []);
});

test('bridge: uncached uuids get batched to presence/query in the background', async () => {
  // Local service stub capturing the query.
  const calls = [];
  const stub = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      calls.push({ url: req.url, auth: req.headers.authorization, body });
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(
        JSON.stringify({
          players: [{ uuid: PEER_UUID, grant: 'g2', loadout: { cabeza: 'orejas_vulpinas' } }],
        }),
      );
    });
  });
  await new Promise((r) => stub.listen(0, '127.0.0.1', r));
  const stubPort = stub.address().port;
  const prevUrl = process.env.ESPECTRAL_ID_URL;
  process.env.ESPECTRAL_ID_URL = `http://127.0.0.1:${stubPort}`;
  try {
    // Owner needs a live session token for the background query.
    identity.updateEntry(OWNER_UUID, {
      token: 'tok-owner',
      token_expires_at: new Date(Date.now() + 3600_000).toISOString(),
    });
    const token = bridge.mintToken();
    bridge.attach({ instance: 'inst-d', uuid: OWNER_UUID, gameDir: dataDir, token });

    const r = await postVisible({ uuids: [PEER_UUID] }, token);
    assert.equal(r.status, 200);
    assert.deepEqual(r.json.players, []); // not cached yet — immediate answer

    // Background flush should hit the stub within a few seconds.
    const deadline = Date.now() + 8000;
    while (calls.length === 0 && Date.now() < deadline) {
      await new Promise((r2) => setTimeout(r2, 100));
    }
    assert.equal(calls.length, 1, 'exactly one batched query');
    assert.equal(calls[0].url, '/v1/presence/query');
    assert.equal(calls[0].auth, 'Bearer tok-owner');
    assert.deepEqual(JSON.parse(calls[0].body).uuids, [PEER_UUID]);

    // Now the cache holds it.
    const r2 = await postVisible({ uuids: [PEER_UUID] }, token);
    assert.equal(r2.json.players.length, 1);
    assert.equal(r2.json.players[0].grant, 'g2');
  } finally {
    if (prevUrl === undefined) delete process.env.ESPECTRAL_ID_URL;
    else process.env.ESPECTRAL_ID_URL = prevUrl;
    await new Promise((r) => stub.close(r));
  }
});

test('bridge: 32-hex and dashed/uuid forms from the mod both resolve', async () => {
  const token = bridge.mintToken();
  bridge.attach({ instance: 'inst-uuid', uuid: OWNER_UUID, gameDir: dataDir, token });
  bridge._seedCache(PEER_UUID, {
    grant: 'g3',
    loadout: { cabeza: 'orejas_vulpinas' },
    expiresAt: Date.now() + 60_000,
  });
  const dashed = '22222222-2222-4222-8222-222222222222';
  const upper = PEER_UUID.toUpperCase();
  const r = await postVisible({ uuids: [dashed, upper, PEER_UUID] }, token);
  assert.equal(r.status, 200);
  // All three spellings fold to one canonical uuid (dedupe, one player row).
  assert.equal(r.json.players.length, 1);
  assert.equal(r.json.players[0].uuid, PEER_UUID);
  assert.equal(r.json.players[0].grant, 'g3');
});

test('bridge: a launch registered with an empty token never authenticates and cannot crash lookups', async () => {
  // Unverified/offline launches register with token '' (resolveLaunch mints
  // only for real uuids). The old authenticate() called timingSafeEqual on
  // different-length buffers -> RangeError -> EVERY /api/bridge/visible 500s
  // while such a launch exists.
  bridge.attach({ instance: 'no-token-launch', uuid: OWNER_UUID, gameDir: dataDir, token: '' });
  const r = await postVisible({ uuids: [PEER_UUID] }, bridge.mintToken());
  assert.equal(r.status, 403);
  assert.equal(r.json.error.code, 'BRIDGE_AUTH');
  assert.equal(bridge.authenticate(''), null);
  assert.equal(bridge.authenticate('f'.repeat(64)), null);
  bridge.detach('no-token-launch');
});

test('bridge: detach scoped by token protects a relaunched registration', async () => {
  // A stale teardown (e.g. a trainer exit after a relaunch re-attached the
  // same instance name) must not evict the newer registration.
  const gameToken = bridge.mintToken();
  bridge.attach({ instance: 'inst-g', uuid: OWNER_UUID, gameDir: dataDir, token: gameToken });
  assert.equal(bridge.detach('inst-g', '0'.repeat(64)), null, 'stale token must not detach');
  assert.equal(bridge.authenticate(gameToken)?.instance, 'inst-g', 'live launch survives');
  const entry = bridge.detach('inst-g', gameToken);
  assert.equal(entry.token, gameToken);
  assert.equal(bridge.authenticate(gameToken), null);
});

test('presence: launchExited detaches only the registration that owns the token', async () => {
  const presence = await import('../src/engine/presence.mjs');
  const uuid = '99999999999999999999999999999999'; // no identity token -> no network on unpublish
  const trainerToken = bridge.mintToken();
  const gameToken = bridge.mintToken();
  // Trainer registered first, then a real relaunch overwrote the instance slot.
  presence.launchStarted({ instance: 'inst-h', uuid, gameDir: dataDir, token: trainerToken });
  presence.launchStarted({ instance: 'inst-h', uuid, gameDir: dataDir, token: gameToken });
  presence.launchExited('inst-h', trainerToken); // stale trainer teardown
  assert.equal(bridge.authenticate(gameToken)?.instance, 'inst-h', 'relaunch registration survives');
  presence.launchExited('inst-h', gameToken);
  assert.equal(bridge.authenticate(gameToken), null);
  presence._reset();
});
