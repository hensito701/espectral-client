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
 *   build/tauri-resources/node-bin/*      -> resources/node-bin/
 *     (node.exe on win32, node on linux — staged for the build host
 *     platform; the Tauri shell probes node-bin/node.exe, then
 *     node-bin/node, then the legacy top-level node.exe)
 *
 * The staged node binary is the official x64 build for the host platform,
 * pinned to a fixed version, so the installed/portable app no longer needs
 * Node on PATH. The download/extract is best-effort: on failure the stage
 * continues and the Tauri shell falls back to a PATH node at runtime.
 *
 * Run: node scripts/stage-tauri-resources.mjs   (before `tauri build`)
 * ========================================================================== */
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
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
// Platform artifact: Windows ships the win-x64 zip (node.exe inside),
// Linux the linux-x64 tar.xz (bin/node inside). The build host platform
// decides — cross-OS staging is not supported.
const NODE_URL =
  process.platform === 'win32'
    ? `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-win-x64.zip`
    : `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz`;
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
 * Stage the bundled node binary for the host platform into node-bin/
 * (best-effort): node.exe from the win-x64 zip on Windows, bin/node from
 * the linux-x64 tar.xz on Linux (chmod 0755 — tar extraction via pipe can
 * lose the exec bit). A manifest recording the staged version + platform
 * sits next to the binary; a stale manifest or a manifest for the other
 * platform (e.g. after an engines.node bump, or a shared checkout that was
 * last staged on the other OS) forces a re-download instead of silently
 * shipping the old binary. On download/extract failure this only warns —
 * the Tauri shell's PATH fallback still resolves, so the build proceeds.
 */
async function stageNodeExe() {
  const binName = process.platform === 'win32' ? 'node.exe' : 'node';
  const binDir = join(STAGE, 'node-bin');
  const dest = join(binDir, binName);
  const manifestPath = join(binDir, 'node.manifest.json');
  let manifest = null;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  } catch {
    // missing/corrupt manifest → treat the staged copy as stale
  }
  if (existsSync(dest) && manifest?.nodeVersion === NODE_VERSION && manifest?.platform === process.platform) {
    console.log(`[stage] ${binName} v${NODE_VERSION} already staged (${statSync(dest).size} bytes) — skipping download`);
    return;
  }
  if (existsSync(dest)) {
    console.log(`[stage] staged ${binName} is ${manifest ? `v${manifest.nodeVersion}/${manifest.platform} (want v${NODE_VERSION}/${process.platform})` : 'unversioned'} — re-staging`);
    rmSync(dest, { force: true });
  }
  mkdirSync(binDir, { recursive: true });
  const work = mkdtempSync(join(tmpdir(), 'espectral-node-'));
  try {
    const archive = join(work, NODE_URL.split('/').pop());
    console.log(`[stage] downloading ${NODE_URL} …`);
    const res = await fetch(NODE_URL, { redirect: 'follow' });
    if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`);
    writeFileSync(archive, Buffer.from(await res.arrayBuffer()));

    console.log(`[stage] extracting ${binName} …`);
    if (process.platform === 'win32') {
      // extract-zip 2.0.1 never settles on large zips under Node 24 on WSL/Linux
      // (verified), while it works on Windows — use the platform's native tool
      // on Linux and reserve extract-zip for win32.
      const extract = (await import('extract-zip')).default;
      await extract(archive, { dir: work });
      const exe = join(work, `node-v${NODE_VERSION}-win-x64`, 'node.exe');
      if (!existsSync(exe)) {
        throw new Error(`node.exe not found inside ${archive}`);
      }
      cpSync(exe, dest);
    } else {
      // argv-style (no shell): tmp paths are controlled, but a $ or backtick
      // in TMPDIR must never reach /bin/sh — same reason runtimes.mjs uses
      // spawnSync('tar', …) for Adoptium archives.
      execFileSync('tar', ['-xJf', archive, '-C', work], { stdio: 'ignore' });
      const bin = join(work, `node-v${NODE_VERSION}-linux-x64`, 'bin', 'node');
      if (!existsSync(bin)) {
        throw new Error(`bin/node not found inside ${archive}`);
      }
      cpSync(bin, dest);
      chmodSync(dest, 0o755);
    }
    writeFileSync(manifestPath, `${JSON.stringify({ nodeVersion: NODE_VERSION, platform: process.platform }, null, 2)}\n`);
    console.log(`[stage] bundled ${binName} v${NODE_VERSION} (${statSync(dest).size} bytes) → ${dest}`);
  } catch (err) {
    console.warn(
      `[stage] ${binName} download/extract failed — continuing without bundled node (PATH fallback still works): ${err.message}`
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

  // 4. bundled node binary for the host platform into node-bin/ (best-effort, see stageNodeExe)
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
