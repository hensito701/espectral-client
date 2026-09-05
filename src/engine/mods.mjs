/**
 * Modrinth v2 performance mod set (1.21–26.2) + local branding preset.
 */
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { mkdir, readFile, readdir, rename, stat, unlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  downloadAll,
  downloadConcurrency,
  emitEvent,
  fetchJson,
  sha1File,
} from './download.mjs';
import { assertValidInstanceName, loadInstanceMeta } from './resolver.mjs';
import { effectiveModsDir } from './instances.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '..', '..');

export const MODRINTH_API = 'https://api.modrinth.com/v2';
export const PERFORMANCE_PRESET = 'performance';
export const BRANDING_PRESET = 'branding';
/** QoL preset: in-client zoom / fullbright / nofog (the "lunar-like" bundle). */
export const QOL_PRESET = 'qol';

export const PINNED_VERSIONS = Object.freeze(['1.21.11', '26.2']);
export const PINNED_VERSION = '1.21.11';

export const PINS_BY_VERSION = Object.freeze({
  '1.21.11': Object.freeze([
    { slug: 'sodium', project_id: 'AANobbMI', version_id: 'UddlN6L4', version_number: '0.8.7' },
    { slug: 'iris', project_id: 'YL57xq9U', version_id: 'fDpuVzVr', version_number: '1.10.7' },
    { slug: 'lithium', project_id: 'gvQqBUqZ', version_id: 'Ow7wA0kG', version_number: '0.21.4' },
    { slug: 'ferrite-core', project_id: 'uXXizFIs', version_id: 'Ii0gP3D8', version_number: '8.2.0' },
    { slug: 'krypton', project_id: 'fQEb0iXm', version_id: 'O9LmWYR7', version_number: '0.2.10' },
    { slug: 'modmenu', project_id: 'mOgUt4GM', version_id: 'Tyk71iSw', version_number: '17.0.0' },
    { slug: 'fabric-api', project_id: 'P7dR8mSH', version_id: '6qAuTtLR', version_number: '0.141.6' },
  ]),
  '26.2': Object.freeze([
    { slug: 'sodium', project_id: 'AANobbMI', version_id: '2Yom1N68', version_number: 'mc26.2-0.9.1-fabric' },
    { slug: 'iris', project_id: 'YL57xq9U', version_id: 'oaD6KQls', version_number: '1.11.2+26.2-fabric' },
    { slug: 'lithium', project_id: 'gvQqBUqZ', version_id: 'f7vZ0VWU', version_number: 'mc26.2-0.25.3-fabric' },
    { slug: 'ferrite-core', project_id: 'uXXizFIs', version_id: 'd5ddUdiB', version_number: '9.0.0-fabric' },
    { slug: 'krypton', project_id: 'fQEb0iXm', version_id: '5WeL0Nkz', version_number: '0.3.1' },
    { slug: 'modmenu', project_id: 'mOgUt4GM', version_id: 'njXb639R', version_number: '20.0.1' },
    { slug: 'fabric-api', project_id: 'P7dR8mSH', version_id: '3gT0I5vt', version_number: '0.156.0+26.2' },
  ]),
});

export const PINS = PINS_BY_VERSION['1.21.11'];

/**
 * QoL bundle pins (gamma-utils / clear-fog) per pinned version. Researched
 * 2026-08-10 against the Modrinth API (project + best version per game
 * version). Kept separate from the performance pins so the two bundles stay
 * independently installable (a 7-pin test depends on the performance set).
 * LEGACY-OPTIONAL: fullbright and nofog are native owned features now
 * (gamma driven live / fog removed live by the mod — no jars, no restart),
 * so these pins are still installable but no longer required; leftover jars
 * are inert. Zoom ships native inside espectral-menu (ZoomEngine, hold Z) —
 * installing Ok Zoomer alongside would double-bind Z.
 */
export const PINS_QOL_BY_VERSION = Object.freeze({
  '1.21.11': Object.freeze([
    { slug: 'gamma-utils', project_id: 'wdLuzzEP', version_id: 'aVJkWMQl', version_number: '2.5.10' },
    { slug: 'clear-fog', project_id: '46n24c6r', version_id: 'CjWMitqz', version_number: '1.21.11_Fabric' },
  ]),
  '26.2': Object.freeze([
    { slug: 'gamma-utils', project_id: 'wdLuzzEP', version_id: 'bR4tEPFq', version_number: '3.1.1+Fabric' },
    { slug: 'clear-fog', project_id: '46n24c6r', version_id: 'GZ2Uiwgv', version_number: '26.2_Fabric' },
  ]),
});

/** Pins for the QoL bundle on a supported version (exact set or dynamic). */
export async function qolPinsForVersion(version) {
  const exact = PINS_QOL_BY_VERSION[version];
  if (exact) return exact;
  if (!supportsPerformanceBundle(version)) return null;
  return resolveDynamicPins(version, QOL_PROJECTS);
}

/**
 * Projects whose required dependencies are jar-in-jar bundled inside their
 * parent mod — resolved but not downloaded separately.
 * eXts2L7r = placeholder-api (Text Placeholder API by Patbox, a Fabric lib
 * embedded in modmenu).
 */
const JAR_IN_JAR = new Set(['eXts2L7r']);

// ---------------------------------------------------------------------------
// Branding preset: the bundled Espectral Menu mod (assets/branding/*.jar +
// branding.json, staged to <resources>/branding next to the engine).
// No Modrinth round trip: install is a sha1-verified local copy.
// ---------------------------------------------------------------------------
function brandingDir() {
  // Bundled layout: the tauri resource dir ships the jars at
  // <resources>/branding (staged beside engine/); REPO_ROOT resolves to the
  // parent of the resource dir in the packaged app. Dev layout keeps the jars
  // under assets/branding at the repo root.
  const candidates = [
    path.join(REPO_ROOT, 'resources', 'branding'),
    path.join(REPO_ROOT, 'branding'),
    path.join(REPO_ROOT, 'assets', 'branding'),
  ];
  for (const dir of candidates) {
    if (existsSync(dir)) return dir;
  }
  return candidates[candidates.length - 1];
}

const brandingManifestCache = new Map();

/** Manifest { mod_id, mod_version, mods: { <mcver>: { filename, sha1, size } } }; null when absent/broken. */
function brandingManifest() {
  const dir = brandingDir();
  if (brandingManifestCache.has(dir)) return brandingManifestCache.get(dir);
  let manifest = null;
  try {
    manifest = JSON.parse(readFileSync(path.join(dir, 'branding.json'), 'utf8'));
  } catch {
    manifest = null;
  }
  brandingManifestCache.set(dir, manifest);
  return manifest;
}

/** Minecraft versions covered by the bundled branding jars. */
export function brandingVersions() {
  return Object.keys(brandingManifest()?.mods ?? {});
}

/** True when a bundled Espectral Menu jar exists for this version. */
export function supportsBranding(version) {
  return brandingManifest()?.mods?.[version] != null;
}

/** Pin-shaped branding entry for a version (null when unsupported). */
export function brandingPinForVersion(version) {
  const manifest = brandingManifest();
  const entry = manifest?.mods?.[version];
  if (!entry) return null;
  return {
    slug: manifest.mod_id,
    filename: entry.filename,
    sha1: entry.sha1,
    size: entry.size,
    version_number: manifest.mod_version,
  };
}

function sha1Buffer(data) {
  return createHash('sha1').update(data).digest('hex');
}

/**
 * True for release-style Minecraft version ids inside the supported range
 * [1.21, 26.2]: classic naming `1.2X[.patch]` with minor >= 21 (1.21, 1.21.11,
 * 1.22.4) and modern naming `NN[.minor]` with 21 <= NN <= 26 (21.0, 25.1,
 * 26.2). Snapshots (`25w11a`), 1.20.x and below, and 27+ are not supported.
 * Pure — no network.
 */
export function supportsPerformanceBundle(version) {
  if (typeof version !== 'string' || version.length === 0) return false;
  if (/^\d{2}w\d/.test(version)) return false; // snapshots (e.g. 25w11a)
  const classic = /^1\.(\d+)(?:\.\d+)*$/.exec(version);
  if (classic) {
    const minor = Number(classic[1]);
    return minor >= 21 && minor <= 26;
  }
  const modern = /^(\d+)(?:\.\d+)*$/.exec(version);
  if (modern) {
    const nn = Number(modern[1]);
    return nn >= 21 && nn <= 26;
  }
  return false;
}

/**
 * Pin set for a version: exact researched pins for 1.21.11/26.2, otherwise
 * dynamically resolved per project for any supported release. Returns null
 * when the version is unsupported or any of the seven projects has no fabric
 * build for it (never throws; fetch failures count as unresolved).
 */
export async function pinsForVersion(version) {
  const exact = PINS_BY_VERSION[version];
  if (exact) return exact;
  if (!supportsPerformanceBundle(version)) return null;
  return resolveDynamicPins(version);
}

export function pinNoteForVersion(version) {
  if (supportsPerformanceBundle(version)) return null;
  return `no performance mod bundle for ${version}; the bundle covers release versions 1.21 through 26.2`;
}

// In-memory Modrinth version metadata cache (refreshed per process).
const metaCache = new Map();

/**
 * Resilient Modrinth fetch with 2 retries, exponential backoff, and clear error formatting.
 */
async function fetchJsonWithRetry(url, { headers = {}, timeoutMs = 30000, retries = 2, backoffBaseMs = 1000 } = {}) {
  let lastErr = null;
  for (let attempt = 0; attempt <= retries; attempt++) {
    try {
      return await fetchJson(url, { headers, timeoutMs });
    } catch (err) {
      lastErr = err;
      if (attempt < retries) {
        const delay = backoffBaseMs * (attempt + 1);
        await new Promise((r) => setTimeout(r, delay));
      }
    }
  }
  const causeMsg = lastErr?.cause?.message || (lastErr?.cause ? String(lastErr.cause) : null);
  const reason = causeMsg && !lastErr?.message.includes(causeMsg)
    ? `${lastErr?.message} (${causeMsg})`
    : (lastErr?.message ?? String(lastErr));
  const err = new Error(`fetch failed for ${url}: ${reason}`);
  err.status = lastErr?.status ?? 502;
  err.url = url;
  err.cause = lastErr;
  throw err;
}

async function versionMeta(versionId) {
  if (metaCache.has(versionId)) return metaCache.get(versionId);
  const url = `${MODRINTH_API}/version/${encodeURIComponent(versionId)}`;
  const meta = await fetchJsonWithRetry(url);
  metaCache.set(versionId, meta);
  return meta;
}

async function bestVersionFor(projectId, gameVersion = PINNED_VERSION, loader = 'fabric') {
  const q =
    `game_versions=${encodeURIComponent(JSON.stringify([gameVersion]))}` +
    `&loaders=${encodeURIComponent(JSON.stringify([loader]))}`;
  const url = `${MODRINTH_API}/project/${encodeURIComponent(projectId)}/version?${q}`;
  const list = await fetchJsonWithRetry(url);
  const ok = (list ?? []).filter((v) => v.status === 'listed' || v.status === 'approved');
  return ok[0] ?? null;
}
// Per (projectId, gameVersion, loader) cache of best-version resolution —
// avoids repeat Modrinth calls across list/preset/single-install paths.
// Only genuine results are cached (including a real "no listed build" null);
// thrown fetch/parse failures return unresolved WITHOUT caching, so a
// transient Modrinth outage can't permanently empty the mod list.
const bestVersionCache = new Map();

async function bestVersionForCached(projectId, gameVersion, loader = 'fabric') {
  const key = `${projectId}\u0000${gameVersion}\u0000${loader}`;
  if (bestVersionCache.has(key)) return bestVersionCache.get(key);
  let meta;
  try {
    meta = await bestVersionFor(projectId, gameVersion, loader);
  } catch {
    return null; // transient failure: unresolved this call, retry next time
  }
  bestVersionCache.set(key, meta);
  return meta;
}

/** The seven canonical performance projects (slug + Modrinth project id). */
const PERFORMANCE_PROJECTS = Object.freeze([
  { slug: 'sodium', project_id: 'AANobbMI' },
  { slug: 'iris', project_id: 'YL57xq9U' },
  { slug: 'lithium', project_id: 'gvQqBUqZ' },
  { slug: 'ferrite-core', project_id: 'uXXizFIs' },
  { slug: 'krypton', project_id: 'fQEb0iXm' },
  { slug: 'modmenu', project_id: 'mOgUt4GM' },
  { slug: 'fabric-api', project_id: 'P7dR8mSH' },
]);

/** Legacy-optional QoL projects (native fullbright/nofog supersede them — zoom is native). Still installable, no longer required. */
const QOL_PROJECTS = Object.freeze([
  { slug: 'gamma-utils', project_id: 'wdLuzzEP' },
  { slug: 'clear-fog', project_id: '46n24c6r' },
]);
/**
 * Resolve a bundle's projects to concrete version pins for a game version.
 * Returns null if ANY project has no listed fabric build (bundle unavailable);
 * individual fetch failures are treated as unresolved.
 */
async function resolveDynamicPins(gameVersion, projects = PERFORMANCE_PROJECTS) {
  const resolved = await Promise.all(
    projects.map(async (p) => {
      const meta = await bestVersionForCached(p.project_id, gameVersion);
      if (!meta) return null;
      return {
        slug: p.slug,
        project_id: p.project_id,
        version_id: meta.id,
        version_number: meta.version_number,
      };
    })
  );
  return resolved.some((r) => !r) ? null : resolved;
}

function primaryFile(meta) {
  return meta.files?.find((f) => f.primary) ?? meta.files?.[0] ?? null;
}

function modsDirOf(instanceOrName) {
  return effectiveModsDir(instanceOrName);
}

function assertSafeFilename(filename) {
  let base;
  if (typeof filename === 'string' && filename.endsWith('.jar.disabled')) {
    base = filename.slice(0, -'.jar.disabled'.length);
  } else if (typeof filename === 'string' && filename.endsWith('.jar')) {
    base = filename.slice(0, -'.jar'.length);
  } else {
    base = null;
  }
  if (
    !base ||
    base.length === 0 ||
    base.length > 120 ||
    base.includes('..') ||
    !/^[A-Za-z0-9 _.+@-]+$/.test(base)
  ) {
    const err = new Error(`invalid mod filename: ${filename}`);
    err.status = 400;
    err.code = 'invalid_filename';
    throw err;
  }
  return base;
}

// ---------------------------------------------------------------------------
// List: scan <instance>/mods + join pin metadata
// ---------------------------------------------------------------------------
export async function listMods(instanceName) {
  assertValidInstanceName(instanceName);
  const instance = await loadInstanceMeta(instanceName);
  const modsDir = modsDirOf(instance);
  await mkdir(modsDir, { recursive: true });

  const pins = await pinsForVersion(instance.version);
  const note = pinNoteForVersion(instance.version);
  if (!pins) {
    console.warn(`[mods] ${note}`);
    return [];
  }

  const metas = await Promise.all(
    pins.map((p) => versionMeta(p.version_id).catch(() => null))
  );
  const metaByVersion = new Map(pins.map((p, i) => [p.version_id, metas[i]]));
  const fileByVersion = new Map();
  const pinFilenameToVersion = new Map();
  for (const p of pins) {
    const meta = metaByVersion.get(p.version_id);
    const file = meta ? primaryFile(meta) : null;
    if (file) {
      fileByVersion.set(p.version_id, file);
      pinFilenameToVersion.set(file.filename, p.version_id);
    }
  }

  const onDisk = (await readdir(modsDir))
    .filter((f) => f.endsWith('.jar') || f.endsWith('.jar.disabled'))
    .sort();

  const entries = [];
  for (const file of onDisk) {
    const disabled = file.endsWith('.jar.disabled');
    const base = disabled ? file.slice(0, -'.disabled'.length) : file;
    const abs = path.join(modsDir, file);
    let sha1 = null;
    let size = null;
    try {
      const st = await stat(abs);
      size = st.size;
      sha1 = await sha1File(abs);
    } catch {
      /* file raced away mid-scan */
    }
    const versionId = pinFilenameToVersion.get(base) ?? null;
    const pin = versionId ? pins.find((p) => p.version_id === versionId) : null;
    entries.push({
      filename: base,
      project_slug: pin?.slug ?? null,
      version_number:
        pin ? (metaByVersion.get(pin.version_id)?.version_number ?? pin.version_number) : null,
      version_id: versionId,
      sha1,
      size,
      enabled: !disabled,
      installed: true,
    });
  }

  const seen = new Set(entries.map((e) => e.version_id).filter(Boolean));
  for (const p of pins) {
    if (seen.has(p.version_id)) continue;
    const meta = metaByVersion.get(p.version_id);
    const file = fileByVersion.get(p.version_id);
    entries.push({
      filename: file?.filename ?? `${p.slug}.jar`,
      project_slug: p.slug,
      version_number: meta?.version_number ?? p.version_number,
      version_id: p.version_id,
      sha1: file?.hashes?.sha1 ?? null,
      size: file?.size ?? null,
      enabled: false,
      installed: false,
    });
  }

  // Tag the bundled Espectral Menu jar with its branding metadata when present
  // in the mods dir (so the UI shows a recognizable, toggleable row).
  const brandingPin = brandingPinForVersion(instance.version);
  if (brandingPin) {
    const existing = entries.find((e) => e.filename === brandingPin.filename);
    if (existing) {
      existing.project_slug = brandingPin.slug;
      existing.version_number = brandingPin.version_number;
    }
  }
  return entries;
}

// ---------------------------------------------------------------------------
// Enable/disable: rename <file>.jar <-> <file>.jar.disabled
// ---------------------------------------------------------------------------
export async function setModEnabled(instanceName, filename, enabled) {
  assertValidInstanceName(instanceName);
  const instance = await loadInstanceMeta(instanceName);
  const base = assertSafeFilename(filename);
  const modsDir = modsDirOf(instance);
  const jar = path.join(modsDir, `${base}.jar`);
  const disabled = path.join(modsDir, `${base}.jar.disabled`);
  const src = enabled ? disabled : jar;
  const dst = enabled ? jar : disabled;
  const srcExists = existsSync(src);
  const dstExists = existsSync(dst);
  if (!srcExists && !dstExists) {
    const err = new Error(`mod not found: ${filename}`);
    err.status = 404;
    err.code = 'mod_not_found';
    throw err;
  }
  if (srcExists) {
    if (dstExists) await unlink(dst).catch(() => {});
    await rename(src, dst);
  }
  return { filename: `${base}.jar`, enabled };
}

// ---------------------------------------------------------------------------
// Install preset (background; sha1-verified, mod-progress events)
// ---------------------------------------------------------------------------
export async function installPreset(instanceName, preset = PERFORMANCE_PRESET) {
  try {
    if (preset !== PERFORMANCE_PRESET && preset !== BRANDING_PRESET && preset !== QOL_PRESET) {
      const err = new Error(`unknown preset: ${preset}`);
      err.status = 400;
      err.code = 'unknown_preset';
      throw err;
    }
    assertValidInstanceName(instanceName);
    const instance = await loadInstanceMeta(instanceName);
    // Every mod-pin preset (performance + QoL) is a Fabric/Quilt mod. A
    // vanilla instance has no Fabric loader, so dropping the jars in is pure
    // clutter (vanilla ignores mods/) and misleads the user into thinking the
    // mods apply. Refuse with a clear note — the vanilla no-fog / fullbright
    // live in the integrated Client options (Settings) instead. The branding
    // preset is version-keyed (bundled resource-pack menu), not loader-keyed,
    // so it stays available on any instance.
    if (preset !== BRANDING_PRESET && instance.loader !== 'fabric') {
      const note = `preset '${preset}' needs a Fabric instance (this one is '${instance.loader}'); ` +
        'integrated fullbright/no-fog are available under Settings → Client';
      emitEvent('mod-progress', {
        instance: instanceName,
        phase: 'error',
        preset,
        note,
        message: note,
      });
      return { installed: [], notes: [note] };
    }
    const modsDir = modsDirOf(instance);
    await mkdir(modsDir, { recursive: true });

    if (preset === BRANDING_PRESET) {
      return installBrandingPreset(instance, modsDir);
    }

    const pins =
      preset === QOL_PRESET ? await qolPinsForVersion(instance.version) : await pinsForVersion(instance.version);
    const note = pinNoteForVersion(instance.version);
    if (!pins) {
      console.warn(`[mods] ${note}`);
      emitEvent('mod-progress', {
        instance: instanceName,
        phase: 'error',
        preset,
        note,
        message: note,
      });
      return { installed: [], notes: [note] };
    }

    emitEvent('mod-progress', {
      instance: instanceName,
      phase: 'start',
      preset,
      total: pins.length,
      index: 0,
      message: `installing ${pins.length} pinned mods`,
    });

    const metas = await Promise.all(pins.map((p) => versionMeta(p.version_id)));
    const installMap = new Map();
    for (let i = 0; i < pins.length; i++) {
      installMap.set(pins[i].version_id, { pin: pins[i], meta: metas[i] });
    }

    // 2. Expand required dependencies (grow-while-scanning).
    const notes = [];
    const queue = [...installMap.values()];
    for (let qi = 0; qi < queue.length; qi++) {
      const { meta } = queue[qi];
      for (const dep of meta.dependencies ?? []) {
        if (dep.dependency_type !== 'required') continue;
        if (dep.version_id) {
          if (!installMap.has(dep.version_id)) {
            const m = await versionMeta(dep.version_id);
            installMap.set(dep.version_id, { pin: null, meta: m });
            queue.push({ pin: null, meta: m });
          }
          continue;
        }
        if (pins.some((p) => p.project_id === dep.project_id)) continue;
        if (JAR_IN_JAR.has(dep.project_id)) {
          notes.push(`dependency ${dep.project_id} is jar-in-jar bundled; skipped`);
          continue;
        }
        const best = await bestVersionFor(dep.project_id, instance.version);
        if (best) {
          if (!installMap.has(best.id)) {
            installMap.set(best.id, { pin: null, meta: best });
            queue.push({ pin: null, meta: best });
          }
        } else {
          notes.push(`could not resolve dependency ${dep.project_id} for ${instance.version}`);
        }
      }
    }

    // 3. Download everything (sha1-verified, concurrency from config).
    const installs = [...installMap.values()];
    const items = installs.map(({ meta }, i) => {
      const file = primaryFile(meta);
      return {
        url: file.url,
        dest: path.join(modsDir, file.filename),
        sha1: file.hashes?.sha1 ?? null,
        size: file.size ?? null,
        eventName: 'mod-progress',
        context: {
          instance: instanceName,
          filename: file.filename,
          preset,
          index: i,
          total: installs.length,
        },
      };
    });
    await downloadAll(items, { concurrency: downloadConcurrency() });

    emitEvent('mod-progress', {
      instance: instanceName,
      phase: 'done',
      preset,
      total: installs.length,
      index: installs.length,
      message: `installed ${installs.length} mods`,
    });
    return {
      installed: installs.map(({ meta }) => primaryFile(meta).filename),
      notes,
    };
  } catch (err) {
    emitEvent('mod-progress', {
      instance: instanceName,
      phase: 'error',
      preset,
      error: err.message,
      message: err.message,
    });
    throw err;
  }
}

// ---------------------------------------------------------------------------
// Branding preset install: sha1-verified local copy of the bundled jar
// ---------------------------------------------------------------------------
async function installBrandingPreset(instance, modsDir) {
  const pin = brandingPinForVersion(instance.version);
  const notes = [];
  if (!pin) {
    const note = `no bundled Espectral Menu for ${instance.version}; branding covers ${brandingVersions().join(', ') || 'nothing'}`;
    notes.push(note);
    console.warn(`[mods] ${note}`);
    emitEvent('mod-progress', {
      instance: instance.name,
      phase: 'error',
      preset: BRANDING_PRESET,
      note,
      total: 1,
      index: 1,
      done: 1,
      message: note,
    });
    return { installed: [], notes };
  }

  const src = path.join(brandingDir(), pin.filename);
  const dest = path.join(modsDir, pin.filename);

  emitEvent('mod-progress', {
    instance: instance.name,
    phase: 'start',
    preset: BRANDING_PRESET,
    filename: pin.filename,
    total: 1,
    index: 0,
    done: 0,
    message: `installing ${pin.filename}`,
  });

  // Verify the bundled source against the manifest, then copy. Any stale
  // disabled twin is removed so the freshly verified jar is the single
  // source of truth (a leftover <name>.jar.disabled would otherwise show as
  // a duplicate row and could be re-enabled over the new jar).
  await unlink(dest + '.disabled').catch(() => {});
  const data = await readFile(src);
  const srcSha1 = sha1Buffer(data);
  if (pin.sha1 && srcSha1 !== pin.sha1) {
    const err = new Error(`branding source sha1 mismatch for ${pin.filename}: ${srcSha1} != ${pin.sha1}`);
    err.code = 'branding_sha1_mismatch';
    throw err;
  }
  await writeFile(dest, data);
  const destSha1 = await sha1File(dest);
  if (destSha1 !== srcSha1) {
    await unlink(dest).catch(() => {});
    const err = new Error(`branding install failed sha1 verification: ${destSha1}`);
    err.code = 'branding_sha1_mismatch';
    throw err;
  }

  emitEvent('mod-progress', {
    instance: instance.name,
    phase: 'done',
    preset: BRANDING_PRESET,
    filename: pin.filename,
    total: 1,
    index: 1,
    done: 1,
    message: `installed ${pin.filename}`,
  });
  return { installed: [pin.filename], notes };
}
/**
 * Retro-seed helper: ensure the bundled Espectral Menu jar exists for old
 * instances that predate the branding preset. No-op for non-fabric or
 * unsupported versions, and silently warns (never throws) on copy failure
 * so launch/detail never fails over branding.
 */
export async function ensureBrandingSeeded(instance) {
  try {
    if (!instance || instance.loader !== 'fabric' || !supportsBranding(instance.version)) return;
    const pin = brandingPinForVersion(instance.version);
    if (!pin) return;
    const modsDir = effectiveModsDir(instance);
    const dest = path.join(modsDir, pin.filename);
    // Prune stale branding jars: an instance upgraded across launcher versions
    // can otherwise carry two espectral-menu jars (same mod id) at once, which
    // conflicts in-game. Only the pinned filename survives.
    const entries = await readdir(modsDir).catch(() => []);
    const stale = entries.filter(
      (f) => f !== pin.filename && f !== `${pin.filename}.disabled` && /^espectral-menu-.*\.jar(\.disabled)?$/i.test(f)
    );
    // Presence alone is NOT freshness: the pinned FILENAME is version-derived
    // (espectral-menu-26.2-1.3.0.jar) and stays identical across rebuilds of the
    // same mod version, so a launcher upgrade shipping a rebuilt jar (new sha1,
    // e.g. a mixin added) would leave the OLD bytes in place forever. Verify the
    // installed copy's sha1 against the pin and reinstall on mismatch.
    let fresh = false;
    try {
      if (existsSync(dest)) {
        const installedSha1 = await sha1File(dest);
        fresh = !pin.sha1 || installedSha1 === pin.sha1;
      }
    } catch {
      fresh = false; // unreadable -> reinstall below
    }
    if (!fresh) await installBrandingPreset(instance, modsDir);
    for (const f of stale) {
      await unlink(path.join(modsDir, f)).catch(() => {});
      console.log(`[mods] pruned stale branding jar ${f} from ${instance.name}`);
    }
  } catch (e) {
    console.warn(`[mods] ensureBrandingSeeded failed for ${instance?.name ?? 'unknown'}: ${e?.message ?? e}`);
  }
}

// ---------------------------------------------------------------------------
// Installed-mod detection (pure, no network)
// ---------------------------------------------------------------------------

/**
 * True when a filename plausibly belongs to the Modrinth project with the
 * given slug: the versioned basename equals the slug, starts with slug- /
 * slug_, or contains the slug as a whole token. Case-insensitive; a trailing
 * '.jar' / '.jar.disabled' is stripped first. No substring matching — a short
 * slug like 'a' only matches a full token.
 */
export function isFilenameForSlug(filename, slug) {
  if (typeof filename !== 'string' || typeof slug !== 'string') return false;
  let base = filename.toLowerCase();
  if (base.endsWith('.jar.disabled')) base = base.slice(0, -'.jar.disabled'.length);
  else if (base.endsWith('.jar')) base = base.slice(0, -'.jar'.length);
  const target = slug.toLowerCase();
  if (base === target) return true;
  if (base.startsWith(target + '-') || base.startsWith(target + '_')) return true;
  return base.split(/[._+\-]/).includes(target);
}

/** 'Ferrite Core' -> 'ferrite-core' (Modrinth titles often differ from slugs). */
export function slugifyTitle(title) {
  return String(title ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

/**
 * *.jar / *.jar.disabled basenames in an instance's mods dir ('.disabled'
 * stripped). Never touches the network; a missing mods dir yields [].
 */
export async function installedModFilenames(instanceName) {
  assertValidInstanceName(instanceName);
  let entries;
  try {
    entries = await readdir(modsDirOf(instanceName));
  } catch {
    return [];
  }
  const filenames = [];
  for (const entry of entries) {
    if (entry.endsWith('.jar')) filenames.push(entry);
    else if (entry.endsWith('.jar.disabled')) filenames.push(entry.slice(0, -'.disabled'.length));
  }
  return filenames;
}

// ---------------------------------------------------------------------------
// Modrinth search: browse mods by query for a target game version
// ---------------------------------------------------------------------------
export async function searchModrinth(query, gameVersion, loader = 'fabric', instanceName = null, opts = {}) {
  const offset = Math.max(0, Number(opts?.offset) || 0);
  const facets = [[ 'project_type:mod' ]];
  if (loader) facets.push([ 'loaders:' + loader ]);
  if (gameVersion) facets.push([ `versions:${gameVersion}` ]);
  const params = new URLSearchParams({
    query: query ?? '',
    facets: JSON.stringify(facets),
    limit: '12',
    offset: String(offset),
    index: 'downloads',
  });
  let data;
  const url = `${MODRINTH_API}/search?${params}`;
  try {
    data = await fetchJsonWithRetry(url);
  } catch (err) {
    const e = new Error(`Modrinth search failed: ${err?.message ?? err}`);
    e.status = err?.status ?? 502;
    e.code = 'MODRINTH_SEARCH_FAILED';
    e.cause = err;
    throw e;
  }
  const results = (data?.hits ?? []).map((hit) => ({
    project_id: hit.project_id,
    slug: hit.slug,
    title: hit.title,
    description: hit.description,
    icon_url: hit.icon_url,
    downloads: hit.downloads,
    game_versions: hit.versions,
    latest_version_number: hit.latest_version,
  }));
  if (instanceName) {
    const installed = await installedModFilenames(instanceName);
    for (const hit of results) {
      hit.installed = installed.some(
        (f) => isFilenameForSlug(f, hit.slug) || isFilenameForSlug(f, slugifyTitle(hit.title)),
      );
    }
  }
  return {
    results,
    total: typeof data?.total_hits === 'number' ? data.total_hits : results.length,
    offset,
    limit: 12,
  };
}

// ---------------------------------------------------------------------------
// Install a single Modrinth mod (background; sha1-verified, mod-progress)
// ---------------------------------------------------------------------------
export async function installModrinthMod(instanceName, projectId, loader = null) {
  assertValidInstanceName(instanceName);
  const instance = await loadInstanceMeta(instanceName);
  loader = loader ?? (instance.loader === 'neoforge' ? 'neoforge' : 'fabric');
  const meta = await bestVersionForCached(projectId, instance.version, loader);
  if (!meta) {
    const err = new Error(
      `no ${loader} build of project ${projectId} for ${instance.version}`
    );
    err.status = 404;
    err.code = 'MOD_NOT_FOUND_FOR_VERSION';
    throw err;
  }
  const file = primaryFile(meta);
  if (!file) {
    const err = new Error(`project ${projectId} has no downloadable file`);
    err.status = 404;
    err.code = 'MOD_NOT_FOUND_FOR_VERSION';
    throw err;
  }
  const modsDir = modsDirOf(instance);
  await mkdir(modsDir, { recursive: true });

  emitEvent('mod-progress', {
    instance: instanceName,
    phase: 'start',
    filename: file.filename,
    index: 0,
    total: 1,
    message: `installing ${file.filename}`,
  });

  await downloadAll(
    [
      {
        url: file.url,
        dest: path.join(modsDir, file.filename),
        sha1: file.hashes?.sha1 ?? null,
        size: file.size ?? null,
        eventName: 'mod-progress',
        context: {
          instance: instanceName,
          filename: file.filename,
          index: 0,
          total: 1,
        },
      },
    ],
    { concurrency: downloadConcurrency() }
  );

  emitEvent('mod-progress', {
    instance: instanceName,
    phase: 'done',
    filename: file.filename,
    index: 1,
    total: 1,
    message: `installed ${file.filename}`,
  });

  return { installed: file.filename };
}
