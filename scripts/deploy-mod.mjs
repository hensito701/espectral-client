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
 *   branding-mod/            mc12111, mc262, mc263   Fabric menu   (Gradle 9: JDK 25)
 *   branding-mod-nf/         espectral-brand         NeoForge      (Gradle 8: JDK 21-24)
 *   branding-mod-forge189/   espectral-menu 1.8.9    Forge         (Gradle 8: JDK 21-24
 *                            to RUN Gradle; the Java 8 toolchain it compiles
 *                            with is resolved by the wrapper/loom itself)
 *
 * The three trees share no outputs and have their own wrappers/daemons, so
 * they build concurrently by default; every child line is tagged
 * ([fabric], [neoforge], [forge189]) to keep the interleaved log readable.
 *
 * Run: npm run deploy:mod
 *   node scripts/deploy-mod.mjs               build everything with tests (default)
 *   node scripts/deploy-mod.mjs --fast        run `assemble` instead of `build`
 *   node scripts/deploy-mod.mjs --skip-tests  alias of --fast
 *   node scripts/deploy-mod.mjs --serial      build the trees one after another
 *   node scripts/deploy-mod.mjs --only=fabric|nf|forge189   rebuild one tree only
 * ========================================================================== */
import { spawn, spawnSync } from 'node:child_process';
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
// `sh gradlew <tasks>`: runs the wrapper script without needing its exec bit
// (git does not always keep it), and the tasks reach Gradle as arguments.
const gradleArgv = isWin ? ['cmd.exe', '/c', 'gradlew.bat'] : ['sh', 'gradlew'];

// ── 1. Flags ──────────────────────────────────────────────────────────────
const argv = process.argv.slice(2);
// `assemble` skips `check`/tests and still produces the same final jars the
// copy step picks up: loom wires assemble.dependsOn(remapJar) for the remapped
// mc12111 build, mc262/mc263 ship the plain jar, NeoForge's java-library jar is
// part of assemble, and forge189 pins tasks.assemble.dependsOn(remapJar)
// (branding-mod-forge189/build.gradle.kts:221).
const FAST = argv.includes('--fast') || argv.includes('--skip-tests');
const SERIAL = argv.includes('--serial');
const onlyArg = argv.find((a) => a.startsWith('--only='));
const ONLY = onlyArg == null ? null : onlyArg.slice('--only='.length);
const TREE_IDS = ['fabric', 'nf', 'forge189'];
if (ONLY !== null && !TREE_IDS.includes(ONLY)) fail(`--only must be one of ${TREE_IDS.join('|')} (got "${ONLY}")`);
const BUILD_TASK = FAST ? 'assemble' : 'build';

// `task` calls run a plain process (manifest regeneration only, after builds).
function run(cwd, [cmd, ...args], env) {
  const r = spawnSync(cmd, args, {
    cwd, stdio: 'inherit', env: env ? { ...process.env, ...env } : process.env,
  });
  if (r.error) fail(`${cmd} failed to start in ${cwd}: ${r.error.message}`);
  if (r.status !== 0) fail(`${cmd} ${args.join(' ')} exited ${r.status} (cwd: ${cwd})`);
}

// ── 2. Per-tree JAVA_HOME ─────────────────────────────────────────────────
const javaBin = (home) => path.join(home, 'bin', isWin ? 'java.exe' : 'java');

const javaVersions = new Map();
function probeJava(bin) {
  if (!javaVersions.has(bin)) {
    const r = spawnSync(bin, ['-version'], { encoding: 'utf8' });
    const m = /version "(\d+)/.exec(`${r.stderr}${r.stdout}`);
    javaVersions.set(bin, m ? parseInt(m[1], 10) : null);
  }
  return javaVersions.get(bin);
}

// The wrapper decides which Gradle runs, and each Gradle line accepts a
// bounded JVM range (docs.gradle.org/current/userguide/compatibility.html):
// 9.1+ starts on Java 25, 8.14 on 24, 8.8 on 22. Reading each tree's wrapper
// keeps the rule correct when a tree bumps Gradle.
function wrapperGradle(dir) {
  const props = path.join(dir, 'gradle', 'wrapper', 'gradle-wrapper.properties');
  if (!existsSync(props)) return null;
  const m = /gradle-(\d+(?:\.\d+)+)-/.exec(readFileSync(props, 'utf8'));
  if (!m) return null;
  const [major, minor, patch = 0] = m[1].split('.').map((n) => parseInt(n, 10));
  return { major, minor, patch, text: m[1] };
}

function maxJavaFor(gv) {
  if (!gv) return null;
  if (gv.major >= 10) return 99;
  if (gv.major === 9) return gv.minor >= 1 ? 25 : 24;
  if (gv.major === 8) {
    if (gv.minor >= 14) return 24;
    if (gv.minor >= 10) return 23;
    if (gv.minor >= 8) return 22;
    if (gv.minor >= 5) return 21;
    return 20;
  }
  return 17;
}

const JDK_ROOTS = isWin
  ? ['C:/Program Files/Eclipse Adoptium', 'C:/Program Files/Java', 'C:/Program Files/Microsoft']
  : ['/usr/lib/jvm', '/opt', '/usr/java'];

// Installs are not always direct children of a root (/opt/<product>/jdk-25),
// so one extra level is scanned; below that only JDK-looking names are worth
// entering, which keeps the /opt walk cheap.
function discoverJdks(dir, depth, out) {
  if (depth > 2) return;
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const ent of entries) {
    if (!ent.isDirectory() && !ent.isSymbolicLink()) continue;
    const home = path.join(dir, ent.name);
    if (existsSync(javaBin(home))) {
      out.push(home);
      continue;
    }
    if (depth >= 1 && !/(jdk|java)/i.test(ent.name)) continue;
    discoverJdks(home, depth + 1, out);
  }
}

let jdkList = null;
function installedJdks() {
  if (!jdkList) {
    jdkList = [];
    for (const root of JDK_ROOTS) {
      if (existsSync(root)) discoverJdks(root, 0, jdkList);
    }
  }
  return jdkList;
}

function findJdk(min, max) {
  let best = null;
  for (const home of installedJdks()) {
    const version = probeJava(javaBin(home));
    if (version == null || version < min || version > max) continue;
    if (!best || version > best.version) best = { home, version };
  }
  return best;
}

// A single shared override is wrong: branding-mod's Gradle 9.5 must run on
// JDK 25 so Loom can build the mc262/mc263 subprojects (it refuses a daemon
// below the MC version's Java), while the Gradle 8.x wrappers cannot start on
// Java 25 at all. Ambient JVM wins when it is inside the tree's range;
// otherwise the highest installed JDK in [minJava, max] is used.
function gradleJavaHome(tree) {
  const gv = wrapperGradle(tree.dir);
  const max = maxJavaFor(gv) ?? 99;
  const jh = process.env.JAVA_HOME;
  const effective = jh ? probeJava(javaBin(jh)) : probeJava('java');
  const source = jh ? 'JAVA_HOME' : 'java on PATH';
  if (effective != null && effective >= tree.minJava && effective <= max) return null;
  const best = findJdk(tree.minJava, max);
  if (!best) {
    log(`warning: ${tree.rel}: ${source} is Java ${effective ?? 'unknown'} and wrapper Gradle ${gv ? gv.text : '?'} needs ${tree.minJava}-${max}; no matching JDK found, letting Gradle choose`);
    return null;
  }
  log(`${tree.rel}: ${source} is Java ${effective ?? 'unknown'}, wrapper Gradle ${gv ? gv.text : '?'} needs ${tree.minJava}-${max} — using ${best.home}`);
  return best.home;
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

// ── 3. Trees ──────────────────────────────────────────────────────────────
// minJava is a project fact, not a Gradle rule: mc262/mc263 declare Java 25
// toolchains (branding-mod/gradle.properties), while nf targets 1.21.1/Java 21
// and forge189 compiles at Java 8 under a 21-capable Gradle.
const TREES = [
  {
    id: 'fabric',
    tag: 'fabric',
    rel: 'branding-mod',
    dir: path.join(ROOT, 'branding-mod'),
    required: true,
    minJava: 25,
    startMessage: `building espectral-menu jars (launcher ${VERSION})…`,
    tasks: [':mc12111:' + BUILD_TASK, ':mc262:' + BUILD_TASK, ':mc263:' + BUILD_TASK],
    jars: [
      [path.join('branding-mod', 'mc12111', 'build', 'libs'), 'espectral-menu-1.21.11-'],
      [path.join('branding-mod', 'mc262', 'build', 'libs'), 'espectral-menu-26.2-'],
      [path.join('branding-mod', 'mc263', 'build', 'libs'), 'espectral-menu-26.3-'],
    ],
    keepPrefixes: ['espectral-menu-1.21.11-', 'espectral-menu-26.2-', 'espectral-menu-26.3-'],
  },
  {
    id: 'nf',
    tag: 'neoforge',
    rel: 'branding-mod-nf',
    dir: path.join(ROOT, 'branding-mod-nf'),
    required: false,
    minJava: 21,
    startMessage: 'building espectral-brand (NeoForge) jar…',
    skipMessage: 'branding-mod-nf is not buildable from this repo — keeping bundled espectral-brand jar',
    tasks: [BUILD_TASK],
    jars: [[path.join('branding-mod-nf', 'build', 'libs'), 'espectral-brand-1.21.1-']],
    keepPrefixes: ['espectral-brand-'],
  },
  {
    id: 'forge189',
    tag: 'forge189',
    rel: 'branding-mod-forge189',
    dir: path.join(ROOT, 'branding-mod-forge189'),
    required: false,
    minJava: 21,
    startMessage: 'building espectral-menu (Forge 1.8.9) jar — Gradle needs JDK 21 to run…',
    skipMessage: 'branding-mod-forge189 is not buildable from this repo — keeping bundled 1.8.9 jar',
    tasks: [BUILD_TASK],
    jars: [[path.join('branding-mod-forge189', 'build', 'libs'), 'espectral-menu-1.8.9-']],
    keepPrefixes: ['espectral-menu-1.8.9-'],
    extraArgs: () => {
      const j8 = jdk8Home();
      if (!j8) log('warning: no JDK 8 found (data/runtimes/jdk-8 or ESPECTRAL_JDK8) — the wrapper will try to provision one');
      return j8 ? [`-Porg.gradle.java.installations.paths=${j8}`] : [];
    },
  },
];

// ── 4. Build ──────────────────────────────────────────────────────────────
// Gradle's --console=plain output is line-oriented; tag whole lines so three
// concurrent feeds stay attributable without splitting mid-line.
function spawnGradle(tree, env, extraArgs) {
  const args = [...gradleArgv.slice(1), ...tree.tasks, ...extraArgs, '--console=plain'];
  const tag = `[${tree.tag}] `;
  return new Promise((resolve) => {
    const child = spawn(gradleArgv[0], args, {
      cwd: tree.dir,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const pipe = (stream) => {
      let partial = '';
      stream.setEncoding('utf8');
      stream.on('data', (chunk) => {
        partial += chunk;
        const lines = partial.split('\n');
        partial = lines.pop();
        for (const line of lines) console.log(tag + line.replace(/\r$/, ''));
      });
      stream.on('end', () => {
        if (partial) console.log(tag + partial.replace(/\r$/, ''));
      });
    };
    pipe(child.stdout);
    pipe(child.stderr);
    let settled = false;
    const settle = (status) => {
      if (settled) return;
      settled = true;
      resolve(status);
    };
    child.on('error', (err) => {
      console.log(`${tag}error: ${err.message}`);
      settle(127);
    });
    child.on('close', (code, signal) => {
      if (code == null && signal) console.log(`${tag}terminated by ${signal}`);
      settle(code ?? 127);
    });
  });
}

async function buildOne(tree) {
  const wrapper = path.join(tree.dir, isWin ? 'gradlew.bat' : 'gradlew');
  if (!existsSync(wrapper)) {
    if (tree.required) fail(`${tree.rel} has no ${isWin ? 'gradlew.bat' : 'gradlew'} — cannot build the required menu jars`);
    log(tree.skipMessage);
    return { status: null, attempted: false };
  }
  const home = gradleJavaHome(tree);
  const env = home ? { ...process.env, JAVA_HOME: home } : process.env;
  log(tree.startMessage);
  const extraArgs = tree.extraArgs ? tree.extraArgs() : [];
  return { status: await spawnGradle(tree, env, extraArgs), attempted: true };
}

const toBuild = TREES.filter((tree) => !ONLY || tree.id === ONLY);
log(`building ${toBuild.map((tree) => tree.rel).join(', ')} with \`${BUILD_TASK}\` (${SERIAL ? 'serial' : 'parallel'})…`);

const statuses = new Map();
if (SERIAL) {
  for (const tree of toBuild) statuses.set(tree.id, await buildOne(tree));
} else {
  // All trees settle before any failure is raised, so a fabric failure never
  // leaves the other two wrappers writing to the log after the exit.
  const results = await Promise.all(toBuild.map((tree) => buildOne(tree).then((result) => [tree.id, result])));
  for (const [id, result] of results) statuses.set(id, result);
}

const fabricResult = statuses.get('fabric');
if (fabricResult && fabricResult.attempted && fabricResult.status !== 0) {
  fail(`branding-mod build failed (exit ${fabricResult.status ?? 127}) — see the [fabric] output above`);
}

// ── 5. Copy fresh jars into assets/branding, drop stale ones ─────────────
mkdirSync(BRANDING_DIR, { recursive: true });
const keep = new Set();
const rebuilt = new Set();
for (const tree of toBuild) {
  const { status, attempted } = statuses.get(tree.id);
  if (!attempted) continue;
  if (status !== 0) {
    log(`warning: ${tree.id} build failed (exit ${status ?? 127}) — keeping bundled jar (${tree.id} changes, if any, are NOT in this deploy)`);
    continue;
  }
  const found = tree.jars.map(([dir, prefix]) => findJar(path.join(ROOT, dir), prefix));
  const missing = found.map((jar, i) => (jar ? null : `${tree.jars[i][1]}${VERSION}.jar`)).filter(Boolean);
  if (missing.length && tree.required) {
    fail(`${missing.join(', ')} not found after build — check the gradle output above`);
  }
  if (missing.length) {
    log(`warning: no ${missing.join(', ')} produced — keeping bundled jar (${tree.id} changes, if any, are NOT in this deploy)`);
    continue;
  }
  rebuilt.add(tree.id);
  for (const jar of found) {
    copyFileSync(jar, path.join(BRANDING_DIR, path.basename(jar)));
    keep.add(path.basename(jar));
    log(`bundled ${path.basename(jar)}`);
  }
}
// Never delete a bundled jar this run did not rebuild — a skipped (--only) or
// failed/unbuildable tree must keep its existing coverage.
for (const tree of TREES) {
  if (rebuilt.has(tree.id)) continue;
  for (const f of readdirSync(BRANDING_DIR)) {
    if (!JAR_RE.test(f) || keep.has(f)) continue;
    if (!tree.keepPrefixes.some((prefix) => f.startsWith(prefix))) continue;
    keep.add(f);
    log(`keeping bundled ${f} (${tree.id} not rebuilt)`);
  }
}
for (const f of readdirSync(BRANDING_DIR)) {
  if (!JAR_RE.test(f) || keep.has(f)) continue;
  rmSync(path.join(BRANDING_DIR, f));
  log(`removed stale ${f}`);
}

// ── 6. Regenerate the manifest ────────────────────────────────────────────
log('regenerating branding.json…');
run(ROOT, [process.execPath, path.join(ROOT, 'scripts', 'update-branding-manifest.mjs')]);

// ── 7. Mirror into the installed desktop client ───────────────────────────
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

// ── 8. Done ───────────────────────────────────────────────────────────────
log('done. Restart Espectral Client (branding.json is read at engine startup), then launch the game.');
