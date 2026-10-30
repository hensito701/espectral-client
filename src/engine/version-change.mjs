/**
 * Change an existing instance's Minecraft version and/or loader in place
 * (worlds, options, servers and the player's mods stay; nothing is recreated).
 *
 *   planVersionChange(name, { version, loader })  — read-only preview
 *   applyVersionChange(name, { version, loader }) — performs it
 *
 * The plan is recomputed on apply (the client's copy is never trusted).
 * Going to an OLDER version can corrupt worlds last opened in a newer one, so
 * a downgrade first copies saves/ to <instanceDir>/backups/. The running-game
 * guard lives in the route (activeInstances), like rename.
 */
import fs from 'node:fs';
import path from 'node:path';
import { httpError } from './error.mjs';
import {
  effectiveGameDir,
  effectiveModsDir,
  getInstance,
  getSummary,
  instanceDir,
  instanceJsonPath,
} from './instances.mjs';
import { applyModsPlan, ensureBrandingSeeded, planModsForTarget, supportsBranding } from './mods.mjs';
import { fabricSupportedForVersion, getVersionManifest, resolveVersion } from './resolver.mjs';
import { latestNeoForgeFor } from './neoforge.mjs';

const LOADERS = new Set(['vanilla', 'fabric', 'neoforge']);

/** Validated target plus the loader build it needs (NeoForge only). */
async function resolveTarget(inst, { version, loader } = {}) {
  const target = {
    version: typeof version === 'string' && version.trim() ? version.trim() : inst.version,
    loader: loader ?? inst.loader,
  };
  if (!LOADERS.has(target.loader)) {
    throw httpError(400, 'BAD_LOADER', "loader must be 'vanilla', 'fabric' or 'neoforge'");
  }
  if (target.version === inst.version && target.loader === inst.loader) {
    throw httpError(400, 'NO_CHANGE', 'the instance already uses that version and platform');
  }
  let info;
  try {
    info = await resolveVersion(target.version);
  } catch (err) {
    if (err?.status === 404) throw httpError(404, 'VERSION_NOT_FOUND', `Minecraft ${target.version} does not exist`);
    throw err;
  }
  if (!info.supported) throw httpError(409, 'VERSION_UNSUPPORTED', info.reason);
  target.loader_version = null;
  if (target.loader === 'fabric' && !(await fabricSupportedForVersion(target.version))) {
    throw httpError(409, 'FABRIC_UNAVAILABLE', `no Fabric loader for Minecraft ${target.version}`);
  }
  if (target.loader === 'neoforge') {
    target.loader_version = await latestNeoForgeFor(target.version);
    if (!target.loader_version) {
      throw httpError(409, 'NEOFORGE_UNAVAILABLE', `no NeoForge build for Minecraft ${target.version}`);
    }
  }
  return target;
}

/** True when `to` was released before `from` (manifest release times). */
async function isDowngrade(from, to) {
  if (from === to) return false;
  const manifest = await getVersionManifest();
  const time = (id) => manifest.versions.find((v) => v.id === id)?.release_time ?? '';
  const a = time(from);
  const b = time(to);
  return Boolean(a && b && b < a);
}

function hasWorlds(inst) {
  try {
    return fs.readdirSync(path.join(effectiveGameDir(inst), 'saves')).length > 0;
  } catch {
    return false;
  }
}

export async function planVersionChange(name, request) {
  const inst = await getInstance(name);
  const target = await resolveTarget(inst, request);
  const downgrade = await isDowngrade(inst.version, target.version);
  const mods = await planModsForTarget(inst, target);
  return {
    from: { version: inst.version, loader: inst.loader },
    to: { version: target.version, loader: target.loader, loader_version: target.loader_version },
    downgrade,
    backup_worlds: downgrade && hasWorlds(inst),
    espectral_menu: supportsBranding(target.version, target.loader),
    mods,
  };
}

function stamp(date = new Date()) {
  const p = (n) => String(n).padStart(2, '0');
  return `${date.getFullYear()}${p(date.getMonth() + 1)}${p(date.getDate())}-${p(date.getHours())}${p(date.getMinutes())}${p(date.getSeconds())}`;
}

export async function applyVersionChange(name, request) {
  const plan = await planVersionChange(name, request);
  const inst = await getInstance(name);

  let backup = null;
  if (plan.backup_worlds) {
    backup = path.join(instanceDir(name), 'backups', `saves-${inst.version}-${stamp()}`);
    await fs.promises.cp(path.join(effectiveGameDir(inst), 'saves'), backup, { recursive: true, errorOnExist: true });
  }

  inst.version = plan.to.version;
  inst.loader = plan.to.loader;
  // Fabric resolves the newest stable loader at launch; NeoForge needs its build.
  inst.loader_version = plan.to.loader_version;
  inst.updated_at = new Date().toISOString();
  const json = instanceJsonPath(name);
  const tmp = `${json}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(inst, null, 2), 'utf8');
  fs.renameSync(tmp, json);

  const mods = await applyModsPlan(name, plan.mods);
  if (plan.espectral_menu) {
    // Swaps the bundled Espectral jar for the new version and prunes the old one.
    await ensureBrandingSeeded(inst);
  } else if (inst.loader !== 'vanilla') {
    // No Espectral build for this version/loader: the old one (e.g. a Fabric
    // jar under NeoForge) would only fail to load — park it like other mods.
    const modsDir = effectiveModsDir(inst);
    for (const f of await fs.promises.readdir(modsDir).catch(() => [])) {
      if (/^espectral-(menu|brand)-.*\.jar$/i.test(f)) {
        await fs.promises.rename(path.join(modsDir, f), path.join(modsDir, `${f}.disabled`));
      }
    }
  }

  return { summary: await getSummary(name), from: plan.from, to: plan.to, backup, mods };
}
