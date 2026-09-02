/**
 * neoforge.mjs — NeoForge loader install without a java subprocess.
 *
 * Ground truth (verified against the real installer): the NeoForge installer
 * jar at
 *   <NEOFORGE_MAVEN>/net/neoforged/neoforge/<v>/neoforge-<v>-installer.jar
 * contains a `version.json` entry that IS the launcher profile the installer
 * would write (byte-for-byte copy in ClientInstall.java). So installing the
 * loader is purely mechanical:
 *
 *   1. download the installer jar (cache dir),
 *   2. extract + parse `version.json` from it,
 *   3. persist it as <dataDir>/versions/neoforge-<v>/neoforge-<v>.json
 *      (the same shape resolver.mjs reads for any version id),
 *   4. delete the installer jar,
 *   5. download the ~47 libraries the profile lists into
 *      <dataDir>/libraries/<artifact.path> (sha1-verified, bounded
 *      concurrency from config).
 *
 * No resolver.mjs import here on purpose: resolver.mjs will lazy-import this
 * module, so importing back would create an ESM cycle. Paths (versions dir,
 * libraries dir) are built locally from config.mjs `dataDir()`.
 *
 * Progress is surfaced as 'mod-progress' events (kind 'neoforge-loader' for
 * the installer download, 'neoforge-library' per library) so the existing UI
 * download bar works unchanged. Errors follow engine conventions:
 * `Object.assign(new Error(msg), { status, code })`.
 */
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { dataDir } from './config.mjs';
import {
  downloadAll,
  downloadConcurrency,
  downloadFile,
  emitEvent,
} from './download.mjs';
import { readZipEntry } from './ziputil.mjs';

/** Root of the NeoForge maven repository. */
export const NEOFORGE_MAVEN = 'https://maven.neoforged.net/releases';

/** URL of the installer jar for a NeoForge version (e.g. '21.1.242'). */
export function installerUrl(version) {
  return `${NEOFORGE_MAVEN}/net/neoforged/neoforge/${version}/neoforge-${version}-installer.jar`;
}

/** Version id used in <dataDir>/versions: 'neoforge-<v>'. */
export function neoforgeVersionId(v) {
  return `neoforge-${v}`;
}

/** Where the extracted launcher profile json lives for a NeoForge version. */
export function neoforgeJsonPath(v) {
  return path.join(dataDir(), 'versions', `neoforge-${v}`, `neoforge-${v}.json`);
}

/** Where the installer jar is staged before extraction. */
function installerJarPath(v) {
  return path.join(dataDir(), 'cache', 'neoforge', `neoforge-${v}-installer.jar`);
}

// ---------------------------------------------------------------------------
// Production launch artifacts
// ---------------------------------------------------------------------------
// FML 4's production launch needs artifacts the official installer GENERATES
// locally (they are not downloadable artifacts): the Mojang-mapped client
// classes (`client-<mc>-<neoform>-srg.jar`), the client resources
// (`...-extra.jar`), the slim jar, and the neoforge client overlay
// (`neoforge-<v>-client.jar`) — plus the neoforge mod jar
// (`neoforge-<v>-universal.jar`, a plain maven artifact). ensureNeoForgeLoader
// invokes the official installer (`--installClient`) once to produce them.

/** '1.21.1-20240808.144430' from the profile's inheritsFrom + --fml.neoFormVersion. */
export function mcAndNeoFormVersion(profile) {
  const mc = String(profile?.inheritsFrom ?? '');
  if (!mc) return null;
  const game = Array.isArray(profile?.arguments?.game) ? profile.arguments.game : [];
  const flat = [];
  for (const entry of game) {
    if (typeof entry === 'string') flat.push(entry);
    else if (entry && Array.isArray(entry.value)) {
      flat.push(...entry.value.filter((v) => typeof v === 'string'));
    }
  }
  for (let i = 0; i < flat.length - 1; i++) {
    if (flat[i] === '--fml.neoFormVersion') return `${mc}-${flat[i + 1]}`;
  }
  return null;
}

/** <dataDir>/libraries/net/minecraft/client/<mcAndNeoForm>/ */
export function clientArtifactsDir(mcAndNeoForm) {
  return path.join(dataDir(), 'libraries', 'net', 'minecraft', 'client', mcAndNeoForm);
}

/** <dataDir>/libraries/net/neoforged/neoforge/<v>/neoforge-<v>[-classifier].jar */
export function neoforgeJarPath(v, classifier = '') {
  return path.join(
    dataDir(),
    'libraries',
    'net',
    'neoforged',
    'neoforge',
    v,
    `neoforge-${v}${classifier ? `-${classifier}` : ''}.jar`
  );
}

/** True when every artifact the production launch needs is on disk. */
export function launchArtifactsReady(v, profile) {
  const mcAnd = mcAndNeoFormVersion(profile);
  if (!mcAnd) return false;
  const dir = clientArtifactsDir(mcAnd);
  const required = [
    path.join(dir, `client-${mcAnd}-srg.jar`),
    path.join(dir, `client-${mcAnd}-extra.jar`),
    neoforgeJarPath(v, 'universal'),
    neoforgeJarPath(v, 'client'),
  ];
  return required.every((p) => fs.existsSync(p));
}

/** A java 21+ executable for the installer (launcher runtime, else PATH). */
async function installerJava() {
  const { runtimeJavaPath } = await import('./runtimes.mjs');
  const rt = runtimeJavaPath(21);
  if (fs.existsSync(rt)) return rt;
  try {
    const { javaForMajorExact } = await import('./runtimes.mjs');
    const info = await javaForMajorExact(21);
    if (info?.path && fs.existsSync(info.path)) return info.path;
  } catch {
    /* fall through to PATH */
  }
  const probe = spawnSync('java', ['-version'], { encoding: 'utf8', windowsHide: true });
  if (probe.status === 0) return 'java';
  throw Object.assign(new Error('NeoForge necesita un JDK 21 para generar sus artefactos de lanzamiento'), {
    status: 502,
    code: 'NEOFORGE_JDK_REQUIRED',
  });
}

/**
 * Run the official installer (`java -jar <installer> --installClient <dataDir>`)
 * so it generates the srg/extra/slim/neoforge-client artifacts into the
 * launcher's libraries dir (the launcher's data dir IS the standard Mojang
 * layout the installer targets). One-time per NeoForge version; the generated
 * files persist. Throws NEOFORGE_INSTALL_FAILED on a non-zero exit.
 */
async function runInstallerFor(v, profile, { onLine = null } = {}) {
  const mc = String(profile.inheritsFrom ?? '');
  // The installer wants the vanilla profile json at versions/<mc>/<mc>.json
  // (the launcher caches it flat at versions/<mc>.json) and a
  // launcher_profiles.json stub at the root.
  const nestedJson = path.join(dataDir(), 'versions', mc, `${mc}.json`);
  if (!fs.existsSync(nestedJson)) {
    const flatJson = path.join(dataDir(), 'versions', `${mc}.json`);
    if (fs.existsSync(flatJson)) {
      fs.mkdirSync(path.dirname(nestedJson), { recursive: true });
      fs.copyFileSync(flatJson, nestedJson);
    } else {
      // Fetch the vanilla profile through the resolver so the installer has it.
      const resolver = await import('./resolver.mjs');
      const json = await resolver.getVersionJson(mc);
      fs.mkdirSync(path.dirname(nestedJson), { recursive: true });
      fs.writeFileSync(nestedJson, JSON.stringify(json), 'utf8');
    }
  }
  const profilesJson = path.join(dataDir(), 'launcher_profiles.json');
  if (!fs.existsSync(profilesJson)) {
    fs.writeFileSync(
      profilesJson,
      JSON.stringify({ profiles: { [mc]: { lastVersionId: mc } }, selectedProfile: mc }),
      'utf8'
    );
  }

  const installer = installerJarPath(v);
  if (!fs.existsSync(installer)) {
    await downloadFile(installerUrl(v), installer);
  }
  const java = await installerJava();
  const result = await new Promise((resolve) => {
    const child = spawn(java, ['-jar', installer, '--installClient', dataDir()], {
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    });
    let out = '';
    const collect = (chunk) => {
      const s = chunk.toString('utf8');
      out += s;
      onLine?.(s);
    };
    child.stdout?.on('data', collect);
    child.stderr?.on('data', collect);
    child.on('close', (code) => resolve({ code, out }));
    child.on('error', (err) => resolve({ code: -1, out: String(err.message) }));
  });
  if (result.code !== 0) {
    throw Object.assign(
      new Error(`El instalador de NeoForge falló (exit ${result.code}): ${result.out.slice(-400)}`),
      { status: 502, code: 'NEOFORGE_INSTALL_FAILED' }
    );
  }
}

/**
 * Read the persisted launcher profile for a NeoForge version (no network).
 *
 * @param {string} v NeoForge version (e.g. '21.1.242')
 * @returns {object | null} parsed profile json, or null when not installed
 */
export function readNeoForgeJson(v) {
  try {
    return JSON.parse(fs.readFileSync(neoforgeJsonPath(v), 'utf8'));
  } catch {
    return null;
  }
}

/**
 * Whether the loader profile for a NeoForge version is already installed.
 *
 * @param {string} v NeoForge version (e.g. '21.1.242')
 * @returns {boolean}
 */
export function isNeoForgeInstalled(v) {
  return fs.existsSync(neoforgeJsonPath(v));
}

// Per-version serialization, mirroring download.mjs's destLocks: two
// concurrent ensure runs for the same version would race two --installClient
// JVMs (plus library downloads) into the same versions/libraries dirs.
const ensureLocks = new Map();

function withEnsureLock(key, fn) {
  const prev = ensureLocks.get(key) ?? Promise.resolve();
  const next = prev.then(() => fn(), () => fn());
  ensureLocks.set(key, next);
  // Cleanup must not introduce an unhandled rejection when `next` rejects —
  // the caller handles that; the cleanup chain swallows it.
  void next
    .catch(() => {})
    .then(() => {
      if (ensureLocks.get(key) === next) ensureLocks.delete(key);
    });
  return next;
}

/**
 * Ensure the NeoForge loader for `v` is installed: profile json present, all
 * its libraries downloaded AND the generated production-launch artifacts
 * (srg/extra/slim/neoforge-client/universal jars — produced by the official
 * installer). Cached installs skip all network work.
 *
 * @param {string} v NeoForge version (e.g. '21.1.242')
 * @param {{ instance?: object | null }} [opts] instance (for event context)
 * @returns {Promise<{ installed: boolean, cached: boolean, json: object }>}
 * @throws {{ status: 404, code: 'neoforge_version_not_found' }} unknown version
 * @throws {{ status: 502, code: 'NEOFORGE_PROFILE_MISSING' }} installer/profile broken
 */
export async function ensureNeoForgeLoader(v, opts = {}) {
  return withEnsureLock(v, () => ensureNeoForgeLoaderInner(v, opts));
}

async function ensureNeoForgeLoaderInner(v, { instance = null } = {}) {
  // Already installed -> nothing to do (profile json AND launch artifacts; a
  // json-only install from an older release re-runs to generate them).
  const existing = readNeoForgeJson(v);
  if (existing && launchArtifactsReady(v, existing)) {
    return { installed: true, cached: true, json: existing };
  }
  if (existing) {
    console.warn(`[neoforge] ${v}: profile present but launch artifacts missing — regenerating`);
  }

  const instanceName = instance?.name ?? null;
  const url = installerUrl(v);
  const jarPath = installerJarPath(v);
  const loaderContext = {
    instance: instanceName,
    kind: 'neoforge-loader',
    filename: path.basename(jarPath),
  };

  // 1. Download the installer jar.
  emitEvent('mod-progress', { instance: instanceName, phase: 'start', index: 0, total: 1, ...loaderContext });
  try {
    await downloadFile(url, jarPath);
  } catch (err) {
    emitEvent('mod-progress', {
      instance: instanceName,
      phase: 'error',
      index: 0,
      total: 1,
      ...loaderContext,
      error: err.message,
      message: err.message,
    });
    if (err.status === 404) {
      throw Object.assign(new Error(`NeoForge ${v} no encontrado`), {
        status: 404,
        code: 'neoforge_version_not_found',
      });
    }
    throw err;
  }
  emitEvent('mod-progress', { instance: instanceName, phase: 'done', index: 0, total: 1, ...loaderContext, dest: jarPath, sha1: null });

  // 2. Extract + parse the launcher profile from the installer jar.
  const profileBuf = readZipEntry(jarPath, 'version.json');
  if (!profileBuf) {
    throw Object.assign(new Error('El instalador de NeoForge no contiene version.json'), {
      status: 502,
      code: 'NEOFORGE_PROFILE_MISSING',
    });
  }
  let profile;
  try {
    profile = JSON.parse(profileBuf.toString('utf8'));
  } catch (err) {
    throw Object.assign(new Error(`version.json del instalador de NeoForge inválido: ${err.message}`), {
      status: 502,
      code: 'NEOFORGE_PROFILE_MISSING',
    });
  }

  // 3. Persist the profile, then generate the production-launch artifacts via
  // the official installer (one-time; the installer needs the installer jar,
  // so it is deleted after this step).
  const jsonPath = neoforgeJsonPath(v);
  fs.mkdirSync(path.dirname(jsonPath), { recursive: true });
  fs.writeFileSync(jsonPath, JSON.stringify(profile, null, 2), 'utf8');

  emitEvent('mod-progress', {
    instance: instanceName,
    phase: 'neoforge-artifacts',
    index: 0,
    total: 1,
    kind: 'neoforge-loader',
    filename: `neoforge-${v} (instalador oficial)`,
  });
  try {
    await runInstallerFor(v, profile, {
      onLine: (line) => console.log(`[neoforge-installer] ${line.trimEnd()}`),
    });
  } catch (err) {
    fs.rmSync(jsonPath, { force: true });
    throw err;
  }
  if (!launchArtifactsReady(v, profile)) {
    fs.rmSync(jsonPath, { force: true });
    throw Object.assign(
      new Error(`El instalador de NeoForge no generó los artefactos de ${v}`),
      { status: 502, code: 'NEOFORGE_ARTIFACTS_MISSING' }
    );
  }
  fs.rmSync(jarPath, { force: true });

  // 4. Download every library the profile lists, sha1-verified. On failure the
  // profile json is rolled back so a later ensure re-runs the whole install
  // (launch also self-heals by installing whatever is missing).
  try {
    const items = (profile.libraries ?? []).map((lib) => {
      const artifact = lib.downloads?.artifact;
      if (!artifact?.url || !artifact?.path) {
        throw Object.assign(new Error(`La librería ${lib.name} no tiene descarga en el perfil NeoForge`), {
          status: 502,
          code: 'NEOFORGE_PROFILE_MISSING',
        });
      }
      const dest = path.join(dataDir(), 'libraries', artifact.path);
      return {
        url: artifact.url,
        dest,
        sha1: artifact.sha1 ?? null,
        size: artifact.size ?? null,
        eventName: 'mod-progress',
        context: {
          instance: instanceName,
          kind: 'neoforge-library',
          filename: path.basename(dest),
        },
      };
    });
    await downloadAll(items, { concurrency: downloadConcurrency() });
  } catch (err) {
    fs.rmSync(jsonPath, { force: true });
    throw err;
  }

  return { installed: true, cached: false, json: profile };
}
