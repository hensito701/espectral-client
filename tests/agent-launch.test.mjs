import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Sandbox the data dir BEFORE importing the engine (same pattern as the other
// engine tests) so config.mjs resolves to a throwaway directory.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-agent-launch-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const launch = await import('../src/engine/launch.mjs');
const agent = await import('../src/engine/agent.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

/** Minimal resolveLaunch() result carrying everything buildArgv reads. */
function makeResolved(overrides = {}) {
  return {
    java: { path: '/jdk/bin/java', major: 25, build: '25.0.4+7' },
    account: { username: 'AgentBot', uuid: '00000000-0000-3000-8000-000000000000', accessToken: 'tok', userType: 'mojang' },
    version: { id: '26.3', mainClass: 'net.minecraft.client.main.Main', assetIndexId: '26.3' },
    gameDir: '/games/agent-spike',
    assetsDir: '/data/assets',
    nativesDir: '/games/agent-spike/natives',
    classpath: ['/data/libraries/a.jar'],
    memoryMb: 4096,
    mode: 'normal',
    aotCachePath: '/data/cache/aot/key/game.aot',
    aotCacheExists: true,
    aotCacheStale: false,
    loader: null,
    bridgeToken: 'b'.repeat(64),
    warnings: [],
    ...overrides,
  };
}

/** Program args after the main class (the argv[0] class differs per loader). */
const progTail = (argv, mainClass = 'net.minecraft.client.main.Main') => {
  const i = argv.indexOf(mainClass);
  assert.ok(i > 0, `main class ${mainClass} present`);
  return argv.slice(i + 1);
};

test('buildArgv: agent launch adds -Despectral.agent=1, ffmpeg and hidden', () => {
  const argv = launch.buildArgv(
    { name: 'agent-spike' },
    makeResolved({ agent: true, ffmpegPath: '/usr/bin/ffmpeg', window: { width: 1080, height: 1920, hidden: true } })
  );
  assert.ok(argv.includes('-Despectral.agent=1'), 'agent property present');
  assert.ok(argv.includes('-Despectral.ffmpeg=/usr/bin/ffmpeg'), 'ffmpeg property present');
  assert.ok(argv.includes('-Despectral.agent.hidden=1'), 'hidden property present');
  // Program args still carry the fixed list, with the window/resolution pair.
  const tail = progTail(argv);
  assert.deepEqual(tail.slice(-4), ['--width', '1080', '--height', '1920']);
});

test('buildArgv: a plain launch carries NO agent properties', () => {
  const argv = launch.buildArgv({ name: 'plain' }, makeResolved());
  for (const flag of ['-Despectral.agent=1', '-Despectral.agent.hidden=1']) {
    assert.ok(!argv.includes(flag), `${flag} must be absent`);
  }
  assert.ok(!argv.some((a) => a.startsWith('-Despectral.ffmpeg=')), 'no ffmpeg property');
  assert.ok(!argv.includes('--quickPlaySingleplayer'));
});

test('buildArgv: join world/server map to --quickPlaySingleplayer / --quickPlayMultiplayer', () => {
  const sp = launch.buildArgv({ name: 'a' }, makeResolved({ agent: true, join: { world: 'Mundo nuevo' } }));
  assert.deepEqual(progTail(sp).slice(-2), ['--quickPlaySingleplayer', 'Mundo nuevo']);

  const mp = launch.buildArgv({ name: 'a' }, makeResolved({ agent: true, join: { server: 'play.example.com:25566' } }));
  assert.deepEqual(progTail(mp).slice(-2), ['--quickPlayMultiplayer', 'play.example.com:25566']);
});

test('buildArgv: NeoForge game template expands the quick-play/resolution features', () => {
  const gameArgs = [
    '--username', '${auth_player_name}',
    { rules: [{ action: 'allow', features: { has_custom_resolution: true } }], value: ['--width', '${resolution_width}', '--height', '${resolution_height}'] },
    { rules: [{ action: 'allow', features: { is_quick_play_singleplayer: true } }], value: ['--quickPlaySingleplayer', '${quickPlaySingleplayer}'] },
    { rules: [{ action: 'allow', features: { is_quick_play_multiplayer: true } }], value: ['--quickPlayMultiplayer', '${quickPlayMultiplayer}'] },
    { rules: [{ action: 'allow', features: { is_demo_user: true } }], value: '--demo' },
  ];
  const base = {
    loader: { mainClass: 'net.neoforged.Main', jvmArgs: [], gameArgs, kind: 'neoforge' },
  };
  // Options present -> features on, placeholders filled.
  const withOpts = launch.buildArgv(
    { name: 'nf' },
    makeResolved({ ...base, agent: true, window: { width: 1280, height: 720, hidden: false }, join: { server: 'localhost:25565' } })
  );
  const tail = progTail(withOpts, 'net.neoforged.Main');
  assert.deepEqual(tail, [
    '--username', 'AgentBot',
    '--width', '1280',
    '--height', '720',
    '--quickPlayMultiplayer', 'localhost:25565',
  ]);
  assert.ok(!tail.includes('--quickPlaySingleplayer'), 'the singleplayer rule stays off');
  assert.ok(!tail.includes('--demo'), 'gated demo rule stays off');

  // No options -> every gated rule drops out (argv identical to a plain launch).
  const plain = launch.buildArgv({ name: 'nf' }, makeResolved(base));
  assert.deepEqual(progTail(plain, 'net.neoforged.Main'), ['--username', 'AgentBot']);
});

test('normalizeJoin: world/server validation', () => {
  assert.deepEqual(agent.normalizeJoin({ world: 'Mundo 1' }), { world: 'Mundo 1' });
  assert.deepEqual(agent.normalizeJoin({ server: 'mc.example.com:25565' }), { server: 'mc.example.com:25565' });
  assert.deepEqual(agent.normalizeJoin({ server: '[::1]:25565' }), { server: '[::1]:25565' });
  assert.equal(agent.normalizeJoin(undefined), null);

  const bad = [
    [{ world: '..' }, 'BAD_JOIN'],
    [{ world: '../etc' }, 'BAD_JOIN'],
    [{ world: 'a/b' }, 'BAD_JOIN'],
    [{ world: 'a\\b' }, 'BAD_JOIN'],
    [{ world: 'w', server: 'h' }, 'BAD_JOIN'],
    [{}, 'BAD_JOIN'],
    [{ server: 'has space' }, 'BAD_JOIN'],
    [{ server: 'host:notaport' }, 'BAD_JOIN'],
    ['localhost', 'BAD_JOIN'],
    // A leading '-' would be parsed as the next option instead of the value.
    [{ world: '--width' }, 'BAD_JOIN'],
    [{ world: '-Survival' }, 'BAD_JOIN'],
  ];
  for (const [input, code] of bad) {
    assert.throws(
      () => agent.normalizeJoin(input),
      (e) => e.status === 400 && e.code === code,
      `expected 400 ${code} for ${JSON.stringify(input)}`
    );
  }
});

test('normalizeWindow: limits and agent-only hidden', () => {
  assert.deepEqual(agent.normalizeWindow({ width: 1080, height: 1920 }, { agent: false }), {
    width: 1080,
    height: 1920,
    hidden: false,
  });
  assert.deepEqual(agent.normalizeWindow({ width: 1080, height: 1920, hidden: true }, { agent: true }), {
    width: 1080,
    height: 1920,
    hidden: true,
  });
  assert.throws(
    () => agent.normalizeWindow({ width: 1080, height: 1920, hidden: true }),
    (e) => e.status === 400 && e.code === 'BAD_WINDOW'
  );
  for (const win of [{ width: 63, height: 100 }, { width: 100, height: 63 }, { width: 7681, height: 100 }, { width: 100, height: 4321 }, { width: 1.5, height: 100 }, { width: 'wide', height: 100 }, { width: 100 }]) {
    assert.throws(
      () => agent.normalizeWindow(win),
      (e) => e.status === 400 && e.code === 'BAD_WINDOW',
      `expected BAD_WINDOW for ${JSON.stringify(win)}`
    );
  }
});

test('normalizeAgent: only booleans', () => {
  assert.equal(agent.normalizeAgent(undefined), false);
  assert.equal(agent.normalizeAgent(true), true);
  assert.throws(() => agent.normalizeAgent('yes'), (e) => e.code === 'BAD_AGENT' && e.status === 400);
});

test('rewriteFileResults: relative capture paths become absolute, preview inlined', () => {
  const gameDir = path.join(dataDir, 'instances', 'cap');
  const caps = path.join(gameDir, 'espectral-captures');
  fs.mkdirSync(caps, { recursive: true });
  // 1x1 PNG
  const png = Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
    'base64'
  );
  fs.writeFileSync(path.join(caps, 'shot.png'), png);

  const out = agent.rewriteFileResults({ image_path: 'espectral-captures/shot.png', preview_path: 'espectral-captures/shot.png', width: 1920 }, gameDir);
  assert.equal(out.image_path, path.join(caps, 'shot.png'));
  assert.equal(out.preview_path, path.join(caps, 'shot.png'));
  assert.equal(out.width, 1920);
  assert.equal(out.preview.mime, 'image/png');
  assert.equal(out.preview.base64, png.toString('base64'));
});

test('rewriteFileResults: paths outside espectral-captures/ are refused', () => {
  const gameDir = path.join(dataDir, 'instances', 'cap2');
  fs.mkdirSync(path.join(gameDir, 'espectral-captures'), { recursive: true });
  fs.writeFileSync(path.join(gameDir, 'options.txt'), 'gamma:1.0\n');
  for (const rel of [
    '../options.txt',
    'espectral-captures/../../options.txt',
    'options.txt',
    path.join(gameDir, '..', 'cap2', 'options.txt'),
    '/etc/passwd',
  ]) {
    const out = agent.rewriteFileResults({ video_path: rel }, gameDir);
    assert.equal(out.video_path, null, `${rel} must be refused`);
    assert.equal(out.preview, undefined, 'no preview for a refused path');
  }
  // Non-path fields pass through untouched.
  const keep = agent.rewriteFileResults({ frames: 30, note: 'x' }, gameDir);
  assert.deepEqual(keep, { frames: 30, note: 'x' });
});

test('rewriteFileResults: is a no-op without a gameDir', () => {
  assert.deepEqual(agent.rewriteFileResults({ video_path: 'a.mp4' }, null), { video_path: 'a.mp4' });
  assert.equal(agent.rewriteFileResults(null, '/tmp'), null);
});

test('ffmpeg: resolution prefers PATH on POSIX and never throws', async () => {
  const ff = await import('../src/engine/ffmpeg.mjs');
  const resolved = ff.resolveFfmpegSync();
  if (process.platform === 'win32') {
    // Without a provisioned build this is null on Windows — a launch then runs
    // without -Despectral.ffmpeg and captures report NO_FFMPEG mod-side.
    assert.ok(resolved === null || path.isAbsolute(resolved));
  } else {
    assert.ok(resolved === null || path.isAbsolute(resolved), 'absolute when found');
    if (resolved) assert.equal(await ff.ensureFfmpeg(), resolved, 'ensure returns the PATH binary, no download');
  }
  assert.ok(path.isAbsolute(ff.provisionedFfmpegPath()), 'provisioned path is absolute and inside the data dir');
  assert.match(ff.FFMPEG_WIN_PIN.sha256, /^[0-9a-f]{64}$/);
  assert.ok(ff.FFMPEG_WIN_PIN.url.startsWith('https://'), 'pinned URL is https');
});

test('capture tools: pins are complete and unsupported loaders are refused', async () => {
  const mods = await import('../src/engine/mods.mjs');
  for (const v of Object.keys(mods.REPLAY_MOD_PINS_BY_VERSION)) {
    const pin = mods.replayModPin(v);
    assert.match(pin.sha1, /^[0-9a-f]{40}$/, `${v} replay sha1`);
    assert.match(pin.version_id, /^[A-Za-z0-9]+$/, `${v} replay version id`);
  }
  assert.equal(mods.supportsCaptureTools({ version: '26.3', loader: 'fabric' }), true);
  assert.equal(mods.supportsCaptureTools({ version: '26.3', loader: 'neoforge' }), false);
  assert.equal(mods.supportsCaptureTools({ version: '1.20.1', loader: 'fabric' }), false);

  assert.match(mods.DEFAULT_SHADER_PACK.sha1, /^[0-9a-f]{40}$/);
  assert.match(mods.FAST_SHADER_PACK.sha1, /^[0-9a-f]{40}$/);
  assert.equal(mods.CAPTURE_SHADER_PACKS.length, 2);
  // Both packs are distinct artifacts, not the same download twice.
  assert.notEqual(mods.DEFAULT_SHADER_PACK.filename, mods.FAST_SHADER_PACK.filename);

  await assert.rejects(
    () => mods.installCaptureTools({ name: 'x', version: '26.3', loader: 'neoforge' }),
    (e) => e.status === 400 && e.code === 'CAPTURE_TOOLS_UNSUPPORTED'
  );
});

test('capture tools: default shader pack is seeded only when none is selected', async () => {
  const mods = await import('../src/engine/mods.mjs');
  const inst = { name: 'seed', version: '26.3', loader: 'fabric' };
  const cfg = path.join(dataDir, 'instances', 'seed', 'config', 'iris.properties');

  const fresh = mods.selectDefaultShaderPack(inst);
  assert.equal(fresh.applied, true);
  assert.equal(fresh.pack, mods.DEFAULT_SHADER_PACK.filename);
  const written = fs.readFileSync(cfg, 'utf8');
  assert.match(written, new RegExp(`shaderPack=${mods.DEFAULT_SHADER_PACK.filename.replace(/\./g, '\\.')}`));
  assert.match(written, /enableShaders=true/);

  fs.writeFileSync(cfg, 'shaderPack=BSL_v10.1.8.zip\nenableShaders=true\ncustom=yes\n');
  const kept = mods.selectDefaultShaderPack(inst);
  assert.equal(kept.applied, false);
  assert.equal(kept.pack, 'BSL_v10.1.8.zip');
  assert.match(fs.readFileSync(cfg, 'utf8'), /custom=yes/, 'user config untouched');

  // An existing file with unrelated keys but no shader pack IS seeded, and
  // keeps the other keys.
  fs.writeFileSync(cfg, 'enableShaders=false\ncolorSpace=SRGB\n');
  const seeded = mods.selectDefaultShaderPack(inst);
  assert.equal(seeded.applied, true);
  const after = fs.readFileSync(cfg, 'utf8');
  assert.match(after, /colorSpace=SRGB/);
  assert.match(after, /enableShaders=true/);
});

test('rewriteFileResults: a symlink pointing out of espectral-captures/ is refused', (t) => {
  if (process.platform === 'win32') return t.skip('creating symlinks needs extra privileges on Windows');
  const gameDir = path.join(dataDir, 'instances', 'cap-link');
  const caps = path.join(gameDir, 'espectral-captures');
  fs.mkdirSync(caps, { recursive: true });
  const secret = path.join(dataDir, 'secret.txt');
  fs.writeFileSync(secret, 'launcher internals');
  fs.symlinkSync(secret, path.join(caps, 'leak.png'));

  // Lexically inside, but the real path is outside: the agent must not be
  // handed a launcher file through a link the game process planted.
  const out = agent.rewriteFileResults(
    { image_path: 'espectral-captures/leak.png', preview_path: 'espectral-captures/leak.png', width: 1 },
    gameDir
  );
  assert.equal(out.image_path, null);
  assert.equal(out.preview_path, null);
  assert.equal(out.preview, undefined);
  assert.equal(out.width, 1, 'non-path fields still pass through');
});

test('token: rotation is atomic, 0600, and leaves no temp file behind', () => {
  const file = agent.tokenPath();
  fs.mkdirSync(path.dirname(file), { recursive: true });
  // A pre-existing looser file (older build / copied data dir) must be fixed.
  fs.writeFileSync(file, '0'.repeat(64) + '\n');
  fs.chmodSync(file, 0o644);

  const token = agent.writeToken();
  assert.match(token, /^[0-9a-f]{64}$/);
  assert.equal(fs.readFileSync(file, 'utf8'), token + '\n');
  if (process.platform !== 'win32') assert.equal(fs.statSync(file).mode & 0o777, 0o600);
  const leftovers = fs
    .readdirSync(path.dirname(file))
    .filter((n) => n.startsWith('agent-token') && n.endsWith('.tmp'));
  assert.deepEqual(leftovers, [], 'the temp file is renamed, never left behind');
});

test('registry: re-registering a launch on spawn keeps its queued commands', async () => {
  agent._reset();
  const key = 'lmerge';
  agent.registerLaunch({ key, instance: 'merge-inst', gameDir: null });
  const { promise } = agent.callTool(key, 'screen_tree', {}, { timeoutMs: 5000 });
  const entry = agent.registerLaunch({ key, instance: 'merge-inst', gameDir: '/games/merge' });
  assert.equal(entry.gameDir, '/games/merge', 'the resolved gameDir replaces the null placeholder');

  const commands = await agent.awaitCommands(entry, { timeoutMs: 50 });
  assert.equal(commands.length, 1, 'the command issued before spawn is still queued');
  // The result must reach the ORIGINAL call: a self-resettling await guards
  // against the entry having been swapped out from under it.
  agent.completeCommand(entry, { id: commands[0].id, ok: true, data: { screen: 'TitleScreen' } });
  const data = await Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error('call was orphaned by the re-register')), 1000)),
  ]);
  assert.deepEqual(data, { screen: 'TitleScreen' });
  agent._reset();
});

test('relay queue: a command batch is delivered to exactly one parked poll', async () => {
  agent._reset();
  const key = 'lone-waiter';
  agent.registerLaunch({ key, instance: 'one-inst' });
  const entry = agent.getLaunch(key);

  const first = agent.awaitCommands(entry, { timeoutMs: 200 });
  const second = agent.awaitCommands(entry, { timeoutMs: 200 });
  const { promise } = agent.callTool(key, 'click', { x: 10, y: 20 }, { timeoutMs: 5000 });
  promise.catch(() => {});

  const [a, b] = await Promise.all([first, second]);
  const delivered = [a, b].filter((c) => c.length > 0);
  assert.equal(delivered.length, 1, 'the batch is handed out once, never replayed to a second poll');
  assert.equal(delivered[0][0].tool, 'click');
  agent.releaseLaunch(key);
  agent._reset();
});

test('relay queue: a call that timed out is dropped, not executed late', async () => {
  agent._reset();
  const key = 'llate';
  agent.registerLaunch({ key, instance: 'late-inst' });
  const entry = agent.getLaunch(key);

  const { promise } = agent.callTool(key, 'record_start', {}, { timeoutMs: 1000 });
  const err = await promise.catch((e) => e);
  assert.equal(err.code, 'AGENT_TIMEOUT');
  assert.equal(entry.queue.length, 0, 'the abandoned command must not reach the mod on its next poll');
  agent._reset();
});
