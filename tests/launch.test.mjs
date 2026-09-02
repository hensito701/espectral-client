import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir BEFORE importing the engine so module evaluation sees it
// too (same pattern as tests/config.test.mjs).
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-launch-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const {
  memoryArgs,
  buildArgv,
  spawnJava,
  childProcess,
  DEFAULT_MEMORY_MB,
  MARKER_RE,
  FIXED_JVM_ARGS,
  bootFlags,
  effectiveMemoryMb,
  resolveAccountFor,
} = await import('../src/engine/launch.mjs');
const { loadConfig, saveConfig } = await import('../src/engine/config.mjs');
// presenceForInstance lives in the route module; importing it is side-effect
// free (server bootstrap lives in cli.mjs).
const { presenceForInstance } = await import('../src/engine/routes/launch.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

/** A minimal resolveLaunch result with everything buildArgv reads. */
function makeResolved(overrides = {}) {
  return {
    java: { path: '/jdk/bin/java.exe', major: 25, build: '25.0.4+7' },
    account: {
      username: 'Tester',
      uuid: '00000000-0000-3000-8000-000000000000',
      accessToken: 'tok',
      userType: 'mojang',
    },
    version: { id: '1.21.11', mainClass: 'net.minecraft.client.main.Main', assetIndexId: '1.21' },
    gameDir: '/games/testinst',
    assetsDir: '/data/assets',
    nativesDir: '/games/testinst/natives',
    classpath: ['/data/libraries/a.jar', '/data/libraries/b.jar'],
    memoryMb: 4096,
    mode: 'normal',
    aotCachePath: '/data/cache/aot/key/game.aot',
    aotCacheExists: false,
    warnings: [], // buildArgv pushes AOT warnings into this array
    loader: null,
    ...overrides,
  };
}

test('memoryArgs emits G/M labels and falls back on invalid input', () => {
  assert.deepEqual(memoryArgs(3072), ['-Xms3G', '-Xmx3G']);
  assert.deepEqual(memoryArgs(4096), ['-Xms4G', '-Xmx4G']);
  assert.deepEqual(memoryArgs(2048), ['-Xms2G', '-Xmx2G']);
  assert.deepEqual(memoryArgs(1024), ['-Xms1G', '-Xmx1G']);
  // Sub-G values that are not a whole GiB keep an M label.
  assert.deepEqual(memoryArgs(1500), ['-Xms1500M', '-Xmx1500M']);
  // Invalid inputs (non-numeric, out-of-range, fractional) fall back to
  // DEFAULT_MEMORY_MB (3072 -> 3G) instead of emitting garbage labels.
  for (const bad of [undefined, null, 'abc', NaN, 0, -512, 1.5]) {
    assert.deepEqual(memoryArgs(bad), ['-Xms3G', '-Xmx3G'], `input ${bad}`);
  }
  assert.equal(DEFAULT_MEMORY_MB, 3072);
});

test('buildArgv argv[0] is the java executable path (H5 contract)', () => {
  const argv = buildArgv({ name: 't' }, makeResolved());
  assert.equal(argv[0], '/jdk/bin/java.exe');
});

test('buildArgv full argv shape: memory, fixed set, natives, -cp, main class, program args', () => {
  const resolved = makeResolved();
  const argv = buildArgv({ name: 't' }, resolved);
  // memory args first (Xms=Xmx)
  assert.deepEqual(argv.slice(1, 3), ['-Xms4G', '-Xmx4G']);
  // fixed arg set immediately after (java major 25 >= 21)
  assert.deepEqual(argv.slice(3, 3 + FIXED_JVM_ARGS.length), FIXED_JVM_ARGS);
  // natives dir right after the fixed set
  const nativesIdx = 3 + FIXED_JVM_ARGS.length;
  assert.equal(argv[nativesIdx], '-Djava.library.path=' + resolved.nativesDir);
  // -cp carries the joined classpath, then the main class, then program args
  const cpIdx = argv.indexOf('-cp');
  assert.ok(cpIdx > nativesIdx, '-cp comes after the natives flag');
  assert.equal(argv[cpIdx + 1], resolved.classpath.join(path.delimiter));
  assert.equal(argv[cpIdx + 2], 'net.minecraft.client.main.Main');
  assert.deepEqual(argv.slice(cpIdx + 3), [
    '--username', 'Tester',
    '--version', '1.21.11',
    '--gameDir', resolved.gameDir,
    '--assetsDir', resolved.assetsDir,
    '--assetIndex', '1.21',
    '--uuid', resolved.account.uuid,
    '--accessToken', 'tok',
    '--userType', 'mojang',
    '--versionType', 'release',
  ]);
});

test('buildArgv AOT mode branches: train/aot/missing cache/pre-25 runtime', () => {
  // train on JDK 25: -XX:AOTCacheOutput, no read-side flags
  const train = buildArgv({ name: 't' }, makeResolved({ mode: 'train' }));
  assert.ok(train.includes('-XX:AOTCacheOutput=/data/cache/aot/key/game.aot'));
  assert.equal(train.includes('-XX:AOTCache='), false);
  assert.equal(train.includes('-Xlog:aot='), false);
  // aot with an existing cache: -XX:AOTCache + relative proof log
  const aot = buildArgv({ name: 't' }, makeResolved({ mode: 'aot', aotCacheExists: true }));
  assert.ok(aot.includes('-XX:AOTCache=/data/cache/aot/key/game.aot'));
  assert.ok(aot.includes('-Xlog:aot=info:file=aot-%p.log'));
  // aot with a missing cache: no flag, warning instead of a dead flag
  const aotMissing = makeResolved({ mode: 'aot', aotCacheExists: false });
  const argvMissing = buildArgv({ name: 't' }, aotMissing);
  assert.equal(argvMissing.includes('-XX:AOTCache='), false);
  assert.equal(argvMissing.includes('-Xlog:aot='), false);
  assert.ok(aotMissing.warnings.some((w) => w.includes('AOT cache') && w.includes('not found')));
  // normal: neither AOT flag
  const normal = buildArgv({ name: 't' }, makeResolved({ mode: 'normal' }));
  assert.equal(normal.includes('-XX:AOTCache'), false);
  assert.equal(normal.includes('-Xlog:aot='), false);
  // train on a pre-25 runtime: no flag, warning; fixed set also skipped (< 21)
  const old = makeResolved({
    mode: 'train',
    java: { path: '/jdk17/bin/java.exe', major: 17, build: '17.0.12+7' },
  });
  const argvOld = buildArgv({ name: 't' }, old);
  assert.equal(argvOld.includes('-XX:AOTCache'), false);
  assert.equal(argvOld.includes('--add-opens'), false);
  assert.ok(old.warnings.some((w) => w.includes('AOT training requires a JDK 25-tier runtime')));
});

test('buildArgv skips -XX:AOTCache when the cache is stale (JEP 483 classpath drift)', () => {
  // A cache whose recorded classpath identity drifted is REFUSED by the JVM
  // ("shared class paths mismatch" -> "Unable to map shared spaces"), so
  // passing the flag only buys a doomed mapping attempt and an error-level log
  // on every boot. The launcher must detect it and boot clean instead.
  const stale = makeResolved({ mode: 'aot', aotCacheExists: true, aotCacheStale: true });
  const argv = buildArgv({ name: 't' }, stale);
  assert.equal(argv.some((a) => a.startsWith('-XX:AOTCache=')), false);
  assert.equal(argv.some((a) => a.startsWith('-Xlog:aot=')), false);
  assert.ok(
    stale.warnings.some((w) => w.includes('no longer matches the classpath')),
    'a stale cache must be reported, not silently ignored'
  );
  // A fresh cache still gets the flag — the gate must not be unconditional.
  const fresh = makeResolved({ mode: 'aot', aotCacheExists: true, aotCacheStale: false });
  assert.ok(buildArgv({ name: 't' }, fresh).includes('-XX:AOTCache=/data/cache/aot/key/game.aot'));
});

test('spawnJava forwards argv.slice(1) and never duplicates the java path (H5)', (t) => {
  const javaPath = '/jdk/bin/java.exe';
  const argv = [javaPath, '-Xms3G', '-Xmx3G', 'net.minecraft.client.main.Main', '--username', 'Tester'];
  const opts = { cwd: '/games/testinst' };
  const mocked = t.mock.method(childProcess, 'spawn', () => ({ pid: 42 }));
  const child = spawnJava(javaPath, argv, opts);
  assert.equal(mocked.mock.calls.length, 1);
  const [exe, spawnArgs, spawnOpts] = mocked.mock.calls[0].arguments;
  assert.equal(exe, javaPath);
  assert.deepEqual(spawnArgs, argv.slice(1));
  assert.equal(spawnArgs.includes(javaPath), false, 'java path must not appear in the spawned args');
  assert.deepEqual(spawnOpts, opts);
  assert.equal(child.pid, 42);
});

test('M2: modern-format versions are multiplayer; snapshots are singleplayer', () => {
  // Regression pin for FINDINGS M2: the old rule negated the 3-group release
  // regex, so every modern-format launch (26.2, 25.1, 21.0) was mislabeled
  // "Jugando singleplayer". Singleplayer is now detected explicitly — only a
  // snapshot build has no server focus.
  assert.equal(presenceForInstance({ name: 'modern', version: '26.2' }).state, 'Jugando servidor multijugador');
  assert.equal(presenceForInstance({ name: 'modern', version: '25.1' }).state, 'Jugando servidor multijugador');
  assert.equal(presenceForInstance({ name: 'modern', version: '21.0' }).state, 'Jugando servidor multijugador');
  assert.equal(presenceForInstance({ name: 'classic', version: '1.21.11' }).state, 'Jugando servidor multijugador');
  assert.equal(presenceForInstance({ name: 'classic', version: '1.21' }).state, 'Jugando servidor multijugador');
  assert.equal(presenceForInstance({ name: 'snap', version: '24w11a' }).state, 'Jugando singleplayer');
  assert.equal(presenceForInstance({ name: 'snap', version: '25w11a' }).state, 'Jugando singleplayer');
  // a known Espectral host in the name still wins over the version rule
  assert.equal(presenceForInstance({ name: 'uhc2', version: '26.2' }).state, 'Jugando en uhc2.espectral.es');
});

test('buildArgv memory fallback order and MARKER_RE', () => {
  // resolved.memoryMb wins, then instance.memory_mb, then DEFAULT_MEMORY_MB.
  const argvDefault = buildArgv({ name: 't' }, makeResolved({ memoryMb: undefined }));
  assert.deepEqual(argvDefault.slice(1, 3), ['-Xms3G', '-Xmx3G']);
  const argvInstance = buildArgv({ name: 't', memory_mb: 2048 }, makeResolved({ memoryMb: undefined }));
  assert.deepEqual(argvInstance.slice(1, 3), ['-Xms2G', '-Xmx2G']);
  const argvResolved = buildArgv({ name: 't', memory_mb: 2048 }, makeResolved());
  assert.deepEqual(argvResolved.slice(1, 3), ['-Xms4G', '-Xmx4G']);
  // MARKER_RE pins the exact menu-marker line the launch watcher listens for.
  assert.equal(MARKER_RE.test('Sound engine started'), true);
  assert.equal(MARKER_RE.test('[Render thread/INFO]: Sound engine started'), true);
  assert.equal(MARKER_RE.test('Starting minecraft server'), false);
});

test('bootFlags: safe pair always, C1 only when fast_boot is on (instance wins, then config)', () => {
  saveConfig({ fast_boot: false });
  // default: safe pair only (javaMajor 25 -> includes -Xverify:none)
  assert.deepEqual(bootFlags({ name: 't' }, 25), ['-XX:-UsePerfData', '-Xverify:none']);
  // instance-level opt-in adds C1
  assert.deepEqual(bootFlags({ name: 't', fast_boot: true }, 25), [
    '-XX:-UsePerfData', '-Xverify:none', '-XX:TieredStopAtLevel=1',
  ]);
  // instance-level explicit OFF beats a config ON
  saveConfig({ fast_boot: true });
  assert.deepEqual(bootFlags({ name: 't', fast_boot: false }, 25), ['-XX:-UsePerfData', '-Xverify:none']);
  // no instance field -> config fallback
  assert.deepEqual(bootFlags({ name: 't' }, 25), [
    '-XX:-UsePerfData', '-Xverify:none', '-XX:TieredStopAtLevel=1',
  ]);
  saveConfig({ fast_boot: false });
});

test('buildArgv: boot flags ride with the JDK21+ fixed set and skip legacy tiers', () => {
  const argv = buildArgv({ name: 't' }, makeResolved());
  const nativesIdx = argv.indexOf('-Djava.library.path=');
  assert.ok(argv.includes('-XX:-UsePerfData'), 'safe pair present on modern tiers');
  assert.ok(argv.includes('-Xverify:none'), 'safe pair present on modern tiers');
  assert.equal(argv.includes('-XX:TieredStopAtLevel=1'), false, 'C1 off by default');
  // legacy tier (< 21): neither the fixed set nor the boot flags appear
  const old = makeResolved({
    java: { path: '/jdk17/bin/java.exe', major: 17, build: '17.0.12+7' },
  });
  const argvOld = buildArgv({ name: 't' }, old);
  assert.equal(argvOld.includes('-XX:-UsePerfData'), false);
  assert.equal(argvOld.includes('-Xverify:none'), false);
});
test('buildArgv token coercion: missing accessToken -> 0 and never undefined', () => {
  // Account lacking accessToken (offline stored shape without token)
  const resolved = makeResolved({
    account: { username: 'NoTok', uuid: '00000000-0000-3000-8000-000000000001', userType: 'mojang' },
  });
  const argv = buildArgv({ name: 't' }, resolved);
  const idx = argv.indexOf('--accessToken');
  assert.ok(idx !== -1, '--accessToken present');
  assert.equal(argv[idx + 1], '0', 'missing token coerced to 0');
  assert.equal(argv.includes('undefined'), false, 'never emits literal undefined');
  // Also check substitution map via fallback program args
  assert.equal(argv.includes('--userType'), true);
  // When token_kind is msa but userType absent, should derive msa
  const resolvedMsa = makeResolved({
    account: { username: 'MsaTok', uuid: '00000000-0000-3000-8000-000000000002', token_kind: 'msa' },
  });
  const argvMsa = buildArgv({ name: 't' }, resolvedMsa);
  const utIdx = argvMsa.indexOf('--userType');
  assert.equal(argvMsa[utIdx + 1], 'msa', 'derived from token_kind');
  const tokIdx2 = argvMsa.indexOf('--accessToken');
  assert.equal(argvMsa[tokIdx2 + 1], '0');
});

test('effectiveMemoryMb clamp math', () => {
  const GB = 1024 * 1024 * 1024;
  // 8GB system, 60% = 4915 MB
  assert.equal(effectiveMemoryMb(8192, 8 * GB), 4915);
  assert.equal(effectiveMemoryMb(4096, 8 * GB), 4096, 'under cap not clamped');
  // Exact cap boundary
  assert.equal(effectiveMemoryMb(4915, 8 * GB), 4915);
  assert.equal(effectiveMemoryMb(4916, 8 * GB), 4915, 'just over cap clamped');
  // 32GB system: 60% = 19660 floor (32768 *0.6)
  assert.equal(effectiveMemoryMb(20000, 32 * GB), 19660);
  assert.equal(effectiveMemoryMb(1000, 32 * GB), 1000);
  // Invalid inputs passthrough (return original)
  assert.equal(effectiveMemoryMb(null, 8 * GB), null);
  assert.equal(effectiveMemoryMb(0, 8 * GB), 0);
  assert.equal(effectiveMemoryMb('bad', 8 * GB), 'bad');
});

test('effectiveMemoryMb clamp via buildArgv pushes warning', () => {
  const totalBytes = os.totalmem();
  const cap = Math.floor(totalBytes / (1024 * 1024) * 0.6);
  const over = cap + 5000;
  const resolved = makeResolved({ memoryMb: over });
  const beforeWarnings = resolved.warnings.length;
  const argv = buildArgv({ name: 't' }, resolved);
  // Should have clamped and pushed warning
  assert.equal(resolved.warnings.length, beforeWarnings + 1);
  assert.ok(resolved.warnings[resolved.warnings.length - 1].includes(`memory_mb ${over} clamped to ${cap}`));
  // Argv should contain clamped value (cap label)
  const expectedLabel = cap % 1024 === 0 ? `${cap / 1024}G` : `${cap}M`;
  assert.ok(argv.includes(`-Xms${expectedLabel}`) || argv.includes(`-Xmx${expectedLabel}`));
  // Request under cap should not push warning
  const under = Math.min(4096, cap - 1 > 0 ? cap - 1 : 1024);
  const resolved2 = makeResolved({ memoryMb: under });
  buildArgv({ name: 't' }, resolved2);
  assert.equal(resolved2.warnings.length, 0, 'no warning when under cap');
});

test('bootFlags JDK27 gate', () => {
  saveConfig({ fast_boot: false });
  // javaMajor 27 -> no -Xverify:none
  assert.deepEqual(bootFlags({ name: 't' }, 27), ['-XX:-UsePerfData']);
  assert.equal(bootFlags({ name: 't' }, 27).includes('-Xverify:none'), false);
  // javaMajor 25 -> present
  assert.ok(bootFlags({ name: 't' }, 25).includes('-Xverify:none'));
  // javaMajor 26 -> present (boundary)
  assert.ok(bootFlags({ name: 't' }, 26).includes('-Xverify:none'));
  // fast_boot with 27 still no verify, but C1 present
  assert.deepEqual(bootFlags({ name: 't', fast_boot: true }, 27), ['-XX:-UsePerfData', '-XX:TieredStopAtLevel=1']);
  // no javaMajor (undefined) -> include (treat as <27 for backwards compat)
  assert.ok(bootFlags({ name: 't' }).includes('-Xverify:none'));
  saveConfig({ fast_boot: false });
});

test('resolveAccountFor offline branch', async () => {
  const warnings = [];
  const stored = { username: 'OfflineUser', uuid: '11111111-1111-3111-8111-111111111111', token_kind: 'offline' };
  const res = await resolveAccountFor(stored, (m) => warnings.push(m));
  assert.equal(res.username, 'OfflineUser');
  assert.equal(res.uuid, stored.uuid);
  assert.equal(res.accessToken, '0');
  assert.equal(res.userType, 'mojang');
  assert.equal(warnings.length, 0, 'no warning for offline');
  // Ensure argv coercion also works for offline stored shape via resolveAccountFor result
  const resolved = makeResolved({ account: res });
  const argv = buildArgv({ name: 't' }, resolved);
  assert.ok(argv.includes('OfflineUser'));
});

test('resolveAccountFor msa without refresh token -> offline fallback without network', async () => {
  const warnings = [];
  // Stored MSA account with no refresh token and no cached token -> ensureMinecraftToken will fail fast without fetch
  const stored = {
    username: 'MsaNoRefresh',
    uuid: '22222222-2222-3222-8222-222222222222',
    token_kind: 'msa',
    microsoft: {},
  };
  // Ensure we do not hit network: stub fetch to throw if called
  const origFetch = globalThis.fetch;
  let fetchCalled = false;
  globalThis.fetch = async () => { fetchCalled = true; throw new Error('network should not be hit'); };
  try {
    const res = await resolveAccountFor(stored, (m) => warnings.push(m));
    assert.equal(res.username, 'MsaNoRefresh');
    assert.equal(res.accessToken, '0');
    assert.equal(res.userType, 'mojang', 'fallback to mojang');
    assert.ok(warnings.length > 0, 'warning emitted for failed MSA');
    assert.ok(warnings[0].includes('MsaNoRefresh'), 'warning mentions username');
    assert.equal(fetchCalled, false, 'must NOT hit network with empty microsoft fixture');
  } finally {
    globalThis.fetch = origFetch;
  }
});
