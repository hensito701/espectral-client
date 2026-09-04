import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-client-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const {
  REGISTRY,
  FEATURE_DEFAULTS,
  clientConfigPath,
  loadClientConfig,
  seedClientConfig,
  patchClientConfig,
  getClientInfo,
  isSupported,
} = await import('../src/engine/client.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

test('REGISTRY: exports Contract A feature entries', () => {
  assert.ok(Array.isArray(REGISTRY));
  const ids = REGISTRY.map((r) => r.id);
  assert.ok(ids.includes('fullbright'));
  assert.ok(ids.includes('nofog'));
  assert.ok(ids.includes('zoom'));
  assert.ok(ids.includes('macros'));
  for (const id of ['potionstatus', 'coords', 'healthstatus', 'armorstatus', 'fpsping', 'lowfire', 'clearwater']) {
    assert.ok(ids.includes(id), `missing registry entry: ${id}`);
  }
  for (const id of ['chatheads', 'skin3d']) {
    assert.ok(ids.includes(id), `missing registry entry: ${id}`);
  }

  for (const item of REGISTRY) {
    assert.ok(typeof item.id === 'string');
    assert.ok(typeof item.name === 'string');
    assert.ok(typeof item.description === 'string');
    assert.equal(item.kind, 'owned');
    assert.ok(typeof item.defaultEnabled === 'boolean');
  }
  const zoom = REGISTRY.find((r) => r.id === 'zoom');
  assert.equal(zoom.keybind, 'key.keyboard.z');
});

test('FEATURE_DEFAULTS: has default feature configurations', () => {
  assert.equal(FEATURE_DEFAULTS.fullbright.enabled, true);
  assert.equal(FEATURE_DEFAULTS.fullbright.gamma, 15.0);
  assert.equal(FEATURE_DEFAULTS.nofog.enabled, false);
  assert.equal(FEATURE_DEFAULTS.zoom.enabled, true);
  assert.equal(FEATURE_DEFAULTS.macros.enabled, true);
  for (const id of ['potionstatus', 'coords', 'healthstatus', 'armorstatus', 'fpsping', 'lowfire', 'clearwater']) {
    assert.equal(FEATURE_DEFAULTS[id].enabled, false, `${id} defaults off`);
  }
  for (const id of ['chatheads', 'skin3d']) {
    assert.equal(FEATURE_DEFAULTS[id].enabled, false, `${id} defaults off`);
  }
});

test('loadClientConfig: returns defaults when file is missing', () => {
  const cfg = loadClientConfig('test-inst');
  assert.equal(cfg.schema, 1);
  assert.deepEqual(cfg.features, FEATURE_DEFAULTS);
  assert.deepEqual(cfg.macros, []);
});

test('seedClientConfig: writes defaults and preserves unknown fields', () => {
  const cfgPath = clientConfigPath('test-inst');
  fs.mkdirSync(path.dirname(cfgPath), { recursive: true });
  fs.writeFileSync(
    cfgPath,
    JSON.stringify({
      schema: 1,
      custom_field: 'keep-me',
      features: { fullbright: { enabled: false }, custom_mod: { enabled: true } },
      macros: [{ id: 'm1', name: 'Test', keybind: 'key.keyboard.h', actions: [{ type: 'chat', text: 'hi' }] }],
    }),
  );

  seedClientConfig('test-inst');
  const seeded = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  assert.equal(seeded.custom_field, 'keep-me');
  assert.equal(seeded.features.fullbright.enabled, false); // preserved user setting
  assert.equal(seeded.features.fullbright.gamma, 15.0); // filled missing default field
  assert.equal(seeded.features.zoom.enabled, true); // added missing default
  assert.equal(seeded.features.custom_mod.enabled, true); // preserved custom feature
  assert.equal(seeded.macros.length, 1);
});
test('patchClientConfig: updates features and macros atomically', async () => {
  const instDir = path.join(dataDir, 'instances', 'test-inst');
  fs.mkdirSync(instDir, { recursive: true });
  fs.writeFileSync(path.join(instDir, 'instance.json'), JSON.stringify({ name: 'test-inst', version: '1.21.11', loader: 'vanilla' }));

  const patch = {
    features: { fullbright: { enabled: false }, zoom: { enabled: false } },
    macros: [
      {
        id: 'macro-1',
        name: 'Say Hello',
        keybind: 'key.keyboard.h',
        actions: [{ type: 'chat', text: 'hello world' }],
      },
    ],
  };
  const res = await patchClientConfig('test-inst', patch);
  assert.equal(res.config.features.fullbright.enabled, false);
  assert.equal(res.config.features.zoom.enabled, false);
  assert.deepEqual(res.errors, []);
  assert.equal(res.config.macros.length, 1);
  assert.equal(res.config.macros[0].id, 'macro-1');

  // Verify file on disk
  const reloaded = loadClientConfig('test-inst');
  assert.equal(reloaded.features.fullbright.enabled, false);
  assert.equal(reloaded.features.zoom.enabled, false);
  assert.equal(reloaded.macros.length, 1);
});

test('patchClientConfig: rejects invalid macros with BAD_MACRO', async () => {
  // Invalid action type
  await assert.rejects(
    () =>
      patchClientConfig('test-inst', {
        macros: [{ id: 'bad1', name: 'Bad', keybind: 'h', actions: [{ type: 'invalid', text: 'test' }] }],
      }),
    (err) => err.code === 'BAD_MACRO',
  );

  // Duplicate macro id
  await assert.rejects(
    () =>
      patchClientConfig('test-inst', {
        macros: [
          { id: 'dup', name: 'M1', keybind: 'h', actions: [] },
          { id: 'dup', name: 'M2', keybind: 'j', actions: [] },
        ],
      }),
    (err) => err.code === 'BAD_MACRO',
  );
});
