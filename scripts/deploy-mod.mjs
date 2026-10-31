#!/usr/bin/env node
/* ==========================================================================
 * deploy-mod.mjs — one-shot branding-mod deploy.
 *
 * Rebuilds the Espectral brand/mod jars, refreshes assets/branding (jars +
 * branding.json manifest), and mirrors the result into the installed launcher's
 * `branding/` dir so the next launch picks it up. Replaces the manual sequence:
 * gradle build → copy jars → update-branding-manifest.mjs → copy to
 * %LOCALAPPDATA%\Espectral Client\branding (Windows) or
 * /usr/lib/Espectral Client/branding (Linux .deb).
 *
 * Build agents per jar (the wrapper is always used, so the Gradle version of
 * each tree wins):
 *   branding-mod/            mc12111, mc262, mc263   Fabric menu   (needs Java 21)
 *   branding-mod-nf/         espectral-brand         NeoForge      (needs Java 21)
 *   branding-mod-forge189/   espectral-menu 1.8.9    Forge         (needs Java 21
 *                            to RUN Gradle; the Java 8 toolchain it compiles
 *                            with is resolved by the wrapper/loom itself)
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
const isWin = process.platform === 'win32';
const gradleArgv = [isWin ? 'cmd.exe' : 'sh', ...(isWin ? ['/c', 'gradlew.bat'] : ['-c', './gradlew'])];

// `task` calls run a plain process; `gradle` calls add Gradle's own flags.
function run(cwd, [cmd, ...args], env) {
  const r = spawnSync(cmd, args, {
    cwd, stdio: 'inherit', env: env ? { ...process.env, ...env } : process.env,
  });
  if (r.error) fail(`${cmd} failed to start in ${cwd}: ${r.error.message}`);
  if (r.status !== 0) fail(`${cmd} ${args.join(' ')} exited ${r.status} (cwd: ${cwd})`);
}

function tryRun(cwd, [cmd, ...args], env) {
  const r = spawnSync(cmd, args, {
    cwd, stdio: 'inherit', env: env ? { ...process.env, ...env } : process.env,
  });
  return r.error ? 127 : r.status;
}

const gradle = (cwd, tasks, env) => run(cwd, [...gradleArgv, ...tasks, '--console=plain'], env);
const tryGradle = (cwd, tasks, env) => tryRun(cwd, [...gradleArgv, ...tasks, '--console=plain'], env);

// Gradle resolves its daemon JVM from JAVA_HOME first, then PATH. Probe
// exactly what the wrapper will use, and if the ambient JVM is too new
// (Gradle 8.x can't run its daemon on Java 25+) point JAVA_HOME at a locally
// installed JDK 21/17 for this build only.
function gradleJavaHome() {
  const javaBin = (home) => path.join(home, 'bin', isWin ? 'java.exe' : 'java');
  const probe = (bin) => {
    const r = spawnSync(bin, ['-version'], { encoding: 'utf8' });
    const m = /version "(\d+)/.exec(`${r.stderr}${r.stdout}`);
    return m ? parseInt(m[1], 10) : null;
  };
  const jh = process.env.JAVA_HOME;
  const effective = jh ? probe(javaBin(jh)) : probe('java');
  if (effective != null && effective <= 24) return null;
  const roots = isWin
    ? ['C:/Program Files/Eclipse Adoptium', 'C:/Program Files/Java', 'C:/Program Files/Microsoft']
    : ['/usr/lib/jvm', '/opt', '/usr/java'];
  const pattern = isWin ? /^jdk-(21|17)[.-]/ : /^(java-|jdk-?)?(21|17)[.-]/;
  for (const root of roots) {
    if (!existsSync(root)) continue;
    const hit = readdirSync(root)
      .filter((d) => pattern.test(d) && existsSync(javaBin(path.join(root, d))))
      .sort()
      .pop();
    if (hit) {
      log(`${jh ? 'JAVA_HOME' : 'java on PATH'} is Java ${effective ?? '?'} — building with ${path.join(root, hit)}`);
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

// Loom needs a real JDK 8 to compile MC 1.8.9: point it at the launcher's
// downloaded runtime when `java -version` on PATH is not 8. Overridable with
// ESPECTRAL_JDK8=<path to a JDK 8 home>.
function jdk8Home() {
  if (process.env.ESPECTRAL_JDK8) return process.env.ESPECTRAL_JDK8;
  const local = path.join(ROOT, 'data', 'runtimes', 'jdk-8');
  return existsSync(local) ? local : null;
}

// ── 1. Build ──────────────────────────────────────────────────────────────
log(`building espectral-menu jars (launcher ${VERSION})…`);
const gradle8Home = gradleJavaHome();
const gradleEnv = gradle8Home ? { JAVA_HOME: gradle8Home } : undefined;

gradle(path.join(ROOT, 'branding-mod'), [':mc12111:build', ':mc262:build', ':mc263:build'], gradleEnv);

// NeoForge companion: same layout, own wrapper. If it isn't buildable from
// this tree, keep whatever espectral-brand jar is already bundled.
const NF_DIR = path.join(ROOT, 'branding-mod-nf');
const NF_LIBS = path.join(NF_DIR, 'build', 'libs');
let nfJar = null;
if (existsSync(path.join(NF_DIR, isWin ? 'gradlew.bat' : 'gradlew'))) {
  log('building espectral-brand (NeoForge) jar…');
  const status = tryGradle(NF_DIR, ['build'], gradleEnv);
  nfJar = status === 0 ? findJar(NF_LIBS, 'espectral-brand-1.21.1-') : null;
  if (!nfJar) log(`warning: no espectral-brand-1.21.1-${VERSION}.jar produced — keeping bundled jar (nf changes, if any, are NOT in this deploy)`);
} else {
  log('branding-mod-nf is not buildable from this repo — keeping bundled espectral-brand jar');
}

// Forge 1.8.9 menu: Gradle 8.8 + gg.essential.loom; Gradle itself needs a
// JDK 21 to run and an actual JDK 8 for the 1.8.9 toolchain. Same
// keep-what-is-bundled rule when the tree can't build here.
const F189_DIR = path.join(ROOT, 'branding-mod-forge189');
const F189_LIBS = path.join(F189_DIR, 'build', 'libs');
let f189Jar = null;
if (existsSync(path.join(F189_DIR, isWin ? 'gradlew.bat' : 'gradlew'))) {
  log('building espectral-menu (Forge 1.8.9) jar — Gradle needs JDK 21 to run…');
  const j8 = jdk8Home();
  if (!j8) log('warning: no JDK 8 found (data/runtimes/jdk-8 or ESPECTRAL_JDK8) — the wrapper will try to provision one');
  const args = ['build'];
  if (j8) args.push(`-Porg.gradle.java.installations.paths=${j8}`);
  const status = tryGradle(F189_DIR, args, gradleEnv);
  f189Jar = status === 0 ? findJar(F189_LIBS, 'espectral-menu-1.8.9-') : null;
  if (!f189Jar) log(`warning: no espectral-menu-1.8.9-${VERSION}.jar produced — keeping bundled jar (Forge 1.8.9 changes, if any, are NOT in this deploy)`);
} else {
  log('branding-mod-forge189 is not buildable from this repo — keeping bundled 1.8.9 jar');
}

// ── 2. Copy fresh jars into assets/branding, drop stale ones ─────────────
const fresh = [
  findJar(path.join(ROOT, 'branding-mod', 'mc12111', 'build', 'libs'), 'espectral-menu-1.21.11-'),
  findJar(path.join(ROOT, 'branding-mod', 'mc262', 'build', 'libs'), 'espectral-menu-26.2-'),
  findJar(path.join(ROOT, 'branding-mod', 'mc263', 'build', 'libs'), 'espectral-menu-26.3-'),
  nfJar,
  f189Jar,
];
for (const [i, name] of ['espectral-menu-1.21.11', 'espectral-menu-26.2', 'espectral-menu-26.3'].entries()) {
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
  // Never delete a bundled jar this run did not rebuild — a failed/unbuildable
  // tree must keep its existing coverage (NeoForge, Forge 1.8.9).
  if (!nfJar && f.startsWith('espectral-brand-')) {
    keep.add(f);
    log(`keeping bundled ${f} (nf not rebuilt)`);
    continue;
  }
  if (!f189Jar && f.startsWith('espectral-menu-1.8.9-')) {
    keep.add(f);
    log(`keeping bundled ${f} (forge189 not rebuilt)`);
    continue;
  }
  rmSync(path.join(BRANDING_DIR, f));
  log(`removed stale ${f}`);
}

// ── 3. Regenerate the manifest ────────────────────────────────────────────
log('regenerating branding.json…');
run(ROOT, [process.execPath, path.join(ROOT, 'scripts', 'update-branding-manifest.mjs')]);

// ── 4. Mirror into the installed desktop client ───────────────────────────
// 1.7.0 layout: the Rust launcher (espectral-client.exe) reads `branding/`
// sitting next to itself, not a Tauri resource dir. Windows install dir is the
// same %LOCALAPPDATA%\Espectral Client\ the Tauri NSIS used; the Linux .deb
// puts the binary and branding under /usr/lib/Espectral Client/.
const CLIENT_DIR = process.env.LOCALAPPDATA
  ? path.join(process.env.LOCALAPPDATA, 'Espectral Client')
  : existsSync('/usr/lib/Espectral Client')
    ? '/usr/lib/Espectral Client'
    : null;
const CLIENT_EXE = CLIENT_DIR
  ? path.join(CLIENT_DIR, isWin ? 'espectral-client.exe' : 'espectral-client')
  : null;
if (!CLIENT_DIR || !CLIENT_EXE || !existsSync(CLIENT_EXE)) {
  log('warning: installed Espectral Client not found — installed client not updated (repo assets are ready)');
  log(`looked for: ${CLIENT_EXE ?? 'no install dir (set LOCALAPPDATA or install the .deb)'}`);
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
