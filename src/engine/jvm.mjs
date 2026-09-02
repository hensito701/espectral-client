/**
 * JVM discovery (B4 contract).
 *
 * Ordered probe, each candidate verified by running
 *   `java -XshowSettings:properties -version` once
 * and parsing java.version / java.runtime.version / java.vendor from the
 * merged stdout+stderr output. The winner is cached in config.jvm
 * ({ path, version, build, vendor, source }) so subsequent calls skip the
 * spawn unless the cached file disappeared or the user override changed.
 *
 * Discovery order:
 *   1. config.jdk_path_override     (if set, must verify as >= 21)
 *   2. FastClient bundled Temurin 25: %APPDATA%\FastClient\runtimes\java-25\jdk-25.0.4+7\bin\java.exe
 *   3. system PATH `java`           (only if version >= 21)
 *   4. Adoptium download: GET https://api.adoptium.net/v3/binary/latest/25/ga/<os>/<arch>/jdk/hotspot/normal/eclipse
 *      -> zip (Windows/macOS) or tar.gz (Linux) extracted to <dataDir>/runtimes/
 *
 * JvmInfo: { path, version, build, vendor, source: 'bundled'|'fastclient'|'path'|'downloaded' }
 *   version = java.version        (e.g. '25.0.4')
 *   build   = java.runtime.version (e.g. '25.0.4+7')  <- the AOT cache key input
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadConfig, saveConfig, dataDir } from './config.mjs';

const MIN_MAJOR = 21;

// ---------------------------------------------------------------------------
// Probe memo — avoids paying ~230ms spawnSync per launch when jdk_path_override
// (or any repeated path) is re-probed with the same file identity.
// Key: `${path}:${mtimeMs}:${size}` — stat is per-probe, O(1) vs the spawn.
// On stat failure we fall through uncached (no throw, just no memo hit).
// ---------------------------------------------------------------------------
const probeMemo = new Map();

/** Build the memo key for a java binary, or null when stat fails. */
function probeMemoKey(javaPath) {
  try {
    const st = fs.statSync(javaPath);
    return `${javaPath}:${st.mtimeMs}:${st.size}`;
  } catch {
    return null;
  }
}

/** @internal — for tests only: number of cached probe entries. */
export function __probeMemoSize() {
  return probeMemo.size;
}

/** @internal — for tests only: build the memo key (null on stat failure). */
export function __probeMemoKey(javaPath) {
  return probeMemoKey(javaPath);
}

/** @internal — for tests only: clear the probe memo. */
export function __clearProbeMemo() {
  probeMemo.clear();
}

// Back-compat alias — some test scaffolds reference __probeCacheKey.
export const __probeCacheKey = __probeMemoKey;

/** A Temurin 25 bundled by FastClient, if that launcher is installed. */
export function fastClientJava() {
  const base = process.env.APPDATA || path.join(process.env.USERPROFILE || '', 'AppData', 'Roaming');
  return path.join(base, 'FastClient', 'runtimes', 'java-25', 'jdk-25.0.4+7', 'bin', 'java.exe');
}

/** Resolve a `java` executable from PATH (first hit), else null. */
export function pathJava() {
  const cmd = process.platform === 'win32' ? 'where' : 'which';
  try {
    const r = spawnSync(cmd, ['java'], { encoding: 'utf8', shell: false, windowsHide: true });
    if (r.status !== 0) return null;
    const first = (r.stdout || '').split(/\r?\n/).map((s) => s.trim()).find(Boolean);
    return first && fs.existsSync(first) ? first : null;
  } catch {
    return null;
  }
}

/**
 * Probe a java executable: run `java -XshowSettings:properties -version` and
 * parse the property block. Returns { ok, major, version, build, vendor, error }.
 * `version` = java.version, `build` = java.runtime.version, `major` = parsed
 * major (legacy 1.8 -> 8 handled).
 * Results are memoized in-process keyed by `${path}:${mtimeMs}:${size}` so
 * repeated launches with the same jdk_path_override avoid the ~230ms spawn.
 */
export function probeJava(javaPath) {
  const memoKey = probeMemoKey(javaPath);
  if (memoKey !== null && probeMemo.has(memoKey)) {
    return probeMemo.get(memoKey);
  }
  const result = { ok: false, major: 0, version: null, build: null, vendor: null, error: null };
  let out = '';
  try {
    const r = spawnSync(javaPath, ['-XshowSettings:properties', '-version'], {
      encoding: 'utf8',
      timeout: 30_000,
      shell: false,
      windowsHide: true,
    });
    out = `${r.stdout || ''}\n${r.stderr || ''}`;
    if (r.status !== 0 && r.error) throw r.error;
    // r.status is 0 for `java -version`; the banner may be on stderr.
  } catch (e) {
    result.error = e.message || String(e);
    if (memoKey !== null) probeMemo.set(memoKey, result);
    return result;
  }
  const props = {};
  for (const line of out.split(/\r?\n/)) {
    const m = line.match(/^\s*([a-zA-Z0-9_.]+)\s*=\s*(.*)$/);
    if (m) props[m[1].trim()] = m[2].trim();
  }
  const version = props['java.version'] ?? null;
  const build = props['java.runtime.version'] ?? props['java.version'] ?? null;
  const vendor = props['java.vendor'] ?? null;
  if (!version) {
    // Fallback: parse the `openjdk version "25.0.4" 2025-...` banner line.
    const banner = out.match(/version "([^"]+)"/);
    if (banner) {
      result.version = banner[1];
      result.build = build ?? banner[1];
      const bm = out.match(/\(build ([^)]+)\)/);
      if (bm && /^[0-9]/.test(bm[1])) result.build = bm[1];
    } else {
      result.error = `unparseable java output: ${out.slice(0, 200)}`;
      if (memoKey !== null) probeMemo.set(memoKey, result);
      return result;
    }
  } else {
    result.version = version;
    result.build = build;
  }
  result.vendor = vendor;
  result.major = parseMajor(result.version ?? result.build);
  // The probe reports ANY parseable JDK; the >= MIN_MAJOR floor is a discovery
  // policy (discoverJvm), not a probe property — the legacy tiers (8/16/17)
  // provisioned by runtimes.mjs are legitimately below 21.
  result.ok = result.major > 0;
  if (!result.ok) result.error = `unparseable java version '${result.version ?? result.build}'`;
  if (memoKey !== null) probeMemo.set(memoKey, result);
  return result;
}

/** '25.0.4' -> 25 ; '1.8.0_392' -> 8 ; garbage -> 0. */
export function parseMajor(version) {
  const m = String(version).match(/^(\d+)(?:\.|$)/);
  if (!m) return 0;
  const major = parseInt(m[1], 10);
  return major === 1 ? 8 : major; // legacy 1.8 -> 8
}

/** Convert a probe result + path into the cached JvmInfo shape.
 * `major` is carried explicitly: consumers gate on it (AOT training requires
 * a 25-tier runtime — aot.mjs — and aotAvailable is derived from it in
 * resolveLaunch), and the jdk_path_override path is the ONLY JvmInfo producer
 * that historically omitted it, silently disabling AOT for every instance
 * launched under an override. */
function toJvmInfo(javaPath, probe, source) {
  return {
    path: javaPath,
    version: probe.version,
    build: probe.build,
    vendor: probe.vendor,
    major: probe.major ?? parseMajor(probe.version ?? probe.build ?? ''),
    source,
  };
}

function cacheJvm(info) {
  try {
    saveConfig({ jvm: info });
  } catch {
    /* cache write is best-effort */
  }
  return info;
}

/** Return the cached JvmInfo if its file still exists, else null.
 * `major` is backfilled when absent: config.jvm records written by older
 * builds (before toJvmInfo carried major) lack the field, and the AOT gates
 * read it — a missing field behaved like major 0 and silently disabled AOT. */
function cachedJvm() {
  const cfg = loadConfig();
  if (cfg.jvm && cfg.jvm.path && fs.existsSync(cfg.jvm.path)) {
    const info = { ...cfg.jvm };
    if (typeof info.major !== 'number' || info.major <= 0) {
      info.major = parseMajor(info.version ?? info.build ?? '');
    }
    return info;
  }
  return null;
}

/**
 * Run the full discovery (probe each candidate in order). Returns JvmInfo.
 * Throws Error('NO_JDK') when nothing >= 21 is found and the Adoptium
 * download also fails — with a descriptive message.
 */
export async function discoverJvm() {
  const cfg = loadConfig();
  const warnings = [];

  // 1. Explicit override (always re-verified when set).
  if (cfg.jdk_path_override) {
    const p = cfg.jdk_path_override;
    const probe = fs.existsSync(p) ? probeJava(p) : { ok: false, error: `not found: ${p}` };
    if (probe.ok && probe.major >= MIN_MAJOR) {
      return cacheJvm(toJvmInfo(p, probe, 'path'));
    }
    warnings.push(`jdk_path_override ${p} unusable (${probe.error}); falling through to discovery`);
    console.warn('[jvm] ' + warnings[warnings.length - 1]);
  }

  // Cached winner fast path (override absent or failed -> recompute below).
  if (!cfg.jdk_path_override) {
    const cached = cachedJvm();
    if (cached) return cached;
  }

  // 2. FastClient bundled Temurin 25.
  const fc = fastClientJava();
  if (fs.existsSync(fc)) {
    const probe = probeJava(fc);
    if (probe.ok && probe.major >= MIN_MAJOR) {
      return cacheJvm(toJvmInfo(fc, probe, 'fastclient'));
    }
    warnings.push(`FastClient JDK unusable (${probe.error})`);
  }

  // 3. PATH java >= 21.
  const fromPath = pathJava();
  if (fromPath) {
    const probe = probeJava(fromPath);
    if (probe.ok && probe.major >= MIN_MAJOR) {
      return cacheJvm(toJvmInfo(fromPath, probe, 'path'));
    }
    warnings.push(`PATH java ${fromPath} unusable (${probe.error})`);
  }

  // 4. Adoptium download.
  try {
    const info = await downloadAdoptium();
    return cacheJvm(info);
  } catch (e) {
    warnings.push(`Adoptium download failed: ${e.message}`);
  }

  throw new Error(
    'NO_JDK: no JDK ' + MIN_MAJOR + '+ found (override, FastClient, PATH, Adoptium). ' + warnings.join(' | ')
  );
}

/** Cached-if-possible JVM info (triggers full discovery only when needed). */
export async function getJvmInfo({ override = null } = {}) {
  if (override) {
    // Per-instance JDK override (B4 PATCH /api/instances/:name) — verified
    // directly (any parseable JDK: legacy tiers may point at JDK 8/16/17),
    // never falls through to discovery. Cached like any winner.
    if (!fs.existsSync(override)) {
      throw new Error(`jdk_path_override not found: ${override}`);
    }
    const probe = probeJava(override);
    if (!probe.ok) {
      throw new Error(`jdk_path_override unusable: ${probe.error}`);
    }
    return cacheJvm(toJvmInfo(override, probe, 'path'));
  }
  const cached = cachedJvm();
  if (cached) return cached;
  return discoverJvm();
}

/**
 * Download + extract the latest Temurin 25 JDK from Adoptium into
 * <dataDir>/runtimes/ and return its JvmInfo (source 'downloaded').
 * Delegates to the single shared Adoptium pipeline in runtimes.mjs (which
 * installs into the canonical <dataDir>/runtimes/jdk-<major>/ layout).
 */
export async function downloadAdoptium() {
  const { downloadTemurin } = await import('./runtimes.mjs');
  return downloadTemurin(25);
}

/** Path to jcmd.exe next to a java executable (graceful-exit fallback). */
export function jcmdPath(javaPath) {
  const dir = path.dirname(javaPath);
  return path.join(dir, process.platform === 'win32' ? 'jcmd.exe' : 'jcmd');
}

// ---------------------------------------------------------------------------
// CLI: node src/engine/jvm.mjs [--discover]  ->  prints the discovered JvmInfo
// ---------------------------------------------------------------------------
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const force = process.argv.includes('--discover');
  try {
    const info = force ? await discoverJvm() : await getJvmInfo();
    console.log(JSON.stringify(info, null, 2));
  } catch (e) {
    console.error(e.message);
    process.exit(1);
  }
}
