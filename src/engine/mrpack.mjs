/**
 * mrpack.mjs — Modrinth modpack (.mrpack) import.
 *
 * A .mrpack is a zip whose `modrinth.index.json` describes the pack:
 *   { formatVersion, game, versionId, name, summary,
 *     dependencies: { minecraft, neoforge|fabric|forge|quilt },
 *     files: [{ path, downloads[], fileSize, hashes: { sha1 } }] }
 * plus an `overrides/` tree that maps into the instance game dir verbatim
 * (config/, mods/, resourcepacks/, …).
 *
 * importMrpack() is synchronous with the caller until the instance exists:
 * it parses the index, validates the loader/MC-version, picks a unique name
 * and creates the instance (which background-seeds vanilla libs + the
 * NeoForge loader via resolver.seedInstance). The pack file downloads and
 * overrides extraction then run fire-and-forget — failures are reported over
 * SSE (`import-done` ok:false) and logged, never thrown to the route, so the
 * instance is playable even before the mods finish landing.
 *
 * Events (contract): 'import-progress' { instance, phase:'files'|'overrides',
 * index, total, filename }, 'import-done' { instance, ok, error? }.
 * File downloads reuse 'mod-progress' (kind 'mrpack') via download.mjs.
 */
import fs from 'node:fs';
import path from 'node:path';
import { readZipEntry, listZipEntries } from './ziputil.mjs';
import { createInstance, effectiveGameDir, listInstances } from './instances.mjs';
import { resolveVersion, instanceDir } from './resolver.mjs';
import { downloadAll, sha1File, downloadConcurrency, USER_AGENT } from './download.mjs';
import { emit } from './events.mjs';
import { httpError } from './error.mjs';

const OVERRIDES_PREFIX = 'overrides/';

// ---------------------------------------------------------------------------
// Index parsing
// ---------------------------------------------------------------------------

/**
 * Parse the modrinth.index.json inside a .mrpack zip.
 * Throws 400 NOT_MRPACK when the entry is missing, 502 BAD_MRPACK on corrupt
 * or unparseable content.
 * @param {string} zipPath
 * @returns {Promise<{ name: string, version_id: string|undefined, summary: string|undefined, dependencies: object, files: object[], format_version: number|undefined }>}
 */
export async function parseMrpackIndex(zipPath) {
  let buf;
  try {
    buf = await readZipEntry(zipPath, 'modrinth.index.json');
  } catch {
    throw httpError(400, 'NOT_MRPACK', `'${zipPath}' no es un .mrpack válido (sin modrinth.index.json)`);
  }
  if (buf === null) {
    throw httpError(400, 'NOT_MRPACK', `'${zipPath}' no es un .mrpack válido (sin modrinth.index.json)`);
  }
  let parsed;
  try {
    parsed = JSON.parse(buf.toString('utf8'));
  } catch {
    throw httpError(502, 'BAD_MRPACK', `modrinth.index.json corrupto en '${zipPath}'`);
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw httpError(502, 'BAD_MRPACK', `modrinth.index.json corrupto en '${zipPath}'`);
  }
  return {
    name: parsed.name,
    version_id: parsed.versionId,
    summary: parsed.summary,
    dependencies: parsed.dependencies ?? {},
    files: Array.isArray(parsed.files) ? parsed.files : [],
    format_version: parsed.formatVersion,
  };
}

/**
 * Resolve a user-supplied import path into something importMrpack can use:
 * either a packed .mrpack zip, or an unpacked modpack folder. Accepts:
 *   - a file path (any extension — treated as a .mrpack zip)
 *   - a folder containing a single top-level .mrpack file
 *   - a folder containing modrinth.index.json (unpacked pack)
 * @param {unknown} input
 * @returns {Promise<{ kind: 'zip', file: string } | { kind: 'dir', dir: string }>}
 */
export async function resolveMrpackInput(input) {
  if (typeof input !== 'string' || input.trim().length === 0) {
    throw httpError(400, 'BAD_PATH', 'path is required and must be a non-empty string');
  }
  let st;
  try {
    st = await fs.promises.stat(input);
  } catch (e) {
    if (e && e.code === 'ENOENT') {
      throw httpError(404, 'FILE_NOT_FOUND', `'${input}' no existe`);
    }
    throw e;
  }
  if (!st.isDirectory()) {
    return { kind: 'zip', file: input };
  }
  // Unpacked pack: modrinth.index.json at the folder root.
  try {
    await fs.promises.stat(path.join(input, 'modrinth.index.json'));
    return { kind: 'dir', dir: input };
  } catch (e) {
    if (!(e && e.code === 'ENOENT')) throw e;
  }
  // Otherwise: exactly one top-level .mrpack.
  const entries = await fs.promises.readdir(input);
  const mrpacks = entries.filter((e) => e.toLowerCase().endsWith('.mrpack'));
  if (mrpacks.length === 1) return { kind: 'zip', file: path.join(input, mrpacks[0]) };
  if (mrpacks.length === 0) {
    throw httpError(400, 'NOT_MRPACK', `'${input}' es una carpeta sin modrinth.index.json ni archivos .mrpack`);
  }
  throw httpError(400, 'NOT_MRPACK', `'${input}' contiene varios archivos .mrpack — indica el archivo exacto`);
}

/**
 * Parse the modrinth.index.json of an unpacked modpack folder. Same return
 * shape as parseMrpackIndex.
 * @param {string} dir
 * @returns {Promise<{ name: string, version_id: string|undefined, summary: string|undefined, dependencies: object, files: object[], format_version: number|undefined }>}
 */
export async function parseMrpackIndexFromDir(dir) {
  let raw;
  try {
    raw = await fs.promises.readFile(path.join(dir, 'modrinth.index.json'), 'utf8');
  } catch {
    throw httpError(502, 'BAD_MRPACK', `modrinth.index.json corrupto en '${dir}'`);
  }
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw httpError(502, 'BAD_MRPACK', `modrinth.index.json corrupto en '${dir}'`);
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw httpError(502, 'BAD_MRPACK', `modrinth.index.json corrupto en '${dir}'`);
  }
  return {
    name: parsed.name,
    version_id: parsed.versionId,
    summary: parsed.summary,
    dependencies: parsed.dependencies ?? {},
    files: Array.isArray(parsed.files) ? parsed.files : [],
    format_version: parsed.formatVersion,
  };
}

// ---------------------------------------------------------------------------
// Name handling
// ---------------------------------------------------------------------------

/**
 * Turn a pack name into a valid instance name: trim, collapse whitespace,
 * keep [A-Za-z0-9 ._-], cap at 40 chars. Falls back to 'Modpack'.
 * @param {unknown} name
 * @returns {string}
 */
export function sanitizePackName(name) {
  let cleaned = String(name ?? '')
    .trim()
    .replace(/\s+/g, ' ')
    .replace(/[^A-Za-z0-9 ._-]/g, '')
    .replace(/^\.+|\.+$/g, '') // leading/trailing dots ('.' / '..' / '...' would escape or shadow)
    .trim();
  if (!cleaned || /^\.+$/.test(cleaned)) cleaned = 'Modpack';
  return cleaned.slice(0, 40);
}

/**
 * First unused variant of `base`: base, base-2, base-3, … compared
 * case-insensitively against the given existing names.
 * @param {string} base
 * @param {string[]} existingNames
 * @returns {string}
 */
export function uniqueInstanceName(base, existingNames) {
  const lower = new Set((existingNames ?? []).map((n) => String(n).toLowerCase()));
  if (!lower.has(String(base).toLowerCase())) return base;
  for (let i = 2; ; i++) {
    const candidate = `${base}-${i}`;
    if (!lower.has(candidate.toLowerCase())) return candidate;
  }
}

/**
 * Find the existing instance summary for a modpack already installed, so a
 * re-import of the same pack dedupes instead of creating a duplicate. The
 * comparison uses sanitizePackName on both sides, case-insensitively. Never
 * matches when the pack name sanitizes to the generic fallback 'Modpack'
 * (that would dedupe unrelated packs).
 * @param {object[]} existingSummaries
 * @param {unknown} packName
 * @returns {object|null}
 */
export function findExistingPackInstance(existingSummaries, packName) {
  const target = sanitizePackName(packName);
  if (target === 'Modpack') return null;
  const list = Array.isArray(existingSummaries) ? existingSummaries : [];
  for (const s of list) {
    if (s && typeof s.modpack === 'string' && sanitizePackName(s.modpack).toLowerCase() === target.toLowerCase()) {
      return s;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Path safety
// ---------------------------------------------------------------------------

/**
 * Join `rel` onto `base`, rejecting anything that could escape: absolute
 * paths (incl. Windows drive letters), `..` segments and empty input.
 * Throws 400 BAD_PATH. Zip entries use '/' separators, but tolerate '\'.
 * @param {string} base
 * @param {string} rel
 * @returns {string}
 */
export function safeJoin(base, rel) {
  if (typeof rel !== 'string' || rel.length === 0) {
    throw httpError(400, 'BAD_PATH', 'la ruta dentro del modpack está vacía');
  }
  const norm = rel.replace(/\\/g, '/');
  if (path.isAbsolute(norm) || /^[A-Za-z]:/.test(norm)) {
    throw httpError(400, 'BAD_PATH', `ruta absoluta no permitida dentro del modpack: '${rel}'`);
  }
  const segments = norm.split('/');
  if (segments.some((s) => s === '..')) {
    throw httpError(400, 'BAD_PATH', `la ruta '${rel}' se sale del directorio de la instancia`);
  }
  return path.join(base, norm);
}

// ---------------------------------------------------------------------------
// Pack file + overrides install (background)
// ---------------------------------------------------------------------------

// Hosts a modpack index may point pack-file downloads at: the Modrinth CDN
// and GitHub raw/release-asset hosts. Everything else is rejected (SSRF
// guard). Redirects are followed manually so every hop's host is validated.
const PACK_FILE_HOSTS = new Set([
  'cdn.modrinth.com',
  'github.com',
  'raw.githubusercontent.com',
  'objects.githubusercontent.com',
]);
const MAX_PACK_FILE_REDIRECTS = 5;

/**
 * Validate one pack-file URL against the host allowlist, following redirects
 * manually so every hop's host is checked. Returns the final (validated) URL.
 * Throws 400 BAD_PACK_URL / PACK_HOST_NOT_ALLOWED on a disallowed target.
 * @param {string} rawUrl
 * @returns {Promise<string>}
 */
async function resolvePackFileUrl(rawUrl) {
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    throw httpError(400, 'BAD_PACK_URL', `modpack file url is not a valid URL: '${rawUrl}'`);
  }
  for (let hop = 0; hop <= MAX_PACK_FILE_REDIRECTS; hop++) {
    if (url.protocol !== 'https:' || !PACK_FILE_HOSTS.has(url.hostname)) {
      throw httpError(400, 'PACK_HOST_NOT_ALLOWED', `modpack file host not allowed: '${url.hostname}'`);
    }
    const res = await fetch(url, { method: 'HEAD', redirect: 'manual', headers: { 'User-Agent': USER_AGENT } });
    const loc = res.headers.get('location');
    if (res.status >= 300 && res.status < 400 && loc) {
      url = new URL(loc, url);
      continue;
    }
    return url.href;
  }
  throw httpError(400, 'PACK_HOST_NOT_ALLOWED', `modpack file url exceeded the redirect limit: '${rawUrl}'`);
}

/**
 * Resolve every pack-file URL (deduplicated, bounded concurrency). A URL that
 * fails validation resolves to null for that entry and is reported through
 * `onFailure` instead of aborting the batch — one bad entry must not kill the
 * rest of the pack.
 * @param {(string|null)[]} rawUrls
 * @param {((raw: string, err: Error) => void)} [onFailure]
 * @returns {Promise<(string|null)[]>}
 */
async function resolvePackFileUrls(rawUrls, onFailure = null) {
  const cache = new Map();
  const one = (raw) => {
    if (!cache.has(raw)) cache.set(raw, resolvePackFileUrl(raw));
    return cache.get(raw);
  };
  const out = new Array(rawUrls.length);
  let cursor = 0;
  const worker = async () => {
    for (;;) {
      const i = cursor++;
      if (i >= rawUrls.length) return;
      const raw = rawUrls[i];
      if (raw === null) {
        out[i] = null;
        continue;
      }
      try {
        out[i] = await one(raw);
      } catch (e) {
        out[i] = null;
        onFailure?.(raw, e);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(8, Math.max(1, rawUrls.length)) }, worker));
  return out;
}

/**
 * Install every pack file into the instance game dir (mirroring file.path).
 * Files already present with a matching sha1 are skipped. When `sourceDir`
 * is set (unpacked pack import), files present under `<sourceDir>/<file.path>`
 * are copied instead of downloaded — unless the file declares a sha1 that
 * the source copy doesn't match, in which case it falls back to downloading.
 * Progress is emitted as 'import-progress' phase 'files'; actual downloads
 * reuse 'mod-progress' (kind 'mrpack'). Per-file failures (404, removed file,
 * disallowed host) are isolated: collected, logged, and returned — only a
 * total failure (every file failed) throws.
 * @param {string} instanceName
 * @param {object[]} files
 * @param {{ onProgress?: (p: object) => void, sourceDir?: string|null }} [opts]
 * @returns {Promise<string[]>} human-readable per-file failure descriptions
 */
export async function installMrpackFiles(instanceName, files, { onProgress = null, sourceDir = null } = {}) {
  const list = Array.isArray(files) ? files : [];
  const total = list.length;
  // Pack files are game-relative paths (mods/, config/, resourcepacks/, …)
  // and belong in the effective game dir when a custom folder is set.
  // instance.json always lives in the instance dir — the collision guard
  // below still points at the metadata home, never at the game dir.
  const destBase = effectiveGameDir(instanceName);
  const jsonPath = path.join(instanceDir(instanceName), 'instance.json');
  const pending = [];

  for (let i = 0; i < total; i++) {
    const file = list[i] ?? {};
    const filename = typeof file.path === 'string' ? file.path : '';
    if (!filename) continue; // malformed entry — skip, do not abort the pack
    const dest = safeJoin(destBase, filename);
    if (dest === jsonPath) {
      throw httpError(400, 'BAD_PATH', `la ruta '${filename}' colisiona con la metadata de la instancia`);
    }
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    const sha1 = file.hashes?.sha1 ?? null;
    if (sha1 && fs.existsSync(dest)) {
      try {
        if ((await sha1File(dest)) === sha1) {
          const p = { instance: instanceName, phase: 'files', index: i, total, filename };
          emit('import-progress', p);
          if (onProgress) onProgress(p);
          continue;
        }
      } catch {
        /* unreadable file — fall through to re-download */
      }
    }
    const src = sourceDir ? path.join(sourceDir, filename) : null;
    if (src && fs.existsSync(src)) {
      if (sha1) {
        try {
          if ((await sha1File(src)) !== sha1) {
            pending.push({ index: i, file, dest, sha1 });
            continue;
          }
        } catch {
          pending.push({ index: i, file, dest, sha1 });
          continue;
        }
      }
      try {
        await fs.promises.copyFile(src, dest);
      } catch (e) {
        // Copy failed (locked/permission) — isolate: fall back to download.
        pending.push({ index: i, file, dest, sha1 });
        continue;
      }
      const p = { instance: instanceName, phase: 'files', index: i, total, filename };
      emit('import-progress', p);
      if (onProgress) onProgress(p);
      continue;
    }
    pending.push({ index: i, file, dest, sha1 });
  }

  if (pending.length === 0) return [];
  // Isolate per-file failures: one bad entry (404, removed file, disallowed
  // host) must not abort the rest of the pack. Throw only when EVERYTHING
  // failed.
  let succeeded = 0;
  const failures = [];

  // SSRF guard: validate every pack-file URL against the host allowlist
  // before downloading; a rejected URL becomes that file's failure.
  const urls = await resolvePackFileUrls(
    pending.map((p) => (typeof p.file.downloads?.[0] === 'string' ? p.file.downloads[0] : null)),
    (raw, e) => failures.push(`${raw}: ${e.message}`)
  );
  const downloadable = [];
  pending.forEach((p, i) => {
    if (urls[i]) {
      downloadable.push({ ...p, url: urls[i] });
    } else if (typeof p.file.downloads?.[0] !== 'string') {
      failures.push(`${p.file.path}: no download url`);
    }
  });

  if (downloadable.length > 0) {
    // downloadAll throws an aggregate error when any item fails; per-item
    // results are already collected via onItem, so the throw is swallowed —
    // overrides extraction must still run after a partial failure.
    await downloadAll(
      downloadable.map((p) => ({
        url: p.url,
        dest: p.dest,
        sha1: p.sha1 ?? null,
        size: typeof p.file.fileSize === 'number' ? p.file.fileSize : null,
        eventName: 'mod-progress',
        context: { instance: instanceName, kind: 'mrpack', index: p.index, filename: p.file.path },
      })),
      {
        concurrency: downloadConcurrency(),
        onItem: ({ item, ok, error }) => {
          const ctx = item.context ?? {};
          if (ok) succeeded++;
          else failures.push(`${ctx.filename ?? '?'}: ${error ?? 'unknown error'}`);
          const p = { instance: instanceName, phase: 'files', index: ctx.index, total, filename: ctx.filename };
          emit('import-progress', p);
          if (onProgress) onProgress(p);
        },
      }
    ).catch(() => {});
  }
  if (failures.length > 0 && succeeded === 0) {
    throw new Error(`could not download any modpack file: ${failures[0]}`);
  }
  if (failures.length > 0) {
    console.warn(`[mrpack] ${failures.length} file(s) failed to download in '${instanceName}': ${failures.join('; ')}`);
  }
  return failures;
}

/**
 * Shared override-writing loop for zip and folder sources: writes each `rel`
 * path into the effective game dir via safeJoin, skips anything colliding with
 * instance.json (with a warning), emits 'import-progress' phase 'overrides'
 * per file, and throws only when overrides were present but none could be
 * written. `writeOne(rel, dest)` writes a single override and returns false
 * when the source yielded nothing (e.g. a missing zip entry).
 * @param {string[]} rels override paths relative to overrides/
 * @param {string} instanceName
 * @param {{ writeOne: (rel: string, dest: string) => Promise<boolean|void>, failMessage: string }} opts
 */
async function writeOverrideFiles(rels, instanceName, { writeOne, failMessage }) {
  const destBase = effectiveGameDir(instanceName);
  const jsonPath = path.join(instanceDir(instanceName), 'instance.json');
  const total = rels.length;
  let index = 0;
  let written = 0;
  for (const rel of rels) {
    const dest = safeJoin(destBase, rel);
    if (dest === jsonPath) {
      console.warn(`[mrpack] override '${rel}' collides with instance metadata; skipped`);
      index++;
      continue;
    }
    try {
      const ok = await writeOne(rel, dest);
      if (ok !== false) written++;
    } catch (e) {
      console.warn(`[mrpack] override '${rel}' not written: ${e.message}`);
    }
    emit('import-progress', { instance: instanceName, phase: 'overrides', index, total, filename: rel });
    index++;
  }
  if (written === 0 && total > 0) {
    throw new Error(failMessage);
  }
}

/**
 * Extract the `overrides/` tree of a .mrpack into the instance game dir
 * verbatim. Directory entries are skipped; files are written safely via
 * safeJoin. Emits 'import-progress' phase 'overrides' per file.
 * @param {string} zipPath
 * @param {string} instanceName
 */
export async function extractOverrides(zipPath, instanceName) {
  const entries = await listZipEntries(zipPath);
  const rels = entries
    .filter((e) => e.startsWith(OVERRIDES_PREFIX) && !e.endsWith('/')) // skip directory entries
    .map((e) => e.slice(OVERRIDES_PREFIX.length))
    .filter((rel) => rel.length > 0);
  return writeOverrideFiles(rels, instanceName, {
    writeOne: async (rel, dest) => {
      const buf = await readZipEntry(zipPath, `${OVERRIDES_PREFIX}${rel}`);
      if (buf === null) return false;
      fs.mkdirSync(path.dirname(dest), { recursive: true });
      await fs.promises.writeFile(dest, buf);
    },
    failMessage: 'no override could be extracted from the modpack',
  });
}

/**
 * Copy the `overrides/` tree of an unpacked modpack folder into the instance
 * game dir verbatim. Directory entries are skipped; files are written safely
 * via safeJoin; anything colliding with instance.json is skipped with a
 * warning. Emits 'import-progress' phase 'overrides' per file, mirroring
 * extractOverrides. A missing overrides/ folder means no overrides — no-op.
 * Throws when overrides are present but none could be written.
 * @param {string} sourceDir
 * @param {string} instanceName
 */
export async function copyOverridesFromDir(sourceDir, instanceName) {
  const overridesRoot = path.join(sourceDir, 'overrides');
  let entries;
  try {
    entries = await fs.promises.readdir(overridesRoot, { withFileTypes: true, recursive: true });
  } catch (e) {
    if (e && e.code === 'ENOENT') return; // no overrides folder — nothing to copy
    throw e;
  }
  const rels = entries
    .filter((d) => d.isFile())
    .map((d) => path.relative(overridesRoot, path.join(d.parentPath ?? overridesRoot, d.name)))
    .filter((rel) => rel.length > 0);
  return writeOverrideFiles(rels, instanceName, {
    writeOne: async (rel, dest) => {
      fs.mkdirSync(path.dirname(dest), { recursive: true });
      await fs.promises.copyFile(path.join(overridesRoot, rel), dest);
    },
    failMessage: 'no override could be copied from the modpack',
  });
}

// ---------------------------------------------------------------------------
// Import entry point
// ---------------------------------------------------------------------------

/**
 * Import a modpack (packed .mrpack or unpacked folder) as a new instance and
 * return its summary. The instance is created synchronously-with-await
 * (validating the pack, choosing the name, seeding the loader); pack file
 * install + overrides extraction run in the background and are reported via
 * 'import-done' — never thrown to the caller. When an instance for the same
 * modpack already exists, nothing is created: the existing summary is
 * returned with `already_exists: true` and 'import-done' reports it.
 * @param {{ file: string, memory_mb?: number|null, jdk_path_override?: string|null }} opts
 * @returns {Promise<{ summary: object, already_exists?: boolean }>}
 */
export async function importMrpack({ file, memory_mb = null, jdk_path_override = null } = {}) {
  const resolved = await resolveMrpackInput(file);
  const index = resolved.kind === 'dir' ? await parseMrpackIndexFromDir(resolved.dir) : await parseMrpackIndex(resolved.file);
  const deps = index.dependencies ?? {};

  const mc = typeof deps.minecraft === 'string' && deps.minecraft.length > 0 ? deps.minecraft : null;
  if (!mc) {
    throw httpError(400, 'MC_VERSION_REQUIRED', 'el modpack no declara una versión de Minecraft compatible');
  }

  let loader;
  let loader_version = null;
  if (deps.neoforge) {
    loader = 'neoforge';
    loader_version = String(deps.neoforge);
  } else if (deps.fabric) {
    loader = 'fabric';
  } else if (deps.forge) {
    throw httpError(400, 'FORGE_UNSUPPORTED', 'Forge no está soportado; el modpack debe usar NeoForge');
  } else if (deps.quilt) {
    throw httpError(400, 'QUILT_UNSUPPORTED', 'Quilt no está soportado');
  } else {
    loader = 'vanilla';
  }

  const versionInfo = await resolveVersion(mc);
  if (!versionInfo.supported) {
    throw httpError(409, 'JDK_UNSUPPORTED', `Minecraft ${mc} no es compatible con este launcher (requiere JDK ${versionInfo.java_major ?? '?'})`);
  }

  const instances = await listInstances();
  const existing = findExistingPackInstance(instances, index.name);
  if (existing) {
    emit('import-done', { instance: existing.name, ok: true, already_exists: true });
    return { summary: existing, already_exists: true };
  }

  const name = uniqueInstanceName(sanitizePackName(index.name), instances.map((i) => i.name));

  const summary = await createInstance({
    name,
    version: mc,
    loader,
    loader_version,
    modpack: typeof index.name === 'string' ? index.name : null,
    modpack_version: typeof index.version_id === 'string' ? index.version_id : null,
    memory_mb: memory_mb ?? null,
    jdk_path_override: jdk_path_override ?? null,
  });

  // Fire-and-forget: the instance already exists and is launchable; pack
  // files land in the background. Errors surface via 'import-done'.
  void (async () => {
    try {
      if (resolved.kind === 'dir') {
        await installMrpackFiles(name, index.files, { sourceDir: resolved.dir });
        await copyOverridesFromDir(resolved.dir, name);
      } else {
        await installMrpackFiles(name, index.files);
        await extractOverrides(resolved.file, name);
      }
      emit('import-done', { instance: name, ok: true });
    } catch (e) {
      console.warn(`[mrpack] background install failed for '${name}':`, e.message);
      emit('import-done', { instance: name, ok: false, error: e.message });
    }
  })();

  return { summary };
}
