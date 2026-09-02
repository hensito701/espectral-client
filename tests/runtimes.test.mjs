import { test, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Isolate dataDir like tests/config.test.mjs
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const originalAppData = process.env.APPDATA;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-runtimes-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { loadConfig, saveConfig } = await import('../src/engine/config.mjs');
const runtimes = await import('../src/engine/runtimes.mjs');

after(() => {
  try { runtimes.__clearProbeOverride(); } catch {}
  try { saveConfig({ jdk_path_override: null }); } catch {}
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  if (originalAppData === undefined) delete process.env.APPDATA;
  else process.env.APPDATA = originalAppData;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

beforeEach(() => {
  runtimes.__clearProbeOverride();
  // ensure no leftover override
  try { saveConfig({ jdk_path_override: null }); } catch {}
  // clean runtimes dir between tests
  const rt = path.join(dataDir, 'runtimes');
  if (fs.existsSync(rt)) fs.rmSync(rt, { recursive: true, force: true });
});

function fakeProbe(major, overrides = {}) {
  return () => ({
    ok: true,
    major,
    version: `${major}.0.4`,
    build: `${major}.0.4+7`,
    vendor: 'Eclipse Temurin',
    ...overrides,
  });
}

test('P3.1 regression: jdk_path_override probe 25 for need 21 must NOT create runtimes/jdk-21/info.json', async () => {
  const fakeJava = path.join(dataDir, 'fake-jdk25', 'bin', 'java.exe');
  fs.mkdirSync(path.dirname(fakeJava), { recursive: true });
  fs.writeFileSync(fakeJava, 'fake');

  saveConfig({ jdk_path_override: fakeJava });
  runtimes.__setProbeOverride(fakeProbe(25));

  const info = await runtimes.javaForVersion(21);
  assert.equal(info.major, 25);
  assert.equal(info.path, fakeJava);
  assert.equal(info.source, 'path');

  const poisonFile = path.join(dataDir, 'runtimes', 'jdk-21', 'info.json');
  assert.equal(fs.existsSync(poisonFile), false, 'must not create jdk-21/info.json with major 25');

  // Also ensure jdk-25 was not created via this path (we do not cache at all)
  const jdk25File = path.join(dataDir, 'runtimes', 'jdk-25', 'info.json');
  assert.equal(fs.existsSync(jdk25File), false, 'override path must not cache under any major');

  runtimes.__clearProbeOverride();
  saveConfig({ jdk_path_override: null });
});

test('P3.1 regression: override 25 must NOT overwrite existing jdk-21/info.json (major 21)', async () => {
  // Prepare a valid existing jdk-21 runtime
  const realJava21 = path.join(dataDir, 'real-jdk21', 'bin', 'java.exe');
  fs.mkdirSync(path.dirname(realJava21), { recursive: true });
  fs.writeFileSync(realJava21, 'fake21');

  const existingInfo = {
    path: realJava21,
    version: '21.0.1',
    build: '21.0.1+12',
    vendor: 'Eclipse Temurin',
    source: 'downloaded',
    major: 21,
  };
  // Use cacheMajor to create the file (first write)
  runtimes.cacheMajor(21, existingInfo);
  const infoPath = path.join(dataDir, 'runtimes', 'jdk-21', 'info.json');
  assert.equal(fs.existsSync(infoPath), true);
  const before = fs.readFileSync(infoPath, 'utf8');
  assert.equal(JSON.parse(before).major, 21);

  // Now set override to JDK25 and call javaForVersion(21)
  const fakeJava25 = path.join(dataDir, 'fake-jdk25-2', 'bin', 'java.exe');
  fs.mkdirSync(path.dirname(fakeJava25), { recursive: true });
  fs.writeFileSync(fakeJava25, 'fake25');
  saveConfig({ jdk_path_override: fakeJava25 });
  runtimes.__setProbeOverride(fakeProbe(25));

  const result = await runtimes.javaForVersion(21);
  assert.equal(result.major, 25);
  assert.equal(result.path, fakeJava25);

  const afterContent = fs.readFileSync(infoPath, 'utf8');
  assert.equal(afterContent, before, 'existing jdk-21/info.json must not be overwritten by mismatched probe');
  assert.equal(JSON.parse(afterContent).major, 21, 'major must stay 21');

  runtimes.__clearProbeOverride();
  saveConfig({ jdk_path_override: null });
});

test('cacheMajor idempotence: second deep-equal write skips disk', async () => {
  const dummyPath = path.join(dataDir, 'dummy-jdk21-bis', 'bin', 'java.exe');
  fs.mkdirSync(path.dirname(dummyPath), { recursive: true });
  fs.writeFileSync(dummyPath, 'x');

  const info = {
    path: dummyPath,
    version: '21.0.7',
    build: '21.0.7+6',
    vendor: 'Eclipse Temurin',
    source: 'downloaded',
    major: 21,
  };

  const f = path.join(dataDir, 'runtimes', 'jdk-21', 'info.json');

  // first write
  runtimes.cacheMajor(21, info);
  assert.equal(fs.existsSync(f), true);
  const stat1 = fs.statSync(f);
  const content1 = fs.readFileSync(f, 'utf8');

  // small delay to ensure mtime would advance if rewritten
  await new Promise((r) => setTimeout(r, 15));

  // second call with deep-equal object (different reference, different key order)
  const infoClone = {
    major: 21,
    source: 'downloaded',
    vendor: 'Eclipse Temurin',
    build: '21.0.7+6',
    version: '21.0.7',
    path: dummyPath,
  };
  runtimes.cacheMajor(21, infoClone);

  const stat2 = fs.statSync(f);
  const content2 = fs.readFileSync(f, 'utf8');

  assert.equal(content2, content1, 'content must stay identical');
  assert.equal(stat2.mtimeMs, stat1.mtimeMs, 'mtime must not change when content deep-equals (write skipped)');

  // third call with different content must overwrite
  await new Promise((r) => setTimeout(r, 15));
  const infoDifferent = { ...info, version: '21.0.8', build: '21.0.8+9' };
  runtimes.cacheMajor(21, infoDifferent);
  const stat3 = fs.statSync(f);
  const content3 = fs.readFileSync(f, 'utf8');
  assert.notEqual(content3, content1, 'different info must overwrite');
  assert.ok(stat3.mtimeMs > stat2.mtimeMs, 'mtime must advance when content differs');
  assert.equal(JSON.parse(content3).version, '21.0.8');
});

test('FastClient/PATH branches with mismatched major must not poison jdk-21', async () => {
  // This test verifies the no-poison rule for the FastClient and PATH tiers.
  // We simulate both by creating the FastClient file and a PATH java file,
  // probing them as 25, and ensuring neither creates jdk-21/info.json.
  // Override is cleared so the FastClient tier is reached.

  // Prepare FastClient fake
  const appDataDir = path.join(dataDir, 'appdata-fake');
  process.env.APPDATA = appDataDir;
  const fcPath = path.join(appDataDir, 'FastClient', 'runtimes', 'java-25', 'jdk-25.0.4+7', 'bin', process.platform === 'win32' ? 'java.exe' : 'java');
  fs.mkdirSync(path.dirname(fcPath), { recursive: true });
  fs.writeFileSync(fcPath, 'fake-fc');

  // Ensure no override
  saveConfig({ jdk_path_override: null });
  // Mock probe to return 25 for any path
  runtimes.__setProbeOverride(fakeProbe(25));

  // Mock pathJava to return a fake PATH java with major 25 as well
  // We cannot easily mock pathJava without seam, so we temporarily
  // ensure PATH contains a dummy java and that fs.existsSync sees it.
  // Instead we verify FastClient tier alone: call javaForVersion(21)
  // should return the FastClient JDK without poisoning jdk-21.

  const info = await runtimes.javaForVersion(21);
  // Should have returned FastClient (since it fits tier 21/25)
  assert.equal(info.source, 'fastclient');
  assert.equal(info.major, 25);
  assert.equal(info.path, fcPath);

  const poison = path.join(dataDir, 'runtimes', 'jdk-21', 'info.json');
  assert.equal(fs.existsSync(poison), false, 'FastClient probe 25 must not poison jdk-21');

  // Now also ensure PATH tier would not poison if FastClient absent:
  // Remove FastClient file, create a fake PATH java
  fs.rmSync(fcPath, { force: true });
  // Create a temp dir that we prepend to PATH and put a fake java there
  const pathDir = path.join(dataDir, 'fake-path-bin');
  fs.mkdirSync(pathDir, { recursive: true });
  const pathJavaName = process.platform === 'win32' ? 'java.exe' : 'java';
  const fakePathJava = path.join(pathDir, pathJavaName);
  fs.writeFileSync(fakePathJava, 'fake-path');
  // `which java` (POSIX) only resolves executables; `where java` (Windows) matches
  // by name. Without the exec bit this probe silently falls through on Linux/macOS.
  fs.chmodSync(fakePathJava, 0o755);
  const originalPath = process.env.PATH || '';
  process.env.PATH = pathDir + path.delimiter + originalPath;

  // Probe still returns 25 for any path
  const info2 = await runtimes.javaForVersion(21);
  assert.equal(info2.source, 'path');
  assert.equal(info2.major, 25);
  assert.equal(fs.existsSync(poison), false, 'PATH probe 25 must not poison jdk-21');

  // restore
  process.env.PATH = originalPath;
  runtimes.__clearProbeOverride();
  process.env.APPDATA = originalAppData || appDataDir; // will be restored in after()
});
