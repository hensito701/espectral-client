/**
 * Instance CRUD at <data>/instances/<name>/.
 *
 * Model (data/instances/<name>/instance.json):
 *   { name, version, loader: 'vanilla'|'fabric'|'neoforge',
 *     loader_version: string|null, modpack: string|null,
 *     modpack_version: string|null, memory_mb, mods: [], imported_from,
 *     jdk_path_override, aot_auto_train, created_at, updated_at,
 *     hue: int 0-359|null, game_dir: absolute-path|null,
 *     merge_optionslc? }
 *
 * Game dir layout: mods/ natives/ logs/ saves/ config/ cache/ options.txt,
 * servers.dat.
 *
 * modSetHash (canonical, exported for aot.mjs/launch.mjs reuse): sha256 hex of
 * the sorted `${filename}:${sha1}` lines of ENABLED mods (files ending in
 * exactly `.jar` — `.jar.disabled` is excluded), joined with '\n'. Memoized per
 * instance by the enabled set's `{name,size,mtimeMs}` fingerprint (see
 * computeModSetHash) — the hash value is byte-identical to the cold run.
 *
 * aot_key: computed by calling aot.cacheKey(version, javaBuild, osArch)
 * when aot.mjs is importable (B4), else null. javaBuild is the config cache
 * only (never probes/downloads a JVM); aot.mjs coerces null to 'unknown'.
 * modSetHash is recorded in meta.json for diagnostics but never part of the key
 * (mods are off-classpath; JEP 483 validates classpath identity only).
 * Create kicks off two-hop import (import.mjs, when import_from given) and
 * background library seeding (resolver.mjs) — both best-effort: failures are
 * logged + surfaced over SSE, never fatal.
 */
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { dataDir, loadConfig } from './config.mjs';
import { httpError } from './error.mjs';
import { emit } from './events.mjs';
import * as nbt from './nbt.mjs';

const NAME_RE = /^[A-Za-z0-9 ._-]{1,40}$/;

// ---------------------------------------------------------------------------
// Paths / validation
// ---------------------------------------------------------------------------

export function instancesRoot() {
  return path.join(dataDir(), 'instances');
}

export function instanceDir(name) {
  return path.join(instancesRoot(), name);
}

export function instanceJsonPath(name) {
  return path.join(instanceDir(name), 'instance.json');
}

export function isValidName(name) {
  return typeof name === 'string' && NAME_RE.test(name);
}

// ---------------------------------------------------------------------------
// Personalization: hue + custom game_dir + icon
// ---------------------------------------------------------------------------

/** Hue is an integer on the 0-359 wheel, or null when unset. */
export function isValidHue(h) {
  return Number.isInteger(h) && h >= 0 && h <= 359;
}

/** Circular distance between two hues on the 0-359 wheel. */
export function hueDistance(a, b) {
  const d = Math.abs(a - b) % 360;
  return Math.min(d, 360 - d);
}

/**
 * Random hue >= 30 degrees (circular distance) from every hue in `existing`.
 * 50 tries, then accept anything (a full wheel cannot always satisfy spacing).
 */
export function pickDistinctHue(existing = []) {
  const taken = (Array.isArray(existing) ? existing : []).filter(isValidHue);
  for (let i = 0; i < 50; i += 1) {
    const h = Math.floor(Math.random() * 360);
    if (taken.every((t) => hueDistance(h, t) >= 30)) return h;
  }
  return Math.floor(Math.random() * 360);
}

/**
 * Validate a game_dir value: undefined/null stays null (default location);
 * a string must be absolute, else 400. Returns the stored value (null|string).
 * mkdir -p happens in createInstance/patchInstance, not here.
 */
export function normalizeGameDir(value) {
  if (value === undefined || value === null) return null;
  if (typeof value !== 'string' || !path.isAbsolute(value)) {
    throw httpError(400, 'BAD_GAME_DIR', 'game_dir must be an absolute path or null');
  }
  return value;
}

/**
 * Effective game dir for an instance: the user-selected `game_dir` when set
 * (a custom absolute folder), else the default `<instanceDir>`.
 * Game files (mods/, config/, logs/, saves/, options.txt, servers.dat) belong
 * here — it is what the game boots with as --gameDir (see selectGameDir).
 * Accepts a full instance object or a bare name (sync-reads instance.json,
 * best-effort; falls back to the default dir when unreadable).
 */
export function effectiveGameDir(instanceOrName) {
  if (typeof instanceOrName === 'string') {
    try {
      const raw = JSON.parse(fs.readFileSync(instanceJsonPath(instanceOrName), 'utf8'));
      if (typeof raw?.game_dir === 'string' && raw.game_dir.length > 0) return raw.game_dir;
    } catch {
      /* unreadable meta — default dir below */
    }
    return instanceDir(instanceOrName);
  }
  const gameDir = instanceOrName?.game_dir;
  if (typeof gameDir === 'string' && gameDir.length > 0) return gameDir;
  return instanceDir(instanceOrName?.name);
}

/**
 * Effective mods dir for an instance: `<game_dir>/mods` when a custom folder
 * is set, else `<instanceDir>/mods`. The ONE helper every mods/config
 * writer+reader must use (mods.mjs, launch -Dfabric.modsDir, mrpack,
 * summaries/hashes, open-folder) so custom-folder instances stop leaking
 * jars into the default profile dir.
 */
export function effectiveModsDir(instanceOrName) {
  return path.join(effectiveGameDir(instanceOrName), 'mods');
}

/** Icon file for an instance (<instanceDir>/icon.png). */
export function instanceIconPath(name) {
  return path.join(instanceDir(name), 'icon.png');
}

/** Hues of all existing instances (auto-assign spacing). Never throws. */
async function existingInstanceHues() {
  const hues = [];
  for (const n of await readdirSafe(instancesRoot())) {
    if (TRASH_DIR_RE.test(n)) continue;
    try {
      const raw = JSON.parse(await fs.promises.readFile(instanceJsonPath(n), 'utf8'));
      if (isValidHue(raw.hue)) hues.push(raw.hue);
    } catch {
      /* skip unreadable entries */
    }
  }
  return hues;
}

function aotCacheDirForKey(key) {
  return path.join(dataDir(), 'cache', 'aot', key);
}

// ---------------------------------------------------------------------------
// Optional cross-slice modules (lazy + cached, never fatal when absent)
// ---------------------------------------------------------------------------

let aotModule = null;
async function getAotModule() {
  if (aotModule) return aotModule;
  try {
    aotModule = await import('./aot.mjs');
    return aotModule;
  } catch {
    // Not landed yet (or mid-write). Node re-resolves failed dynamic imports,
    // so this is retried on the next call instead of being cached as null.
    return null;
  }
}

/** Safe java build string for instance summaries: config cache only — never
 *  probes or downloads a JVM. A listing request must not trigger discovery
 *  (getJvmInfo() can spawn -version probes and fall through to a 200MB
 *  Adoptium download on a cold install). After the first launch/train the
 *  config holds the probed build and the AOT key converges on the real one. */
function configJavaBuild() {
  try {
    return loadConfig().jvm?.build ?? null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

async function readdirSafe(dir) {
  try {
    return await fs.promises.readdir(dir);
  } catch {
    return [];
  }
}

function existsSyncSafe(p) {
  try {
    return fs.existsSync(p);
  } catch {
    return false;
  }
}

/** stat() that never throws — returns null on failure. */
function statSafe(p) {
  try {
    return fs.statSync(p);
  } catch {
    return null;
  }
}

async function sha1File(p) {
  const hash = crypto.createHash('sha1');
  await new Promise((resolve, reject) => {
    const s = fs.createReadStream(p);
    s.on('data', (d) => hash.update(d));
    s.on('end', resolve);
    s.on('error', reject);
  });
  return hash.digest('hex');
}

function writeJson(p, obj) {
  fs.mkdirSync(path.dirname(p), { recursive: true });
  const tmp = p + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(obj, null, 2), 'utf8');
  fs.renameSync(tmp, p);
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/** Load an instance model; throws 404 when missing, 409 on invalid name. */
export async function getInstance(name) {
  if (!isValidName(name)) {
    throw httpError(409, 'INVALID_NAME', 'instance name must match ^[A-Za-z0-9 ._-]{1,40}$');
  }
  try {
    return JSON.parse(await fs.promises.readFile(instanceJsonPath(name), 'utf8'));
  } catch {
    throw httpError(404, 'NOT_FOUND', `instance '${name}' does not exist`);
  }
}

export async function listInstances() {
  const names = await readdirSafe(instancesRoot());
  const out = [];
  for (const n of names) {
    if (TRASH_DIR_RE.test(n)) continue; // deleteInstance trash — not an instance
    try {
      const st = await fs.promises.stat(instanceJsonPath(n));
      if (!st.isFile()) continue;
      out.push(await getSummary(n));
    } catch {
      // dir without instance.json (or deleted concurrently) — skip
    }
  }
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}

/**
 * Canonical modSetHash (see module doc). Accepts an instance object or name.
 *
 * Memoized: the returned sha256 is byte-identical to the "cold" computation —
 * this only skips the full content stream when the enabled set is unchanged.
 * The cache key is the per-file `{name,size,mtimeMs}` fingerprint of the
 * enabled jars; any stat change (add/remove/rename/overwrite) recomputes the
 * content hash. A stat-error is not treated as "unchanged" (a jar can be
 * mid-write with a stable size/mtime), so it falls back to the cold path.
 */
const modSetHashCache = new Map(); // instanceDir -> { statKey, hash }

export async function computeModSetHash(instance) {
  const dir = effectiveGameDir(instance);
  const modsDir = path.join(dir, 'mods');
  const files = await readdirSafe(modsDir);
  const enabled = files.filter((f) => f.endsWith('.jar') && !f.endsWith('.jar.disabled'));
  const statRows = [];
  let statError = false;
  for (const f of enabled) {
    try {
      const st = await fs.promises.stat(path.join(modsDir, f));
      statRows.push(`${f}:${st.size}:${st.mtimeMs}`);
    } catch {
      statError = true; // unreadable stat — don't trust the cache
    }
  }
  statRows.sort();
  const statKey = statError ? null : statRows.join('\n');

  const cached = modSetHashCache.get(dir);
  if (statKey !== null && cached && cached.statKey === statKey) {
    return cached.hash;
  }

  const lines = [];
  for (const f of enabled) {
    try {
      lines.push(`${f}:${await sha1File(path.join(modsDir, f))}`);
    } catch {
      lines.push(`${f}:error`); // unreadable mod — include deterministically
    }
  }
  lines.sort();
  const hash = crypto.createHash('sha256').update(lines.join('\n'), 'utf8').digest('hex');
  if (statKey !== null) {
    modSetHashCache.set(dir, { statKey, hash });
  }
  return hash;
}

/** InstanceSummary for one instance (contract shape). */
export async function getSummary(name) {
  const inst = await getInstance(name);
  const modsDir = effectiveModsDir(inst);
  const files = await readdirSafe(modsDir);
  const enabled = files.filter((f) => f.endsWith('.jar') && !f.endsWith('.jar.disabled'));
  const disabled = files.filter((f) => f.endsWith('.jar.disabled'));

  let aotKey = null;
  let aotCacheExists = false;
  const aot = await getAotModule();
  if (aot && typeof aot.cacheKey === 'function') {
    try {
      aotKey = aot.cacheKey(inst.version, configJavaBuild(), process.arch);
      aotCacheExists = aotKey ? existsSyncSafe(path.join(aotCacheDirForKey(aotKey), 'game.aot')) : false;
    } catch (e) {
      console.warn(`[instances] aot.cacheKey failed for '${name}':`, e.message);
    }
  }

  return {
    name: inst.name,
    version: inst.version,
    loader: inst.loader,
    loader_version: inst.loader_version ?? null,
    modpack: inst.modpack ?? null,
    modpack_version: inst.modpack_version ?? null,
    memory_mb: inst.memory_mb,
    hue: inst.hue ?? null,
    game_dir: inst.game_dir ?? null,
    has_icon: existsSyncSafe(instanceIconPath(name)),
    mod_count: enabled.length + disabled.length,
    enabled_mod_count: enabled.length,
    aot_key: aotKey,
    aot_cache_exists: aotCacheExists,
    imported_from: inst.imported_from ?? null,
    created_at: inst.created_at,
  };
}

/** GET /api/instances/:name detail: { summary, mods, servers, options, aot }. */
export async function getInstanceDetail(name) {
  const inst = await getInstance(name);
  const [summary, mods, servers, options, aot] = await Promise.all([
    getSummary(name),
    listModsFor(inst),
    readServers(inst),
    readOptionsFor(inst),
    aotStatus(inst),
  ]);
  return { summary, mods, servers, options, aot };
}

// ---------------------------------------------------------------------------
// Create / update / delete
// ---------------------------------------------------------------------------

const GAME_SUBDIRS = ['mods', 'natives', 'logs', 'saves', 'config', 'cache'];

/**
 * Create an instance. With import_from, runs the two-hop import (import.mjs)
 * first, then kicks off background library seeding and automatic AOT training.
 * Neither background operation blocks creation; failures leave the instance
 * usable and are emitted/logged for diagnostics.
 */
export async function createInstance(opts = {}) {
  const {
    name,
    version,
    loader = 'vanilla',
    loader_version = null,
    modpack = null,
    modpack_version = null,
    memory_mb,
    jdk_path_override = null,
    import_from = null,
    merge_optionslc = false,
    defer_auto_train = false,
    hue = null,
    game_dir = null,
  } = opts;
  if (!isValidName(name)) {
    throw httpError(409, 'INVALID_NAME', 'instance name must match ^[A-Za-z0-9 ._-]{1,40}$');
  }
  if (typeof version !== 'string' || version.length === 0) {
    throw httpError(400, 'VERSION_REQUIRED', 'version is required');
  }
  if (loader !== 'vanilla' && loader !== 'fabric' && loader !== 'neoforge') {
    throw httpError(400, 'BAD_LOADER', "loader must be 'vanilla', 'fabric' or 'neoforge'");
  }
  if (loader === 'neoforge' && !loader_version) {
    throw httpError(400, 'LOADER_VERSION_REQUIRED', 'loader_version es obligatorio para NeoForge');
  }
  // Fail a Fabric-instance CREATE with a clear message when no Fabric loader
  // exists for this game version, instead of the user discovering
  // 'fabric loader not found' at first launch. Best-effort probe: on a network
  // failure we optimistically continue (resolveFabric at launch is the
  // authority). resolveFabric() itself still throws the 404 at seed/launch.
  if (loader === 'fabric') {
    const resolver = await import('./resolver.mjs');
    const ok = await resolver.fabricSupportedForVersion(version);
    if (!ok) {
      throw httpError(409, 'FABRIC_UNAVAILABLE', `no Fabric loader for Minecraft ${version}`);
    }
  }
  if (existsSyncSafe(instanceDir(name))) {
    throw httpError(409, 'ALREADY_EXISTS', `instance '${name}' already exists`);
  }
  // Hue: explicit int wins (validated); absent/null auto-assigns a hue spaced
  // >= 30 degrees from every existing instance hue.
  let hueValue = hue;
  if (hueValue === undefined || hueValue === null) {
    hueValue = pickDistinctHue(await existingInstanceHues());
  } else if (!isValidHue(hueValue)) {
    throw httpError(400, 'BAD_HUE', 'hue must be an integer between 0 and 359 or null');
  }
  // Custom game dir: must be absolute when present; created up front.
  const gameDirValue = normalizeGameDir(game_dir);
  if (gameDirValue !== null) {
    fs.mkdirSync(gameDirValue, { recursive: true });
  }

  const now = new Date().toISOString();
  const inst = {
    name,
    version,
    loader,
    loader_version: loader_version ?? null,
    modpack: modpack ?? null,
    modpack_version: modpack_version ?? null,
    memory_mb:
      Number.isInteger(memory_mb) && memory_mb >= 512
        ? memory_mb
        : loadConfig().default_memory_mb ?? 3072,
    mods: [],
    imported_from: import_from || null,
    jdk_path_override: jdk_path_override ?? null,
    aot_auto_train: loadConfig().aot_auto_train ?? true,
    hue: hueValue,
    game_dir: gameDirValue,
    created_at: now,
    updated_at: now,
  };
  if (merge_optionslc) inst.merge_optionslc = true;

  // Game subdirs live where the game runs: the custom game_dir when set,
  // else the default instance dir. instance.json/icon always stay in the
  // instance dir (writeJson below).
  fs.mkdirSync(instanceDir(name), { recursive: true });
  const gameBase = gameDirValue ?? instanceDir(name);
  for (const sub of GAME_SUBDIRS) {
    fs.mkdirSync(path.join(gameBase, sub), { recursive: true });
  }
  writeJson(instanceJsonPath(name), inst);

  if (import_from) {
    try {
      const imp = await import('./import.mjs');
      if (typeof imp.importProfile === 'function') {
        await imp.importProfile(inst, import_from, 'never', { merge_optionslc });
      } else {
        console.warn(`[instances] import.mjs has no importProfile; two-hop import skipped for '${name}'`);
      }
    } catch (e) {
      console.warn(`[instances] two-hop import from '${import_from}' failed for '${name}':`, e.message);
      emit('mod-progress', { instance: name, phase: 'import', status: 'error', message: e.message });
    }
  }
  seedLibraries(inst);
  // Contract decision 6: when creating a Fabric instance, auto-queue the
  // performance mod preset right after instance creation.
  if (inst.loader === 'fabric') {
    void import('./mods.mjs')
      .then((mods) => {
        if (typeof mods.supportsPerformanceBundle === 'function' && mods.supportsPerformanceBundle(inst.version)) {
          if (typeof mods.installPreset === 'function') {
            return mods.installPreset(name, mods.PERFORMANCE_PRESET);
          }
        }
        return null;
      })
      .catch((e) => {
        console.warn(`[instances] auto-install performance preset for '${name}' failed:`, e?.message ?? e);
      });
  }
  // AOT is pre-provisioned / ready-to-train: the instance retains
  // `aot_auto_train: true` (or the global setting), its AOT key is derived in
  // getSummary, and the cache warms either on the first real launch once the
  // game reaches the main menu (routes/launch.mjs onMarker autoTrain) or when
  // the user explicitly triggers training via POST /api/instances/:name/train.
  // We do not auto-run the heavy training JVM during instance creation to avoid
  // booting a game window in the background while the user is still in the wizard.
  return getSummary(name);
}

/**
 * Delete an instance by moving its directory to a recoverable trash sibling
 * `<name>.deleted.<ms>` inside the instances root (M12: never destroy data
 * permanently). The move is a rename on the same filesystem, so it is
 * atomic-ish and nothing is lost; the instance is simply no longer listed or
 * resolvable. 404 when the instance does not exist.
 */
export async function deleteInstance(name) {
  await getInstance(name); // 404 when missing
  const trash = trashDirName(name);
  await fs.promises.rename(instanceDir(name), path.join(instancesRoot(), trash));
  return { deleted: name, trash };
}

/** Trash dirs from deleteInstance (`<name>.deleted.<ms>[-<n>]`) are never listed as instances. */
const TRASH_DIR_RE = /\.deleted\.\d+(-\d+)?$/;

/** Unique trash dir name for a deleted instance; on a same-millisecond collision a `-<n>` suffix is appended. */
function trashDirName(name) {
  const stamp = Date.now();
  let candidate = `${name}.deleted.${stamp}`;
  for (let n = 2; fs.existsSync(path.join(instancesRoot(), candidate)); n += 1) {
    candidate = `${name}.deleted.${stamp}-${n}`;
  }
  return candidate;
}

/**
 * PATCH an instance: memory_mb / jdk_path_override / aot_auto_train /
 * enabled_mods (rename *.jar <-> *.jar.disabled) + hue / game_dir (null
 * clears either). Mod toggles no longer invalidate the AOT key (mods are
 * off-classpath; see aot.mjs) — the cache is keyed by version|javaBuild|osArch
 * only; mod_set_hash is still recorded in meta.json for diagnostics.
 */
export async function patchInstance(name, patch = {}) {
  const inst = await getInstance(name);
  if (patch.memory_mb !== undefined) {
    const mb = Number(patch.memory_mb);
    if (!Number.isInteger(mb) || mb < 512 || mb > 65536) {
      throw httpError(400, 'BAD_MEMORY', 'memory_mb must be an integer between 512 and 65536');
    }
    inst.memory_mb = mb;
  }
  if (patch.jdk_path_override !== undefined) {
    if (patch.jdk_path_override !== null && typeof patch.jdk_path_override !== 'string') {
      throw httpError(400, 'BAD_JDK', 'jdk_path_override must be a path string or null');
    }
    inst.jdk_path_override = patch.jdk_path_override;
  }
  if (patch.aot_auto_train !== undefined) inst.aot_auto_train = !!patch.aot_auto_train;
  if (patch.hue !== undefined) {
    if (patch.hue !== null && !isValidHue(patch.hue)) {
      throw httpError(400, 'BAD_HUE', 'hue must be an integer between 0 and 359 or null');
    }
    inst.hue = patch.hue;
  }
  if (patch.game_dir !== undefined) {
    const next = normalizeGameDir(patch.game_dir);
    if (next !== null) fs.mkdirSync(next, { recursive: true });
    inst.game_dir = next;
  }
  if (Array.isArray(patch.enabled_mods)) {
    await applyEnabledMods(inst, patch.enabled_mods);
  }
  inst.updated_at = new Date().toISOString();
  writeJson(instanceJsonPath(name), inst);
  return getSummary(name);
}

// ---------------------------------------------------------------------------
// Instance icon (<instanceDir>/icon.png) — PNG bytes validated at the route.
// ---------------------------------------------------------------------------

/** Read an instance icon (PNG bytes). 404 when the instance or icon is missing. */
export async function readInstanceIcon(name) {
  await getInstance(name); // 404 when missing
  try {
    return await fs.promises.readFile(instanceIconPath(name));
  } catch {
    throw httpError(404, 'NOT_FOUND', `instance '${name}' has no icon`);
  }
}

/** Store an instance icon (validated PNG bytes). */
export async function writeInstanceIcon(name, buffer) {
  await getInstance(name); // 404 when missing
  await fs.promises.writeFile(instanceIconPath(name), buffer);
  return { ok: true, has_icon: true };
}

/** Remove an instance icon (missing file is a no-op). */
export async function removeInstanceIcon(name) {
  await getInstance(name); // 404 when missing
  try {
    await fs.promises.unlink(instanceIconPath(name));
  } catch {
    /* already absent */
  }
  return { ok: true, has_icon: false };
}

/** Rename mod jars to match the desired enabled set (Fabric ignores non-.jar). */
async function applyEnabledMods(inst, enabledMods) {
  const modsDir = effectiveModsDir(inst);
  const wanted = new Set(enabledMods.map((f) => String(f).replace(/\.disabled$/, '')));
  const files = await readdirSafe(modsDir);
  for (const f of files) {
    const isJar = f.endsWith('.jar');
    const isDisabledJar = f.endsWith('.jar.disabled');
    if (!isJar && !isDisabledJar) continue;
    const base = isDisabledJar ? f.slice(0, -'.disabled'.length) : f;
    const enable = wanted.has(base);
    if (enable && isDisabledJar) {
      await fs.promises.rename(path.join(modsDir, f), path.join(modsDir, base));
    } else if (!enable && isJar) {
      await fs.promises.rename(path.join(modsDir, f), path.join(modsDir, base + '.disabled'));
    }
  }
}

// ---------------------------------------------------------------------------
// Background library seeding (resolver.mjs, B2) — best effort
// ---------------------------------------------------------------------------

function seedLibraries(instance) {
  (async () => {
    emit('mod-progress', { instance: instance.name, phase: 'seeding', status: 'start' });
    try {
      const resolver = await import('./resolver.mjs');
      await resolver.seedInstance(instance);
      emit('mod-progress', { instance: instance.name, phase: 'seeding', status: 'done' });
    } catch (e) {
      console.warn(`[instances] background library seeding skipped for '${instance.name}':`, e.message);
      emit('mod-progress', { instance: instance.name, phase: 'seeding', status: 'error', message: e.message });
    }
  })();
}

// ---------------------------------------------------------------------------
// Mods / servers.dat / options.txt accessors (fallbacks used until B2/B3 land)
// ---------------------------------------------------------------------------

/** ModEntry[]: delegate to mods.mjs when available, else directory scan. */
async function listModsFor(inst) {
  try {
    const mods = await import('./mods.mjs');
    if (typeof mods.listMods === 'function') return await mods.listMods(inst);
  } catch { /* fall through */ }
  const modsDir = effectiveModsDir(inst);
  const files = await readdirSafe(modsDir);
  const out = [];
  for (const f of files) {
    if (!f.endsWith('.jar') && !f.endsWith('.jar.disabled')) continue;
    const base = f.replace(/\.disabled$/, '');
    let size = 0;
    try {
      size = (await fs.promises.stat(path.join(modsDir, f))).size;
    } catch { /* size unknown */ }
    out.push({
      filename: base,
      project_slug: null,
      version_number: null,
      version_id: null,
      sha1: null,
      size,
      enabled: f.endsWith('.jar'),
      installed: true,
    });
  }
  out.sort((a, b) => a.filename.localeCompare(b.filename));
  return out;
}

async function readServers(inst) {
  try {
    const buf = await fs.promises.readFile(path.join(effectiveGameDir(inst), 'servers.dat'));
    return nbt.parseServersDat(buf);
  } catch {
    return [];
  }
}

/** GET /api/instances/:name/servers */
export async function readServersDat(name) {
  const inst = await getInstance(name);
  return readServers(inst);
}

/** PUT /api/instances/:name/servers — write servers.dat (modern format). */
export async function writeServers(name, servers) {
  const inst = await getInstance(name);
  if (!Array.isArray(servers)) {
    throw httpError(400, 'BAD_SERVERS', 'servers must be an array');
  }
  const cleaned = servers.map((s) => ({
    name: String(s?.name ?? ''),
    ip: String(s?.ip ?? ''),
    hidden: !!(s && s.hidden),
    accept_textures: !!(s && s.accept_textures),
    has_icon: !!(s && s.has_icon),
  }));
  await fs.promises.writeFile(path.join(effectiveGameDir(inst), 'servers.dat'), nbt.writeServersDat(cleaned));
  return { count: cleaned.length };
}

/** Parse options.txt (properties lines, optional 'version:N' header). */
export function parseOptionsTxt(content) {
  const pairs = [];
  for (const line of String(content).split(/\r?\n/)) {
    if (line.length === 0) continue;
    const idx = line.indexOf(':');
    if (idx === -1) pairs.push([line, '']);
    else pairs.push([line.slice(0, idx), line.slice(idx + 1)]);
  }
  return pairs;
}

async function readOptionsFor(inst) {
  try {
    const imp = await import('./import.mjs');
    if (typeof imp.readOptionsTxt === 'function') {
      const raw = await fs.promises.readFile(path.join(effectiveGameDir(inst), 'options.txt'), 'utf8');
      const pairs = imp.readOptionsTxt(raw);
      return pairs.map((p) => (Array.isArray(p) ? p : [p[0], p[1]]));
    }
  } catch { /* fall through */ }
  try {
    const raw = await fs.promises.readFile(path.join(effectiveGameDir(inst), 'options.txt'), 'utf8');
    return parseOptionsTxt(raw);
  } catch {
    return [];
  }
}

/** GET /api/instances/:name/options -> [ [k,v], ... ] */
export async function readOptions(name) {
  const inst = await getInstance(name);
  return readOptionsFor(inst);
}

/**
 * Copy <source>/options.txt into <target>/options.txt. A missing source file
 * is a no-op ({ copied: false, count: 0 }) — the target is never modified in
 * that case. count is the parsed pair count of the copied file.
 */
export async function importOptionsFromInstance(targetName, sourceName) {
  if (!isValidName(targetName) || !isValidName(sourceName)) {
    throw httpError(409, 'INVALID_NAME', 'invalid instance name');
  }
  const target = await getInstance(targetName);
  const source = await getInstance(sourceName);
  if (targetName === sourceName) {
    throw httpError(409, 'SAME_INSTANCE', 'no puedes importar opciones de la misma instancia');
  }
  const sourcePath = path.join(effectiveGameDir(source), 'options.txt');
  try {
    await fs.promises.access(sourcePath);
  } catch {
    return { copied: false, count: 0, options: await readOptionsFor(target) };
  }
  await fs.promises.copyFile(sourcePath, path.join(effectiveGameDir(target), 'options.txt'));
  const options = await readOptionsFor(target);
  return {
    copied: true,
    // count mirrors the options pairs the UI shows (readOptionsFor skips the
    // 'version:N' header), so the two numbers can never disagree.
    count: options.length,
    options,
  };
}

// ---------------------------------------------------------------------------
// AOT status (B4 owns train/launch; this is the read-side snapshot)
// ---------------------------------------------------------------------------

/** AotStatus for an instance (contract shape). */
export async function aotStatus(inst) {
  const summary = await getSummary(inst.name);
  const key = summary.aot_key;
  if (!key) {
    return { key: null, cache_path: null, cache_exists: false, cache_size_bytes: 0 };
  }
  const cacheDir = aotCacheDirForKey(key);
  const cachePath = path.join(cacheDir, 'game.aot');
  let cacheExists = false;
  let cacheSize = 0;
  try {
    const st = await fs.promises.stat(cachePath);
    cacheExists = st.isFile();
    cacheSize = st.size;
  } catch { /* not trained yet */ }

  let trainedAt = null;
  try {
    const meta = JSON.parse(await fs.promises.readFile(path.join(cacheDir, 'meta.json'), 'utf8'));
    if (meta && typeof meta.trained_at === 'string') trainedAt = meta.trained_at;
  } catch { /* no meta yet */ }

  let proof = null;
  try {
    const gameBase = effectiveGameDir(inst); // AOT logs land where the game runs (cwd = gameDir)
    const dirEntries = await readdirSafe(gameBase);
    // Sort by mtime, newest last — filename sort is lexical, so aot-10.log
    // would sort before aot-2.log and the proof would point at a stale run.
    const logs = dirEntries
      .filter((f) => /^aot-\d+\.log$/.test(f))
      .map((f) => ({ f, m: statSafe(path.join(gameBase, f))?.mtimeMs ?? 0 }))
      .sort((a, b) => a.m - b.m)
      .map((x) => x.f);
    // L19b: keep the log sweep bounded — retain at most the 8 newest AOT logs
    // and drop anything older than 24h. aot-%p.log accumulates one file per
    // training JVM forever otherwise.
    const nowMs = Date.now();
    const HOUR = 3600_000;
    const MAX_AOT_LOGS = 8;
    for (let i = 0; i < logs.length - MAX_AOT_LOGS; i++) {
      try {
        await fs.promises.unlink(path.join(gameBase, logs[i]));
      } catch { /* ignore */ }
    }
    const recent = logs.filter((f) => {
      const m = statSafe(path.join(gameBase, f))?.mtimeMs ?? 0;
      return nowMs - m < 24 * HOUR;
    });
    if (recent.length > 0) {
      const logPath = path.join(gameBase, recent[recent.length - 1]);
      const content = await fs.promises.readFile(logPath, 'utf8');
      proof = {
        log_path: logPath,
        using_aot_linked_classes: /Using AOT-linked classes: true/.test(content),
      };
    }
  } catch { /* proof unavailable */ }

  return {
    key,
    cache_path: cachePath,
    cache_exists: cacheExists,
    cache_size_bytes: cacheSize,
    ready_to_train: inst.loader !== 'neoforge',
    ...(trainedAt ? { trained_at: trainedAt } : {}),
    ...(proof ? { proof } : {}),
  };
}
