import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// dataDir() resolves ESPECTRAL_DATA_DIR at call time, but set it before
// importing so module evaluation sees the sandbox too.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-stats-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { recordLaunchStat, readLaunchStats } = await import('../src/engine/stats.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const RECORD_A = { key: 'a', instance: 'one', version: '1.21.11', started_at: 1000, menu_at: null, menu_ms: null, played_ms: 12000 };
const RECORD_B = { key: 'b', instance: 'two', version: '26.2', started_at: 2000, menu_at: 6000, menu_ms: 4000, played_ms: 20000 };
const RECORD_C = { key: 'c', instance: 'three', version: '1.20.4', started_at: 3000, menu_at: null, menu_ms: null, played_ms: 8000 };

test('readLaunchStats on a missing file returns []', async () => {
  assert.deepEqual(await readLaunchStats(), []);
});

test('recordLaunchStat / readLaunchStats round-trip (newest first, limit)', async () => {
  recordLaunchStat(RECORD_A);
  recordLaunchStat(RECORD_B);
  recordLaunchStat(RECORD_C);

  const all = await readLaunchStats(10);
  assert.equal(all.length, 3);
  assert.deepEqual(all.map((r) => r.key), ['c', 'b', 'a']);
  // JSON round-trip preserves every field, including nulls and numbers
  assert.deepEqual(all[1], RECORD_B);

  const limited = await readLaunchStats(2);
  assert.deepEqual(limited.map((r) => r.key), ['c', 'b']);
});

test('corrupt JSON lines are skipped, not fatal', async () => {
  const file = path.join(dataDir, 'launch-stats.jsonl');
  fs.appendFileSync(file, 'this is not json\n{"key": "partial", "broken": \n', 'utf8');
  recordLaunchStat({ key: 'd', instance: 'four', version: '21.0', started_at: 4000, menu_at: null, menu_ms: null, played_ms: 5000 });

  const records = await readLaunchStats(10);
  assert.deepEqual(records.map((r) => r.key), ['d', 'c', 'b', 'a']);
});

test('readLaunchStats honors the default limit of 10', async () => {
  for (let i = 0; i < 12; i++) {
    recordLaunchStat({ key: `bulk-${i}`, instance: 'bulk', version: '1.21.11', started_at: i, menu_at: null, menu_ms: null, played_ms: 1 });
  }
  const records = await readLaunchStats();
  assert.equal(records.length, 10);
  assert.equal(records[0].key, 'bulk-11'); // newest first
});

test('recordLaunchStat preserves spawn_ms, boot_ms, phases (new shape)', async () => {
  const rec = {
    key: 'new-shape-1',
    instance: 'inst',
    version: '1.21.11',
    started_at: 5000,
    menu_at: 9000,
    menu_ms: 4000,
    played_ms: 15000,
    spawn_ms: 120,
    boot_ms: 3880,
    phases: { loading_mc: 5100, mixin: 5200, openal: 8900 },
  };
  recordLaunchStat(rec);
  const all = await readLaunchStats(20);
  const found = all.find((r) => r.key === 'new-shape-1');
  assert.ok(found, 'new shape record found');
  assert.equal(found.spawn_ms, 120);
  assert.equal(found.boot_ms, 3880);
  assert.deepEqual(found.phases, { loading_mc: 5100, mixin: 5200, openal: 8900 });
  // Back-compat: old records still readable alongside new ones
  const old = all.find((r) => r.key === 'a');
  assert.ok(old, 'old record still present after new-shape writes');
});

test('old JSONL rows without spawn_ms/boot_ms/phases do not crash readers (back-compat)', async () => {
  const file = path.join(dataDir, 'launch-stats.jsonl');
  // Directly append a legacy-shaped line (pre-C5) — no spawn_ms, boot_ms, phases
  const legacy = { key: 'legacy-1', instance: 'legacy', version: '1.20.1', started_at: 999, menu_at: null, menu_ms: null, played_ms: 1234 };
  fs.appendFileSync(file, JSON.stringify(legacy) + '\n', 'utf8');
  // Also append a garbage-ish legacy where phases is missing entirely
  const legacy2 = { key: 'legacy-2', instance: 'legacy2', version: '1.20.1', started_at: 1000, menu_at: 2000, menu_ms: 1000, played_ms: 2000 };
  fs.appendFileSync(file, JSON.stringify(legacy2) + '\n', 'utf8');

  const records = await readLaunchStats(30);
  const l1 = records.find((r) => r.key === 'legacy-1');
  const l2 = records.find((r) => r.key === 'legacy-2');
  assert.ok(l1, 'legacy-1 readable');
  assert.ok(l2, 'legacy-2 readable');
  // Missing new fields should be undefined, not throw
  assert.equal(l1.spawn_ms, undefined);
  assert.equal(l1.boot_ms, undefined);
  assert.equal(l1.phases, undefined);
  // New-shape record from previous test still intact
  const newer = records.find((r) => r.key === 'new-shape-1');
  assert.ok(newer, 'new-shape record still readable after legacy appends');
  assert.equal(newer.spawn_ms, 120);
});
