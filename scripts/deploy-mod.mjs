#!/usr/bin/env node
/* ==========================================================================
 * deploy-mod.mjs — one-shot branding-mod deploy.
 *
 * Rebuilds the Espectral brand/mod jars, refreshes assets/branding (jars +
 * branding.json manifest), and mirrors the result into the installed desktop
 * client's branding dir so the next launch picks it up. Replaces the manual
 * sequence: gradle build → copy jars → update-branding-manifest.mjs → copy to
 * %LOCALAPPDATA%\Espectral Client\branding.
 *
 * Run: npm run deploy:mod
 * ========================================================================== */
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync, copyFileSync, rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PKG = JSON.parse(readFileSync(path.join(ROOT, 'package.json'), 'utf8'));
const VERSION = PKG.version;
const BRANDING_DIR = path.join(ROOT, 'assets', 'branding');
const JAR_RE = /^(espectral-menu|espectral-brand)-.*\.jar$/;

const log = (msg) => console.log(`[deploy:mod] ${msg}`);
const fail = (msg) => { console.error(`[deploy:mod] ${msg}`); process.exit(1); };
const gradle = (cwd, tasks, env) => run('cmd.exe', ['/c', 'gradlew.bat', ...tasks], cwd, env);

function run(cmd, args, cwd, env) {
  const r = spawnSync(cmd, args, { cwd, stdio: 'inherit', env: env ? { ...process.env, ...env } : process.env });
  if (r.error) fail(`${cmd} failed to start in ${cwd}: ${r.error.message}`);
  if (r.status !== 0) fail(`${cmd} ${args.join(' ')} exited ${r.status} (cwd: ${cwd})`);
}

function tryRun(cmd, args, cwd, env) {
  const r = spawnSync(cmd, args, { cwd, stdio: 'inherit', env: env ? { ...process.env, ...env } : process.env });
  return r.error ? 127 : r.status;
}

// Gradle 8.x (nf wrapper) can't run the daemon on Java 25+. If the ambient JVM
// is too new, point JAVA_HOME at a locally installed JDK 21/17 for this build.
function gradle8JavaHome() {
  // Gradle resolves the daemon JVM from JAVA_HOME first, then PATH — so probe
  // exactly what gradlew will use, not just `java` on PATH.
  const jh = process.env.JAVA_HOME;
  const probe = (javaBin) => {
    const r = spawnSync(javaBin, ['-version'], { encoding: 'utf8' });
    const m = /version "(\d+)/.exec(`${r.stderr}${r.stdout}`);
    return m ? parseInt(m[1], 10) : null;
  };
  const effective = jh ? probe(path.join(jh, 'bin', 'java.exe')) : probe('java');
  if (effective != null && effective <= 24) return null;
  for (const root of ['C:/Program Files/Eclipse Adoptium', 'C:/Program Files/Java', 'C:/Program Files/Microsoft']) {
    if (!existsSync(root)) continue;
    const hit = readdirSync(root)
      .filter((d) => /^jdk-(21|17)[.-]/.test(d) && existsSync(path.join(root, d, 'bin', 'java.exe')))
      .sort()
      .pop();
    if (hit) {
      log(`${jh ? 'JAVA_HOME' : 'java on PATH'} is Java ${effective ?? '?'} — building nf with ${path.join(root, hit)}`);
      return path.join(root, hit);
    }
  }
  return null;
}

function findJar(dir, prefix) {
  if (!existsSync(dir)) return null;
  const hit = readdirSync(dir).find((f) => f.startsWith(prefix) && f.endsWith(`-${VERSION}.jar`));
  return hit ? path.join(dir, hit) : null;
}

// ── 1. Build ──────────────────────────────────────────────────────────────
log(`building espectral-menu jars (launcher ${VERSION})…`);
gradle(path.join(ROOT, 'branding-mod'), [':mc12111:build', ':mc262:build']);

// NeoForge companion: same layout, own wrapper. If it isn't buildable from
// this tree, keep whatever espectral-brand jar is already bundled.
const NF_DIR = path.join(ROOT, 'branding-mod-nf');
const NF_LIBS = path.join(NF_DIR, 'build', 'libs');
let nfJar = null;
if (existsSync(path.join(NF_DIR, 'gradlew.bat'))) {
  log('building espectral-brand (NeoForge) jar…');
  const javaHome = gradle8JavaHome();
  const status = tryRun('cmd.exe', ['/c', 'gradlew.bat', 'build'], NF_DIR,
    javaHome ? { JAVA_HOME: javaHome } : undefined);
  nfJar = status === 0 ? findJar(NF_LIBS, 'espectral-brand-1.21.1-') : null;
  if (!nfJar) log(`warning: no espectral-brand-1.21.1-${VERSION}.jar produced — keeping bundled jar (nf changes, if any, are NOT in this deploy)`);
} else {
  log('branding-mod-nf is not buildable from this repo — keeping bundled espectral-brand jar');
}

// ── 2. Copy fresh jars into assets/branding, drop stale ones ─────────────
const fresh = [
  findJar(path.join(ROOT, 'branding-mod', 'mc12111', 'build', 'libs'), 'espectral-menu-1.21.11-'),
  findJar(path.join(ROOT, 'branding-mod', 'mc262', 'build', 'libs'), 'espectral-menu-26.2-'),
  nfJar,
];
for (const [i, name] of ['espectral-menu-1.21.11', 'espectral-menu-26.2'].entries()) {
  if (!fresh[i]) fail(`${name}-${VERSION}.jar not found after build — check the gradle output above`);
}

mkdirSync(BRANDING_DIR, { recursive: true });
const keep = new Set();
for (const jar of fresh) {
  if (!jar) continue;
  copyFileSync(jar, path.join(BRANDING_DIR, path.basename(jar)));
  keep.add(path.basename(jar));
  log(`bundled ${path.basename(jar)}`);
}
for (const f of readdirSync(BRANDING_DIR)) {
  if (!JAR_RE.test(f) || keep.has(f)) continue;
  // Never delete a bundled espectral-brand jar we didn't rebuild this run —
  // a failed/unbuildable nf build must keep the existing NeoForge coverage.
  if (!nfJar && f.startsWith('espectral-brand-')) {
    keep.add(f);
    log(`keeping bundled ${f} (nf not rebuilt)`);
    continue;
  }
  rmSync(path.join(BRANDING_DIR, f));
  log(`removed stale ${f}`);
}

// ── 3. Regenerate the manifest ────────────────────────────────────────────
log('regenerating branding.json…');
run(process.execPath, [path.join(ROOT, 'scripts', 'update-branding-manifest.mjs')], ROOT);

// ── 4. Mirror into the installed desktop client ───────────────────────────
const CLIENT_DIR = process.env.LOCALAPPDATA ? path.join(process.env.LOCALAPPDATA, 'Espectral Client') : null;
if (!CLIENT_DIR || !existsSync(CLIENT_DIR)) {
  log('warning: %LOCALAPPDATA%\\Espectral Client not found — installed client not updated (repo assets are ready)');
} else {
  const dest = path.join(CLIENT_DIR, 'branding');
  mkdirSync(dest, { recursive: true });
  for (const f of readdirSync(BRANDING_DIR)) {
    copyFileSync(path.join(BRANDING_DIR, f), path.join(dest, f));
  }
  for (const f of readdirSync(dest)) {
    if (JAR_RE.test(f) && !existsSync(path.join(BRANDING_DIR, f))) rmSync(path.join(dest, f));
  }
  log(`installed branding → ${dest}`);
}

// ── 5. Done ───────────────────────────────────────────────────────────────
log('done. Restart Espectral Client (branding.json is read at engine startup), then launch the game.');
