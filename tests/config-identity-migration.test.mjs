import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// The espectral.es zone lapsed (~2026-10-22); espectral-id is canonical at
// https://id.uhce.top. Every existing install has
// `"identity_url": "https://id.espectral.es"` persisted, so loadConfig() must
// migrate exactly that value and leave any custom service URL alone.
//
// loadConfig() caches per module instance, so each case loads a FRESH copy of
// config.mjs via a cache-busting query — that is the only way to exercise
// "reload the persisted file" without spawning processes.
const origDataDir = process.env.ESPECTRAL_DATA_DIR;
const origIdUrl = process.env.ESPECTRAL_ID_URL;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-identity-migration-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const cfgPath = path.join(dataDir, 'config.json');

function writeConfig(obj) {
  fs.writeFileSync(cfgPath, JSON.stringify(obj, null, 2), 'utf8');
}
function readConfig() {
  return JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
}

after(() => {
  if (origDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = origDataDir;
  if (origIdUrl === undefined) delete process.env.ESPECTRAL_ID_URL;
  else process.env.ESPECTRAL_ID_URL = origIdUrl;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

test('a persisted legacy identity_url resolves to id.uhce.top and is rewritten', async () => {
  writeConfig({ identity_url: 'https://id.espectral.es', default_memory_mb: 2048 });
  const cfg = await import('../src/engine/config.mjs?legacy');
  const live = cfg.loadConfig();
  assert.equal(live.identity_url, 'https://id.uhce.top');
  assert.equal(live.default_memory_mb, 2048); // the rest of the document survives
  assert.equal(readConfig().identity_url, 'https://id.uhce.top'); // persisted, not just in memory
});

test('the legacy value with a trailing slash migrates too', async () => {
  writeConfig({ identity_url: 'https://id.espectral.es/' });
  const cfg = await import('../src/engine/config.mjs?legacy-slash');
  assert.equal(cfg.loadConfig().identity_url, 'https://id.uhce.top');
});

test('a custom (self-hosted) identity_url is never touched', async () => {
  writeConfig({ identity_url: 'https://id.example.test/v1' });
  const cfg = await import('../src/engine/config.mjs?custom');
  assert.equal(cfg.loadConfig().identity_url, 'https://id.example.test/v1');
  assert.equal(readConfig().identity_url, 'https://id.example.test/v1');
});

test('isLegacyIdentityUrl matches only the bare legacy host', async () => {
  const mod = await import('../src/engine/config.mjs?helpers');
  assert.deepEqual(mod.LEGACY_IDENTITY_URLS, ['https://id.espectral.es']);
  for (const url of [
    'https://id.espectral.es',
    'https://id.espectral.es/',
    'https://id.espectral.es//',
    '  https://id.espectral.es  ',
    'https://ID.espectral.es',
  ]) {
    assert.equal(mod.isLegacyIdentityUrl(url), true, url);
  }
  for (const url of [
    'https://id.espectral.es.evil.example',
    'http://id.espectral.es',
    'https://id.espectral.es/v1',
    'https://id.uhce.top',
    'https://espectral.es',
    '',
    null,
    undefined,
    42,
  ]) {
    assert.equal(mod.isLegacyIdentityUrl(url), false, String(url));
  }
});

test('identityUrl() defaults to id.uhce.top, honours config, then the env override', async () => {
  // config.mjs here is the instance identity.mjs imports (no query), which was
  // first loaded with the legacy document written above by the helpers case —
  // rewrite and load explicitly so the assertion is order-independent.
  writeConfig({ identity_url: 'https://id.espectral.es' });
  const identity = await import('../src/engine/identity.mjs');
  assert.equal(identity.DEFAULT_IDENTITY_URL, 'https://id.uhce.top');

  delete process.env.ESPECTRAL_ID_URL;
  assert.equal(identity.identityUrl(), 'https://id.uhce.top');

  const cfg = await import('../src/engine/config.mjs');
  const fromConfig = cfg.loadConfig();
  assert.equal(fromConfig.identity_url, 'https://id.uhce.top');
  assert.equal(identity.identityUrl(), 'https://id.uhce.top');

  process.env.ESPECTRAL_ID_URL = 'http://127.0.0.1:9/';
  try {
    assert.equal(identity.identityUrl(), 'http://127.0.0.1:9'); // trailing slash trimmed
  } finally {
    delete process.env.ESPECTRAL_ID_URL;
  }
});
