import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// The game is spawned detached so it survives the launcher closing. A fresh
// engine must then re-adopt it (block a second Play, let Stop kill it) — and
// must NOT adopt a stale record whose pid now belongs to something else.

const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-sessions-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const sessions = await import('../src/engine/game-sessions.mjs');
const { start } = await import('../src/engine/server.mjs');
const launchRoutes = await import('../src/engine/routes/launch.mjs');
// Same switch the launch tests use: no Discord IPC / SSE side effects.
launchRoutes.__testHooks.emit = () => {};

const games = [];
after(() => {
  for (const g of games) {
    try {
      process.kill(g.pid);
    } catch {
      /* already gone */
    }
  }
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

/** A stand-in game: a long-lived process whose command line carries --gameDir. */
function fakeGame(gameDir) {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)', '--', '--gameDir', gameDir], {
    stdio: 'ignore',
    detached: true,
    windowsHide: true,
  });
  child.unref();
  games.push(child);
  return child;
}

function makeInstance(name) {
  const dir = path.join(dataDir, 'instances', name);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'instance.json'), JSON.stringify({ name, version: '26.3', loader: 'fabric' }));
  return dir;
}

test('liveSessions keeps the running game and drops stale or reused pids', async () => {
  const dir = makeInstance('live-check');
  const game = fakeGame(dir);
  sessions.recordSession('live-check', { pid: game.pid, game_dir: dir, version: '26.3', started_at: Date.now() });
  // A pid that is alive but is NOT that game (pid reuse after a crash).
  sessions.recordSession('reused', { pid: process.pid, game_dir: path.join(dataDir, 'instances', 'reused'), version: '26.3' });
  // A pid that is gone.
  sessions.recordSession('dead', { pid: 2 ** 22 + 12345, game_dir: dir, version: '26.3' });

  const live = await sessions.liveSessions();
  assert.deepEqual(live.map((s) => s.instance), ['live-check']);
  assert.deepEqual(Object.keys(sessions.readSessions()), ['live-check'], 'stale records are pruned from disk');
  sessions.removeSession('live-check');
  process.kill(game.pid);
});

test('a reopened engine adopts the running game: Play is refused, Stop kills it', async () => {
  const dir = makeInstance('adopted');
  const game = fakeGame(dir);
  sessions.recordSession('adopted', {
    key: 'lprevious',
    pid: game.pid,
    game_dir: dir,
    version: '26.3',
    started_at: Date.now() - 60_000,
    menu_at: Date.now() - 50_000,
  });

  const server = await start(0);
  const base = `http://127.0.0.1:${server.address().port}`;
  const CLIENT = { 'Content-Type': 'application/json', 'x-espectral-client': '1' };
  try {
    // register() kicks adoption off; await an explicit pass so the test does
    // not race the startup one (adoption skips instances already active).
    await launchRoutes.adoptRunningGames();
    let rows = (await (await fetch(`${base}/api/launches`)).json()).launches;
    const row = rows.find((r) => r.instance === 'adopted');
    assert.ok(row?.running, 'adopted game is listed as running');

    const play = await fetch(`${base}/api/instances/adopted/launch`, { method: 'POST', headers: CLIENT, body: '{}' });
    assert.equal(play.status, 409, 'a second game in the same folder is refused');
    assert.equal((await play.json()).error.code, 'ALREADY_RUNNING');

    const stop = await fetch(`${base}/api/instances/adopted/stop`, { method: 'POST', headers: CLIENT, body: '{}' });
    assert.equal(stop.status, 200);
    assert.equal(sessions.isPidAlive(game.pid), false, 'Stop killed the adopted process');

    rows = (await (await fetch(`${base}/api/launches`)).json()).launches;
    assert.equal(rows.find((r) => r.instance === 'adopted')?.running, false);
    assert.equal('adopted' in sessions.readSessions(), false, 'session record removed');
  } finally {
    launchRoutes.__resetLaunchState();
    await new Promise((r) => server.close(r));
  }
});
