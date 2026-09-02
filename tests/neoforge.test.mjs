import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  readLocalVersionJson,
  resolveVersionChain,
  mergeArgs,
  dedupeByName,
} from '../src/engine/resolver.mjs';
import {
  expandJvmArgs,
  expandGameArgs,
  substitutePlaceholders,
  buildArgv,
} from '../src/engine/launch.mjs';
import {
  neoforgeVersionId,
  neoforgeJsonPath,
  readNeoForgeJson,
  isNeoForgeInstalled,
} from '../src/engine/neoforge.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES = path.join(__dirname, 'fixtures');

function fixture(name) {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES, name), 'utf8'));
}

// --- neoforge version identity ---

test('neoforgeVersionId: neoforge-<version>', () => {
  assert.equal(neoforgeVersionId('21.1.242'), 'neoforge-21.1.242');
});

test('readNeoForgeJson: reads the persisted profile from the data dir (temp)', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-nf-'));
  const old = process.env.ESPECTRAL_DATA_DIR;
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    assert.equal(readNeoForgeJson('21.1.242'), null, 'not installed yet');
    assert.equal(isNeoForgeInstalled('21.1.242'), false);
    fs.mkdirSync(path.dirname(neoforgeJsonPath('21.1.242')), { recursive: true });
    fs.copyFileSync(
      path.join(FIXTURES, 'neoforge-21.1.242-version.json'),
      neoforgeJsonPath('21.1.242')
    );
    const json = readNeoForgeJson('21.1.242');
    assert.ok(json, 'installed profile must parse');
    assert.equal(json.id, 'neoforge-21.1.242');
    assert.equal(json.libraries.length, 47);
    assert.equal(isNeoForgeInstalled('21.1.242'), true);
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('readLocalVersionJson: reads local-only version jsons (temp)', async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-lv-'));
  const old = process.env.ESPECTRAL_DATA_DIR;
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const id = 'neoforge-21.1.242';
    const dir = path.join(tmp, 'versions', id);
    fs.mkdirSync(dir, { recursive: true });
    fs.copyFileSync(path.join(FIXTURES, 'neoforge-21.1.242-version.json'), path.join(dir, `${id}.json`));
    const json = await readLocalVersionJson(id);
    assert.equal(json.id, id);
    assert.equal(await readLocalVersionJson('does-not-exist'), null);
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

// --- inheritance chain merge ---

test('resolveVersionChain: merges the neoforge profile over the vanilla parent', async () => {
  const child = fixture('neoforge-21.1.242-version.json');
  const parent = fixture('vanilla-1.21.1-mini.json');
  const merged = await resolveVersionChain(child, { getParent: async () => parent });

  assert.equal(merged.id, 'neoforge-21.1.242');
  assert.equal(merged.mainClass, 'cpw.mods.bootstraplauncher.BootstrapLauncher');
  assert.equal(merged.assetIndex.id, '17'); // from parent
  assert.equal(merged.javaVersion.majorVersion, 21); // from parent
  // 47 child libs + 4 parent libs, minus the gson name collision (the child
  // profile bundles com.google.code.gson:gson:2.10.1 itself) = 50
  assert.equal(merged.libraries.length, 50);
  // child-first ordering: the first library is a neoforge loader lib
  assert.match(merged.libraries[0].name, /^cpw\.mods:|^net\.neoforged/);
  // parent's client download survives (merged json drives installLibraries)
  assert.ok(merged.downloads?.client?.url, 'client download must come from the parent');
});

test('resolveVersionChain: jvm args merge child-first with -p and -cp present', async () => {
  const merged = await resolveVersionChain(fixture('neoforge-21.1.242-version.json'), {
    getParent: async () => fixture('vanilla-1.21.1-mini.json'),
  });
  const jvm = merged.arguments.jvm;
  assert.ok(jvm.includes('-p'), 'module path flag');
  assert.ok(jvm.includes('-cp'), 'classpath flag from the parent');
  assert.ok(jvm.some((a) => typeof a === 'string' && a.startsWith('-DlibraryDirectory=')));
  assert.ok(
    jvm.some((a) => typeof a === 'string' && a.includes('${classpath_separator}')),
    'module path keeps its placeholder for buildArgv substitution'
  );
  // child jvm args come first
  assert.equal(jvm[0], '-Djava.net.preferIPv6Addresses=system');
  // rule objects survive the merge
  assert.ok(jvm.some((a) => a && typeof a === 'object' && Array.isArray(a.rules)));
});

test('resolveVersionChain: game args merge child-first (fml args before auth args)', async () => {
  const merged = await resolveVersionChain(fixture('neoforge-21.1.242-version.json'), {
    getParent: async () => fixture('vanilla-1.21.1-mini.json'),
  });
  const game = merged.arguments.game;
  assert.equal(game[0], '--fml.neoForgeVersion');
  assert.ok(game.includes('--launchTarget'));
  assert.ok(game.includes('--username'), 'auth args come from the parent');
  assert.ok(game.some((a) => a && typeof a === 'object' && Array.isArray(a.rules)));
});

test('resolveVersionChain: multi-level chains and cycle guard', async () => {
  // two-level: b inherits a, a has no inheritsFrom
  const a = { id: 'a', mainClass: 'A', libraries: [{ name: 'x:1', url: 'u' }] };
  const b = { id: 'b', inheritsFrom: 'a', libraries: [{ name: 'b:1' }] };
  const calls = [];
  const merged = await resolveVersionChain(b, { getParent: async (id) => { calls.push(id); return id === 'a' ? a : null; } });
  assert.deepEqual(calls, ['a']);
  assert.equal(merged.id, 'b');
  assert.equal(merged.mainClass, 'A');

  const cyc = { id: 'c1', inheritsFrom: 'c2', libraries: [] };
  await assert.rejects(
    () => resolveVersionChain(cyc, { getParent: async () => ({ id: 'c2', inheritsFrom: 'c1', libraries: [] }) }),
    (err) => err.code === 'INHERITS_BREAK'
  );

  // missing parent -> INHERITS_BREAK
  await assert.rejects(
    () => resolveVersionChain({ id: 'x', inheritsFrom: 'nope', libraries: [] }, { getParent: async () => null }),
    (err) => err.code === 'INHERITS_BREAK'
  );
});

test('mergeArgs: child-first, values deduped, FLAGS kept (repeated flags must not orphan their values)', () => {
  const out = mergeArgs(['--a', '--b'], ['--b', '--c', { rules: [], value: '--z' }]);
  // --b is a flag: the NeoForge profile repeats flags like --add-opens once
  // per export; dropping the second copy orphans its value (java then treats
  // the value as the main class). Non-flag strings still dedupe.
  assert.deepEqual(out, ['--a', '--b', '--b', '--c', { rules: [], value: '--z' }]);
});

test('dedupeByName: earlier lists win', () => {
  const a = [{ name: 'x:1', url: 'child' }, { name: 'y:1', url: 'child' }];
  const b = [{ name: 'x:1', url: 'parent' }, { name: 'z:1', url: 'parent' }];
  const out = dedupeByName(a, b);
  assert.deepEqual(out.map((l) => l.url), ['child', 'child', 'parent']);
});

// --- template expansion + substitution ---

test('expandJvmArgs: os rules evaluated on windows/x64', () => {
  const args = [
    { rules: [{ action: 'allow', os: { name: 'osx' } }], value: ['-XstartOnFirstThread'] },
    { rules: [{ action: 'allow', os: { name: 'windows' } }], value: '-XX:HeapDumpPath=x' },
    { rules: [{ action: 'allow', os: { arch: 'x86' } }], value: '-Xss1M' },
    '-plain',
  ];
  assert.deepEqual(expandJvmArgs(args, { os: 'windows', arch: 'x64' }), [
    '-XX:HeapDumpPath=x',
    '-plain',
  ]);
});

test('expandGameArgs: features {} drops demo/custom-resolution/quickplay', () => {
  const args = [
    '--username',
    '${auth_player_name}',
    { rules: [{ action: 'allow', features: { is_demo_user: true } }], value: '--demo' },
    { rules: [{ action: 'allow', features: { has_custom_resolution: true } }], value: ['--width', '${resolution_width}'] },
  ];
  assert.deepEqual(expandGameArgs(args, {}), ['--username', '${auth_player_name}']);
});

test('substitutePlaceholders: known keys replaced, unknown -> empty', () => {
  const map = { classpath: 'A', natives_directory: 'B' };
  assert.equal(substitutePlaceholders('${classpath}${natives_directory}${nope}', map), 'AB');
  assert.equal(substitutePlaceholders('--x=${nope}', map), '--x=');
  assert.equal(substitutePlaceholders('no tokens', map), 'no tokens');
});

// --- buildArgv for a neoforge launch (pure) ---

function fakeNeoforgeResolved({ mode = 'normal', javaMajor = 21 } = {}) {
  return {
    java: { path: 'C:/jdk/java.exe', major: javaMajor, build: '21.0.7+6', version: '21.0.7' },
    memoryMb: 3072,
    nativesDir: 'N:/natives',
    loader: {
      mainClass: 'cpw.mods.bootstraplauncher.BootstrapLauncher',
      jvmArgs: [
        '-Djava.net.preferIPv6Addresses=system',
        '-DignoreList=client-extra,${version_name}.jar',
        '-DlibraryDirectory=${library_directory}',
        '-p',
        '${library_directory}/cpw/mods/bootstraplauncher/2.0.2/bootstraplauncher-2.0.2.jar${classpath_separator}${library_directory}/cpw/mods/securejarhandler/3.0.8/securejarhandler-3.0.8.jar',
        '--add-modules',
        'ALL-MODULE-PATH',
        '-Djava.library.path=${natives_directory}',
        '-cp',
        '${classpath}',
      ],
      gameArgs: [
        '--fml.neoForgeVersion',
        '21.1.242',
        '--fml.mcVersion',
        '1.21.1',
        '--launchTarget',
        'forgeclient',
        '--username',
        '${auth_player_name}',
        '--version',
        '${version_name}',
        '--gameDir',
        '${game_directory}',
        '--assetsDir',
        '${assets_root}',
        '--assetIndex',
        '${assets_index_name}',
        '--uuid',
        '${auth_uuid}',
        '--accessToken',
        '${auth_access_token}',
        '--userType',
        '${user_type}',
        '--versionType',
        '${version_type}',
      ],
      kind: 'neoforge',
    },
    version: { id: 'neoforge-21.1.242', mainClass: 'cpw.mods.bootstraplauncher.BootstrapLauncher', assetIndexId: '17' },
    classpath: ['C:/libs/a.jar', 'C:/libs/b.jar'],
    gameDir: 'G:/instance',
    assetsDir: 'A:/assets',
    account: { username: 'Tester', uuid: '11111111-2222-3333-4444-555555555555', accessToken: 'tok', userType: 'mojang' },
    mode,
    aotKey: 'k',
    aotCachePath: 'C:/cache/aot/game.aot',
    aotCacheExists: false,
    warnings: [],
  };
}

test('buildArgv: neoforge launch (module path, fml args, substituted placeholders)', () => {
  const argv = buildArgv({ name: 'Keke', memory_mb: 3072 }, fakeNeoforgeResolved());
  const text = argv.join(' ');
  // java path first
  assert.equal(argv[0], 'C:/jdk/java.exe');
  // main class
  assert.ok(text.includes('cpw.mods.bootstraplauncher.BootstrapLauncher'));
  // module path flag + substituted separators
  assert.ok(text.includes(' -p '));
  assert.ok(text.includes('bootstraplauncher-2.0.2.jar;'), 'classpath_separator substituted');
  // -DlibraryDirectory points at the real libraries dir
  assert.ok(text.includes('-DlibraryDirectory='), 'library_directory substituted');
  assert.ok(!text.includes('${'), 'no unresolved placeholders');
  // fml game args + substituted auth args
  assert.ok(text.includes('--fml.neoForgeVersion 21.1.242'));
  assert.ok(text.includes('--fml.mcVersion 1.21.1'));
  assert.ok(text.includes('--username Tester'));
  assert.ok(text.includes('--version neoforge-21.1.242'));
  assert.ok(text.includes('--gameDir G:/instance'));
  assert.ok(text.includes('--assetIndex 17'));
  // classpath flag present (parent template + explicit -cp, same value)
  assert.ok(text.includes('-cp C:/libs/a.jar;C:/libs/b.jar'));
  // no AOT flags on JDK 21
  assert.ok(!text.includes('-XX:AOTCache'));
});

test('buildArgv: AOT mode on JDK 21 is skipped with a warning', () => {
  const resolved = fakeNeoforgeResolved({ mode: 'aot' });
  const argv = buildArgv({ name: 'Keke', memory_mb: 3072 }, resolved);
  assert.ok(!argv.join(' ').includes('-XX:AOTCache'));
  assert.ok(resolved.warnings.some((w) => w.includes('JDK 25-tier')));
});

test('buildArgv: vanilla/fabric program args unchanged (no gameArgs template)', () => {
  const resolved = fakeNeoforgeResolved();
  resolved.loader = { mainClass: 'net.minecraft.client.main.Main', jvmArgs: [], gameArgs: [] };
  resolved.version = { id: '1.21.11', mainClass: 'net.minecraft.client.main.Main', assetIndexId: '17' };
  const argv = buildArgv({ name: 'V', memory_mb: 2048 }, resolved);
  const text = argv.join(' ');
  assert.ok(text.includes('--version 1.21.11'));
  assert.ok(text.includes('--username Tester'));
  assert.ok(text.includes('--versionType release'));
});
