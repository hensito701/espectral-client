import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox BOTH data dir (ESPECTRAL_DATA_DIR) and Lunar's app-data root
// (APPDATA) before importing, so the engine modules only ever see fake files.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const originalAppData = process.env.APPDATA;
const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-import-test-'));
const dataDir = path.join(sandbox, 'data');
process.env.ESPECTRAL_DATA_DIR = dataDir;
process.env.APPDATA = path.join(sandbox, 'appdata');

const { makeLunarSource, detectImportSources, importProfile, pathEscapes, ImportError } = await import('../src/engine/import.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  if (originalAppData === undefined) delete process.env.APPDATA;
  else process.env.APPDATA = originalAppData;
  fs.rmSync(sandbox, { recursive: true, force: true });
});

// Fake Lunar install: <APPDATA>/.lunarclient/settings/launcher.json with the
// given gameDirectory. The APPDATA candidate always wins over the homedir
// probe, so tests never consult the real ~/.lunarclient.
function writeLunarLauncher(gameDirectory) {
  const settingsDir = path.join(process.env.APPDATA, '.lunarclient', 'settings');
  fs.mkdirSync(settingsDir, { recursive: true });
  fs.writeFileSync(
    path.join(settingsDir, 'launcher.json'),
    JSON.stringify({ settings: { gameDirectory } }),
    'utf8'
  );
  return settingsDir;
}

// ---------------------------------------------------------------------------
// pathEscapes — the importProfile per-file containment guard, verbatim
// (FINDINGS iteration 36 H6 layer 2; iteration 39 backlog item).
// ---------------------------------------------------------------------------

const ROOT = path.join(sandbox, 'root');

test('pathEscapes: a child inside the root is contained', () => {
  assert.equal(pathEscapes(ROOT, path.join(ROOT, 'options.txt')), false);
  assert.equal(pathEscapes(ROOT, path.join(ROOT, 'game', 'servers.dat')), false);
});

test('pathEscapes: a traversal candidate escapes', () => {
  assert.equal(pathEscapes(ROOT, path.join(ROOT, '..', 'evil', 'options.txt')), true);
  assert.equal(pathEscapes(ROOT, path.join(ROOT, '..', '..', 'windows', 'system32')), true);
});

test('pathEscapes: a prefix-sibling escapes (startsWith(root + sep), not raw prefix)', () => {
  // 'root-evil/options.txt' shares the string prefix but is NOT inside 'root/'.
  assert.equal(pathEscapes(ROOT, path.join(`${ROOT}-evil`, 'options.txt')), true);
});

test('pathEscapes: an absolute path outside the root escapes', () => {
  assert.equal(pathEscapes(ROOT, path.join(os.tmpdir(), 'elsewhere', 'options.txt')), true);
});

test('pathEscapes: equality with the root itself counts as escaping (strict-subpath semantics)', () => {
  assert.equal(pathEscapes(ROOT, ROOT), true);
});

test('pathEscapes: a filesystem root contains nothing (root + sep double-separator edge)', () => {
  // The guard's only firing input in importProfile: a targetDir resolving to
  // the fs root. path.join(root, file) then starts with root, not root + sep.
  const fsRoot = path.parse(os.tmpdir()).root; // 'C:\\' on win32, '/' on POSIX
  assert.equal(pathEscapes(fsRoot, path.join(fsRoot, 'options.txt')), true);
});

// ---------------------------------------------------------------------------
// makeLunarSource — containment of launcher.json gameDirectory (H6 layer 1).
// A gameDirectory that escapes the Lunar install root yields null (source
// absent, graceful) instead of an arbitrary-file copy/read oracle.
// ---------------------------------------------------------------------------

test('makeLunarSource rejects a traversal-relative gameDirectory', async () => {
  writeLunarLauncher('../../../../windows/system32');
  assert.equal(await makeLunarSource(), null);
});

test('makeLunarSource rejects an absolute gameDirectory outside the install root', async () => {
  writeLunarLauncher(path.join(os.tmpdir(), 'windows', 'system32'));
  assert.equal(await makeLunarSource(), null);
});

test('makeLunarSource rejects a prefix-sibling escape (.lunarclient_evil)', async () => {
  writeLunarLauncher(path.join(process.env.APPDATA, '.lunarclient_evil'));
  assert.equal(await makeLunarSource(), null);
});

test('makeLunarSource accepts a gameDirectory inside the install root', async () => {
  const gameDir = path.join(process.env.APPDATA, '.lunarclient', 'game');
  fs.mkdirSync(gameDir, { recursive: true });
  fs.writeFileSync(path.join(gameDir, 'options.txt'), 'version:15\ngamma:1.0\nfov:0.7\n', 'utf8');
  writeLunarLauncher(gameDir);
  const source = await makeLunarSource();
  assert.notEqual(source, null);
  assert.equal(source.id, 'lunar');
  assert.equal(path.resolve(source.path), path.resolve(gameDir));
  assert.equal(source.options_exists, true);
});

test('detectImportSources offers no lunar source when gameDirectory escapes', async () => {
  writeLunarLauncher('../../../../windows/system32');
  const sources = await detectImportSources();
  assert.equal(sources.some((s) => s.id === 'lunar'), false);
});

// ---------------------------------------------------------------------------
// importProfile — end-to-end run through the guard loop, plus the reachable
// TARGET_PATH_ESCAPE throw (instance.dir is caller-controlled).
// ---------------------------------------------------------------------------

test('importProfile copies the whitelist from a legit lunar source', async () => {
  const gameDir = path.join(process.env.APPDATA, '.lunarclient', 'game');
  fs.mkdirSync(gameDir, { recursive: true });
  fs.writeFileSync(path.join(gameDir, 'options.txt'), 'version:15\ngamma:1.0\nfov:0.7\n', 'utf8');
  writeLunarLauncher(gameDir);
  const targetDir = path.join(dataDir, 'instances', 'imported');
  const result = await importProfile({ name: 'imported', dir: targetDir }, 'lunar', 'never');
  assert.deepEqual(result.copied, ['options.txt']);
  assert.equal(result.skipped.length, 1);
  assert.equal(result.skipped[0].file, 'servers.dat'); // not present in source
  assert.equal(fs.readFileSync(path.join(targetDir, 'options.txt'), 'utf8'), 'version:15\ngamma:1.0\nfov:0.7\n');
  assert.equal(result.options_keys, 2);
});

// The only input that makes the target guard fire is a targetDir resolving to
// the filesystem root (path.join(root, file) then no longer starts with
// root + sep). On win32 fsp.mkdir refuses the drive root (EPERM) before the
// guard runs, so the throw is unreachable end-to-end there; the guard logic
// itself is pinned on every platform by the pathEscapes tests above.
test('importProfile throws TARGET_PATH_ESCAPE when the target dir is the filesystem root', { skip: process.platform === 'win32' }, async () => {
  const gameDir = path.join(process.env.APPDATA, '.lunarclient', 'game');
  fs.mkdirSync(gameDir, { recursive: true });
  writeLunarLauncher(gameDir);
  const fsRoot = path.parse(os.tmpdir()).root; // '/' on POSIX
  await assert.rejects(
    () => importProfile({ name: 'root-target', dir: fsRoot }, 'lunar', 'never'),
    (err) => err instanceof ImportError && err.code === 'TARGET_PATH_ESCAPE'
  );
});
