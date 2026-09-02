import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import {
  cacheKey,
  pruneAotProofLogs,
  evictOldAotCaches,
  aotRootDir,
  classpathStamp,
  stampMatches,
  isCacheStale,
  cacheDirFor,
  metaPathFor,
} from '../src/engine/aot.mjs';

function sha256Hex(s) {
  return crypto.createHash('sha256').update(s).digest('hex');
}

const BASE = { version: '1.21.11', javaBuild: '25.0.4+7-LTS', osArch: 'x64' };

test('cacheKey is deterministic and follows sha256(version|javaBuild|osArch)', () => {
  const first = cacheKey(BASE.version, BASE.javaBuild, BASE.osArch);
  const second = cacheKey(BASE.version, BASE.javaBuild, BASE.osArch);
  assert.equal(first, second);
  assert.equal(first, sha256Hex('1.21.11|25.0.4+7-LTS|x64'));
  assert.match(first, /^[0-9a-f]{64}$/);
});

test('cacheKey differs when version/javaBuild/osArch changes', () => {
  const base = cacheKey(BASE.version, BASE.javaBuild, BASE.osArch);
  assert.notEqual(cacheKey('26.2', BASE.javaBuild, BASE.osArch), base, 'version');
  assert.notEqual(cacheKey(BASE.version, '21.0.1+12-LTS', BASE.osArch), base, 'javaBuild');
  assert.notEqual(cacheKey(BASE.version, BASE.javaBuild, 'arm64'), base, 'osArch');
});

test('cacheKey is null-safe for javaBuild and osArch', () => {
  const expected = sha256Hex(`${BASE.version}|unknown|${process.arch}`);
  assert.equal(cacheKey(BASE.version, null, null), expected);
  assert.equal(cacheKey(BASE.version, undefined, undefined), expected);
  assert.equal(cacheKey(BASE.version, null, 'x64'), sha256Hex(`${BASE.version}|unknown|x64`));
  assert.equal(cacheKey(BASE.version, undefined, 'x64'), sha256Hex(`${BASE.version}|unknown|x64`));
  assert.equal(cacheKey(BASE.version, BASE.javaBuild, null), sha256Hex(`${BASE.version}|${BASE.javaBuild}|${process.arch}`));
  assert.equal(cacheKey(BASE.version, BASE.javaBuild, undefined), sha256Hex(`${BASE.version}|${BASE.javaBuild}|${process.arch}`));
});

test('cacheKey no longer depends on modSetHash — mods are off-classpath', () => {
  // New key ignores the mods dir. Two different mod sets must produce the
  // same key when version/javaBuild/osArch are equal.
  const k1 = cacheKey(BASE.version, BASE.javaBuild, BASE.osArch);
  const k2 = cacheKey(BASE.version, BASE.javaBuild, BASE.osArch);
  assert.equal(k1, k2, 'same inputs must give same key regardless of mod set');
  // Prove the old 4-arg formula WOULD have differed on modSetHash.
  const oldK1 = sha256Hex(`${BASE.version}|abc123|${BASE.javaBuild}|${BASE.osArch}`);
  const oldK2 = sha256Hex(`${BASE.version}|def456|${BASE.javaBuild}|${BASE.osArch}`);
  assert.notEqual(oldK1, oldK2, 'old modSetHash-dependent keys would differ');
  // New keys remain equal even though old ones differed.
  assert.equal(k1, k2);
  // Also verify that the exported cacheKey arity is 3 — passing a 4th arg is not part of contract.
  // If someone passes (version, modSetHash, javaBuild, osArch) the hash would be wrong;
  // we ensure the 3-arg form is the canonical one.
  assert.equal(cacheKey.length, 3);
});

// ---------------------------------------------------------------------------
// pruneAotProofLogs — temp-dir isolation (mirrors config.test.mjs pattern)
// ---------------------------------------------------------------------------

function withTempDataDir(fn) {
  const orig = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-aot-test-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  return fn(tmp).finally(() => {
    if (orig === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = orig;
    try { fs.rmSync(tmp, { recursive: true, force: true }); } catch {}
  });
}

function instanceDirFor(tmp, name) {
  return path.join(tmp, 'instances', name);
}

function touchAotLog(dir, pid, mtimeMs) {
  const full = path.join(dir, `aot-${pid}.log`);
  fs.writeFileSync(full, `log ${pid}`, 'utf8');
  const d = new Date(mtimeMs);
  fs.utimesSync(full, d, d);
  return full;
}

test('pruneAotProofLogs keeps at most 8 newest aot-<pid>.log', async () => {
  await withTempDataDir(async (tmp) => {
    const name = 'prune-keep8';
    const dir = instanceDirFor(tmp, name);
    fs.mkdirSync(dir, { recursive: true });
    const now = Date.now();
    // 12 logs, pid 1 oldest, pid 12 newest
    for (let i = 1; i <= 12; i++) {
      touchAotLog(dir, i, now - (12 - i) * 1000);
    }
    assert.equal(fs.readdirSync(dir).filter(f => /^aot-\d+\.log$/.test(f)).length, 12);
    await pruneAotProofLogs({ name });
    const remaining = fs.readdirSync(dir).filter(f => /^aot-\d+\.log$/.test(f)).sort();
    assert.equal(remaining.length, 8, `expected 8 remain, got ${remaining}`);
    // Oldest 4 (pid 1-4) should be gone, newest 8 (5-12) remain
    for (let i = 1; i <= 4; i++) assert.equal(remaining.includes(`aot-${i}.log`), false, `aot-${i}.log should be deleted`);
    for (let i = 5; i <= 12; i++) assert.equal(remaining.includes(`aot-${i}.log`), true, `aot-${i}.log should remain`);
  });
});

test('pruneAotProofLogs deletes logs older than 24h', async () => {
  await withTempDataDir(async (tmp) => {
    const name = 'prune-24h';
    const dir = instanceDirFor(tmp, name);
    fs.mkdirSync(dir, { recursive: true });
    const now = Date.now();
    const HOUR = 3600_000;
    touchAotLog(dir, 1, now - 25 * HOUR); // old
    touchAotLog(dir, 2, now - 26 * HOUR); // old
    touchAotLog(dir, 3, now - 1000); // fresh
    touchAotLog(dir, 4, now); // fresh
    await pruneAotProofLogs(name); // string form
    const remaining = fs.readdirSync(dir).filter(f => /^aot-\d+\.log$/.test(f)).sort();
    assert.deepEqual(remaining, ['aot-3.log', 'aot-4.log']);
  });
});

test('pruneAotProofLogs combined: >24h plus cap 8', async () => {
  await withTempDataDir(async (tmp) => {
    const name = 'prune-combined';
    const dir = instanceDirFor(tmp, name);
    fs.mkdirSync(dir, { recursive: true });
    const now = Date.now();
    const HOUR = 3600_000;
    // 5 old logs + 7 fresh = 12 total. Old ones should be deleted first, leaving 7 fresh (<8 cap)
    for (let i = 1; i <= 5; i++) touchAotLog(dir, i, now - 25 * HOUR - i * 1000);
    for (let i = 6; i <= 12; i++) touchAotLog(dir, i, now - (12 - i) * 1000);
    await pruneAotProofLogs({ name });
    const remaining = fs.readdirSync(dir).filter(f => /^aot-\d+\.log$/.test(f)).sort();
    assert.equal(remaining.length, 7);
    for (let i = 1; i <= 5; i++) assert.equal(remaining.includes(`aot-${i}.log`), false);
    for (let i = 6; i <= 12; i++) assert.equal(remaining.includes(`aot-${i}.log`), true);
  });
});

test('pruneAotProofLogs does not delete when <8 and all fresh', async () => {
  await withTempDataDir(async (tmp) => {
    const name = 'prune-small';
    const dir = instanceDirFor(tmp, name);
    fs.mkdirSync(dir, { recursive: true });
    const now = Date.now();
    for (let i = 1; i <= 3; i++) touchAotLog(dir, i, now - i * 100);
    await pruneAotProofLogs({ name });
    const remaining = fs.readdirSync(dir).filter(f => /^aot-\d+\.log$/.test(f));
    assert.equal(remaining.length, 3);
  });
});

test('pruneAotProofLogs ignores non-aot files and handles missing dir', async () => {
  await withTempDataDir(async (tmp) => {
    const name = 'prune-missing';
    // no dir created — should not throw
    await pruneAotProofLogs({ name });
    // create dir with unrelated files
    const dir = instanceDirFor(tmp, name);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'latest.log'), 'hello');
    fs.writeFileSync(path.join(dir, 'aot-bad.log'), 'bad');
    await pruneAotProofLogs({ name });
    assert.equal(fs.existsSync(path.join(dir, 'latest.log')), true);
    assert.equal(fs.existsSync(path.join(dir, 'aot-bad.log')), true);
  });
});

// ---------------------------------------------------------------------------
// evictOldAotCaches — bounded cache eviction
// ---------------------------------------------------------------------------

function makeCacheKeyDir(root, key, trainedAtIso, mtimeFallbackMs = null) {
  const dir = path.join(root, key);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'game.aot'), 'fake-aot', 'utf8');
  if (trainedAtIso !== null) {
    fs.writeFileSync(path.join(dir, 'meta.json'), JSON.stringify({ trained_at: trainedAtIso, key }, null, 2), 'utf8');
  }
  if (mtimeFallbackMs !== null) {
    const d = new Date(mtimeFallbackMs);
    fs.utimesSync(dir, d, d);
  }
  return dir;
}

test('evictOldAotCaches deletes oldest when >4 keys', async () => {
  await withTempDataDir(async (tmp) => {
    const root = path.join(tmp, 'cache', 'aot');
    fs.mkdirSync(root, { recursive: true });
    const now = Date.now();
    // 5 keys with distinct trained_at
    const keys = ['k1', 'k2', 'k3', 'k4', 'k5'];
    for (let i = 0; i < keys.length; i++) {
      const iso = new Date(now - (keys.length - i) * 10000).toISOString(); // k1 oldest
      makeCacheKeyDir(root, keys[i], iso);
    }
    // current key is k5 (newest) — should delete k1 (oldest) to get to 4
    await evictOldAotCaches('k5');
    const remaining = fs.readdirSync(root).sort();
    assert.equal(remaining.length, 4);
    assert.equal(remaining.includes('k1'), false, 'oldest should be evicted');
    assert.equal(remaining.includes('k5'), true, 'current key never deleted');
    assert.deepEqual(remaining, ['k2', 'k3', 'k4', 'k5']);
  });
});

test('evictOldAotCaches never deletes the key just written even if oldest', async () => {
  await withTempDataDir(async (tmp) => {
    const root = path.join(tmp, 'cache', 'aot');
    fs.mkdirSync(root, { recursive: true });
    const now = Date.now();
    // 5 keys, current is the OLDEST — it must survive, next oldest deleted instead
    const keys = ['current', 'k2', 'k3', 'k4', 'k5'];
    // current is oldest
    makeCacheKeyDir(root, 'current', new Date(now - 50000).toISOString());
    makeCacheKeyDir(root, 'k2', new Date(now - 40000).toISOString());
    makeCacheKeyDir(root, 'k3', new Date(now - 30000).toISOString());
    makeCacheKeyDir(root, 'k4', new Date(now - 20000).toISOString());
    makeCacheKeyDir(root, 'k5', new Date(now - 10000).toISOString());
    await evictOldAotCaches('current');
    const remaining = fs.readdirSync(root).sort();
    assert.equal(remaining.length, 4);
    assert.equal(remaining.includes('current'), true, 'current key must never be deleted even if oldest');
    assert.equal(remaining.includes('k2'), false, 'k2 (next oldest) should be deleted instead');
  });
});

test('evictOldAotCaches fallback to dir mtime when meta.json missing', async () => {
  await withTempDataDir(async (tmp) => {
    const root = path.join(tmp, 'cache', 'aot');
    fs.mkdirSync(root, { recursive: true });
    const now = Date.now();
    // k1 oldest via mtime, no meta; k2-k5 with meta
    makeCacheKeyDir(root, 'k1', null, now - 50000);
    makeCacheKeyDir(root, 'k2', new Date(now - 40000).toISOString());
    makeCacheKeyDir(root, 'k3', new Date(now - 30000).toISOString());
    makeCacheKeyDir(root, 'k4', new Date(now - 20000).toISOString());
    makeCacheKeyDir(root, 'k5', new Date(now - 10000).toISOString());
    await evictOldAotCaches('k5');
    const remaining = fs.readdirSync(root).sort();
    assert.equal(remaining.includes('k1'), false, 'k1 (oldest via mtime fallback) should be evicted');
    assert.equal(remaining.length, 4);
  });
});

test('evictOldAotCaches is no-op when <=4 keys', async () => {
  await withTempDataDir(async (tmp) => {
    const root = path.join(tmp, 'cache', 'aot');
    fs.mkdirSync(root, { recursive: true });
    const now = Date.now();
    for (let i = 1; i <= 4; i++) makeCacheKeyDir(root, `k${i}`, new Date(now - i * 1000).toISOString());
    await evictOldAotCaches('k4');
    const remaining = fs.readdirSync(root).sort();
    assert.equal(remaining.length, 4);
  });
});

test('evictOldAotCaches deletes multiple when >5 keys', async () => {
  await withTempDataDir(async (tmp) => {
    const root = path.join(tmp, 'cache', 'aot');
    fs.mkdirSync(root, { recursive: true });
    const now = Date.now();
    const keys = ['k1', 'k2', 'k3', 'k4', 'k5', 'k6'];
    for (let i = 0; i < keys.length; i++) {
      makeCacheKeyDir(root, keys[i], new Date(now - (keys.length - i) * 10000).toISOString());
    }
    await evictOldAotCaches('k6');
    const remaining = fs.readdirSync(root).sort();
    assert.equal(remaining.length, 4);
    assert.equal(remaining.includes('k1'), false);
    assert.equal(remaining.includes('k2'), false);
    assert.deepEqual(remaining, ['k3', 'k4', 'k5', 'k6']);
  });
});

// ---------------------------------------------------------------------------
// classpathStamp / stampMatches / isCacheStale — JEP 483 identity guard.
// The JVM records path+size+timestamp for every classpath entry when it writes
// the AOT cache and refuses the cache when any of the three drifts. These are
// the checks that keep a refused cache from being handed to the JVM forever.
// ---------------------------------------------------------------------------

function writeJar(dir, name, bytes) {
  const full = path.join(dir, name);
  fs.writeFileSync(full, bytes);
  return full;
}

test('classpathStamp records path/size/mtime and marks unstattable entries', async () => {
  await withTempDataDir(async (tmp) => {
    const a = writeJar(tmp, 'a.jar', 'aaaa');
    const missing = path.join(tmp, 'nope.jar');
    const stamp = classpathStamp([a, missing]);
    assert.equal(stamp.length, 2);
    assert.equal(stamp[0].path, a);
    assert.equal(stamp[0].size, 4);
    assert.ok(stamp[0].mtime_ms > 0);
    assert.deepEqual(stamp[1], { path: missing, size: -1, mtime_ms: -1 });
  });
});

test('stampMatches: identical stamp matches; mtime touch or size change does not', async () => {
  await withTempDataDir(async (tmp) => {
    const a = writeJar(tmp, 'a.jar', 'aaaa');
    const before = classpathStamp([a]);
    assert.equal(stampMatches(before, classpathStamp([a])), true);

    // A rewrite with IDENTICAL bytes still moves the mtime — exactly the churn
    // that silently invalidated the cache (Fabric's hashless intermediary jar).
    const future = new Date(Date.now() + 60_000);
    fs.utimesSync(a, future, future);
    assert.equal(stampMatches(before, classpathStamp([a])), false, 'mtime drift must not match');

    fs.writeFileSync(a, 'aaaaa');
    assert.equal(stampMatches(before, classpathStamp([a])), false, 'size drift must not match');
  });
});

test('stampMatches: absent/empty/short/reordered stamps never match', async () => {
  await withTempDataDir(async (tmp) => {
    const a = writeJar(tmp, 'a.jar', 'aaaa');
    const b = writeJar(tmp, 'b.jar', 'bbbb');
    const current = classpathStamp([a, b]);
    // No evidence -> cannot claim validity.
    assert.equal(stampMatches(null, current), false);
    assert.equal(stampMatches(undefined, current), false);
    assert.equal(stampMatches([], current), false);
    // Different length / order = a different classpath to the JVM.
    assert.equal(stampMatches(classpathStamp([a]), current), false);
    assert.equal(stampMatches(classpathStamp([b, a]), current), false);
    // Missing files on both sides must not compare equal (-1 === -1 trap).
    const ghost = path.join(tmp, 'ghost.jar');
    assert.equal(stampMatches(classpathStamp([ghost]), classpathStamp([ghost])), false);
  });
});

test('isCacheStale: fresh meta stamp is valid, drifted or stampless meta is stale', async () => {
  await withTempDataDir(async (tmp) => {
    const key = 'stalekey';
    const jar = writeJar(tmp, 'lib.jar', 'contents');
    fs.mkdirSync(cacheDirFor(key), { recursive: true });
    const writeMeta = (extra) =>
      fs.writeFileSync(metaPathFor(key), JSON.stringify({ key, ...extra }, null, 2), 'utf8');

    writeMeta({ classpath_stamp: classpathStamp([jar]) });
    assert.equal(isCacheStale(key, [jar]), false, 'a matching stamp is not stale');

    const future = new Date(Date.now() + 60_000);
    fs.utimesSync(jar, future, future);
    assert.equal(isCacheStale(key, [jar]), true, 'mtime churn makes the cache stale');

    // Pre-stamp caches (old meta.json, or none at all) are unverifiable, so
    // they must read as stale: one retrain fixes it instead of every boot
    // paying a refused mapping.
    writeMeta({ classpath_manifest: ['lib.jar'] });
    assert.equal(isCacheStale(key, [jar]), true, 'legacy meta without a stamp is stale');
    fs.rmSync(metaPathFor(key));
    assert.equal(isCacheStale(key, [jar]), true, 'missing meta is stale');
  });
});
