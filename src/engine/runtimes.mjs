/**
 * runtimes.mjs — multi-JDK provisioning for the version range 1.15.2 – 26.2.
 *
 * Each Minecraft version JSON declares a `javaVersion.majorVersion` (e.g.
 * 1.16.5 → 8, 1.18.2 → 17, 1.21.11 → 21, 26.2 → 25). This module maps that
 * requirement to a concrete Temurin runtime:
 *
 *   javaForVersion(major) -> JvmInfo { path, version, build, vendor, source, major }
 *
 * A candidate JDK fits a tier when its major is >= the required major AND within
 * the tier's ceiling (8→8, 16→16, 17→≤21, 21/25→any). A JDK 25 on the machine is
 * therefore never used for the 8/16/17 tiers.
 *
 * Resolution order per tier (fastest first, all verified by probe):
 *   1. config.jdk_path_override         (must fit the tier)
 *   2. a running same-tier JDK already cached in config.jvm
 *   3. FastClient bundled Temurin 25    (tiers that may use it)
 *   4. system PATH `java`               (probe must fit the tier)
 *   5. already-downloaded runtime under <dataDir>/runtimes/jdk-<major>/
 *   6. Adoptium download                (major 8/16/17/21/25; Temurin LTS)
 *
 * Runtimes are downloaded once per major into <dataDir>/runtimes/ and reused;
 * the AOT cache stays a JDK-25-tier feature — older tiers boot without it.
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { createWriteStream } from 'node:fs';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';
import { pathToFileURL } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import { loadConfig, dataDir } from './config.mjs';
import { probeJava as _probeJava, parseMajor } from './jvm.mjs';
import { USER_AGENT } from './download.mjs';

// Test seam: allow tests to inject a fake probe without spawning a real JVM.
let _probeOverride = null;
export function __setProbeOverride(fn) {
  _probeOverride = fn;
}
export function __clearProbeOverride() {
  _probeOverride = null;
}
function _probe(p) {
  return _probeOverride ? _probeOverride(p) : _probeJava(p);
}

/** Temurin LTS majors the launcher provisions on demand (16 for 1.17.x). */
export const SUPPORTED_MAJORS = Object.freeze([25, 21, 17, 16, 8]);

/** Minecraft version-string prefix -> Java major (the same cuts used by the UI). */
export function javaMajorForVersionId(id) {
  const s = String(id ?? '');
  const snap = s.match(/^(\d{2})w/);
  if (snap) {
    const yy = parseInt(snap[1], 10);
    if (yy >= 26) return 25;
    return yy >= 24 ? 21 : 17;
  }
  const pre = s.match(/^(\d+\.\d+(?:\.\d+)?)/);
  if (pre && pre[1] !== s) return javaMajorForVersionId(pre[1]);
  const rel = s.match(/^1\.(\d+)(?:\.(\d+))?/);
  if (rel) {
    const minor = parseInt(rel[1], 10);
    const patch = rel[2] ? parseInt(rel[2], 10) : 0;
    if (minor >= 21) return 21;
    if (minor === 20) return patch >= 5 ? 21 : 17;
    if (minor === 18 || minor === 19) return 17;
    if (minor === 17) return 16;
    return 8;
  }
  const modern = s.match(/^(\d+)\./);
  if (modern) {
    const major = parseInt(modern[1], 10);
    return major >= 25 ? 25 : 21;
  }
  return 17;
}

/** Windows java.exe / POSIX java name for a runtime dir. */
function javaBinName() {
  return process.platform === 'win32' ? 'java.exe' : 'java';
}

/** <dataDir>/runtimes/jdk-<major>/bin/java(.exe) */
export function runtimeJavaPath(major) {
  return path.join(dataDir(), 'runtimes', `jdk-${major}`, 'bin', javaBinName());
}

/** Windows: %APPDATA%\FastClient\runtimes\java-25\jdk-25.0.4+7\bin\java.exe */
export function fastClientJava() {
  const base = process.env.APPDATA || path.join(process.env.USERPROFILE || '', 'AppData', 'Roaming');
  return path.join(base, 'FastClient', 'runtimes', 'java-25', 'jdk-25.0.4+7', 'bin', javaBinName());
}

/** First `java` from PATH, else null. */
export function pathJava() {
  const cmd = process.platform === 'win32' ? 'where' : 'which';
  try {
    const r = spawnSync(cmd, ['java'], { encoding: 'utf8', shell: false, windowsHide: true });
    if (r.status !== 0) return null;
    const first = (r.stdout || '').split(/\r?\n/).map((x) => x.trim()).find(Boolean);
    return first && fs.existsSync(first) ? first : null;
  } catch {
    return null;
  }
}

/** Cached JvmInfo (config.jvm) if it exists on disk. */
function cachedJvm() {
  try {
    const cfg = loadConfig();
    if (cfg.jvm && cfg.jvm.path && fs.existsSync(cfg.jvm.path)) return { ...cfg.jvm };
  } catch {
    /* ignore */
  }
  return null;
}

/** Persistent per-major JvmInfo cache: <dataDir>/runtimes/jdk-<major>/info.json. */
export function cachedMajor(major) {
  try {
    const f = path.join(dataDir(), 'runtimes', `jdk-${major}`, 'info.json');
    if (!fs.existsSync(f)) return null;
    const info = JSON.parse(fs.readFileSync(f, 'utf8'));
    if (info && info.path && fs.existsSync(info.path)) return info;
  } catch {
    /* ignore */
  }
  return null;
}

export function cacheMajor(major, info) {
  try {
    const f = path.join(dataDir(), 'runtimes', `jdk-${major}`, 'info.json');
    if (fs.existsSync(f)) {
      try {
        const existing = JSON.parse(fs.readFileSync(f, 'utf8'));
        if (isDeepStrictEqual(existing, info)) return info;
      } catch {
        /* corrupted file -> overwrite below */
      }
    }
    const dir = path.join(dataDir(), 'runtimes', `jdk-${major}`);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(f, JSON.stringify(info, null, 2));
  } catch {
    /* best-effort */
  }
  return info;
}

/** Adoptium os/arch tokens for the current platform. */
function adoptiumOsArch() {
  const os = process.platform === 'win32' ? 'windows' : process.platform === 'darwin' ? 'mac' : 'linux';
  const arch = process.arch === 'arm64' ? 'aarch64' : process.arch === 'x64' ? 'x64' : process.arch;
  return { os, arch };
}
// Per-major lock, mirroring download.mjs's destLocks: two concurrent
// downloadTemurin(major) calls would otherwise interleave on the same
// `<archive>.part` (flags:'w' truncates while the other appends) and tear
// each other's download. With the lock the second caller waits, then re-runs
// the already-present checks in its own path and short-circuits.
const majorLocks = new Map();

function withMajorLock(major, fn) {
  const prev = majorLocks.get(major) ?? Promise.resolve();
  const next = prev.then(() => fn(), () => fn());
  majorLocks.set(major, next);
  // Cleanup must not introduce an unhandled rejection when `next` rejects —
  // the caller handles that; the cleanup chain swallows it.
  void next
    .catch(() => {})
    .then(() => {
      if (majorLocks.get(major) === next) majorLocks.delete(major);
    });
  return next;
}

/**
 * Download + extract a Temurin JDK for `major` into <dataDir>/runtimes/jdk-<major>/,
 * probe it, and persist info.json. Serialized per major. Returns the JvmInfo.
 */
export function downloadTemurin(major) {
  return withMajorLock(major, () => downloadTemurinInner(major));
}

async function downloadTemurinInner(major) {
  const { os, arch } = adoptiumOsArch();
  const url = `https://api.adoptium.net/v3/binary/latest/${major}/ga/${os}/${arch}/jdk/hotspot/normal/eclipse`;

  const dir = path.join(dataDir(), 'runtimes', `jdk-${major}`);
  fs.mkdirSync(dir, { recursive: true });
  const isZip = process.platform !== 'linux';
  const archive = path.join(dir, `jdk-${major}.${isZip ? 'zip' : 'tar.gz'}.part`);
  const finalArchive = archive.replace(/\.part$/, '');
  fs.rmSync(finalArchive, { force: true });

  const res = await fetch(url, { redirect: 'follow', headers: { 'User-Agent': USER_AGENT } });
  if (!res.ok || !res.body) {
    throw new Error(`Adoptium ${res.status} ${res.statusText}`);
  }
  await pipeline(Readable.fromWeb(res.body), createWriteStream(archive));
  fs.renameSync(archive, finalArchive);

  if (isZip) {
    const { default: extract } = await import('extract-zip');
    await extract(finalArchive, { dir });
  } else {
    const r = spawnSync('tar', ['-xzf', finalArchive, '-C', dir], { stdio: 'pipe', windowsHide: true });
    if (r.status !== 0) throw new Error(`tar extract failed: ${(r.stderr || '').toString()}`);
  }
  fs.rmSync(finalArchive, { force: true });

  // Locate bin/java(.exe): Adoptium archives nest the JDK under a top-level
  // folder whose name varies (jdk8u412-b08, jdk-21.0.7+6, and for major 8
  // exactly "jdk-8" — which collides with the target dir name and must NOT be
  // excluded). If the binary is not at <dir>/bin, hoist the single nested
  // runtime dir's contents up so the canonical <dir>/bin/java(.exe) holds.
  if (!fs.existsSync(runtimeJavaPath(major))) {
    const javaBin = javaBinName();
    const nested = fs
      .readdirSync(dir, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .find((e) => fs.existsSync(path.join(dir, e.name, 'bin', javaBin)));
    if (nested) {
      const inner = path.join(dir, nested.name);
      for (const entry of fs.readdirSync(inner)) {
        fs.renameSync(path.join(inner, entry), path.join(dir, entry));
      }
      fs.rmSync(inner, { recursive: true, force: true });
    }
  }
  const javaPath = runtimeJavaPath(major);
  if (!fs.existsSync(javaPath)) {
    throw new Error(`Adoptium ${major} extracted but no bin/${javaBinName()} found under ${dir}`);
  }
  const probe = _probe(javaPath);
  if (!probe.ok) throw new Error(`downloaded JDK ${major} failed probe: ${probe.error}`);
  return cacheMajor(major, { path: javaPath, version: probe.version, build: probe.build, vendor: probe.vendor, source: 'downloaded', major });
}

/**
 * Highest Java major each tier will accept. Old-era Minecraft breaks on modern
 * JVMs (removed APIs, LWJGL 2, reflection restrictions), so a version declaring
 * JDK 8 / 16 must get an exact-major runtime; the 17 tier may upgrade to 21 but
 * never to 25; the 21/25 tiers may use the latest available JVM.
 */
export function maxMajorFor(need) {
  if (need <= 8) return 8; // 1.15.2 – 1.16.x era
  if (need === 16) return 16; // 1.17.x
  if (need === 17) return 21; // 1.18 – 1.20.4
  return Infinity; // 21/25 tiers may upgrade freely
}

/**
 * Resolve a JDK for the given required major (8/16/17/21/25).
 * Accepts the first candidate whose major is >= need and <= maxMajorFor(need),
 * so a JDK 25 on the machine is never used for the 8/16/17 tiers.
 * Throws Error('NO_JDK_<major>') with a descriptive message when nothing works.
 */
export async function javaForVersion(major) {
  const need = Number(major) || 17;
  const maxMajor = maxMajorFor(need);
  const fits = (probe) => probe.ok && probe.major >= need && probe.major <= maxMajor;
  const cfg = loadConfig();

  // 1. Explicit override — must fit the tier. Never persist a mismatched probe
  // into runtimes/jdk-<need>/info.json; return the JvmInfo directly.
  if (cfg.jdk_path_override) {
    const p = cfg.jdk_path_override;
    const probe = fs.existsSync(p) ? _probe(p) : { ok: false, error: `not found: ${p}` };
    if (fits(probe)) {
      return { path: p, version: probe.version, build: probe.build, vendor: probe.vendor, source: 'path', major: probe.major };
    }
    console.warn(`[runtimes] jdk_path_override ${p} unusable for JDK ${need}+ (${probe.error || `probed major ${probe.major} outside tier`}); falling through`);
  }

  // 2. Already-discovered same-tier JDK (config.jvm).
  const cached = cachedJvm();
  if (cached) {
    const cm = parseMajor(cached.build ?? cached.version ?? '');
    if (fits({ ok: true, major: cm })) {
      return cacheMajor(need, { ...cached, major: cm });
    }
  }

  // 3. FastClient bundled Temurin 25 — matches only tiers that may use it.
  // Return directly; do not persist a probe whose major may differ from need
  // into jdk-<need>/info.json (would poison javaForMajorExact).
  const fc = fastClientJava();
  if (fs.existsSync(fc)) {
    const probe = _probe(fc);
    if (fits(probe)) {
      return { path: fc, version: probe.version, build: probe.build, vendor: probe.vendor, source: 'fastclient', major: probe.major };
    }
  }

  // 4. PATH java — same no-poison rule as FastClient.
  const fromPath = pathJava();
  if (fromPath) {
    const probe = _probe(fromPath);
    if (fits(probe)) {
      return { path: fromPath, version: probe.version, build: probe.build, vendor: probe.vendor, source: 'path', major: probe.major };
    }
  }

  // 5. Already-downloaded runtime.
  const existing = cachedMajor(need);
  if (existing) return existing;

  // 6. Adoptium download.
  const info = await downloadTemurin(need);
  try {
    saveConfig({ jvm: info });
  } catch {
    /* best-effort */
  }
  return info;
}

/**
 * Resolve a JDK with EXACTLY the given major — no tier upgrades. NeoForge
 * 1.21.x must boot on an exact Java 21 runtime (a JDK 25 breaks
 * bootstraplauncher module resolution), so a newer major is never accepted.
 * Order: cached exact-major runtime -> PATH java (probed major must match) ->
 * Adoptium download. Throws Error('NO_JDK_<major>') when nothing works.
 */
export async function javaForMajorExact(major) {
  const need = Number(major) || 21;

  // 1. Already-downloaded exact-major runtime (fastest, fully verified).
  //    The per-major cache must actually BE the requested major: a stale
  //    info.json (e.g. an old tier-cache pointing at a JDK 25) must never
  //    satisfy an exact-21 request — NeoForge breaks on anything newer.
  const existing = cachedMajor(need);
  if (existing && Number(existing.major) === need) return existing;

  // 2. PATH java, only when its probed major is EXACTLY the required one.
  const fromPath = pathJava();
  if (fromPath) {
    const probe = _probe(fromPath);
    if (probe.ok && probe.major === need) {
      return cacheMajor(need, {
        path: fromPath,
        version: probe.version,
        build: probe.build,
        vendor: probe.vendor,
        source: 'path',
        major: probe.major,
      });
    }
  }

  // 3. Adoptium download (exact-major Temurin LTS). downloadTemurin caches its
  // own per-major info.json; deliberately NOT written to config.jvm (that is
  // the primary/default JVM cache and must not be downgraded to 21 by a
  // NeoForge launch).
  try {
    return await downloadTemurin(need);
  } catch (cause) {
    throw new Error(
      `NO_JDK_${need}: no JDK ${need} available (PATH probe mismatch and Adoptium download failed: ${cause?.message ?? cause})`
    );
  }
}

// ---------------------------------------------------------------------------
// CLI: node src/engine/runtimes.mjs <major>  ->  prints the resolved JvmInfo
// ---------------------------------------------------------------------------
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const major = Number(process.argv[2] || 25);
  try {
    const info = await javaForVersion(major);
    console.log(JSON.stringify(info, null, 2));
  } catch (e) {
    console.error(e.message);
    process.exit(1);
  }
}
