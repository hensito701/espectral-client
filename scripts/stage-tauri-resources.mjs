#!/usr/bin/env node
/* ==========================================================================
 * stage-tauri-resources.mjs — stage the runtime bits the Tauri shell bundles.
 *
 * The engine imports exactly one npm package at runtime (`extract-zip`, used
 * to unpack Adoptium JDKs), plus its transitive deps. Everything else is
 * node: builtins or relative files. This script copies only those packages
 * plus src/engine/ into a staging dir the Tauri bundler packages as
 * `bundle.resources` — so the installer ships a lean payload instead of the
 * whole node_modules tree.
 *
 * Staging layout (matches tauri.conf `bundle.resources` keys):
 *   build/tauri-resources/engine/*        -> resources/engine/
 *   build/tauri-resources/package.json    -> resources/package.json
 *   build/tauri-resources/node_modules/*  -> resources/node_modules/
 *   build/tauri-resources/node.exe        -> resources/node.exe
 *
 * The staged node.exe is the official Windows x64 build, pinned to a fixed
 * version, so the installed/portable app no longer needs Node on PATH. The
 * download/extract is best-effort: on failure the stage continues and the
 * Tauri shell falls back to a PATH node at runtime.
 *
 * Run: node scripts/stage-tauri-resources.mjs   (before `tauri build`)
 * ========================================================================== */
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const STAGE = join(ROOT, 'build', 'tauri-resources');

// Bundled Node version — the explicit `espectral.bundledNode` pin when set
// (the shipped runtime can be newer than the engines.node floor), else the
// engines floor itself.
const PKG = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'));
const NODE_VERSION = PKG.espectral?.bundledNode ?? PKG.engines.node.replace(/^>=/, '');
const NODE_URL = `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-win-x64.zip`;
// Transitive runtime closure of `extract-zip`, derived from package-lock.json
// at run time (BFS over `dependencies`) so a dependency bump can never leave
// this list stale. Fails loudly if the lock or any resolved package is
// missing — shipping an incomplete closure would break JDK extraction at
// runtime, which is worse than a failed build.
function deriveNpmClosure() {
  const lockPath = join(ROOT, 'package-lock.json');
  if (!existsSync(lockPath)) {
    console.error('[stage] package-lock.json missing — run npm install first');
    process.exit(1);
  }
  const pkgs = JSON.parse(readFileSync(lockPath, 'utf8')).packages || {};
  const closure = new Set();
  const queue = ['node_modules/extract-zip'];
  while (queue.length) {
    const key = queue.pop();
    if (closure.has(key)) continue;
    const entry = pkgs[key];
    if (!entry) {
      console.error(`[stage] ${key} not found in package-lock.json — run npm install first`);
      process.exit(1);
    }
    closure.add(key);
    for (const dep of Object.keys(entry.dependencies || {})) {
      // Prefer a nested resolution next to the parent, else hoisted root.
      const nestedKey = key.replace(/node_modules\/[^/]+$/, `node_modules/${dep}`);
      queue.push(pkgs[nestedKey] ? nestedKey : `node_modules/${dep}`);
    }
  }
  return [...closure].map((k) => k.replace(/^node_modules\//, '')).sort();
}

const REQ = deriveNpmClosure();

/** Recursive directory size in KB (cross-platform; `du` is not on Windows). */
function dirSizeKb(dir) {
  let bytes = 0;
  const walk = (d) => {
    for (const entry of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.isFile()) bytes += statSync(p).size;
    }
  };
  walk(dir);
  return Math.round(bytes / 1024);
}

/**
 * Stage the bundled Windows node.exe (best-effort). A manifest recording the
 * staged version sits next to the exe; a stale or missing manifest (e.g.
 * after an engines.node bump) forces a re-download instead of silently
 * shipping the old binary. On download/extract failure this only warns —
 * the Tauri shell's PATH fallback still resolves, so the build proceeds.
 */
async function stageNodeExe() {
  const dest = join(STAGE, 'node.exe');
  const manifestPath = join(STAGE, 'node.manifest.json');
  let manifest = null;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  } catch {
    // missing/corrupt manifest → treat the staged copy as stale
  }
  if (existsSync(dest) && manifest?.nodeVersion === NODE_VERSION) {
    console.log(`[stage] node.exe v${NODE_VERSION} already staged (${statSync(dest).size} bytes) — skipping download`);
    return;
  }
  if (existsSync(dest)) {
    console.log(`[stage] staged node.exe is ${manifest ? `v${manifest.nodeVersion} (want v${NODE_VERSION})` : 'unversioned'} — re-staging`);
    rmSync(dest, { force: true });
  }
  const work = mkdtempSync(join(tmpdir(), 'espectral-node-'));
  try {
    const zip = join(work, `node-v${NODE_VERSION}-win-x64.zip`);
    console.log(`[stage] downloading ${NODE_URL} …`);
    const res = await fetch(NODE_URL, { redirect: 'follow' });
    if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`);
    writeFileSync(zip, Buffer.from(await res.arrayBuffer()));

    console.log('[stage] extracting node.exe …');
    // extract-zip 2.0.1 never settles on large zips under Node 24 on WSL/Linux
    // (verified), while it works on Windows — use the platform's native tool
    // on Linux and reserve extract-zip for win32.
    if (process.platform === 'win32') {
      const extract = (await import('extract-zip')).default;
      await extract(zip, { dir: work });
    } else {
      try {
        execSync(`unzip -q -o ${JSON.stringify(zip)} -d ${JSON.stringify(work)}`, { stdio: 'ignore' });
      } catch {
        execSync(`python3 -m zipfile -e ${JSON.stringify(zip)} ${JSON.stringify(work)}`, { stdio: 'ignore' });
      }
    }
    const exe = join(work, `node-v${NODE_VERSION}-win-x64`, 'node.exe');
    if (!existsSync(exe)) {
      throw new Error(`node.exe not found inside ${zip}`);
    }
    cpSync(exe, dest);
    writeFileSync(manifestPath, `${JSON.stringify({ nodeVersion: NODE_VERSION }, null, 2)}\n`);
    console.log(`[stage] bundled node.exe v${NODE_VERSION} (${statSync(dest).size} bytes) → ${dest}`);
  } catch (err) {
    console.warn(
      `[stage] node.exe download/extract failed — continuing without bundled node (PATH fallback still works): ${err.message}`
    );
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

async function stage() {
  // 1. engine sources
  const engineSrc = join(ROOT, 'src', 'engine');
  const engineDst = join(STAGE, 'engine');
  rmSync(engineDst, { recursive: true, force: true });
  cpSync(engineSrc, engineDst, { recursive: true });

  // 2. package.json
  cpSync(join(ROOT, 'package.json'), join(STAGE, 'package.json'));

  // 3. only the extract-zip closure
  const nmDst = join(STAGE, 'node_modules');
  rmSync(nmDst, { recursive: true, force: true });
  const nmSrc = join(ROOT, 'node_modules');
  if (!existsSync(nmSrc)) {
    console.error('[stage] repo node_modules missing — run npm install first');
    process.exit(1);
  }
  mkdirSync(nmDst, { recursive: true });
  for (const pkg of REQ) {
    const src = join(nmSrc, pkg);
    if (!existsSync(src)) {
      // Fail loud: an incomplete closure breaks JDK extraction at runtime.
      console.error(`[stage] ${pkg} missing from node_modules — run npm install first`);
      process.exit(1);
    }
    cpSync(src, join(nmDst, pkg), { recursive: true });
  }

  const size = dirSizeKb(nmDst);
  console.log(`[stage] staged engine + ${REQ.length} npm pkgs (${size} KB) → ${STAGE}`);

  // 4. bundled Windows node.exe (best-effort, see stageNodeExe)
  await stageNodeExe();

  // 5. bundled Espectral Menu jars (assets/branding -> resources/branding)
  const brandingSrc = join(ROOT, 'assets', 'branding');
  const brandingDst = join(STAGE, 'branding');
  if (existsSync(brandingSrc)) {
    rmSync(brandingDst, { recursive: true, force: true });
    cpSync(brandingSrc, brandingDst, { recursive: true });
    console.log(`[stage] staged branding jars (${dirSizeKb(brandingDst)} KB) → ${brandingDst}`);
  } else {
    console.warn('[stage] assets/branding missing — continuing without bundled Espectral Menu');
  }
}

await stage();
