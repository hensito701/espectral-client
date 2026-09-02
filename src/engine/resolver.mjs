/**
 * Mojang manifests, version JSON, libraries/natives/assets + Fabric profile.
 * On-disk layout is documented in README.md.
 */
import { closeSync, existsSync, openSync, readSync } from 'node:fs';
import {
  copyFile,
  link,
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  unlink,
  writeFile,
} from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import extract from 'extract-zip';
import {
  dataDir,
  downloadAll,
  downloadConcurrency,
  downloadFile,
  emitEvent,
  fetchJson,
  fetchText,
  sha1File,
  sha1Hex,
} from './download.mjs';
export { downloadConcurrency } from './download.mjs';


// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
export const MANIFEST_URL = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json';
export const FABRIC_META_BASE = 'https://meta.fabricmc.net/v2';
export const FABRIC_MAVEN = 'https://maven.fabricmc.net/';
export const ASSET_BASE = 'https://resources.download.minecraft.net';
export const MAVEN_CENTRAL = 'https://repo1.maven.org/maven2';
export const MANIFEST_TTL_MS = 6 * 60 * 60 * 1000; // 6h

/** Java majors the launcher provisions (Temurin LTS; 16 for 1.17.x). */
export const SUPPORTED_MAJORS = new Set([8, 16, 17, 21, 25]);

const INSTANCE_NAME_RE = /^[A-Za-z0-9 ._-]{1,40}$/;

// Memo for manifest id->entry Map keyed by file mtime+size (C16)
let _manifestEntryMemo = { key: null, map: null };

// ---------------------------------------------------------------------------
// Platform identity (shared with launch.mjs)
// ---------------------------------------------------------------------------
export function osName() {
  if (process.platform === 'win32') return 'windows';
  if (process.platform === 'darwin') return 'osx';
  return 'linux';
}

export function osArch() {
  return process.arch;
}

/**
 * Mojang rule evaluation: no rules -> allowed; otherwise the LAST matching rule
 * wins. A rule matches when its os.name / os.arch constraints (when present)
 * match the current platform and its feature requirements are a subset of the
 * enabled `features` map (default {}).
 */
export function rulesAllow(lib, { os = osName(), arch = osArch(), features = {} } = {}) {
  if (!Array.isArray(lib.rules) || lib.rules.length === 0) return true;
  let allowed = false;
  for (const rule of lib.rules) {
    let osOk = true;
    if (rule.os) {
      if (rule.os.name && rule.os.name !== os) osOk = false;
      else if (rule.os.arch && rule.os.arch !== arch) osOk = false;
    }
    let featOk = true;
    if (rule.features) {
      for (const [k, v] of Object.entries(rule.features)) {
        if (!!features[k] !== !!v) {
          featOk = false;
          break;
        }
      }
    }
    if (osOk && featOk) allowed = rule.action === 'allow';
  }
  return allowed;
}

/**
 * Maven coordinates -> relative library path.
 *   org.lwjgl:lwjgl:3.3.3            -> org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar
 *   org.lwjgl:lwjgl:3.3.3:natives-windows -> org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar
 */
export function mavenPath(name) {
  const parts = String(name).split(':');
  const [group, artifact, version] = parts;
  const classifier = parts.length > 3 ? parts[3] : null;
  const file = `${artifact}-${version}${classifier ? `-${classifier}` : ''}.jar`;
  return `${group.replace(/\./g, '/')}/${artifact}/${version}/${file}`;
}

/** Classifier when the 4th coordinate segment starts with 'natives', else null. */
export function nativesClassifier(name) {
  const parts = String(name).split(':');
  if (parts.length <= 3) return null;
  const classifier = parts[3];
  return classifier && classifier.startsWith('natives') ? classifier : null;
}

/**
 * Resolve the concrete download info for a library entry (Mojang or Fabric
 * shape): downloads.artifact.url when present, else a Maven-base url + path,
 * else Maven Central.
 */
export function libraryDownload(lib) {
  const name = lib.name;
  const relPath = mavenPath(name);
  if (lib.downloads?.artifact) {
    const a = lib.downloads.artifact;
    return { name, relPath, url: a.url, sha1: a.sha1 ?? null, size: a.size ?? null };
  }
  if (lib.url) {
    const base = lib.url.endsWith('/') ? lib.url : `${lib.url}/`;
    return { name, relPath, url: base + relPath, sha1: lib.sha1 ?? null, size: lib.size ?? null };
  }
  return { name, relPath, url: `${MAVEN_CENTRAL}/${relPath}`, sha1: lib.sha1 ?? null, size: lib.size ?? null };
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------
export function versionsDir() {
  return path.join(dataDir(), 'versions');
}
export function librariesDir() {
  return path.join(dataDir(), 'libraries');
}
export function assetsDir() {
  return path.join(dataDir(), 'assets');
}
export function instancesDir() {
  return path.join(dataDir(), 'instances');
}
export function instanceDir(name) {
  return path.join(instancesDir(), name);
}
export function instanceNativesDir(name) {
  return path.join(instanceDir(name), 'natives');
}
export function instanceModsDir(name) {
  return path.join(instanceDir(name), 'mods');
}
export function clientJarPath(version) {
  return path.join(versionsDir(), version, `${version}.jar`);
}

/** %APPDATA%/.fastclient (FastClient game dir) — source of the seed fast-path. */
export function fastClientRoot() {
  const appdata = process.env.APPDATA;
  if (appdata) return path.join(appdata, '.fastclient');
  const home = process.env.USERPROFILE ?? process.env.HOME;
  return home ? path.join(home, '.fastclient') : null;
}

export function assertValidInstanceName(name) {
  if (typeof name !== 'string' || !INSTANCE_NAME_RE.test(name)) {
    const err = new Error(`invalid instance name: ${name}`);
    err.status = 409;
    err.code = 'invalid_instance_name';
    throw err;
  }
}

export async function loadInstanceMeta(name) {
  assertValidInstanceName(name);
  try {
    return JSON.parse(await readFile(path.join(instanceDir(name), 'instance.json'), 'utf8'));
  } catch {
    const err = new Error(`instance not found: ${name}`);
    err.status = 404;
    err.code = 'instance_not_found';
    throw err;
  }
}

// ---------------------------------------------------------------------------
// Version manifest (cached 6h in data/versions/manifest.json)
// ---------------------------------------------------------------------------
export async function getVersionManifest({ force = false } = {}) {
  const file = path.join(versionsDir(), 'manifest.json');
  if (!force) {
    try {
      const cached = JSON.parse(await readFile(file, 'utf8'));
      if (
        cached &&
        cached.fetched_at &&
        Array.isArray(cached.versions) &&
        Date.now() - cached.fetched_at < MANIFEST_TTL_MS
      ) {
        return cached;
      }
      if (cached && Array.isArray(cached.versions)) {
        // Stale-while-revalidate: TTL lapsed but stale parses -> return stale + background refresh
        (async () => {
          try {
            const raw = await fetchJson(MANIFEST_URL);
            const manifest = {
              fetched_at: Date.now(),
              latest_release: raw.latest?.release ?? '',
              latest_snapshot: raw.latest?.snapshot ?? '',
              versions: (raw.versions ?? []).map((v) => ({
                id: v.id,
                type: v.type ?? 'release',
                release_time: v.releaseTime ?? v.time ?? '',
                sha1: v.sha1 ?? null,
                url: v.url ?? null,
              })),
            };
            await mkdir(versionsDir(), { recursive: true });
            await writeFile(file, JSON.stringify(manifest, null, 2));
          } catch {}
        })();
        return cached;
      }
    } catch {
      /* stale/missing/corrupt -> refresh below */
    }
  }
  try {
    const raw = await fetchJson(MANIFEST_URL);
    const manifest = {
      fetched_at: Date.now(),
      latest_release: raw.latest?.release ?? '',
      latest_snapshot: raw.latest?.snapshot ?? '',
      versions: (raw.versions ?? []).map((v) => ({
        id: v.id,
        type: v.type ?? 'release',
        release_time: v.releaseTime ?? v.time ?? '',
        sha1: v.sha1 ?? null,
        url: v.url ?? null,
      })),
    };
    await mkdir(versionsDir(), { recursive: true });
    await writeFile(file, JSON.stringify(manifest, null, 2));
    return manifest;
  } catch (err) {
    // Offline resilience: fall back to whatever is cached, however stale.
    try {
      const stale = JSON.parse(await readFile(file, 'utf8'));
      if (stale && Array.isArray(stale.versions)) return stale;
    } catch {
      /* no cache at all */
    }
    throw err;
  }
}

// ---------------------------------------------------------------------------
// Version JSON (cached + sha1-reverified against the manifest entry)
// ---------------------------------------------------------------------------
export async function getVersionJson(id, entry = null) {
  if (!entry) {
    const manifestFile = path.join(versionsDir(), 'manifest.json');
    let memoHit = false;
    try {
      const s = await stat(manifestFile);
      const key = `${s.mtimeMs}:${s.size}`;
      if (_manifestEntryMemo.key === key && _manifestEntryMemo.map) {
        entry = _manifestEntryMemo.map.get(id) ?? null;
        memoHit = true;
      }
    } catch {}
    if (!memoHit) {
      const manifest = await getVersionManifest();
      const map = new Map();
      for (const v of manifest.versions ?? []) if (v && v.id) map.set(v.id, v);
      try {
        const s2 = await stat(manifestFile);
        const key2 = `${s2.mtimeMs}:${s2.size}`;
        _manifestEntryMemo = { key: key2, map };
      } catch {
        try {
          const s2b = await stat(manifestFile);
          _manifestEntryMemo = { key: `${s2b.mtimeMs}:${s2b.size}`, map };
        } catch {
          _manifestEntryMemo = { key: `${Date.now()}:${map.size}`, map };
        }
      }
      entry = map.get(id) ?? null;
    }
  }
  if (!entry || !entry.url) {
    const err = new Error(`version not found in manifest: ${id}`);
    err.status = 404;
    err.code = 'version_not_found';
    throw err;
  }
  const file = path.join(versionsDir(), `${id}.json`);
  try {
    const buf = await readFile(file);
    if (entry.sha1) {
      if (sha1Hex(buf) === entry.sha1) return JSON.parse(buf.toString('utf8'));
    } else {
      return JSON.parse(buf.toString('utf8'));
    }
  } catch {
    /* re-download */
  }
  const text = await fetchText(entry.url);
  if (entry.sha1 && sha1Hex(Buffer.from(text, 'utf8')) !== entry.sha1) {
    const err = new Error(`sha1 mismatch for version ${id} (${entry.url})`);
    err.status = 502;
    err.code = 'sha1_mismatch';
    throw err;
  }
  await mkdir(versionsDir(), { recursive: true });
  await writeFile(file, text);
  return JSON.parse(text);
}
export function __clearManifestMemo() {
  _manifestEntryMemo = { key: null, map: null };
}

/**
 * Resolve a version into VersionInfo. `supported` = the launcher has a
 * provisioning path for the version's required Java tier (JDK 8/16/17/21/25
 * runtimes auto-download from Adoptium) AND a compatible Fabric loader exists
 * when the instance asks for Fabric. Everything in the 1.15.2 – 26.2 range
 * resolves; unknown/very-new versions are flagged unsupported until the map
 * covers them.
 */
export async function resolveVersion(id) {
  const manifest = await getVersionManifest();
  const entry = manifest.versions.find((v) => v.id === id);
  if (!entry) {
    const err = new Error(`version not found in manifest: ${id}`);
    err.status = 404;
    err.code = 'version_not_found';
    throw err;
  }
  const json = await getVersionJson(id, entry);
  const javaMajor = Number(json.javaVersion?.majorVersion ?? 8);
  const supported = SUPPORTED_MAJORS.has(javaMajor);
  const reason = supported
    ? ''
    : `requires JDK ${javaMajor} — the launcher provisions Temurin ${[...SUPPORTED_MAJORS].join('/')}`;
  return { id, java_major: javaMajor, supported, reason };
}

// ---------------------------------------------------------------------------
// Local version JSON + inheritance chain merge (NeoForge profiles inherit
// the vanilla json — <dataDir>/versions/<id>/<id>.json, no network).
// ---------------------------------------------------------------------------
/**
 * Read a locally-persisted version JSON from <dataDir>/versions/<id>/<id>.json
 * (the layout neoforge.mjs writes), or null when absent/corrupt.
 */
export async function readLocalVersionJson(id) {
  try {
    return JSON.parse(await readFile(path.join(versionsDir(), id, `${id}.json`), 'utf8'));
  } catch {
    return null;
  }
}

/**
 * Dedupe library entries by `name`; EARLIER lists win, so callers pass the
 * child first (child's copy of a same-named library is kept).
 */
export function dedupeByName(...lists) {
  const seen = new Map();
  for (const list of lists) {
    for (const lib of list ?? []) {
      if (lib && typeof lib.name === 'string' && !seen.has(lib.name)) seen.set(lib.name, lib);
    }
  }
  return [...seen.values()];
}

/**
 * Merge two Mojang argument templates (child first): string entries deduped
 * by value, rule-object entries kept as-is and deduped by JSON.stringify.
 * JVM flag strings ('-…') are NEVER deduped: the NeoForge profile repeats
 * flags like `--add-opens` once per module export, and dropping the second
 * one orphans its value (java then treats the value as the main class).
 */
export function mergeArgs(child, parent) {
  const out = [];
  const strings = new Set();
  const objects = new Set();
  for (const list of [child, parent]) {
    for (const entry of list ?? []) {
      if (typeof entry === 'string') {
        const isFlag = entry.startsWith('-');
        if (isFlag || !strings.has(entry)) {
          strings.add(entry);
          out.push(entry);
        }
      } else {
        const key = JSON.stringify(entry);
        if (!objects.has(key)) {
          objects.add(key);
          out.push(entry);
        }
      }
    }
  }
  return out;
}

/**
 * Resolve a version JSON's inheritance chain: while `inheritsFrom` is set,
 * fetch the parent and merge child-over-parent (child's id/mainClass/asset
 * index/javaVersion/downloads win; libraries deduped by name with the child
 * first; game/jvm argument templates merged child-first). Multi-level chains
 * resolve iteratively with a cycle guard. `getParent` defaults to the Mojang
 * version JSON cache; tests inject a stub.
 */
export async function resolveVersionChain(versionInfo, { getParent = null } = {}) {
  const parentGetter = getParent ?? (async (id) => await getVersionJson(id));
  let merged = versionInfo;
  const seen = new Set();
  while (merged.inheritsFrom) {
    const parentId = merged.inheritsFrom;
    if (seen.has(parentId)) {
      const err = new Error(`inheritance cycle at ${parentId}`);
      err.status = 502;
      err.code = 'INHERITS_BREAK';
      throw err;
    }
    seen.add(parentId);
    const parent = await parentGetter(parentId);
    if (!parent) {
      const err = new Error(`parent version ${parentId} not found for ${merged.id}`);
      err.status = 502;
      err.code = 'INHERITS_BREAK';
      throw err;
    }
    merged = {
      ...parent,
      id: merged.id,
      mainClass: merged.mainClass ?? parent.mainClass,
      assetIndex: merged.assetIndex ?? parent.assetIndex,
      javaVersion: merged.javaVersion ?? parent.javaVersion,
      downloads: merged.downloads ?? parent.downloads,
      libraries: dedupeByName(merged.libraries ?? [], parent.libraries ?? []),
      arguments: {
        game: mergeArgs(merged.arguments?.game, parent.arguments?.game),
        jvm: mergeArgs(merged.arguments?.jvm, parent.arguments?.jvm),
      },
    };
  }
  return merged;
}

// ---------------------------------------------------------------------------
// FastClient seed fast-path: identical files under %APPDATA%/.fastclient are
// hardlinked (or copied when the link crosses volumes) instead of downloaded.
// ---------------------------------------------------------------------------
async function seedFromFastClient(relPath, sha1, dest) {
  const root = fastClientRoot();
  if (!root) return false;
  const src = path.join(root, relPath);
  if (!existsSync(src)) return false;
  try {
    if (sha1 && (await sha1File(src)) !== sha1) return false;
  } catch {
    return false;
  }
  try {
    await mkdir(path.dirname(dest), { recursive: true });
    if (existsSync(dest)) {
      try {
        if (!sha1 || (await sha1File(dest)) === sha1) return true;
      } catch {
        /* fall through to replace */
      }
      await unlink(dest).catch(() => {});
    }
    try {
      await link(src, dest); // hardlink — same volume
    } catch {
      await copyFile(src, dest); // EXDEV / permission — copy
    }
    return true;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// Library installation (classpath jars + natives extraction + client jar)
// ---------------------------------------------------------------------------
/**
 * Download every library the version JSON's rules allow. Natives-classifier
 * jars (org.lwjgl:lwjgl:3.3.3:natives-windows) are downloaded but NOT part of
 * the classpath; they are extracted into <instance>/natives for
 * -Djava.library.path. extract.exclude (legacy schema) is honored. Extraction
 * order is deterministic: arch-suffixed natives first, the plain (default-arch)
 * natives last, so on x64 the x64 dlls win on filename collisions.
 *
 * `versionInfo` = parsed Mojang version JSON (from getVersionJson()).
 */
export async function installLibraries(instance, versionInfo, { seedFromFastClient: seed = true, forceExtract = false } = {}) {
  assertValidInstanceName(instance.name);
  const version = instance.version;
  const libs = Array.isArray(versionInfo.libraries) ? versionInfo.libraries : [];

  const classpathJars = []; // absolute paths, natives excluded
  const natives = []; // { dest, classifier, extractExclude }
  const jobs = []; // downloadAll items
  let seeded = 0;

  for (const lib of libs) {
    if (!rulesAllow(lib)) continue;
    const classifier = nativesClassifier(lib.name);
    const info = libraryDownload(lib);
    const dest = path.join(librariesDir(), info.relPath);
    const item = {
      url: info.url,
      dest,
      sha1: info.sha1,
      size: info.size,
      eventName: 'mod-progress',
      context: {
        instance: instance.name,
        filename: path.basename(dest),
        kind: classifier ? 'natives' : 'library',
      },
    };
    if (classifier) {
      natives.push({ dest, classifier, extractExclude: lib.extract?.exclude ?? [] });
    } else {
      classpathJars.push(dest);
      // Legacy schema (<= 1.16.x era): natives are declared on the base lib as
      // `natives: { windows: "natives-windows", ... }` with the actual archive
      // under `downloads.classifiers[<os-classifier>]` — not as a
      // `:natives-<os>` name coordinate. Download + extract that classifier.
      const osClass = lib.natives?.[osName()];
      const cls = osClass ? lib.downloads?.classifiers?.[osClass] : null;
      if (cls) {
        const ndest = path.join(librariesDir(), cls.path);
        natives.push({ dest: ndest, classifier: osClass, extractExclude: lib.extract?.exclude ?? [] });
        const nitem = {
          url: cls.url,
          dest: ndest,
          sha1: cls.sha1 ?? null,
          size: cls.size ?? null,
          eventName: 'mod-progress',
          context: {
            instance: instance.name,
            filename: path.basename(ndest),
            kind: 'natives',
          },
        };
        if (seed && (await seedFromFastClient(`libraries/${cls.path}`, cls.sha1 ?? null, ndest))) {
          seeded++;
        } else {
          jobs.push(nitem);
        }
      }
    }
    if (seed && (await seedFromFastClient(`libraries/${info.relPath}`, info.sha1, dest))) {
      seeded++;
    } else {
      jobs.push(item);
    }
  }

  // Client jar (downloads.client) -> data/versions/<id>/<id>.jar.
  let clientJar = null;
  const client = versionInfo.downloads?.client;
  if (client?.url) {
    clientJar = clientJarPath(version);
    const item = {
      url: client.url,
      dest: clientJar,
      sha1: client.sha1 ?? null,
      size: client.size ?? null,
      eventName: 'mod-progress',
      context: { instance: instance.name, filename: path.basename(clientJar), kind: 'client' },
    };
    if (seed && (await seedFromFastClient(`versions/${version}/${version}.jar`, client.sha1 ?? null, clientJar))) {
      seeded++;
    } else {
      jobs.push(item);
    }
  }

  if (jobs.length > 0) {
    await downloadAll(jobs, { concurrency: downloadConcurrency() });
  }

  const nativesDir = instanceNativesDir(instance.name);
  await mkdir(nativesDir, { recursive: true });
  if (forceExtract) {
    try {
      const ents = await readdir(nativesDir);
      await Promise.all(
        ents.filter((n) => n.startsWith('.extract-')).map((n) => rm(path.join(nativesDir, n), { force: true }))
      );
    } catch {}
  }
  natives.sort((a, b) => {
    const aPlain = /^natives-[a-z0-9]+$/.test(a.classifier);
    const bPlain = /^natives-[a-z0-9]+$/.test(b.classifier);
    if (aPlain !== bPlain) return aPlain ? 1 : -1; // plain (default arch) last
    return a.classifier.localeCompare(b.classifier);
  });
  for (const n of natives) {
    await extractNatives(n.dest, nativesDir, n.extractExclude);
  }

  return {
    library_count: classpathJars.length,
    seeded,
    downloaded: jobs.length,
    natives_extracted: natives.length,
    client_jar: clientJar,
    classpath: classpathJars,
  };
}

let nativesStageSeq = 0;

async function extractNatives(jarPath, nativesDir, extractExclude = []) {
  const idHash = sha1Hex(jarPath);
  const markerPath = path.join(nativesDir, `.extract-ok-${idHash}`);
  const metaPath = path.join(nativesDir, `.extract-meta-${idHash}`);
  let jarStat = null;
  try {
    const s = await stat(jarPath);
    jarStat = { size: s.size, mtimeMs: s.mtimeMs };
  } catch {}
  if (jarStat) {
    try {
      if (existsSync(markerPath)) {
        const metaRaw = await readFile(metaPath, 'utf8');
        const meta = JSON.parse(metaRaw);
        if (meta && meta.size === jarStat.size && meta.mtimeMs === jarStat.mtimeMs) {
          return;
        }
      }
    } catch {}
  }
  const exclude = (extractExclude ?? []).map((e) => e.replace(/\\/g, '/').replace(/^\.?\//, ''));
  // Unique stage dir per call: concurrent launches (or a retried verify) must
  // never rm() a stage dir another extraction is mid-write into — that is the
  // one way extract-zip's lazy-entry chain stalls forever (readEntry is only
  // called after the previous entry's write settles, so a dead write stops
  // the whole stream with no pending I/O).
  const stage = path.join(
    nativesDir,
    `.stage-${path.basename(jarPath, '.jar')}-${process.pid}-${++nativesStageSeq}`
  );
  try {
    await rm(stage, { recursive: true, force: true });
    await mkdir(stage, { recursive: true });
    // Bound the extraction: a genuine stall must surface as a launch error
    // (streamed over SSE), never as an infinite "verificando dependencias…".
    // Retry with a fresh stage dir before giving up.
    let lastErr = null;
    for (let attempt = 1; attempt <= 3; attempt++) {
      await rm(stage, { recursive: true, force: true }).catch(() => {});
      await mkdir(stage, { recursive: true });
      try {
        await withTimeout(
          extract(jarPath, { dir: stage }),
          60_000,
          `natives extraction timed out: ${path.basename(jarPath)}`
        );
        lastErr = null;
        break;
      } catch (err) {
        lastErr = err;
      }
    }
    if (lastErr) throw lastErr;

    const moveStaged = async (dir) => {
      for (const dirent of await readdir(dir, { withFileTypes: true })) {
        const abs = path.join(dir, dirent.name);
        const rel = path.relative(stage, abs).replace(/\\/g, '/');
        const relSlash = `${rel}/`;
        const excluded = exclude.some(
          (prefix) => rel.startsWith(prefix) || (prefix.endsWith('/') && relSlash.startsWith(prefix))
        );
        if (excluded) continue;
        const dst = path.join(nativesDir, rel);
        if (dirent.isDirectory()) {
          await mkdir(dst, { recursive: true });
          await moveStaged(abs);
        } else {
          await mkdir(path.dirname(dst), { recursive: true });
          // Another concurrent verify may be moving the same entry; rm+rename
          // can collide transiently on Windows — retry briefly instead of
          // failing the launch over a race.
          await retryOnEbusy(async () => {
            await rm(dst, { force: true });
            await rename(abs, dst);
          }, 5);
        }
      }
    };
    await moveStaged(stage);
  } catch (err) {
    // Legacy natives entries are sometimes raw native libs, not archives;
    // dropping the file into the natives dir keeps -Djava.library.path able to
    // find it. Genuine zip failures (corrupt archive) still surface.
    if (!isZipFile(jarPath)) {
      await copyFile(jarPath, path.join(nativesDir, path.basename(jarPath)));
      if (jarStat) {
        try {
          await writeFile(markerPath, '');
          await writeFile(metaPath, JSON.stringify(jarStat));
        } catch {}
      }
      return;
    } else {
      throw err;
    }
  } finally {
    await rm(stage, { recursive: true, force: true }).catch(() => {});
  }
  if (jarStat) {
    try {
      await writeFile(markerPath, '');
      await writeFile(metaPath, JSON.stringify(jarStat));
    } catch {}
  }
}

/** Reject after timeoutMs; the original promise's late settle is swallowed. */
function withTimeout(promise, timeoutMs, message) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), timeoutMs);
    promise.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      }
    );
  });
}

/** Retry a rename/rm pair a few times on transient Windows sharing errors. */
async function retryOnEbusy(fn, attempts) {
  let lastErr;
  for (let i = 0; i < attempts; i++) {
    try {
      return await fn();
    } catch (err) {
      lastErr = err;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw lastErr;
}

/** Cheap zip sniff: local-file/empty-zip signatures start with 'PK'. */
function isZipFile(file) {
  let fd;
  try {
    fd = openSync(file, 'r');
    const b = Buffer.alloc(4);
    readSync(fd, b, 0, 4, 0);
    return b[0] === 0x50 && b[1] === 0x4b;
  } catch {
    return false;
  } finally {
    if (fd !== undefined) closeSync(fd);
  }
}

// ---------------------------------------------------------------------------
// Assets (index + full object set, concurrency 6, .part, progress events)
// ---------------------------------------------------------------------------
export async function installAssets(instance, versionInfo) {
  const index = versionInfo.assetIndex;
  if (!index?.url) return { index: null, objects: 0 };
  const indexPath = path.join(assetsDir(), 'indexes', `${index.id}.json`);
  await downloadFile(index.url, indexPath, {
    sha1: index.sha1 ?? null,
    size: index.size ?? null,
    eventName: 'asset-progress',
    context: { instance: instance.name, filename: path.basename(indexPath), kind: 'index' },
  });
  const parsed = JSON.parse(await readFile(indexPath, 'utf8'));
  const objects = Object.entries(parsed.objects ?? {});
  // Asset indexes list the same object under multiple paths; each hash maps to
  // ONE dest file. Without dedupe, concurrent workers race the same .part and
  // the rename can fail on Windows — so download each hash once.
  const seen = new Set();
  const items = [];
  for (const [rel, o] of objects) {
    if (seen.has(o.hash)) continue;
    seen.add(o.hash);
    items.push({
      url: `${ASSET_BASE}/${o.hash.slice(0, 2)}/${o.hash}`,
      dest: path.join(assetsDir(), 'objects', o.hash.slice(0, 2), o.hash),
      sha1: o.hash,
      size: o.size ?? null,
      eventName: 'asset-progress',
      context: { instance: instance.name, asset: rel, kind: 'object' },
    });
  }
  await downloadAll(items, { concurrency: downloadConcurrency() });
  emitEvent('asset-progress', {
    instance: instance.name,
    phase: 'done',
    kind: 'index',
    index: index.id,
    total: objects.length,
    message: `asset index ${index.id}: ${objects.length} objects`,
  });
  return { index: index.id, objects: objects.length };
}

// ---------------------------------------------------------------------------
// Fabric loader profile
// ---------------------------------------------------------------------------
export async function getFabricLoaders({ force = false } = {}) {
  const file = path.join(versionsDir(), 'fabric-loaders.json');
  if (!force) {
    try {
      const cached = JSON.parse(await readFile(file, 'utf8'));
      if (
        cached &&
        cached.fetched_at &&
        Array.isArray(cached.loaders) &&
        Date.now() - cached.fetched_at < MANIFEST_TTL_MS
      ) {
        return cached;
      }
      if (cached && Array.isArray(cached.loaders) && cached.loaders.length > 0) {
        // Stale-while-revalidate
        (async () => {
          try {
            const loaders = await fetchJson(`${FABRIC_META_BASE}/versions/loader`);
            const stable =
              loaders.filter((l) => l.stable).sort((a, b) => (b.build ?? 0) - (a.build ?? 0))[0] ??
              loaders[0] ??
              null;
            const out = { fetched_at: Date.now(), loaders, latest_stable: stable?.version ?? null };
            await mkdir(versionsDir(), { recursive: true });
            await writeFile(file, JSON.stringify(out, null, 2));
          } catch {}
        })();
        return cached;
      }
    } catch {
      /* refresh */
    }
  }
  let loaders;
  try {
    loaders = await fetchJson(`${FABRIC_META_BASE}/versions/loader`);
  } catch (err) {
    // Stale-cache fallback (mirrors getVersionManifest): a meta.fabricmc.net
    // outage must not block Fabric launches once the 6h TTL lapses — the
    // expired list still resolves every loader that existed when it was
    // fetched. Only rethrow when there is no usable cache at all.
    try {
      const stale = JSON.parse(await readFile(file, 'utf8'));
      if (stale && Array.isArray(stale.loaders) && stale.loaders.length > 0) return stale;
    } catch {
      /* no cache at all */
    }
    err.code = err.code ?? 'fabric_loader_unreachable';
    throw err;
  }
  const stable =
    loaders.filter((l) => l.stable).sort((a, b) => (b.build ?? 0) - (a.build ?? 0))[0] ??
    loaders[0] ??
    null;
  const out = { fetched_at: Date.now(), loaders, latest_stable: stable?.version ?? null };
  await mkdir(versionsDir(), { recursive: true });
  await writeFile(file, JSON.stringify(out, null, 2));
  return out;
}

/**
 * Does Fabric have a loader for this game version? Probes the per-version
 * endpoint (meta.fabricmc.net/v2/versions/loader/{mc}). Used to fail a
 * Fabric-instance CREATE with a clear message instead of letting the user
 * discover "fabric loader not found" at first launch. Non-blocking and
 * best-effort: a network failure returns true (assume the best; the real
 * resolveFabric at launch is still authoritative).
 */
export async function fabricSupportedForVersion(mcVersion) {
  try {
    const data = await fetchJson(`${FABRIC_META_BASE}/versions/loader/${encodeURIComponent(mcVersion)}`);
    return Array.isArray(data) && data.length > 0;
  } catch (err) {
    // Fabric meta returns HTTP 400 for versions with no loader — that's a
    // definitive "no fabric". Only a transport/network failure (or 5xx) means
    // "couldn't tell", and for those we optimistically allow creation (the real
    // resolveFabric at launch is still authoritative).
    const status = err?.status ?? 0;
    if (status >= 400 && status < 500) return false;
    return true; // offline / transient — don't block creation on a probe
  }
}

const SHA1_RE = /^[0-9a-f]{40}$/;

/**
 * Fetch maven's `<artifact>.sha1` sidecar. Returns the 40-hex digest or null
 * (offline / 404 / garbage body). Never throws: a missing sidecar only means
 * the artifact stays hashless, which is the pre-existing behaviour.
 */
async function mavenSha1(url) {
  try {
    const body = await fetchText(`${url}.sha1`, { timeoutMs: 15_000 });
    const hash = String(body).trim().split(/\s+/)[0]?.toLowerCase() ?? '';
    return SHA1_RE.test(hash) ? hash : null;
  } catch {
    return null;
  }
}

/**
 * Fill in sha1 for library entries Fabric publishes without one.
 *
 * launcherMeta.libraries.common carries sha1+size for the asm/sponge-mixin
 * set, but the intermediary and fabric-loader jars are named by maven
 * coordinate ONLY — so libraryDownload() yields `sha1: null, size: null` for
 * both. A hashless download can never be short-circuited (download.mjs has
 * nothing to compare against), so every verification pass re-fetched those two
 * jars and replaced them with byte-identical content — and a new mtime. JEP 483
 * stamps path+size+timestamp for each classpath entry, so that single touch
 * invalidated the AOT cache for good ("timestamp has changed" -> "Unable to map
 * shared spaces"), which is why a trained instance kept booting the slow path.
 *
 * maven.fabricmc.net publishes a .sha1 sidecar for every artifact, so the
 * hashes are simply fetched. Results are memoized INSIDE the cached combo JSON
 * (namespaced key) — one request per loader combo ever, not one per launch, and
 * offline launches keep working off the cache.
 */
async function hydrateMavenSha1(libraries, combo, cacheFile) {
  const missing = libraries.filter((lib) => !lib.sha1);
  if (missing.length === 0) return;
  const memo = (combo.espectral_maven_sha1 ??= {});
  const unknown = missing.filter((lib) => !SHA1_RE.test(memo[lib.relPath] ?? ''));
  if (unknown.length > 0) {
    const fetched = await Promise.all(unknown.map((lib) => mavenSha1(lib.url)));
    let learned = false;
    for (let i = 0; i < unknown.length; i++) {
      if (fetched[i]) {
        memo[unknown[i].relPath] = fetched[i];
        learned = true;
      }
    }
    if (learned) {
      try {
        await mkdir(versionsDir(), { recursive: true });
        await writeFile(cacheFile, JSON.stringify(combo, null, 2));
      } catch {
        /* cache rewrite is an optimization; hashes still apply to this run */
      }
    }
  }
  for (const lib of missing) {
    const hash = memo[lib.relPath];
    if (SHA1_RE.test(hash ?? '')) lib.sha1 = hash;
  }
}

/**
 * Resolve the Fabric loader profile for a game version. Libraries = the
 * launcherMeta.libraries.common set PLUS the intermediary and fabric-loader
 * jars (maven.fabricmc.net) — the full 8-entry loader classpath the launcher
 * uses (common alone is only the asm x5 + sponge-mixin support set).
 */
export async function resolveFabric(mcVersion, loaderVersion = null) {
  const loaders = await getFabricLoaders();
  const chosen = loaderVersion
    ? (loaders.loaders.find((l) => l.version === loaderVersion) ?? null)
    : (loaders.loaders.find((l) => l.version === loaders.latest_stable) ?? null);
  if (!chosen) {
    const err = new Error(`fabric loader not found: ${loaderVersion ?? loaders.latest_stable}`);
    err.status = 404;
    err.code = 'fabric_loader_not_found';
    throw err;
  }
  const cacheFile = path.join(versionsDir(), `fabric-loader-${mcVersion}-${chosen.version}.json`);
  let combo;
  try {
    combo = JSON.parse(await readFile(cacheFile, 'utf8'));
  } catch {
    combo = await fetchJson(
      `${FABRIC_META_BASE}/versions/loader/${encodeURIComponent(mcVersion)}/${encodeURIComponent(chosen.version)}`
    );
    await mkdir(versionsDir(), { recursive: true });
    await writeFile(cacheFile, JSON.stringify(combo, null, 2));
  }
  const launcherMeta = combo.launcherMeta ?? {};
  const common = Array.isArray(launcherMeta.libraries?.common) ? launcherMeta.libraries.common : [];
  const libraries = common.map((lib) => libraryDownload(lib));
  if (combo.intermediary?.maven) {
    libraries.push(libraryDownload({ name: combo.intermediary.maven, url: FABRIC_MAVEN }));
  }
  if (chosen.maven) {
    libraries.push(libraryDownload({ name: chosen.maven, url: FABRIC_MAVEN }));
  }
  // Fabric names the intermediary/loader jars by maven coordinate only (no
  // sha1) — hydrate from the .sha1 sidecars so download.mjs can short-circuit
  // them and their mtimes stay stable for the AOT cache.
  await hydrateMavenSha1(libraries, combo, cacheFile);
  const mainClass =
    launcherMeta.mainClass?.client ?? 'net.fabricmc.loader.impl.launch.knot.KnotClient';
  const jvmArgs = Array.isArray(combo.arguments?.jvm) && combo.arguments.jvm.length > 0
    ? combo.arguments.jvm
    : ['-DFabricMcEmu= net.minecraft.client.main.Main '];
  return {
    mc_version: mcVersion,
    loader_version: chosen.version,
    loader_maven: chosen.maven ?? null,
    intermediary: combo.intermediary ?? null,
    main_class: mainClass,
    jvm_args: jvmArgs,
    libraries,
    min_java_version: launcherMeta.min_java_version ?? 8,
  };
}

export async function installFabricLibraries(instance, fabricProfile, { seedFromFastClient: seed = true } = {}) {
  assertValidInstanceName(instance.name);
  const jobs = [];
  let seeded = 0;
  for (const lib of fabricProfile.libraries) {
    const dest = path.join(librariesDir(), lib.relPath);
    if (seed && (await seedFromFastClient(`libraries/${lib.relPath}`, lib.sha1, dest))) {
      seeded++;
      continue;
    }
    jobs.push({
      url: lib.url,
      dest,
      sha1: lib.sha1,
      size: lib.size,
      eventName: 'mod-progress',
      context: { instance: instance.name, filename: path.basename(dest), kind: 'fabric-library' },
    });
  }
  if (jobs.length > 0) {
    await downloadAll(jobs, { concurrency: downloadConcurrency() });
  }
  return { library_count: fabricProfile.libraries.length, seeded, downloaded: jobs.length };
}

// ---------------------------------------------------------------------------
// Classpath assembly (loader libs first, then vanilla libs, then client jar)
// ---------------------------------------------------------------------------
export function resolveClasspath(instance, versionInfo, fabricProfile = null) {
  const loader = fabricProfile
    ? fabricProfile.libraries.map((l) => path.join(librariesDir(), l.relPath))
    : [];
  const vanilla = (versionInfo.libraries ?? [])
    .filter((lib) => rulesAllow(lib) && !nativesClassifier(lib.name))
    .map((lib) => path.join(librariesDir(), mavenPath(lib.name)));
  return [...loader, ...vanilla, clientJarPath(instance.version)];
}

// ---------------------------------------------------------------------------
// Full background seed (used by B1 instance creation and the CLI)
// ---------------------------------------------------------------------------
/**
 * Resolve + download everything an instance needs to launch: libraries,
 * natives, client jar, Fabric profile (when loader === 'fabric'), the NeoForge
 * loader + its libraries (when loader === 'neoforge') and assets.
 * `instance` must look like the stored instance.json ({name, version, loader,
 * loader_version}).
 * Does NOT write instance.json (B1 owns that); the CLI wrapper does.
 */
export async function seedInstance(instance, { seedFromFastClient: seed = true } = {}) {
  assertValidInstanceName(instance.name);
  if (!instance.version) throw new Error('instance.version is required');
  const versionJson = await getVersionJson(instance.version);
  const dir = instanceDir(instance.name);
  for (const sub of ['mods', 'natives', 'logs', 'saves', 'config', 'cache']) {
    await mkdir(path.join(dir, sub), { recursive: true });
  }
  const libraries = await installLibraries(instance, versionJson, { seedFromFastClient: seed });
  let fabric = null;
  if (instance.loader === 'fabric') {
    fabric = await resolveFabric(instance.version);
    const counts = await installFabricLibraries(instance, fabric, { seedFromFastClient: seed });
    fabric.library_count = counts.library_count;
    fabric.seeded = counts.seeded;
    fabric.downloaded = counts.downloaded;
    await writeFile(
      path.join(dir, 'fabric-profile.json'),
      JSON.stringify(
        {
          mc_version: fabric.mc_version,
          loader_version: fabric.loader_version,
          main_class: fabric.main_class,
          jvm_args: fabric.jvm_args,
          libraries: fabric.libraries,
        },
        null,
        2
      )
    );
  }
  let neoforge = null;
  if (instance.loader === 'neoforge') {
    if (!instance.loader_version) {
      throw Object.assign(new Error('loader_version es obligatorio para NeoForge'), {
        status: 400,
        code: 'LOADER_VERSION_REQUIRED',
      });
    }
    // Lazy import: neoforge.mjs is owned by a sibling slice and must not be
    // pulled into resolver.mjs's static graph (no cycle either way, but the
    // loader download only matters for neoforge instances).
    const { ensureNeoForgeLoader } = await import('./neoforge.mjs');
    const { json, ...rest } = await ensureNeoForgeLoader(instance.loader_version, { instance });
    neoforge = { loader_version: instance.loader_version, ...rest };
  }
  const assets = await installAssets(instance, versionJson);
  return { version_json: versionJson, libraries, fabric, neoforge, assets };
}

// ---------------------------------------------------------------------------
// CLI
//   node src/engine/resolver.mjs install --version <id> --loader vanilla|fabric|neoforge
//        [--loader-version <v>] [--name <name>] [--memory <mb>]
// Creates data/instances/<name>/instance.json + dirs and resolves libs,
// natives, client jar, fabric/neoforge loader and assets synchronously
// ---------------------------------------------------------------------------
export async function installInstance({
  name,
  version,
  loader = 'vanilla',
  loader_version = null,
  memory_mb = 3072,
  ...extra
} = {}) {
  assertValidInstanceName(name);
  if (!version) throw new Error('version is required');
  if (!['vanilla', 'fabric', 'neoforge'].includes(loader)) throw new Error(`unknown loader: ${loader}`);
  if (loader === 'neoforge' && !loader_version) throw new Error('loader_version is required for neoforge');
  const versionInfo = await resolveVersion(version);
  if (!versionInfo.supported) {
    const err = new Error(versionInfo.reason || `version ${version} is not supported`);
    err.status = 409;
    err.code = 'jdk_unsupported';
    throw err;
  }
  const instance = {
    name,
    version,
    loader,
    loader_version,
    memory_mb: Number.isInteger(memory_mb) && memory_mb > 0 ? memory_mb : 3072,
    jdk_path_override: extra.jdk_path_override ?? null,
    aot_auto_train: extra.aot_auto_train ?? false,
    imported_from: extra.imported_from ?? null,
    created_at: new Date().toISOString(),
  };
  const result = await seedInstance(instance);
  await writeFile(path.join(instanceDir(name), 'instance.json'), JSON.stringify(instance, null, 2));
  return { instance, version_info: versionInfo, ...result };
}

function parseCliArgs(argv) {
  const out = { command: null, flags: {} };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => {
      i += 1;
      if (i >= argv.length) throw new Error(`missing value for ${a}`);
      return argv[i];
    };
    if (a === 'install') out.command = 'install';
    else if (a === '--version') out.flags.version = next();
    else if (a === '--loader') out.flags.loader = next();
    else if (a === '--loader-version') out.flags.loader_version = next();
    else if (a === '--name') out.flags.name = next();
    else if (a === '--memory') out.flags.memory = Number(next());
    else if (a === '--help' || a === '-h') out.command = 'help';
    else throw new Error(`unknown argument: ${a}`);
  }
  return out;
}

async function cli() {
  const args = parseCliArgs(process.argv.slice(2));
  if (!args.command || args.command === 'help') {
    console.log(
      'usage: node src/engine/resolver.mjs install --version <id> --loader vanilla|fabric|neoforge [--loader-version <v>] [--name <name>] [--memory <mb>]'
    );
    process.exitCode = args.command === 'help' ? 0 : 2;
    return;
  }
  const { version, loader = 'vanilla', loader_version = null, name, memory } = args.flags;
  if (!version) {
    console.error('--version is required');
    process.exitCode = 2;
    return;
  }
  const result = await installInstance({
    name: name ?? `${version}-${loader}`.replace(/[^A-Za-z0-9 ._-]/g, '_'),
    version,
    loader,
    loader_version,
    memory_mb: memory ?? 3072,
  });
  console.log(`installed instance "${result.instance.name}"`);
  console.log(
    `  version: ${result.instance.version} (java ${result.version_info.java_major}, supported: ${result.version_info.supported})`
  );
  console.log(
    `  libraries: ${result.libraries.library_count} classpath jars (${result.libraries.seeded} seeded from FastClient, ${result.libraries.downloaded} downloaded), natives extracted: ${result.libraries.natives_extracted}`
  );
  console.log(`  client jar: ${result.libraries.client_jar ?? 'none'}`);
  if (result.fabric) {
    console.log(
      `  fabric: loader ${result.fabric.loader_version}, main class ${result.fabric.main_class}, libs ${result.fabric.library_count}`
    );
  }
  if (result.neoforge) {
    console.log(
      `  neoforge: loader ${result.neoforge.loader_version}, installed ${result.neoforge.installed}, cached ${result.neoforge.cached}`
    );
  }
  console.log(`  assets: index ${result.assets.index ?? 'none'}, objects ${result.assets.objects}`);
  console.log(`  instance dir: ${instanceDir(result.instance.name)}`);
}

const isCli =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isCli) {
  // process.exitCode (not process.exit): process.exit during an in-flight fetch
  // trips a libuv assertion on Windows; letting the loop drain exits cleanly.
  cli().catch((err) => {
    console.error(`error: ${err?.message ?? err}`);
    process.exitCode = 1;
  });
}
