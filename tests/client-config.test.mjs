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
    assert.ok(typeof item.category === 'string', `${item.id} has a category`);
  }
  assert.equal(REGISTRY.length, 13);
  assert.deepEqual(
    [...new Set(REGISTRY.map((r) => r.category))].sort(),
    ['chat', 'controls', 'hud', 'visual'],
  );
  const zoom = REGISTRY.find((r) => r.id === 'zoom');
  assert.equal(zoom.keybind, 'key.keyboard.z');
});

test('FEATURE_DEFAULTS: has default feature configurations', () => {
  assert.equal(FEATURE_DEFAULTS.fullbright.enabled, true);
  assert.equal(FEATURE_DEFAULTS.fullbright.gamma, 1.0);
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
  const cfg = loadClientConfig('missing-inst');
  assert.equal(cfg.schema, 2);
  assert.equal(cfg.suite.enabled, true);
  assert.deepEqual(cfg.features, FEATURE_DEFAULTS);
  assert.deepEqual(cfg.macros, []);
  assert.equal(cfg.features.nofog.enabled, false);
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
  assert.equal(seeded.schema, 2); // v1 upgraded in place
  assert.equal(seeded.suite.enabled, true); // master switch added, on by default
  assert.equal(seeded.custom_field, 'keep-me');
  assert.equal(seeded.features.fullbright.enabled, false); // preserved user setting
  assert.equal(seeded.features.fullbright.gamma, 1.0); // filled missing default field
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

test('seedClientConfig: fresh file gets schema 2 and an enabled suite', () => {
  seedClientConfig('fresh-suite-inst');
  const seeded = JSON.parse(fs.readFileSync(clientConfigPath('fresh-suite-inst'), 'utf8'));
  assert.equal(seeded.schema, 2);
  assert.equal(seeded.suite.enabled, true);
  assert.equal(seeded.features.nofog.enabled, false);
  assert.equal(seeded.features.fullbright.gamma, 1.0);
});

test('seedClientConfig: v1 upgrade preserves flags, suite choice and unknown fields', () => {
  const cfgPath = clientConfigPath('v1-upgrade-inst');
  fs.mkdirSync(path.dirname(cfgPath), { recursive: true });
  fs.writeFileSync(
    cfgPath,
    JSON.stringify({
      schema: 1,
      custom_field: 'keep-me',
      suite: { enabled: false },
      features: { fullbright: { enabled: false, gamma: 3.5 }, custom_mod: { enabled: true } },
      macros: [],
    }),
  );

  seedClientConfig('v1-upgrade-inst');
  const seeded = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  assert.equal(seeded.schema, 2);
  assert.equal(seeded.suite.enabled, false); // stored switch choice untouched
  assert.equal(seeded.custom_field, 'keep-me');
  assert.equal(seeded.features.fullbright.enabled, false); // stored flag untouched
  assert.equal(seeded.features.fullbright.gamma, 3.5); // stored value untouched
  assert.equal(seeded.features.custom_mod.enabled, true);
});

test('patchClientConfig: suite master switch round-trips', async () => {
  const off = await patchClientConfig('test-inst', { suite: { enabled: false } });
  assert.equal(off.config.suite.enabled, false);
  assert.equal(off.config.schema, 2);

  const on = await patchClientConfig('test-inst', { suite: { enabled: true } });
  assert.equal(on.config.suite.enabled, true);

  const raw = JSON.parse(fs.readFileSync(clientConfigPath('test-inst'), 'utf8'));
  assert.equal(raw.suite.enabled, true);
});

test('patchClientConfig: unknown top-level fields are rejected', async () => {
  await assert.rejects(
    () => patchClientConfig('test-inst', { bogus_field: true }),
    (err) => err.code === 'BAD_PATCH',
  );
});

test('bare-boolean feature entries are tolerated', async () => {
  const cfgPath = clientConfigPath('bool-entry-inst');
  fs.mkdirSync(path.dirname(cfgPath), { recursive: true });
  fs.writeFileSync(
    cfgPath,
    JSON.stringify({ schema: 2, suite: { enabled: true }, features: { coords: true }, macros: [] }),
  );

  const loaded = loadClientConfig('bool-entry-inst');
  assert.equal(loaded.features.coords.enabled, true);

  seedClientConfig('bool-entry-inst');
  const seeded = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  assert.equal(seeded.features.coords.enabled, true); // flag preserved through seed
});
