import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-multi-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const resolver = await import('../src/engine/resolver.mjs');
const accounts = await import('../src/engine/accounts.mjs');
const launch = await import('../src/engine/launch.mjs');
const routesLaunch = await import('../src/engine/routes/launch.mjs');
const instances = await import('../src/engine/instances.mjs');
const events = await import('../src/engine/events.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

test('resolveLaunch: active account uses base instance directory', async () => {
  accounts.createAccount('PlayerActive1');
  accounts.setActiveAccount('PlayerActive1');

  const instance = {
    name: 'test-inst',
    version: '1.21.11',
    loader: 'vanilla',
    java_path: null,
    memory_mb: 2048,
  };

  const resolved = await launch.resolveLaunch(instance, { dryRun: true });
  assert.equal(resolved.account.username, 'PlayerActive1');
  assert.equal(resolved.gameDir, resolver.instanceDir('test-inst'));
});

test('resolveLaunch: account override uses per-account profile directory', async () => {
  accounts.createAccount('PlayerActive2');
  const acc2 = accounts.createAccount('PlayerAlt2');
  accounts.setActiveAccount('PlayerActive2');
  const instance = {
    name: 'test-inst',
    version: '1.21.11',
    loader: 'vanilla',
    java_path: null,
    memory_mb: 2048,
  };

  // Launch with explicit account override
  const resolved = await launch.resolveLaunch(instance, { dryRun: true, account: acc2 });
  assert.equal(resolved.account.username, 'PlayerAlt2');
  const expectedProfileDir = path.join(resolver.instanceDir('test-inst'), 'profiles', acc2.uuid);
  assert.equal(resolved.gameDir, expectedProfileDir);
});

function makeFakeApp() {
  const handlers = new Map();
  const app = {
    post: (p, fn) => handlers.set(`POST ${p}`, fn),
    get: (p, fn) => handlers.set(`GET ${p}`, fn),
  };
  return { app, handlers };
}

test('stop during STARTING returns ok+cancelled and does NOT spawn', async (t) => {
  const instName = 'inst-stop-test';
  // Ensure a clean slate for the in-memory sets
  routesLaunch.activeInstances.delete(instName);
  routesLaunch.pendingStops.delete(instName);

  // Create instance on disk so getInstance succeeds without mocking ESM namespace
  const instDir = path.join(dataDir, 'instances', instName);
  fs.mkdirSync(path.join(instDir, 'mods'), { recursive: true });
  fs.mkdirSync(path.join(instDir, 'natives'), { recursive: true });
  const instanceJson = {
    name: instName,
    version: '1.21.11',
    loader: 'vanilla',
    loader_version: null,
    modpack: null,
    modpack_version: null,
    memory_mb: 2048,
    mods: [],
    imported_from: null,
    jdk_path_override: null,
    aot_auto_train: false,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  };
  fs.writeFileSync(path.join(instDir, 'instance.json'), JSON.stringify(instanceJson, null, 2));

  // Deferred resolveLaunch — use route test hooks (ESM namespaces are read-only in Node 24)
  let releaseResolve;
  const resolveDeferred = new Promise((res) => { releaseResolve = res; });
  let resolveCalls = 0;
  const fakeResolved = {
    java: { path: '/fake/java', major: 25, build: '25-test' },
    version: { id: '1.21.11', mainClass: 'net.minecraft.client.main.Main', assetIndexId: '1.21' },
    gameDir: path.join(dataDir, 'instances', instName),
    nativesDir: path.join(dataDir, 'instances', instName, 'natives'),
    classpath: [],
    assetsDir: path.join(dataDir, 'assets'),
    memoryMb: 2048,
    account: { username: 'Tester', uuid: '00000000-0000-0000-0000-000000000000', accessToken: '0', userType: 'mojang' },
    mode: 'normal',
    aotKey: 'test-key',
    aotCachePath: path.join(dataDir, 'cache', 'aot', 'test-key', 'game.aot'),
    aotCacheExists: false,
    aotAvailable: true,
    warnings: [],
  };
  let spawnCalls = 0;
  const emitted = [];
  routesLaunch.__testHooks.resolveLaunch = async () => {
    resolveCalls++;
    await resolveDeferred;
    return fakeResolved;
  };
  routesLaunch.__testHooks.launchInstance = () => {
    spawnCalls++;
    return { pid: 9999, child: { pid: 9999, exitCode: null, signalCode: null, on() {}, once() {}, kill() {} } };
  };
  routesLaunch.__testHooks.emit = (ev, payload) => {
    emitted.push({ ev, payload });
  };
  t.after(() => {
    routesLaunch.__testHooks.resolveLaunch = null;
    routesLaunch.__testHooks.launchInstance = null;
    routesLaunch.__testHooks.emit = null;
    routesLaunch.__testHooks.resolveAccountFor = null;
    routesLaunch.activeInstances.delete(instName);
    routesLaunch.pendingStops.delete(instName);
    fs.rmSync(instDir, { recursive: true, force: true });
  });

  const { app, handlers } = makeFakeApp();
  await routesLaunch.register(app);
  const launchHandler = handlers.get('POST /api/instances/:name/launch');
  const stopHandler = handlers.get('POST /api/instances/:name/stop');
  assert.ok(launchHandler, 'launch handler registered');
  assert.ok(stopHandler, 'stop handler registered');

  // Kick off a launch — it will hang in resolveLaunch
  const launchPromise = launchHandler({ url: '/api/instances/inst-stop-test/launch', headers: {} }, {}, { name: instName }, { mode: 'normal' });
  const launchReply = await launchPromise;
  assert.ok(launchReply.key, 'launch returns a key');
  assert.equal(launchReply.preparing, true);
  assert.ok(routesLaunch.activeInstances.has(instName), 'activeInstances tracks preparing launch');

  // While still STARTING (no running process), stop should return cancelled
  const stopReply = await stopHandler({ url: '/api/instances/inst-stop-test/stop', headers: {} }, {}, { name: instName }, {});
  assert.deepEqual(stopReply, { ok: true, instance: instName, cancelled: 'starting' });
  assert.ok(routesLaunch.pendingStops.has(instName), 'pendingStops tracks cancellation');

  // Release resolveLaunch — background task should detect pendingStops and NOT spawn
  releaseResolve();
  // Give the background async task a few ticks to process the pendingStops branch
  await new Promise((r) => setTimeout(r, 50));
  await new Promise((r) => setImmediate(r));

  assert.equal(spawnCalls, 0, 'launchInstance must NOT be called after cancelled during prepare');
  assert.equal(resolveCalls, 1);
  assert.ok(!routesLaunch.activeInstances.has(instName), 'activeInstances cleared after cancelled');
  assert.ok(!routesLaunch.pendingStops.has(instName), 'pendingStops cleared after handling');

  const exitEvents = emitted.filter((e) => e.ev === 'launch-exit');
  assert.ok(exitEvents.length >= 1, 'launch-exit emitted');
  const cancelledExit = exitEvents.find((e) => e.payload.error === 'cancelled during prepare');
  assert.ok(cancelledExit, 'launch-exit carries cancelled during prepare error');
  assert.equal(cancelledExit.payload.instance, instName);
  assert.equal(cancelledExit.payload.spawn_ms, null);
  assert.equal(cancelledExit.payload.boot_ms, null);
  assert.ok(cancelledExit.payload.phases && typeof cancelledExit.payload.phases === 'object', 'phases present');
});
