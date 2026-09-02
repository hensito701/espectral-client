import { mkdir, readFile, writeFile, stat, readdir } from 'node:fs/promises';
import path from 'node:path';

function fpPath(instanceName) {
  const base = process.env.ESPECTRAL_DATA_DIR
    ? path.resolve(process.env.ESPECTRAL_DATA_DIR)
    : path.resolve(path.dirname(new URL(import.meta.url).pathname), '..', '..', 'data');
  return path.join(base, 'instances', instanceName, 'resolved.json');
}

export function pathFor(instanceName) {
  return fpPath(instanceName);
}

export async function readFingerprint(instanceName) {
  try { return JSON.parse(await readFile(fpPath(instanceName), 'utf8')); } catch { return null; }
}

export async function buildExpectedAsync(instance, versionJson, fabricProfile) {
  let resolver = null;
  try { resolver = await import('./resolver.mjs'); } catch { resolver = null; }
  const libs = Array.isArray(versionJson.libraries) ? versionJson.libraries : [];
  const vanilla = [];
  let nativesCount = 0;
  for (const lib of libs) {
    const parts = String(lib.name).split(':');
    const classifier = parts.length > 3 ? parts[3] : null;
    const isNatives = !!(classifier && classifier.startsWith('natives'));
    let allowed = true;
    if (resolver && typeof resolver.rulesAllow === 'function') allowed = resolver.rulesAllow(lib);
    else {
      if (!Array.isArray(lib.rules) || lib.rules.length === 0) allowed = true;
      else {
        allowed = false;
        const os = process.platform === 'win32' ? 'windows' : process.platform === 'darwin' ? 'osx' : 'linux';
        const arch = process.arch;
        for (const rule of lib.rules) {
          let osOk = true;
          if (rule.os) {
            if (rule.os.name && rule.os.name !== os) osOk = false;
            else if (rule.os.arch && rule.os.arch !== arch) osOk = false;
          }
          let featOk = true;
          if (rule.features) {
            // Mirror resolver.mjs rulesAllow exactly (its default features = {}):
            // a feature requirement is satisfied only when its value is falsy
            // here, since no launcher feature flags are enabled. The previous
            // `!!featOk !== !!v` compared the running flag against the requirement
            // and INVERTED the match — kept in sync so this fallback can never
            // diverge from the authoritative classpath resolution.
            const features = {};
            for (const [k, v] of Object.entries(rule.features)) {
              if (!!features[k] !== !!v) { featOk = false; break; }
            }
          }
          if (osOk && featOk) allowed = rule.action === 'allow';
        }
      }
    }
    if (!allowed) continue;
    if (isNatives) nativesCount++;
    else vanilla.push({ name: lib.name, sha1: lib.downloads?.artifact?.sha1 ?? null });
  }
  return {
    version: instance.version,
    loader: instance.loader,
    fabric_loader_version: fabricProfile?.loader_version ?? null,
    asset_index: versionJson.assetIndex ? { id: versionJson.assetIndex.id, sha1: versionJson.assetIndex.sha1 ?? null } : null,
    client: versionJson.downloads?.client ? { sha1: versionJson.downloads.client.sha1 ?? null } : null,
    libraries: vanilla,
    natives_count: nativesCount,
    fabric_libs: fabricProfile ? fabricProfile.libraries.map((l) => ({ relPath: l.relPath, sha1: l.sha1 ?? null })) : [],
  };
}

export function expectedEqual(a, b) {
  if (!a || !b) return false;
  return JSON.stringify(a) === JSON.stringify(b);
}

export async function collectSentinelStats(instance, versionJson) {
  let resolver;
  try { resolver = await import('./resolver.mjs'); } catch { return null; }
  const out = [];
  try {
    const p = resolver.clientJarPath(instance.version);
    const st = await stat(p);
    if (!st.isFile()) return null;
    out.push({ relPath: `versions/${instance.version}/${instance.version}.jar`, size: st.size, mtimeMs: st.mtimeMs });
  } catch { return null; }
  const idxId = versionJson.assetIndex?.id;
  if (idxId) {
    try {
      const p = path.join(resolver.assetsDir(), 'indexes', `${idxId}.json`);
      const st = await stat(p);
      if (!st.isFile()) return null;
      out.push({ relPath: `assets/indexes/${idxId}.json`, size: st.size, mtimeMs: st.mtimeMs });
    } catch { return null; }
  }
  try {
    const nd = resolver.instanceNativesDir(instance.name);
    const st = await stat(nd);
    if (!st.isDirectory()) return null;
    const entries = (await readdir(nd)).filter(stableNativesEntry).sort();
    if (entries.length === 0) return null;
    // Content-based freshness: the game's own LWJGL/JNA extraction writes into
    // this dir every boot — the same dlls (lwjgl) plus a NEW random-name
    // jna<rand>.dll pair per run. Compare the STABLE entry set (filtered of
    // volatile names), not the mtime and not the raw names, or every launch
    // re-verifies + re-extracts.
    out.push({
      relPath: `instances/${instance.name}/natives`,
      size: 0,
      mtimeMs: st.mtimeMs,
      isDir: true,
      names: entries,
    });
  } catch { return null; }
  return out;
}

/** Natives-dir entries that churn per game boot must not invalidate the
 * fingerprint: our own .stage-* extraction dirs and JNA's random-name
 * jna<rand>.dll / jna<rand>.dll.x temp natives. */
export function stableNativesEntry(name) {
  if (name.startsWith('.stage-')) return false;
  if (/^jna\d+\.dll(\.x)?$/.test(name)) return false;
  return true;
}

export async function isSentinelFresh(files) {
  if (!Array.isArray(files) || files.length === 0) return false;
  const base = process.env.ESPECTRAL_DATA_DIR
    ? path.resolve(process.env.ESPECTRAL_DATA_DIR)
    : path.resolve(path.dirname(new URL(import.meta.url).pathname), '..', '..', 'data');
  for (const f of files) {
    const abs = path.join(base, f.relPath);
    try {
      const st = await stat(abs);
      if (f.isDir) {
        if (!st.isDirectory()) return false;
        if (Array.isArray(f.names)) {
          try {
            const now = (await readdir(abs)).filter(stableNativesEntry).sort();
            if (JSON.stringify(now) !== JSON.stringify(f.names)) return false;
          } catch {
            return false;
          }
        } else if (Math.abs(st.mtimeMs - f.mtimeMs) > 5000) {
          return false;
        }
      } else {
        if (st.size !== f.size) return false;
        if (Math.abs(st.mtimeMs - f.mtimeMs) > 5000) return false;
      }
    } catch { return false; }
  }
  return true;
}

export async function checkFingerprint(instance, versionJson, fabricProfile) {
  const stored = await readFingerprint(instance.name);
  if (!stored) return { hit: false, reason: 'no fingerprint' };
  const expected = await buildExpectedAsync(instance, versionJson, fabricProfile);
  if (!expectedEqual(expected, stored.expected)) return { hit: false, reason: 'manifest changed' };
  if (!Array.isArray(stored.files) || stored.files.length === 0) return { hit: false, reason: 'no sentinel' };
  const fresh = await isSentinelFresh(stored.files);
  if (!fresh) return { hit: false, reason: 'sentinel changed' };
  return { hit: true, fingerprint: stored };
}

export async function writeFingerprintFile(instance, versionJson, fabricProfile) {
  const expected = await buildExpectedAsync(instance, versionJson, fabricProfile);
  const files = await collectSentinelStats(instance, versionJson);
  if (!files) return null;
  const doc = {
    version: 1,
    created_at: new Date().toISOString(),
    instance: { name: instance.name, version: instance.version, loader: instance.loader },
    expected,
    files,
  };
  const file = fpPath(instance.name);
  await mkdir(path.dirname(file), { recursive: true });
  const tmp = file + '.tmp';
  await writeFile(tmp, JSON.stringify(doc, null, 2), 'utf8');
  const { rename } = await import('node:fs/promises');
  await rename(tmp, file);
  return doc;
}
