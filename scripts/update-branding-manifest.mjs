#!/usr/bin/env node
/* ==========================================================================
 * update-branding-manifest.mjs — (re)generate assets/branding/branding.json
 * from the built menu-mod jars.
 *
 * The branding preset installs the Espectral main-menu mod from bundled jars
 * (assets/branding/*.jar) into an instance's mods/ dir — no Modrinth round
 * trip. The manifest pins filename/sha1/size per Minecraft version so the
 * engine can sha1-verify the copy and list the mod in the UI.
 *
 * Run: node scripts/update-branding-manifest.mjs   (after building the mod)
 * ========================================================================== */
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BRANDING_DIR = path.join(ROOT, 'assets', 'branding');

const RE = /^espectral-menu-(\d[\w.]*)-(\d+\.\d+\.\d+)\.jar$/;

const jars = readdirSync(BRANDING_DIR)
  .filter((f) => RE.test(f))
  .map((f) => {
    const m = RE.exec(f);
    const abs = path.join(BRANDING_DIR, f);
    const data = readFileSync(abs);
    return {
      filename: f,
      game_version: m[1],
      mod_version: m[2],
      sha1: createHash('sha1').update(data).digest('hex'),
      size: data.length,
      mtimeMs: statSync(abs).mtimeMs,
    };
  })
  .sort((a, b) => a.game_version.localeCompare(b.game_version));

if (jars.length === 0) {
  console.error('[branding] no espectral-menu-*.jar found in assets/branding — build the mod first');
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
  }])),
};

writeFileSync(path.join(BRANDING_DIR, 'branding.json'), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`[branding] wrote branding.json for ${jars.map((j) => `${j.game_version} (${j.mod_version}, ${j.size} B)`).join(', ')}`);
