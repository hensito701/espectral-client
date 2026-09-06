#!/usr/bin/env node
/* ==========================================================================
 * update-branding-manifest.mjs — (re)generate assets/branding/branding.json
 * from the built brand-mod jars.
 *
 * The branding preset installs the Espectral brand mod from bundled jars
 * (assets/branding/*.jar) into an instance's mods/ dir — no Modrinth round
 * trip. One jar per loader: espectral-menu-* (Fabric, full menu) and
 * espectral-brand-* (NeoForge, brand-handshake companion). The manifest pins
 * filename/sha1/size/loaders per Minecraft version so the engine can pick
 * the loader-correct jar, sha1-verify the copy and list the mod in the UI.
 *
 * Run: node scripts/update-branding-manifest.mjs   (after building the mods)
 * ========================================================================== */
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BRANDING_DIR = path.join(ROOT, 'assets', 'branding');

const RE = /^(espectral-menu|espectral-brand)-(\d[\w.]*)-(\d+\.\d+\.\d+)\.jar$/;
const LOADER_OF = { 'espectral-menu': 'fabric', 'espectral-brand': 'neoforge' };
const MOD_OF = {
  'espectral-menu': { mod_id: 'espectral-menu', mod_name: 'Espectral Menu' },
  'espectral-brand': { mod_id: 'espectral-brand', mod_name: 'Espectral Brand' },
};

const jars = readdirSync(BRANDING_DIR)
  .filter((f) => RE.test(f))
  .map((f) => {
    const m = RE.exec(f);
    const abs = path.join(BRANDING_DIR, f);
    const data = readFileSync(abs);
    return {
      filename: f,
      prefix: m[1],
      game_version: m[2],
      mod_version: m[3],
      sha1: createHash('sha1').update(data).digest('hex'),
      size: data.length,
      mtimeMs: statSync(abs).mtimeMs,
    };
  })
  .sort((a, b) => a.game_version.localeCompare(b.game_version));

// One game version maps to one jar: a second loader for the same MC version
// would silently overwrite the first in the mods map (fail loud instead).
const seenVersions = new Map();
for (const j of jars) {
  const prev = seenVersions.get(j.game_version);
  if (prev && prev !== j.prefix) {
    console.error(`[branding] ${j.game_version} has jars for two loaders (${prev}, ${j.prefix}) — manifest keys one jar per version`);
    process.exit(1);
  }
  seenVersions.set(j.game_version, j.prefix);
}

// All jars ship one launcher release: the manifest pins a single mod_version.
const builtVersions = new Set(jars.map((j) => j.mod_version));
if (builtVersions.size > 1) {
  console.error(`[branding] mixed mod versions: ${[...builtVersions].join(', ')} — rebuild all jars from the same tree`);
  process.exit(1);
}

if (jars.length === 0) {
  console.error('[branding] no espectral-menu-*.jar / espectral-brand-*.jar found in assets/branding — build the mods first');
  process.exit(1);
}

const manifest = {
  mod_id: 'espectral-menu',
  mod_name: 'Espectral Menu',
  mod_version: jars[0].mod_version,
  mods: Object.fromEntries(jars.map((j) => [j.game_version, {
    filename: j.filename,
    sha1: j.sha1,
    size: j.size,
    loaders: [LOADER_OF[j.prefix]],
    mod_id: MOD_OF[j.prefix].mod_id,
    mod_name: MOD_OF[j.prefix].mod_name,
  }])),
};

writeFileSync(path.join(BRANDING_DIR, 'branding.json'), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`[branding] wrote branding.json for ${jars.map((j) => `${j.game_version}/${LOADER_OF[j.prefix]} (${j.mod_version}, ${j.size} B)`).join(', ')}`);
