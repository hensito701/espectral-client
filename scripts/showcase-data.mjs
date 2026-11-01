#!/usr/bin/env node
/* ==========================================================================
 * showcase-data.mjs — build a clean launcher data dir for promo videos,
 * README images and store shots.
 *
 * Real dev data shows test instance names, test accounts, personalized
 * taglines and home-folder paths. This writes a fresh dir instead:
 *   config.json   one offline account (default "TuNick"), dark theme, gold
 *                 accent, onboarding done
 *   instances/    PvP 1.8.9 (Forge), UHC 26.3 (Fabric), Forge 1.12.2,
 *                 Fabric 1.21.11, NeoForge 1.21.1 — instance.json only, so
 *                 nothing is downloaded until one is launched
 *
 * Point the headless screenshot tool at it:
 *   node scripts/showcase-data.mjs /var/tmp/showcase
 *   ESPECTRAL_SHOT_DATA=/var/tmp/showcase native/app/scripts/screenshot.sh '#/' home.png --lang es
 *
 * Options: --nick <name> (2-16 of [A-Za-z0-9_])  --no-account (empty roster,
 * for "before" shots)  --force (replace an existing dir).
 * The dir must not be the real launcher data dir; the script refuses one
 * that already has a config.json unless --force is given.
 * ========================================================================== */
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const INSTANCES = [
  { name: 'PvP 1.8.9', version: '1.8.9', loader: 'forge', loader_version: '11.15.1.2318', hue: 30 },
  { name: 'UHC 26.3', version: '26.3', loader: 'fabric', loader_version: null, hue: 280 },
  { name: 'Forge 1.12.2', version: '1.12.2', loader: 'forge', loader_version: '14.23.5.2859', hue: 200 },
  { name: 'Fabric 1.21.11', version: '1.21.11', loader: 'fabric', loader_version: null, hue: 150 },
  { name: 'NeoForge 1.21.1', version: '1.21.1', loader: 'neoforge', loader_version: '21.1.265', hue: 20 },
];

/** Java-compatible offline UUID (same algorithm as native/engine/src/accounts.rs). */
export function offlineUuid(name) {
  const b = createHash('md5').update(`OfflinePlayer:${name}`).digest();
  b[6] = (b[6] & 0x0f) | 0x30;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = b.toString('hex');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

function parseArgs(argv) {
  const out = { dir: null, nick: 'TuNick', account: true, force: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--nick') out.nick = argv[++i];
    else if (a === '--no-account') out.account = false;
    else if (a === '--force') out.force = true;
    else if (!a.startsWith('--') && !out.dir) out.dir = a;
    else throw new Error(`unknown argument: ${a}`);
  }
  if (!out.dir) throw new Error('usage: node scripts/showcase-data.mjs <dir> [--nick TuNick] [--no-account] [--force]');
  if (!/^[A-Za-z0-9_]{2,16}$/.test(out.nick ?? '')) throw new Error('--nick must be 2-16 characters of [A-Za-z0-9_]');
  return out;
}

export function buildShowcase({ dir, nick = 'TuNick', account = true, force = false, now = new Date() }) {
  const root = path.resolve(dir);
  if (existsSync(path.join(root, 'config.json'))) {
    if (!force) throw new Error(`${root} already holds launcher data; pass --force to replace it`);
    rmSync(root, { recursive: true, force: true });
  }
  const stamp = now.toISOString();
  const accounts = account
    ? [{ username: nick, uuid: offlineUuid(nick), token_kind: 'offline', avatar_color: 47, created_at: stamp, last_used: stamp }]
    : [];
  mkdirSync(path.join(root, 'instances'), { recursive: true });
  const config = {
    theme: 'dark',
    accent: 'oro',
    discord_enabled: false,
    first_launch_setup_complete: true,
    accounts,
    active_username: account ? nick : null,
  };
  writeFileSync(path.join(root, 'config.json'), `${JSON.stringify(config, null, 2)}\n`);
  INSTANCES.forEach((inst, i) => {
    const instDir = path.join(root, 'instances', inst.name);
    mkdirSync(instDir, { recursive: true });
    // Newest first in the launcher's list: stagger updated_at.
    const updated = new Date(now.getTime() - i * 60_000).toISOString();
    const meta = {
      name: inst.name,
      version: inst.version,
      loader: inst.loader,
      loader_version: inst.loader_version,
      modpack: null,
      modpack_version: null,
      memory_mb: 4096,
      memory_auto_migrated: true,
      mods: [],
      imported_from: null,
      jdk_path_override: null,
      aot_auto_train: true,
      hue: inst.hue,
      game_dir: null,
      created_at: stamp,
      updated_at: updated,
    };
    writeFileSync(path.join(instDir, 'instance.json'), `${JSON.stringify(meta, null, 2)}\n`);
  });
  return root;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    console.log(buildShowcase(parseArgs(process.argv.slice(2))));
  } catch (e) {
    console.error(e.message);
    process.exit(2);
  }
}
