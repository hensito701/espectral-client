import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// dataDir() resolves ESPECTRAL_DATA_DIR at call time, but set it before
// importing so module evaluation sees the sandbox too.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-config-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { loadConfig, saveConfig, appConfig, DEFAULT_CONFIG, configPath } = await import('../src/engine/config.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

test('loadConfig creates the document from defaults when missing', () => {
  assert.equal(fs.existsSync(configPath()), false);
  const cfg = loadConfig();
  assert.equal(cfg.default_memory_mb, DEFAULT_CONFIG.default_memory_mb);
  assert.equal(cfg.theme, 'dark');
  assert.equal(cfg.accounts.length, 0);
  assert.equal(fs.existsSync(configPath()), true); // lazily created
});

test('saveConfig merges the patch and persists atomically', () => {
  saveConfig({ default_memory_mb: 4096, download_concurrency: 4, jdk_path_override: '/opt/jdk25' });
  assert.equal(fs.existsSync(configPath() + '.tmp'), false); // no leftover tmp file
  const parsed = JSON.parse(fs.readFileSync(configPath(), 'utf8'));
  assert.equal(parsed.default_memory_mb, 4096);
  assert.equal(parsed.download_concurrency, 4);
  assert.equal(parsed.jdk_path_override, '/opt/jdk25');
  assert.equal(parsed.theme, 'dark'); // defaults still merged under the patch
});

test('H2-R regression: MC access token and XBL/XSTS chain are NEVER written to disk', () => {
  // The live object carries the token in memory for same-session reuse ...
  const live = saveConfig({
    access_token: 'ACCESS_SECRET_VALUE',
    xbl_token: 'XBL_SECRET_VALUE',
    xsts_token: 'XSTS_SECRET_VALUE',
    user_hash: 'USER_HASH_SECRET_VALUE',
    discord_enabled: true,
  });
  assert.equal(live.access_token, 'ACCESS_SECRET_VALUE'); // memory keeps it
  // ... but the persisted document must not contain any of them. This pins the
  // JSON.stringify(value, replacer, 2) call shape — the replacer is the 2nd
  // argument; a 4th argument is silently ignored (H2-R regression).
  const raw = fs.readFileSync(configPath(), 'utf8');
  assert.equal(raw.includes('ACCESS_SECRET_VALUE'), false, 'access_token leaked to disk');
  assert.equal(raw.includes('XBL_SECRET_VALUE'), false, 'xbl_token leaked to disk');
  assert.equal(raw.includes('XSTS_SECRET_VALUE'), false, 'xsts_token leaked to disk');
  assert.equal(raw.includes('USER_HASH_SECRET_VALUE'), false, 'user_hash leaked to disk');
  // The document is still valid JSON with the non-secret patch applied.
  const parsed = JSON.parse(raw);
  assert.equal(parsed.discord_enabled, true);
  assert.equal(parsed.default_memory_mb, 4096);
});

test('appConfig exposes only the whitelisted AppConfig subset', () => {
  const keys = Object.keys(appConfig()).sort();
  assert.deepEqual(keys, [
    'aot_auto_train',
    'data_dir',
    'default_memory_mb',
    'discord_enabled',
    'download_concurrency',
    'fast_boot',
    'fullbright_on_launch',
    'jdk_path_override',
  ]);
  assert.equal('access_token' in appConfig(), false);
  assert.equal('accounts' in appConfig(), false);
  assert.equal('jvm' in appConfig(), false);
  // nofog_on_launch was removed — vanilla has no options.txt fog key, so the
  // integrated no-fog feature was dropped (see FINDINGS iteration 40).
  assert.equal('nofog_on_launch' in appConfig(), false);
});

test('appConfig coerces QoL flags to strict booleans', () => {
  saveConfig({ fullbright_on_launch: 1 });
  const cfg = appConfig();
  assert.equal(cfg.fullbright_on_launch, false); // 1 !== true
  saveConfig({ fullbright_on_launch: true });
  assert.equal(appConfig().fullbright_on_launch, true);
});

test('appConfig coerces fast_boot to a strict boolean', () => {
  saveConfig({ fast_boot: 1 });
  assert.equal(appConfig().fast_boot, false); // 1 !== true
  saveConfig({ fast_boot: true });
  assert.equal(appConfig().fast_boot, true);
  saveConfig({ fast_boot: false });
  assert.equal(appConfig().fast_boot, false);
});
