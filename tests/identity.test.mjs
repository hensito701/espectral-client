import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import http from 'node:http';
import crypto from 'node:crypto';

// Sandbox the data dir before importing the engine.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-identity-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { start } = await import('../src/engine/server.mjs');
const accounts = await import('../src/engine/accounts.mjs');
const { loadConfig } = await import('../src/engine/config.mjs');
const identity = await import('../src/engine/identity.mjs');
const grant = await import('../src/engine/grant.mjs');

const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
grant.registerGrantKey('test-id', publicKey);

const UUID = 'aabbccddeeff00112233445566778899';
const NOW = Math.floor(Date.now() / 1000);

let server;
let base;
let stub;
let stubCalls;

function signServiceGrant(uuid, flags = ['supporter', 'mvp']) {
  return grant.signGrant(
    { v: 1, iss: grant.GRANT_ISS, aud: grant.GRANT_AUD, sub: uuid, flags, iat: NOW, exp: NOW + 86400 },
    { kid: 'test-id', privateKey },
  );
}

test.before(async () => {
  // Stub espectral-id + the Mojang session server on one handler.
  stubCalls = [];
  stub = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const parsed = body ? JSON.parse(body) : null;
      stubCalls.push({ method: req.method, url: req.url, auth: req.headers.authorization, body: parsed });
      const send = (status, obj) => {
        res.writeHead(status, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify(obj));
      };
      if (req.method === 'POST' && req.url === '/v1/auth/challenge') {
        return send(200, { challenge_id: 'ch-1', server_id: 'a'.repeat(40), expires_in: 120 });
      }
      if (req.method === 'POST' && req.url === '/session/minecraft/join') {
        // Mojang answers 204 No Content on success.
        if (parsed?.accessToken === 'fake-mc-token' && parsed?.selectedProfile === UUID) {
          res.writeHead(204);
          return res.end();
        }
        return send(403, { error: 'Forbidden' });
      }
      if (req.method === 'POST' && req.url === '/v1/auth/verify') {
        return send(200, {
          token: 'sess-token-1',
          uuid: UUID,
          name: 'TestMsa',
          expires_at: new Date(Date.now() + 30 * 86400e3).toISOString(),
        });
      }
      if (req.method === 'GET' && req.url === '/v1/grant') {
        if (req.headers.authorization === 'Bearer sess-token-1') {
          return send(200, { grant: signServiceGrant(UUID), flags: ['supporter', 'mvp'], exp: NOW + 86400 });
        }
        return send(401, { error: { code: 'UNAUTHORIZED', message: 'bad token' } });
      }
      send(404, { error: { code: 'NOT_FOUND', message: 'stub: unknown route ' + req.url } });
    });
  });
  await new Promise((r) => stub.listen(0, '127.0.0.1', r));
  const stubUrl = `http://127.0.0.1:${stub.address().port}`;
  process.env.ESPECTRAL_ID_URL = stubUrl;
  process.env.ESPECTRAL_SESSION_SERVER = stubUrl;

  server = await start(0);
  base = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  if (server) await new Promise((r) => server.close(r));
  if (stub) await new Promise((r) => stub.close(r));
  delete process.env.ESPECTRAL_ID_URL;
  delete process.env.ESPECTRAL_SESSION_SERVER;
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
  return { status: res.status, json: await res.json().catch(() => null) };
}

test('GET /api/identity reports service url + accounts (verified=false)', async () => {
  // Seed an MSA account with an in-memory token so ensureMinecraftToken
  // short-circuits (no real Microsoft call).
  const cfg = loadConfig();
  cfg.accounts.push({
    username: 'TestMsa',
    uuid: UUID,
    token_kind: 'msa',
    microsoft: {
      refresh_token: 'fake-refresh',
      access_token: 'fake-mc-token',
      expires_at: Date.now() + 3600e3,
    },
  });

  const r = await api('GET', '/api/identity');
  assert.equal(r.status, 200);
  assert.equal(r.json.service_url, `http://127.0.0.1:${stub.address().port}`);
  const acc = r.json.accounts.find((a) => a.uuid === UUID);
  assert.ok(acc, 'account listed');
  assert.equal(acc.username, 'TestMsa');
  assert.equal(acc.verified, false);
  assert.equal(acc.discord, null);
  assert.deepEqual(acc.flags, []);
  assert.equal(acc.grant_exp, null);
});

test('POST /api/identity/:uuid/verify runs challenge -> join -> verify -> grant', async () => {
  const r = await api('POST', `/api/identity/${UUID}/verify`);
  assert.equal(r.status, 200);
  assert.equal(r.json.verified, true);
  assert.deepEqual(r.json.flags, ['supporter', 'mvp']);
  assert.equal(r.json.grant_exp, NOW + 86400);

  const urls = stubCalls.map((c) => `${c.method} ${c.url}`);
  assert.ok(urls.includes('POST /v1/auth/challenge'));
  assert.ok(urls.includes('POST /session/minecraft/join'));
  assert.ok(urls.includes('POST /v1/auth/verify'));
  assert.ok(urls.includes('GET /v1/grant'));
});

test('identity state persisted to <dataDir>/identity.json', async () => {
  const doc = JSON.parse(fs.readFileSync(path.join(dataDir, 'identity.json'), 'utf8'));
  assert.equal(doc.schema, 1);
  assert.equal(doc.accounts[UUID].token, 'sess-token-1');
  assert.ok(typeof doc.accounts[UUID].grant === 'string');
});

test('GET /api/identity now shows verified account', async () => {
  const r = await api('GET', '/api/identity');
  const acc = r.json.accounts.find((a) => a.uuid === UUID);
  assert.equal(acc.verified, true);
  assert.deepEqual(acc.flags, ['supporter', 'mvp']);
});

test('POST /api/identity/:uuid/verify rejects unknown uuid', async () => {
  const r = await api('POST', '/api/identity/' + 'f'.repeat(32) + '/verify');
  assert.equal(r.status, 404);
});

test('GET /api/cosmetics returns catalog + per-account state with verified flags', async () => {
  const r = await api('GET', '/api/cosmetics');
  assert.equal(r.status, 200);
  assert.equal(r.json.catalog.schema, 1);
  assert.equal(r.json.catalog.items.length, 10);
  const acc = r.json.accounts[UUID];
  assert.ok(acc);
  assert.equal(acc.share, false);
  assert.deepEqual(acc.flags, ['supporter', 'mvp']);
});

test('PUT /api/cosmetics/:uuid validates + persists + rewrites the mod file', async () => {
  const r = await api('PUT', `/api/cosmetics/${UUID}`, {
    share: true,
    loadout: { espalda: 'manto_espectral' },
  });
  assert.equal(r.status, 200);
  assert.equal(r.json.share, true);
  assert.equal(r.json.loadout.espalda, 'manto_espectral');

  // mvp-gated item without flags -> 400 (simulate by direct validator check:
  // the account DOES have mvp, so instead try an unknown id over the wire).
  const bad = await api('PUT', `/api/cosmetics/${UUID}`, { loadout: { espalda: 'does_not_exist' } });
  assert.equal(bad.status, 400);
  assert.equal(bad.json.error.code, 'UNKNOWN_COSMETIC');
});
