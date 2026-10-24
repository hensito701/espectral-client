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
  for (const id of ['chatheads', 'skin3d', 'tabheads']) {
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
  assert.equal(REGISTRY.length, 16);
  assert.deepEqual(
    [...new Set(REGISTRY.map((r) => r.category))].sort(),
    ['chat', 'controls', 'cosmetics', 'hud', 'visual'],
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

// --- personalization (Workstream 4) -----------------------------------------

test('patchClientConfig: personalization validates ids and rejects unknown keys', async () => {
  const ok = await patchClientConfig('test-inst', {
    personalization: { theme: 'carmesi', hud_theme: 'pixel', background: 'nebula-night', reduced_motion: true },
  });
  assert.equal(ok.config.personalization.theme, 'carmesi');
  assert.equal(ok.config.personalization.hud_theme, 'pixel');
  assert.equal(ok.config.personalization.background, 'nebula-night');
  assert.equal(ok.config.personalization.reduced_motion, true);

  // null background is the explicit "default panorama" value.
  const cleared = await patchClientConfig('test-inst', { personalization: { background: null } });
  assert.equal(cleared.config.personalization.background, null);

  for (const [label, body] of [
    ['bad theme id', { personalization: { theme: 'obsidiana' } }],
    ['non-string theme', { personalization: { theme: 4 } }],
    ['bad hud_theme id', { personalization: { hud_theme: 'neon' } }],
    ['bad background type', { personalization: { background: 12 } }],
    ['bad background chars', { personalization: { background: '../evil' } }],
    ['bad reduced_motion type', { personalization: { reduced_motion: 'yes' } }],
    ['unknown personalization key', { personalization: { sparkle: true } }],
    ['non-object personalization', { personalization: 'gold' }],
  ]) {
    await assert.rejects(
      () => patchClientConfig('test-inst', body),
      (err) => err.code === 'BAD_PERSONALIZATION',
      label,
    );
  }
});

test('loadClientConfig: personalization defaults fill and preserve stored keys', async () => {
  const cfgPath = clientConfigPath('perso-inst');
  fs.mkdirSync(path.dirname(cfgPath), { recursive: true });
  fs.writeFileSync(
    cfgPath,
    JSON.stringify({
      schema: 2,
      features: {},
      personalization: { theme: 'glaciar', future_key: 7 },
    }),
  );
  const cfg = loadClientConfig('perso-inst');
  assert.equal(cfg.personalization.theme, 'glaciar');
  assert.equal(cfg.personalization.hud_theme, 'cristal'); // default filled
  assert.equal(cfg.personalization.background, null);
  assert.equal(cfg.personalization.reduced_motion, false);
  assert.equal(cfg.personalization.future_key, 7); // unknown file keys survive

  const bare = loadClientConfig('no-such-inst');
  assert.deepEqual(bare.personalization, {
    theme: 'oro',
    hud_theme: 'cristal',
    background: null,
    reduced_motion: false,
  });
});

test('seedClientConfig: mirrors launcher accent into personalization.theme', async () => {
  const { saveConfig } = await import('../src/engine/config.mjs');
  saveConfig({ accent: 'amatista', reduced_motion: true });

  const cfgPath = clientConfigPath('perso-seed-inst');
  fs.mkdirSync(path.dirname(cfgPath), { recursive: true });
  fs.writeFileSync(
    cfgPath,
    JSON.stringify({
      schema: 2,
      features: {},
      personalization: { hud_theme: 'minimo', background: 'aurora' },
    }),
  );

  seedClientConfig('perso-seed-inst');
  const seeded = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  assert.equal(seeded.personalization.theme, 'amatista'); // accent mirrored
  assert.equal(seeded.personalization.reduced_motion, true); // global choice mirrored
  assert.equal(seeded.personalization.hud_theme, 'minimo'); // instance choice kept
  assert.equal(seeded.personalization.background, 'aurora'); // instance pack kept

  saveConfig({ accent: 'oro', reduced_motion: false }); // restore defaults
});

// --- cosmetics viewer state (Suite → Cosméticos sub-screen) -----------------

test('FEATURE_DEFAULTS: cosmetics carries the viewer state keys', () => {
  const cosmetics = FEATURE_DEFAULTS.cosmetics;
  assert.equal(cosmetics.enabled, true);
  for (const key of [
    'own',
    'others',
    'slot_espalda',
    'slot_cuello',
    'slot_cabeza',
    'slot_elitros',
    'emotes',
    'physics',
  ]) {
    assert.equal(cosmetics[key], true, `cosmetics.${key} defaults on`);
  }
});

test('patchClientConfig: cosmetics state keys round-trip inside the feature', async () => {
  const instDir = path.join(dataDir, 'instances', 'cosmetics-inst');
  fs.mkdirSync(instDir, { recursive: true });
  fs.writeFileSync(
    path.join(instDir, 'instance.json'),
    JSON.stringify({ name: 'cosmetics-inst', version: '1.21.11', loader: 'vanilla' }),
  );
  const patched = await patchClientConfig('cosmetics-inst', {
    features: { cosmetics: { physics: false, slot_cabeza: false } },
  });
  assert.equal(patched.config.features.cosmetics.physics, false);
  assert.equal(patched.config.features.cosmetics.slot_cabeza, false);
  // Untouched sibling keys keep their defaults — the mod writes one key at a time.
  assert.equal(patched.config.features.cosmetics.own, true);
  assert.equal(patched.config.features.cosmetics.slot_espalda, true);

  // The raw file only carries what PATCH wrote (defaults are filled by the
  // normalizer, and by the mod's ClientConfig when the game writes them).
  const raw = JSON.parse(fs.readFileSync(clientConfigPath('cosmetics-inst'), 'utf8'));
  assert.equal(raw.features.cosmetics.physics, false);
  assert.equal(raw.features.cosmetics.slot_cabeza, false);

  const reloaded = loadClientConfig('cosmetics-inst');
  assert.equal(reloaded.features.cosmetics.own, true); // default filled
  assert.equal(reloaded.features.cosmetics.physics, false);
  assert.equal(reloaded.features.cosmetics.emotes, true);
});

// --- macros written by the in-game editor (Suite → Macros sub-screen) -------

test('patchClientConfig: accepts macros in the shape the in-game editor writes', async () => {
  const instDir = path.join(dataDir, 'instances', 'macro-inst');
  fs.mkdirSync(instDir, { recursive: true });
  fs.writeFileSync(
    path.join(instDir, 'instance.json'),
    JSON.stringify({ name: 'macro-inst', version: '1.21.11', loader: 'vanilla' }),
  );

  // Exactly what ClientConfig.putMacro writes: sanitized id, one action whose
  // type follows the text, vanilla keybind strings (incl. layout keysyms).
  const macros = [
    {
      id: 'macro-helpop',
      name: 'macro-helpop',
      keybind: 'key.keyboard.semicolon', // Ñ on an ES layout
      actions: [{ type: 'command', text: '/helpop ayuda' }],
    },
    {
      id: 'saludo',
      name: 'saludo',
      keybind: 'key.keyboard.231', // Ç: keysym with no vanilla name entry
      actions: [{ type: 'chat', text: 'hola a todos' }],
    },
    {
      id: 'tl',
      name: 'tl',
      keybind: 'key.keyboard.t',
      actions: [{ type: 'command', text: '/tl' }],
    },
  ];

  const res = await patchClientConfig('macro-inst', { macros });
  assert.deepEqual(res.errors, []);
  assert.equal(res.config.macros.length, 3);
  for (const macro of res.config.macros) {
    assert.match(macro.id, /^[A-Za-z0-9_-]{1,32}$/);
    assert.ok(macro.name.length > 0 && macro.name.length <= 40);
    assert.ok(macro.keybind.length > 0 && macro.keybind.length <= 32);
    assert.equal(macro.actions.length, 1);
    assert.ok(['chat', 'command'].includes(macro.actions[0].type));
    assert.ok(macro.actions[0].text.length <= 256);
  }

  const reloaded = loadClientConfig('macro-inst');
  assert.deepEqual(
    reloaded.macros.map((m) => [m.id, m.keybind]),
    [
      ['macro-helpop', 'key.keyboard.semicolon'],
      ['saludo', 'key.keyboard.231'],
      ['tl', 'key.keyboard.t'],
    ],
  );
});
