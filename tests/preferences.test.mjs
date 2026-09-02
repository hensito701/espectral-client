import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir BEFORE importing the engine so module evaluation sees it
// too (same pattern as tests/config.test.mjs). applyLaunchPreferences writes
// <dataDir>/instances/<name>/options.txt and loadConfig reads
// <dataDir>/config.json — both land in the sandbox, never the real data dir.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-prefs-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { applyLaunchPreferences } = await import('../src/engine/launch.mjs');
const { saveConfig } = await import('../src/engine/config.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const INSTANCE = 'testinst';

function optionsPath() {
  return path.join(dataDir, 'instances', INSTANCE, 'options.txt');
}

test('fullbright on + existing options.txt -> gamma:1.0 seeded, other keys byte-identical', () => {
  saveConfig({ fullbright_on_launch: true });
  fs.mkdirSync(path.dirname(optionsPath()), { recursive: true });
  const before = 'version:1343\nkey_forward.0:true\n';
  fs.writeFileSync(optionsPath(), before, 'utf8');
  applyLaunchPreferences({ name: INSTANCE });
  const after = fs.readFileSync(optionsPath(), 'utf8');
  // header kept, untouched key round-trips verbatim, gamma appended
  assert.equal(after, 'version:1343\nkey_forward.0:true\ngamma:1.0\n');
  assert.ok(after.startsWith('version:1343\n'));
  assert.ok(after.includes('key_forward.0:true'));
});

test('fullbright on + no options.txt -> file created with gamma:1.0', () => {
  saveConfig({ fullbright_on_launch: true });
  fs.rmSync(optionsPath(), { force: true });
  applyLaunchPreferences({ name: INSTANCE });
  assert.equal(fs.readFileSync(optionsPath(), 'utf8'), 'gamma:1.0\n');
});

test('fullbright off -> options.txt untouched (not created)', () => {
  saveConfig({ fullbright_on_launch: false });
  // no file -> still no file
  fs.rmSync(optionsPath(), { force: true });
  applyLaunchPreferences({ name: INSTANCE });
  assert.equal(fs.existsSync(optionsPath()), false);
  // existing file -> byte-identical (the preference is off, so no write at all)
  const before = 'version:1343\ngamma:0.5\nkey_forward.0:true\n';
  fs.writeFileSync(optionsPath(), before, 'utf8');
  applyLaunchPreferences({ name: INSTANCE });
  assert.equal(fs.readFileSync(optionsPath(), 'utf8'), before);
});

test('fullbright on + existing gamma -> replaced with 1.0, rest intact', () => {
  saveConfig({ fullbright_on_launch: true });
  const before = 'version:1343\ngamma:0.5\nkey_forward.0:true\n';
  fs.writeFileSync(optionsPath(), before, 'utf8');
  applyLaunchPreferences({ name: INSTANCE });
  // gamma re-serialized in place; header and other keys untouched
  assert.equal(fs.readFileSync(optionsPath(), 'utf8'), 'version:1343\ngamma:1.0\nkey_forward.0:true\n');
});
