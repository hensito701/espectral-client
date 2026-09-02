import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { rulesAllow, fabricSupportedForVersion, getFabricLoaders, getVersionManifest, getVersionJson, installLibraries, resolveFabric, __clearManifestMemo, MANIFEST_TTL_MS, MANIFEST_URL, FABRIC_META_BASE } from '../src/engine/resolver.mjs';

test('rulesAllow: no rules -> always allowed', () => {
  assert.equal(rulesAllow({}), true);
  assert.equal(rulesAllow({ name: 'example.jar' }), true);
  assert.equal(rulesAllow({ rules: [] }), true);
});

test('rulesAllow: allow rule matches os.name', () => {
  const lib = { rules: [{ action: 'allow', os: { name: 'windows' } }] };
  assert.equal(rulesAllow(lib, { os: 'windows' }), true);
  assert.equal(rulesAllow(lib, { os: 'linux' }), false);
  assert.equal(rulesAllow(lib, { os: 'osx' }), false);
});

test('rulesAllow: allow rule matches os.arch', () => {
  const lib = { rules: [{ action: 'allow', os: { arch: 'x86_64' } }] };
  assert.equal(rulesAllow(lib, { os: 'windows', arch: 'x86_64' }), true);
  assert.equal(rulesAllow(lib, { os: 'windows', arch: 'arm64' }), false);
});

test('rulesAllow: allow rule requires os.name and os.arch together', () => {
  const lib = { rules: [{ action: 'allow', os: { name: 'windows', arch: 'x86_64' } }] };
  assert.equal(rulesAllow(lib, { os: 'windows', arch: 'x86_64' }), true);
  assert.equal(rulesAllow(lib, { os: 'windows', arch: 'arm64' }), false);
  assert.equal(rulesAllow(lib, { os: 'linux', arch: 'x86_64' }), false);
});

test('rulesAllow: disallow rule blocks its os', () => {
  const lib = { rules: [{ action: 'disallow', os: { name: 'osx' } }] };
  // No matching rule anywhere -> default is deny (rules present, nothing allows).
  assert.equal(rulesAllow(lib, { os: 'windows' }), false);
  assert.equal(rulesAllow(lib, { os: 'osx' }), false);
});

test('rulesAllow: feature requirements present vs absent', () => {
  const needsDemo = { rules: [{ action: 'allow', features: { is_demo_user: true } }] };
  assert.equal(rulesAllow(needsDemo, { os: 'linux', features: { is_demo_user: true } }), true);
  assert.equal(rulesAllow(needsDemo, { os: 'linux', features: {} }), false);
  assert.equal(rulesAllow(needsDemo, { os: 'linux' }), false); // default features {}

  // A false-valued requirement is satisfied when the feature is absent
  const rejectsCustomRes = { rules: [{ action: 'allow', features: { has_custom_resolution: false } }] };
  assert.equal(rulesAllow(rejectsCustomRes, { os: 'linux', features: {} }), true);
  assert.equal(rulesAllow(rejectsCustomRes, { os: 'linux', features: { has_custom_resolution: true } }), false);
});

test('rulesAllow: last matching rule wins (Mojang os-guard shape)', () => {
  const lib = {
    rules: [
      { action: 'disallow', os: { name: 'osx' } },
      { action: 'allow', os: { name: 'windows' } },
    ],
  };
  assert.equal(rulesAllow(lib, { os: 'windows' }), true); // allow wins on windows
  assert.equal(rulesAllow(lib, { os: 'osx' }), false); // disallow wins on osx
  assert.equal(rulesAllow(lib, { os: 'linux' }), false); // no matching rule -> denied
});

test('rulesAllow: non-matching rules do not flip the result', () => {
  const lib = { rules: [{ action: 'disallow', os: { name: 'osx' } }, { action: 'allow', os: { name: 'windows' } }] };
  // linux matches neither rule: the "no matching rule" default must stay false
  assert.equal(rulesAllow(lib, { os: 'linux' }), false);
});

test('fabricSupportedForVersion: non-empty loader list -> true', async (t) => {
  t.mock.method(globalThis, 'fetch', async () => ({
    ok: true,
    text: async () => JSON.stringify([{ version: '0.16.5' }, { version: '0.16.4' }]),
  }));
  assert.equal(await fabricSupportedForVersion('26.2'), true);
});

test('fabricSupportedForVersion: empty loader list -> false', async (t) => {
  t.mock.method(globalThis, 'fetch', async () => ({
    ok: true,
    text: async () => JSON.stringify([]),
  }));
  assert.equal(await fabricSupportedForVersion('1.99.99'), false);
});

test('fabricSupportedForVersion: HTTP 400 (no loader) -> false, definitive (H4)', async (t) => {
  t.mock.method(globalThis, 'fetch', async () => ({ ok: false, status: 400 }));
  assert.equal(await fabricSupportedForVersion('1.20.4'), false);
});

test('fabricSupportedForVersion: HTTP 500 -> true (optimistic, never blocks creation)', async (t) => {
  t.mock.method(globalThis, 'fetch', async () => ({ ok: false, status: 500 }));
  assert.equal(await fabricSupportedForVersion('26.2'), true);
});

test('fabricSupportedForVersion: network failure -> true (offline resilience)', async (t) => {
  t.mock.method(globalThis, 'fetch', async () => {
    throw new TypeError('fetch failed');
  });
  assert.equal(await fabricSupportedForVersion('26.2'), true);
});

test('getFabricLoaders: TTL-expired cache survives a meta outage (stale fallback)', async (t) => {
  // Sandbox the data dir for this test only (dataDir() resolves the env var
  // at call time — same pattern as tests/neoforge.test.mjs).
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-fabric-meta-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const versionsDir = path.join(tmp, 'versions');
    fs.mkdirSync(versionsDir, { recursive: true });
    const stale = {
      fetched_at: Date.now() - 7 * 24 * 60 * 60 * 1000, // far past the 6h TTL
      loaders: [{ version: '0.17.2', stable: true, build: 10 }, { version: '0.17.1', stable: false, build: 9 }],
      latest_stable: '0.17.2',
    };
    fs.writeFileSync(path.join(versionsDir, 'fabric-loaders.json'), JSON.stringify(stale));
    t.mock.method(globalThis, 'fetch', async () => {
      throw new TypeError('fetch failed'); // meta.fabricmc.net unreachable
    });
    const out = await getFabricLoaders();
    assert.equal(out.latest_stable, '0.17.2');
    assert.equal(out.loaders.length, 2);
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('getFabricLoaders: outage with no cache at all still fails loudly', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-fabric-nocache-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    t.mock.method(globalThis, 'fetch', async () => {
      throw new TypeError('fetch failed');
    });
    await assert.rejects(getFabricLoaders(), (err) => err.code === 'fabric_loader_unreachable');
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

import { createHash } from 'node:crypto';

function sha1HexBuf(buf) { return createHash('sha1').update(buf).digest('hex'); }

// ---------------------------------------------------------------------------
// C15: stale-while-revalidate
// ---------------------------------------------------------------------------
test('getVersionManifest SWR: TTL-lapsed + stale present returns stale and kicks background refresh', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-swr-manifest-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const versionsDir = path.join(tmp, 'versions');
    fs.mkdirSync(versionsDir, { recursive: true });
    const stale = {
      fetched_at: Date.now() - MANIFEST_TTL_MS - 1000,
      latest_release: '1.21.1',
      latest_snapshot: '25w01a',
      versions: [{ id: '1.21.1', type: 'release', release_time: '2024-08-08T00:00:00Z', sha1: null, url: 'https://example.com/1.21.1.json' }],
    };
    fs.writeFileSync(path.join(versionsDir, 'manifest.json'), JSON.stringify(stale));
    let fetchCalled = false;
    let bgResolve;
    const bgDone = new Promise((r) => { bgResolve = r; });
    const freshRaw = {
      latest: { release: '1.21.2', snapshot: '25w02a' },
      versions: [{ id: '1.21.2', type: 'release', releaseTime: '2024-09-01T00:00:00Z', sha1: 'abc', url: 'https://example.com/1.21.2.json' }],
    };
    t.mock.method(globalThis, 'fetch', async () => {
      fetchCalled = true;
      // small async delay to prove we returned stale before fetch settled
      await new Promise((r) => setTimeout(r, 20));
      bgResolve();
      return { ok: true, text: async () => JSON.stringify(freshRaw) };
    });
    const out = await getVersionManifest();
    // should return stale immediately
    assert.equal(out.latest_release, '1.21.1');
    assert.equal(out.versions[0].id, '1.21.1');
    // background refresh should have been kicked
    await bgDone;
    // give background write a moment
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(fetchCalled, true);
    const updated = JSON.parse(fs.readFileSync(path.join(versionsDir, 'manifest.json'), 'utf8'));
    assert.equal(updated.latest_release, '1.21.2');
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('getFabricLoaders SWR: TTL-lapsed + stale present returns stale and kicks background refresh', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-swr-fabric-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const versionsDir = path.join(tmp, 'versions');
    fs.mkdirSync(versionsDir, { recursive: true });
    const stale = {
      fetched_at: Date.now() - MANIFEST_TTL_MS - 1000,
      loaders: [{ version: '0.15.0', stable: true, build: 5 }],
      latest_stable: '0.15.0',
    };
    fs.writeFileSync(path.join(versionsDir, 'fabric-loaders.json'), JSON.stringify(stale));
    let fetchCalled = false;
    let bgResolve;
    const bgDone = new Promise((r) => { bgResolve = r; });
    t.mock.method(globalThis, 'fetch', async () => {
      fetchCalled = true;
      await new Promise((r) => setTimeout(r, 20));
      bgResolve();
      return { ok: true, text: async () => JSON.stringify([{ version: '0.16.0', stable: true, build: 6 }]) };
    });
    const out = await getFabricLoaders();
    assert.equal(out.latest_stable, '0.15.0');
    assert.equal(out.loaders[0].version, '0.15.0');
    await bgDone;
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(fetchCalled, true);
    const updated = JSON.parse(fs.readFileSync(path.join(versionsDir, 'fabric-loaders.json'), 'utf8'));
    assert.equal(updated.latest_stable, '0.16.0');
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('SWR respects force flag: force=true bypasses stale and fetches fresh', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-swr-force-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const versionsDir = path.join(tmp, 'versions');
    fs.mkdirSync(versionsDir, { recursive: true });
    const stale = {
      fetched_at: Date.now() - MANIFEST_TTL_MS - 1000,
      latest_release: 'old',
      latest_snapshot: 'old',
      versions: [{ id: 'old', type: 'release', release_time: '', sha1: null, url: 'https://example.com/old.json' }],
    };
    fs.writeFileSync(path.join(versionsDir, 'manifest.json'), JSON.stringify(stale));
    const freshRaw = {
      latest: { release: 'fresh', snapshot: 'fresh' },
      versions: [{ id: 'fresh', type: 'release', releaseTime: '2024-09-01T00:00:00Z', sha1: null, url: 'https://example.com/fresh.json' }],
    };
    t.mock.method(globalThis, 'fetch', async () => ({
      ok: true, text: async () => JSON.stringify(freshRaw),
    }));
    const out = await getVersionManifest({ force: true });
    assert.equal(out.latest_release, 'fresh');
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------------------
// C14: natives freshness markers
// ---------------------------------------------------------------------------
test('natives marker: second install skips when marker+meta match, forceExtract re-extracts', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-natives-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const instance = { name: 'test-natives', version: '1.21.1' };
    // prepare a dummy natives jar as plain file (non-zip) to trigger copy fallback
    const dummyContent = Buffer.from('dummy native dll content ' + Date.now());
    const sha = sha1HexBuf(dummyContent);
    // Compute dest path that installLibraries will use: librariesDir() + relPath from mavenPath
    // For name with natives classifier, mavenPath includes classifier in filename.
    // Use a natives classifier name to land in natives list.
    const libName = 'org.test:natives-test:1.0:natives-windows';
    // libraryDownload will derive relPath; we set url to dummy but sha must match dummyContent
    const versionInfo = {
      libraries: [
        {
          name: libName,
          downloads: { artifact: { url: 'https://example.com/fake-natives.jar', path: 'org/test/natives-test/1.0/natives-test-1.0-natives-windows.jar', sha1: sha, size: dummyContent.length } },
        },
      ],
      downloads: {},
    };
    // Pre-create the library file at its dest so download skips network
    const { librariesDir, instanceNativesDir } = await import('../src/engine/resolver.mjs');
    const libDest = path.join(librariesDir(), 'org/test/natives-test/1.0/natives-test-1.0-natives-windows.jar');
    fs.mkdirSync(path.dirname(libDest), { recursive: true });
    fs.writeFileSync(libDest, dummyContent);
    // Prevent actual network fetches for any stray downloads
    t.mock.method(globalThis, 'fetch', async () => {
      throw new Error('unexpected fetch in natives test');
    });

    // First install: should extract and write markers
    const r1 = await installLibraries(instance, versionInfo, { seedFromFastClient: false });
    assert.equal(r1.natives_extracted, 1);
    const nativesDir = instanceNativesDir(instance.name);
    const extractedFile = path.join(nativesDir, path.basename(libDest));
    assert.equal(fs.existsSync(extractedFile), true);
    // Find marker files
    const ents1 = fs.readdirSync(nativesDir);
    const markers = ents1.filter((n) => n.startsWith('.extract-ok-'));
    const metas = ents1.filter((n) => n.startsWith('.extract-meta-'));
    assert.equal(markers.length, 1);
    assert.equal(metas.length, 1);
    const meta = JSON.parse(fs.readFileSync(path.join(nativesDir, metas[0]), 'utf8'));
    const srcStat = fs.statSync(libDest);
    assert.equal(meta.size, srcStat.size);
    assert.equal(meta.mtimeMs, srcStat.mtimeMs);

    // Delete the extracted file to make skip observable
    fs.rmSync(extractedFile, { force: true });
    assert.equal(fs.existsSync(extractedFile), false);

    // Second install without force: should SKIP (file stays missing)
    const r2 = await installLibraries(instance, versionInfo, { seedFromFastClient: false });
    assert.equal(fs.existsSync(extractedFile), false);
    assert.equal(fs.existsSync(path.join(nativesDir, markers[0])), true);

    // Third install with forceExtract: should clear markers and re-extract (file reappears)
    const r3 = await installLibraries(instance, versionInfo, { seedFromFastClient: false, forceExtract: true });
    assert.equal(fs.existsSync(extractedFile), true);
    const ents3 = fs.readdirSync(nativesDir);
    assert.equal(ents3.filter((n) => n.startsWith('.extract-ok-')).length, 1);
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('natives marker: source mtime change invalidates marker and re-extracts', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-natives-mtime-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const instance = { name: 'test-natives2', version: '1.21.1' };
    const content1 = Buffer.from('content v1');
    const sha1 = sha1HexBuf(content1);
    const libName = 'org.test:natives-mtime:1.0:natives-windows';
    const versionInfo = {
      libraries: [{ name: libName, downloads: { artifact: { url: 'https://example.com/fake.jar', path: 'org/test/natives-mtime/1.0/natives-mtime-1.0-natives-windows.jar', sha1, size: content1.length } } }],
      downloads: {},
    };
    const { librariesDir, instanceNativesDir } = await import('../src/engine/resolver.mjs');
    const libDest = path.join(librariesDir(), 'org/test/natives-mtime/1.0/natives-mtime-1.0-natives-windows.jar');
    fs.mkdirSync(path.dirname(libDest), { recursive: true });
    fs.writeFileSync(libDest, content1);
    t.mock.method(globalThis, 'fetch', async () => { throw new Error('unexpected fetch'); });
    await installLibraries(instance, versionInfo, { seedFromFastClient: false });
    const nativesDir = instanceNativesDir(instance.name);
    const extractedFile = path.join(nativesDir, path.basename(libDest));
    fs.rmSync(extractedFile, { force: true });
    // Touch source jar with new content + mtime
    const content2 = Buffer.from('content v2 different');
    fs.writeFileSync(libDest, content2);
    // Update versionInfo sha to match new content so download still skips (sha matches new file)
    versionInfo.libraries[0].downloads.artifact.sha1 = sha1HexBuf(content2);
    versionInfo.libraries[0].downloads.artifact.size = content2.length;
    // Without force, but mtime changed -> marker invalid -> should re-extract
    await installLibraries(instance, versionInfo, { seedFromFastClient: false });
    assert.equal(fs.existsSync(extractedFile), true);
    assert.equal(fs.readFileSync(extractedFile, 'utf8'), 'content v2 different');
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------------------
// C16: version-json single read + manifest memo
// ---------------------------------------------------------------------------
test('getVersionJson single-read: verifies sha from single Buffer and parses same Buffer', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-vjson-single-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    __clearManifestMemo();
    const versionsDir = path.join(tmp, 'versions');
    fs.mkdirSync(versionsDir, { recursive: true });
    const versionId = 'single-read-test';
    const versionObj = { id: versionId, mainClass: 'net.minecraft.client.main.Main', minimumLauncherVersion: 21 };
    const versionText = JSON.stringify(versionObj);
    const versionBuf = Buffer.from(versionText, 'utf8');
    const sha = sha1HexBuf(versionBuf);
    const versionFile = path.join(versionsDir, `${versionId}.json`);
    fs.writeFileSync(versionFile, versionBuf);
    const manifest = {
      fetched_at: Date.now(),
      latest_release: versionId,
      latest_snapshot: versionId,
      versions: [{ id: versionId, type: 'release', release_time: '2024-01-01T00:00:00Z', sha1: sha, url: 'https://example.com/single-read-test.json' }],
    };
    fs.writeFileSync(path.join(versionsDir, 'manifest.json'), JSON.stringify(manifest));

    // Mock fetch to ensure no network on cache hit; if getVersionJson double-reads, it would still succeed but we want to ensure single read path works
    let fetchCalled = false;
    t.mock.method(globalThis, 'fetch', async () => { fetchCalled = true; throw new Error('should not fetch when cached sha matches'); });

    // First call should hit cache file via single Buffer read
    const out = await getVersionJson(versionId);
    assert.equal(fetchCalled, false);
    assert.equal(out.mainClass, 'net.minecraft.client.main.Main');

    // Corrupt sha case: change file content -> sha mismatch triggers re-download
    fs.writeFileSync(versionFile, JSON.stringify({ id: versionId, corrupted: true }));
    const freshObj = { id: versionId, mainClass: 'fresh' };
    const freshText = JSON.stringify(freshObj);
    const freshSha = sha1HexBuf(Buffer.from(freshText, 'utf8'));
    // Update manifest entry sha to match the fresh payload so verification passes
    const manifest2 = JSON.parse(fs.readFileSync(path.join(versionsDir, 'manifest.json'), 'utf8'));
    const ent = manifest2.versions.find((v) => v.id === versionId);
    ent.sha1 = freshSha;
    fs.writeFileSync(path.join(versionsDir, 'manifest.json'), JSON.stringify(manifest2));
    __clearManifestMemo();
    t.mock.restoreAll();
    t.mock.method(globalThis, 'fetch', async () => ({
      ok: true, text: async () => freshText,
    }));
    const out2 = await getVersionJson(versionId);
    assert.equal(out2.mainClass, 'fresh');
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
    __clearManifestMemo();
  }
});

test('getVersionJson manifest memo: second call hits memo when file unchanged', async (t) => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-vjson-memo-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    __clearManifestMemo();
    const versionsDir = path.join(tmp, 'versions');
    fs.mkdirSync(versionsDir, { recursive: true });
    const v1 = 'memo-a';
    const v2 = 'memo-b';
    const manifest = {
      fetched_at: Date.now(),
      latest_release: v1,
      latest_snapshot: v1,
      versions: [
        { id: v1, type: 'release', release_time: '', sha1: null, url: 'https://example.com/memo-a.json' },
        { id: v2, type: 'release', release_time: '', sha1: null, url: 'https://example.com/memo-b.json' },
      ],
    };
    fs.writeFileSync(path.join(versionsDir, 'manifest.json'), JSON.stringify(manifest));
    // Write both version json files so getVersionJson resolves without fetch
    for (const vid of [v1, v2]) {
      fs.writeFileSync(path.join(versionsDir, `${vid}.json`), JSON.stringify({ id: vid }));
    }
    let fetchCount = 0;
    t.mock.method(globalThis, 'fetch', async () => { fetchCount++; throw new Error('fetch should not be needed for memo hit'); });

    const a1 = await getVersionJson(v1);
    assert.equal(a1.id, v1);
    const countAfterFirst = fetchCount;
    // Second call for different id but same manifest file (same mtime/size) should hit memo and not increase fetch count
    const b1 = await getVersionJson(v2);
    assert.equal(b1.id, v2);
    assert.equal(fetchCount, countAfterFirst, 'memo should prevent extra manifest fetch');

    // After clearing memo, next call should still work (rebuild memo)
    __clearManifestMemo();
    const a2 = await getVersionJson(v1);
    assert.equal(a2.id, v1);
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
    __clearManifestMemo();
  }
});

// ---------------------------------------------------------------------------
// resolveFabric sha1 hydration — the AOT-cache churn fix.
// Fabric's launcherMeta names the intermediary/fabric-loader jars by maven
// coordinate only (no sha1/size), so a hashless download re-fetched them on
// every verification pass and touched their mtimes. JEP 483 stamps
// path+size+timestamp per classpath entry, so that churn permanently
// invalidated the AOT cache. The hashes come from maven's .sha1 sidecars.
// ---------------------------------------------------------------------------

const FABRIC_COMBO = {
  loader: { version: '0.19.3', maven: 'net.fabricmc:fabric-loader:0.19.3' },
  intermediary: { version: '0.0.0', maven: 'net.fabricmc:intermediary:0.0.0' },
  launcherMeta: {
    version: 2,
    mainClass: { client: 'net.fabricmc.loader.impl.launch.knot.KnotClient' },
    libraries: {
      common: [
        { name: 'org.ow2.asm:asm:9.10.1', url: 'https://maven.fabricmc.net/', sha1: 'a'.repeat(40), size: 126151 },
      ],
    },
  },
};

const INTERMEDIARY_SHA1 = '29daec33f16264c910f74a926406bae5cf36d81e';
const LOADER_SHA1 = '354dfaa02d0552e11867f85dff7cdbfaf813ba3e';

function withFabricDataDir(fn) {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-fabric-sha1-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  const versionsDir = path.join(tmp, 'versions');
  fs.mkdirSync(versionsDir, { recursive: true });
  fs.writeFileSync(
    path.join(versionsDir, 'fabric-loaders.json'),
    JSON.stringify({
      fetched_at: Date.now(),
      loaders: [{ version: '0.19.3', stable: true, build: 3, maven: 'net.fabricmc:fabric-loader:0.19.3' }],
      latest_stable: '0.19.3',
    })
  );
  fs.writeFileSync(path.join(versionsDir, 'fabric-loader-26.2-0.19.3.json'), JSON.stringify(FABRIC_COMBO));
  return Promise.resolve(fn(tmp, versionsDir)).finally(() => {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  });
}

/** Answers only `<artifact>.sha1` requests; records every URL it saw. */
function sha1Sidecars(seen) {
  return async (url) => {
    seen.push(String(url));
    if (String(url).endsWith('intermediary-0.0.0.jar.sha1')) {
      return { ok: true, status: 200, text: async () => `${INTERMEDIARY_SHA1}\n` };
    }
    if (String(url).endsWith('fabric-loader-0.19.3.jar.sha1')) {
      return { ok: true, status: 200, text: async () => LOADER_SHA1 };
    }
    throw new Error(`unexpected fetch: ${url}`);
  };
}

test('resolveFabric hydrates intermediary/loader sha1 from maven sidecars (AOT mtime churn fix)', async (t) => {
  await withFabricDataDir(async () => {
    const seen = [];
    t.mock.method(globalThis, 'fetch', sha1Sidecars(seen));
    const profile = await resolveFabric('26.2');
    const byPath = new Map(profile.libraries.map((l) => [l.relPath, l]));
    assert.equal(
      byPath.get('net/fabricmc/intermediary/0.0.0/intermediary-0.0.0.jar').sha1,
      INTERMEDIARY_SHA1
    );
    assert.equal(
      byPath.get('net/fabricmc/fabric-loader/0.19.3/fabric-loader-0.19.3.jar').sha1,
      LOADER_SHA1
    );
    // The pre-hashed common entry must be left alone (no wasted requests).
    assert.equal(byPath.get('org/ow2/asm/asm/9.10.1/asm-9.10.1.jar').sha1, 'a'.repeat(40));
    assert.equal(seen.filter((u) => u.endsWith('.sha1')).length, 2);
    assert.equal(seen.some((u) => u.includes('asm')), false);
  });
});

test('resolveFabric memoizes hydrated hashes in the combo cache (one fetch ever, offline-safe)', async (t) => {
  await withFabricDataDir(async (_tmp, versionsDir) => {
    const seen = [];
    t.mock.method(globalThis, 'fetch', sha1Sidecars(seen));
    await resolveFabric('26.2');
    const cached = JSON.parse(
      fs.readFileSync(path.join(versionsDir, 'fabric-loader-26.2-0.19.3.json'), 'utf8')
    );
    assert.equal(
      cached.espectral_maven_sha1['net/fabricmc/intermediary/0.0.0/intermediary-0.0.0.jar'],
      INTERMEDIARY_SHA1
    );

    // Second resolve: the memo must satisfy both hashes with no network at all,
    // so an offline launch still short-circuits the two jars.
    t.mock.restoreAll();
    t.mock.method(globalThis, 'fetch', async (url) => {
      throw new Error(`memoized hashes must not re-fetch: ${url}`);
    });
    const again = await resolveFabric('26.2');
    const byPath = new Map(again.libraries.map((l) => [l.relPath, l]));
    assert.equal(
      byPath.get('net/fabricmc/intermediary/0.0.0/intermediary-0.0.0.jar').sha1,
      INTERMEDIARY_SHA1
    );
    assert.equal(byPath.get('net/fabricmc/fabric-loader/0.19.3/fabric-loader-0.19.3.jar').sha1, LOADER_SHA1);
  });
});

test('resolveFabric survives an unreachable sidecar: hashless entries, no throw', async (t) => {
  await withFabricDataDir(async () => {
    t.mock.method(globalThis, 'fetch', async () => {
      throw new TypeError('fetch failed'); // maven unreachable
    });
    const profile = await resolveFabric('26.2');
    const inter = profile.libraries.find((l) => l.relPath.includes('intermediary'));
    assert.equal(inter.sha1, null, 'no sidecar -> pre-existing hashless behaviour, not a failed launch');
    assert.equal(profile.loader_version, '0.19.3');
  });
});
