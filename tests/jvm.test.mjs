import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

import {
  probeJava,
  parseMajor,
  __probeMemoSize,
  __probeMemoKey,
  __clearProbeMemo,
  __probeCacheKey,
} from '../src/engine/jvm.mjs';

const JDK = 'C:/Program Files/Eclipse Adoptium/jdk-25.0.4.7-hotspot/bin/java.exe';
const jdkExists = fs.existsSync(JDK);
const jdkSkip = !jdkExists ? `SKIP: JDK not found at ${JDK} — no local Temurin 25 installed` : false;

// ---------------------------------------------------------------------------
// Helpers: skip gracefully when JDK absent
// ---------------------------------------------------------------------------
function skipIfNoJdk(t) {
  if (!jdkExists) {
    // node:test supports t.skip; fallback to console note + return true
    if (t && typeof t.skip === 'function') t.skip(jdkSkip);
    else console.log(`[jvm.test] ${jdkSkip}`);
    return true;
  }
  return false;
}

// Ensure a clean memo before each relevant test.
function clearMemo() {
  try { __clearProbeMemo(); } catch {}
}

// ---------------------------------------------------------------------------
// C10: memoization + key shape
// ---------------------------------------------------------------------------

test('probeJava memo: two calls with same binary return identical result and memo size is 1', { skip: jdkSkip || undefined }, () => {
  clearMemo();
  assert.equal(__probeMemoSize(), 0, 'memo should start empty');

  const a = probeJava(JDK);
  assert.equal(a.ok, true, `first probe must succeed: ${a.error ?? ''}`);
  assert.equal(__probeMemoSize(), 1, 'first probe fills memo to 1');

  const b = probeJava(JDK);
  // Deep equality: same JDK identity must yield same parsed values.
  assert.deepStrictEqual(b, a, 'second probe must return identical result');
  // Reference identity proves memo hit (same object, no second spawn).
  // We assert reference equality as extra signal; deep equality is the contract.
  assert.equal(b, a, 'second probe should return the cached object reference');
  assert.equal(__probeMemoSize(), 1, 'second probe must not grow memo');

  clearMemo();
});

test('probe memo key includes path, mtimeMs and size', { skip: jdkSkip || undefined }, () => {
  clearMemo();
  const st = fs.statSync(JDK);
  const key = __probeMemoKey(JDK);
  assert.ok(key, 'key must be non-null for existing file');
  // Key shape: `${path}:${mtimeMs}:${size}` — verify all three components.
  const expected = `${JDK}:${st.mtimeMs}:${st.size}`;
  assert.equal(key, expected, 'memo key must be `${path}:${mtimeMs}:${size}`');
  // Also verify via alias
  assert.equal(__probeCacheKey(JDK), expected, 'alias __probeCacheKey must match');

  // The key must contain the size and mtimeMs substrings explicitly.
  assert.ok(key.includes(String(st.size)), 'key must embed file size');
  assert.ok(key.includes(String(st.mtimeMs)), 'key must embed mtimeMs');
  assert.ok(key.startsWith(JDK + ':'), 'key must start with the java path');
  clearMemo();
});

test('probe memo key returns null on stat failure (uncached fallthrough)', () => {
  clearMemo();
  const missing = 'C:/__no_such__/java.exe';
  const key = __probeMemoKey(missing);
  assert.equal(key, null, 'missing file must yield null key');
  assert.equal(__probeCacheKey(missing), null, 'alias must also yield null');
  // probeJava on a missing binary must not populate the memo (stat failure path)
  const before = __probeMemoSize();
  const r = probeJava(missing);
  // On Windows spawnSync with missing exe throws; result.ok is false.
  assert.equal(r.ok, false, 'probe of missing binary must be not ok');
  assert.equal(__probeMemoSize(), before, 'stat-failure probe must not grow memo');
  clearMemo();
});

test('probe memo is per file identity: different path yields different entry', { skip: jdkSkip || undefined }, () => {
  clearMemo();
  const a = probeJava(JDK);
  assert.equal(a.ok, true);
  assert.equal(__probeMemoSize(), 1);
  // Probe a synthetic second path that does not exist — should not hit memo and not cache.
  const miss = 'C:/__no_such2__/java.exe';
  probeJava(miss);
  assert.equal(__probeMemoSize(), 1, 'miss must not add memo entry');
  // Re-probing the real JDK still hits memo.
  const b = probeJava(JDK);
  assert.deepStrictEqual(b, a);
  assert.equal(__probeMemoSize(), 1);
  clearMemo();
});

test('parseMajor handles modern and legacy version strings', () => {
  assert.equal(parseMajor('25.0.4'), 25);
  assert.equal(parseMajor('21.0.1'), 21);
  assert.equal(parseMajor('1.8.0_392'), 8);
  assert.equal(parseMajor('17.0.9'), 17);
  assert.equal(parseMajor('garbage'), 0);
  assert.equal(parseMajor(''), 0);
});

test('probe memo survives getJvmInfo override path (no extra spawn)', { skip: jdkSkip || undefined }, async () => {
  // This test verifies the integration point: getJvmInfo({override}) delegates
  // to probeJava and therefore benefits from the same memo. We cannot count
  // spawns without mocking, but we can assert memo size stays 1 after the
  // getJvmInfo call when the probe was already cached.
  clearMemo();
  const { getJvmInfo } = await import('../src/engine/jvm.mjs');
  // Prime memo via direct probe.
  const p = probeJava(JDK);
  assert.equal(p.ok, true);
  assert.equal(__probeMemoSize(), 1);
  // Isolate config side-effects: use a temp data dir so cacheJvm does not
  // pollute the real repo data/. We only care that getJvmInfo does not add
  // a second memo entry.
  const os = await import('node:os');
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-jvm-test-'));
  const prev = process.env.ESPECTRAL_DATA_DIR;
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    // Ensure config's jvm cache is isolated (loadConfig reads from tmp).
    const info = await getJvmInfo({ override: JDK });
    assert.ok(info.path, 'getJvmInfo with override must return JvmInfo');
    assert.equal(info.path, JDK);
    assert.equal(__probeMemoSize(), 1, 'getJvmInfo override must reuse memo, not grow it');
  } finally {
    process.env.ESPECTRAL_DATA_DIR = prev;
    try { fs.rmSync(tmp, { recursive: true, force: true }); } catch {}
    clearMemo();
  }
});

test('JvmInfo carries major on the override path (AOT gates read it)', { skip: jdkSkip || undefined }, async () => {
  // Regression: toJvmInfo historically omitted `major`, so any consumer gating
  // on resolved.java.major (aot.mjs training gate, resolveLaunch aotAvailable)
  // saw 0 on the jdk_path_override path and silently disabled AOT. The probe
  // result carries major; the JvmInfo must too.
  clearMemo();
  const { getJvmInfo } = await import('../src/engine/jvm.mjs');
  const os = await import('node:os');
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-jvm-major-'));
  const prev = process.env.ESPECTRAL_DATA_DIR;
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const info = await getJvmInfo({ override: JDK });
    assert.equal(info.major, 25, 'override JvmInfo must expose major=25');
    // cachedJvm backfill: a config.jvm record WITHOUT major (legacy shape)
    // must gain a backfilled major, not behave like 0.
    const { loadConfig, saveConfig } = await import('../src/engine/config.mjs');
    saveConfig({ jvm: { path: info.path, version: info.version, build: info.build, vendor: info.vendor, source: 'path' } });
    const cfg = loadConfig();
    assert.equal(cfg.jvm.major, undefined, 'fixture setup: legacy record has no major');
    const noOverride = await getJvmInfo({});
    assert.equal(noOverride.major, 25, 'cachedJvm must backfill major from version/build');
  } finally {
    process.env.ESPECTRAL_DATA_DIR = prev;
    try { fs.rmSync(tmp, { recursive: true, force: true }); } catch {}
    clearMemo();
  }
});
