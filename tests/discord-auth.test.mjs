import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import http from 'node:http';

// Sandbox the data dir + env BEFORE importing engine modules.
const origEnv = {};
for (const k of ['ESPECTRAL_DATA_DIR', 'ESPECTRAL_ID_URL', 'DISCORD_CLIENT_ID', 'ESPECTRAL_DISCORD_MOCK']) {
  origEnv[k] = process.env[k];
}
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-discord-auth-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;
delete process.env.ESPECTRAL_DISCORD_MOCK;
process.env.DISCORD_CLIENT_ID = 'test-client-id';

const discordAuth = await import('../src/engine/discordAuth.mjs');
const identity = await import('../src/engine/identity.mjs');

const LINK_UUID = 'aabbccddeeff00112233445566778899';

let stub;
let identifyDelayMs = 0;

before(async () => {
  // Stub espectral-id: identify/link endpoints used by the OAuth callback.
  stub = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', async () => {
      const parsed = body ? JSON.parse(body) : null;
      const send = (s, o) => {
        res.writeHead(s, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify(o));
      };
      if (req.method === 'POST' && req.url === '/v1/discord/identify') {
        if (parsed?.code === 'fail-code') {
          return send(502, { error: { code: 'DISCORD_IDENTIFY_FAILED', message: 'exchange failed' } });
        }
        if (identifyDelayMs) await new Promise((r) => setTimeout(r, identifyDelayMs));
        return send(200, { discord: { id: '9999', username: 'ServiceUser', avatar: null } });
      }
      if (req.method === 'POST' && req.url === '/v1/discord/link') {
        if (identifyDelayMs) await new Promise((r) => setTimeout(r, identifyDelayMs));
        return send(200, { discord: { id: '8888', username: 'LinkUser', avatar: null }, flags: ['supporter'] });
      }
      if (req.method === 'GET' && req.url === '/v1/grant') {
        return send(401, { error: { code: 'UNAUTHORIZED', message: 'bad token' } });
      }
      send(404, { error: { code: 'NOT_FOUND', message: 'stub: ' + req.url } });
    });
  });
  await new Promise((r) => stub.listen(0, '127.0.0.1', r));
  process.env.ESPECTRAL_ID_URL = `http://127.0.0.1:${stub.address().port}`;
});

after(async () => {
  if (stub) await new Promise((r) => stub.close(r));
  for (const [k, v] of Object.entries(origEnv)) {
    if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }
  fs.rmSync(dataDir, { recursive: true, force: true });
});

test('claim stays pending while the code exchange is in flight (no invalid race)', async () => {
  const { state } = discordAuth.getAuthStartUrl();
  identifyDelayMs = 200;
  try {
    const cb = discordAuth.handleCallback({ code: 'real-code', state });
    // The callback holds the state while the service exchange runs — a poll in
    // this window must answer 'pending', not 'invalid' (the old code deleted
    // the state before awaiting, so every poll raced to invalid).
    assert.equal(discordAuth.claimSessionUser(state).status, 'pending');
    const result = await cb;
    assert.equal(result.user.id, '9999');
    const claim = discordAuth.claimSessionUser(state);
    assert.equal(claim.status, 'ok');
    assert.equal(claim.user.id, '9999');
    // One-time claim: the second poll is invalid.
    assert.equal(discordAuth.claimSessionUser(state).status, 'invalid');
  } finally {
    identifyDelayMs = 0;
  }
});

test('a duplicate callback for the same state is rejected while the exchange runs', async () => {
  const { state } = discordAuth.getAuthStartUrl();
  identifyDelayMs = 200;
  try {
    const cb = discordAuth.handleCallback({ code: 'real-code', state });
    await assert.rejects(discordAuth.handleCallback({ code: 'real-code-2', state }), /already used/i);
    const r = await cb;
    assert.equal(r.user.id, '9999');
    assert.equal(discordAuth.claimSessionUser(state).status, 'ok');
  } finally {
    identifyDelayMs = 0;
  }
});

test('link flow: claim is pending mid-flight and resolves linked:true for the uuid', async () => {
  // The link path needs a live identity session token on the uuid.
  identity.updateEntry(LINK_UUID, {
    token: 'sess-1',
    token_expires_at: new Date(Date.now() + 3600e3).toISOString(),
  });
  const { url, state } = discordAuth.getAuthStartUrl({ linkUuid: LINK_UUID });
  assert.ok(url.includes(state));
  identifyDelayMs = 200;
  try {
    const cb = discordAuth.handleCallback({ code: 'link-code', state });
    assert.equal(discordAuth.claimSessionUser(state).status, 'pending');
    const r = await cb;
    assert.equal(r.linked, true);
    assert.equal(r.sessionId, null);
    const claim = discordAuth.claimSessionUser(state);
    assert.equal(claim.status, 'ok');
    assert.equal(claim.linked, true);
    assert.equal(claim.uuid, LINK_UUID);
    assert.equal(claim.user.id, '8888');
    const entry = identity.getEntry(LINK_UUID);
    assert.equal(entry.discord.id, '8888');
  } finally {
    identifyDelayMs = 0;
  }
});

test('failed exchange drops the state so the next claim reports invalid', async () => {
  const { state } = discordAuth.getAuthStartUrl();
  await assert.rejects(discordAuth.handleCallback({ code: 'fail-code', state }));
  assert.equal(discordAuth.claimSessionUser(state).status, 'invalid');
});

test('mock mode still mints a state and completes the flow', async () => {
  // Mock mode must not depend on credentials at all: the synthetic code
  // short-circuits before any Discord round-trip even when a client id is
  // configured (DISCORD_CLIENT_ID is set for the whole suite).
  process.env.ESPECTRAL_DISCORD_MOCK = '1';
  try {
    const { url, state } = discordAuth.getAuthStartUrl();
    assert.ok(url.includes('/api/discord/callback'), url);
    assert.ok(url.includes('code=mock_code'), url);
    const r = await discordAuth.handleCallback({ code: 'mock_code', state });
    assert.equal(r.user.name, 'TestPlayer');
    assert.ok(r.sessionId);
    assert.equal(discordAuth.claimSessionUser(state).status, 'ok');
  } finally {
    delete process.env.ESPECTRAL_DISCORD_MOCK;
  }
});

test('a stock install uses the public Espectral app id with no config', () => {
  const prevDiscord = process.env.DISCORD_CLIENT_ID;
  const prevEspectral = process.env.ESPECTRAL_DISCORD_CLIENT_ID;
  delete process.env.DISCORD_CLIENT_ID;
  delete process.env.ESPECTRAL_DISCORD_CLIENT_ID;
  try {
    assert.equal(discordAuth.DEFAULT_DISCORD_CLIENT_ID, '1454618588659843072');
    assert.equal(discordAuth.getDiscordCredentials().clientId, '1454618588659843072');
    const { url } = discordAuth.getAuthStartUrl({ linkUuid: LINK_UUID });
    const parsed = new URL(url);
    assert.equal(parsed.origin + parsed.pathname, 'https://discord.com/oauth2/authorize');
    assert.equal(parsed.searchParams.get('client_id'), '1454618588659843072');
    assert.equal(parsed.searchParams.get('redirect_uri'), discordAuth.REDIRECT_URI);
    assert.equal(parsed.searchParams.get('response_type'), 'code');
    assert.equal(parsed.searchParams.get('scope'), 'identify');
    assert.equal(parsed.searchParams.get('state').length, 32);
  } finally {
    if (prevDiscord !== undefined) process.env.DISCORD_CLIENT_ID = prevDiscord;
    if (prevEspectral !== undefined) process.env.ESPECTRAL_DISCORD_CLIENT_ID = prevEspectral;
  }
});

test('config.json discord.client_id overrides the env and the built-in default', async () => {
  const { saveConfig } = await import('../src/engine/config.mjs');
  saveConfig({ discord: { client_id: 'config-app-id' } });
  try {
    assert.equal(discordAuth.getDiscordCredentials().clientId, 'config-app-id');
    const { url } = discordAuth.getAuthStartUrl();
    assert.equal(new URL(url).searchParams.get('client_id'), 'config-app-id');
  } finally {
    saveConfig({ discord: null });
  }
  // Back to the env override the suite set up.
  assert.equal(discordAuth.getDiscordCredentials().clientId, 'test-client-id');
});

test('unknown or missing state claims invalid', () => {
  assert.equal(discordAuth.claimSessionUser('deadbeefcafe').status, 'invalid');
  assert.equal(discordAuth.claimSessionUser('').status, 'invalid');
});
