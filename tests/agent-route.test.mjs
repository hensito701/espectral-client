import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Agent-launch bookkeeping at the ROUTE level, without spawning a game:
//  - the buffer / /api/launches row is tagged agent: true,
//  - Discord presence is never pushed for the agent launch (and the agent
//    launch does not hold the presence refcount),
//  - the launch registers in the agent registry and a release on exit fails
//    in-flight calls (GAME_GONE).
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-agent-route-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const routesLaunch = await import('../src/engine/routes/launch.mjs');
const agent = await import('../src/engine/agent.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

function makeInstance(name) {
  const dir = path.join(dataDir, 'instances', name);
  fs.mkdirSync(path.join(dir, 'mods'), { recursive: true });
  fs.writeFileSync(
    path.join(dir, 'instance.json'),
    JSON.stringify({
      name,
      version: '26.3',
      loader: 'fabric',
      loader_version: null,
      memory_mb: 2048,
      mods: [],
      memory_auto_migrated: true,
      jdk_path_override: null,
      aot_auto_train: false,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    })
  );
  return dir;
}

function fakeResolved(instanceName) {
  return {
    java: { path: '/fake/java', major: 25, build: '25-test' },
    version: { id: '26.3', mainClass: 'net.minecraft.client.main.Main', assetIndexId: '26.3' },
    gameDir: path.join(dataDir, 'instances', instanceName),
    nativesDir: path.join(dataDir, 'instances', instanceName, 'natives'),
    classpath: [],
    assetsDir: path.join(dataDir, 'assets'),
    memoryMb: 2048,
    account: { username: 'AgentBot', uuid: '00000000-0000-0000-0000-000000000000', accessToken: '0', userType: 'mojang' },
    mode: 'normal',
    aotKey: 'k',
    aotCachePath: path.join(dataDir, 'cache', 'aot', 'k', 'game.aot'),
    aotCacheExists: false,
    aotAvailable: true,
    warnings: [],
    bridgeToken: 'd'.repeat(64),
  };
}

/** Install the route hooks for one test; returns the recorded presence/replies. */
function installHooks(t, instanceName) {
  const presence = [];
  const emitted = [];
  const resolveArgs = [];
  routesLaunch.__testHooks.resolveLaunch = async (instance, opts) => {
    resolveArgs.push(opts);
    return fakeResolved(instanceName);
  };
  // pid must be ALIVE: reconcileRunning() finalizes any launch whose pid the OS
  // does not hold (that is what makes it release the agent registry entry).
  // onSpawned is fired like the real launchInstance does — it is what hands the
  // resolved gameDir to the agent registry.
  routesLaunch.__testHooks.launchInstance = (instance, resolved, hooks = {}) => {
    try {
      hooks.onSpawned?.(Date.now());
    } catch {
      /* ignore */
    }
    return {
      pid: process.pid,
      child: { pid: process.pid, exitCode: null, signalCode: null, on() {}, once() {}, kill() {} },
    };
  };
  routesLaunch.__testHooks.emit = (ev, payload) => emitted.push({ ev, payload });
  routesLaunch.__testHooks.presence = presence;
  t.after(() => {
    routesLaunch.__testHooks.resolveLaunch = null;
    routesLaunch.__testHooks.launchInstance = null;
    routesLaunch.__testHooks.emit = null;
    routesLaunch.__testHooks.presence = null;
    routesLaunch.__resetLaunchState();
    agent._reset();
  });
  return { presence, emitted, resolveArgs };
}

async function settle(ms = 60) {
  await new Promise((r) => setTimeout(r, ms));
}

test('agent launch: tagged in /api/launches, no Discord presence, registered for the relay', async (t) => {
  const name = 'agent-route-1';
  makeInstance(name);
  const { presence, resolveArgs } = installHooks(t, name);

  const reply = await routesLaunch.startLaunch(name, {
    agent: true,
    join: { world: 'Survival' },
    window: { width: 1080, height: 1920, hidden: true },
  });
  assert.ok(reply.key, 'key returned');
  assert.equal(reply.preparing, true);
  await settle();

  // Launch options reach resolveLaunch (→ argv in buildArgv).
  assert.equal(resolveArgs.length, 1);
  assert.equal(resolveArgs[0].agent, true);
  assert.deepEqual(resolveArgs[0].join, { world: 'Survival' });
  assert.deepEqual(resolveArgs[0].window, { width: 1080, height: 1920, hidden: true });

  const rows = routesLaunch.launchSummaries();
  const row = rows.find((r) => r.key === reply.key);
  assert.ok(row, 'launch row exists');
  assert.equal(row.agent, true, 'row carries agent: true');

  assert.equal(
    presence.filter((p) => p.kind !== 'clear').length,
    0,
    'no presence push for an agent launch'
  );

  // Registered in the agent registry with the resolved gameDir.
  const entry = agent.getLaunch(reply.key);
  assert.ok(entry, 'agent launch registered');
  assert.equal(entry.gameDir, path.join(dataDir, 'instances', name));

  // Releasing (game exit) makes pending calls fail GAME_GONE.
  const { promise } = agent.callTool(reply.key, 'screenshot', {}, { timeoutMs: 5000 });
  const settled = promise.catch((e) => e);
  agent.releaseLaunch(reply.key);
  const err = await settled;
  assert.equal(err.code, 'GAME_GONE');
});

test('agent launch: does not hold the presence refcount for player launches', async (t) => {
  const name = 'agent-route-2';
  makeInstance(name);
  const { presence } = installHooks(t, name);

  const reply = await routesLaunch.startLaunch(name, { agent: true });
  await settle();
  // A player launch would have pushed presence here; the agent launch must not.
  assert.equal(presence.length, 0);
  // And its row is still visible to the UI/agent API.
  assert.equal(routesLaunch.launchSummaries().find((r) => r.key === reply.key)?.agent, true);
});

test('plain launch: unchanged — presence pushed, agent: false on the row', async (t) => {
  const name = 'plain-route';
  makeInstance(name);
  const { presence, resolveArgs } = installHooks(t, name);

  const reply = await routesLaunch.startLaunch(name, {});
  await settle();
  assert.equal(resolveArgs[0].agent, false);
  assert.equal(resolveArgs[0].join, null);
  const row = routesLaunch.launchSummaries().find((r) => r.key === reply.key);
  assert.equal(row.agent, false);
  assert.ok(presence.some((p) => p.kind === 'set'), 'a player launch still pushes presence');
  assert.equal(agent.getLaunch(reply.key), null, 'a plain launch is not in the agent registry');
});
