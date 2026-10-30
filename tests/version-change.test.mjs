import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Sandbox the data dir before importing the engine.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-version-change-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const vc = await import('../src/engine/version-change.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const sha1 = (s) => createHash('sha1').update(s).digest('hex');

// Local version manifest + version jsons: no Mojang traffic.
const versionsDir = path.join(dataDir, 'versions');
fs.mkdirSync(versionsDir, { recursive: true });
fs.writeFileSync(path.join(versionsDir, 'manifest.json'), JSON.stringify({
  fetched_at: Date.now(),
  latest_release: '26.3',
  latest_snapshot: '26.3',
  versions: [
    { id: '26.3', type: 'release', release_time: '2026-09-15T00:00:00+00:00', sha1: null, url: 'https://mock/26.3.json' },
    { id: '26.2', type: 'release', release_time: '2026-06-16T00:00:00+00:00', sha1: null, url: 'https://mock/26.2.json' },
  ],
}));
for (const id of ['26.2', '26.3']) {
  fs.writeFileSync(path.join(versionsDir, `${id}.json`), JSON.stringify({ id, javaVersion: { majorVersion: 25 } }));
}

/**
 * Fake Modrinth: each project has one build per game version. A jar's bytes
 * are `<project>@<version>` so its sha1 says exactly which build it is.
 */
const BUILDS = {
  projA: ['26.2', '26.3'],
  projB: ['26.2', '26.3'],
  projC: ['26.3'], // no 26.2 build
  projD: ['26.2', '26.3'],
};
const fileFor = (pid, v) => `${pid}-${v}.jar`;
const versionFor = (pid, v) => ({
  id: `${pid}-v${v}`,
  project_id: pid,
  version_number: `${v}-build`,
  status: 'listed',
  dependencies: [],
  files: [{
    primary: true,
    filename: fileFor(pid, v),
    url: `https://cdn.modrinth.com/data/${pid}/${fileFor(pid, v)}`,
    hashes: { sha1: sha1(`${pid}@${v}`) },
    size: `${pid}@${v}`.length,
  }],
});
// projD ships the SAME file for both versions -> "keep".
const projDFile = (v) => ({ ...versionFor('projD', '26.3'), id: `projD-v${v}` });

function mockFetch(t) {
  t.mock.method(globalThis, 'fetch', async (url, opts = {}) => {
    const u = new URL(String(url));
    if (u.host === 'meta.fabricmc.net') return Response.json([{ loader: { version: '0.20.1', stable: true } }]);
    if (u.pathname === '/v2/version_files' && opts.method === 'POST') {
      const { hashes } = JSON.parse(opts.body);
      const out = {};
      for (const [pid, vs] of Object.entries(BUILDS)) {
        for (const v of vs) {
          const meta = pid === 'projD' ? projDFile(v) : versionFor(pid, v);
          if (hashes.includes(meta.files[0].hashes.sha1)) out[meta.files[0].hashes.sha1] = meta;
        }
      }
      return Response.json(out);
    }
    const proj = /^\/v2\/project\/([^/]+)\/version$/.exec(u.pathname);
    if (proj) {
      const game = JSON.parse(u.searchParams.get('game_versions'))[0];
      const pid = proj[1];
      if (!BUILDS[pid]?.includes(game)) return Response.json([]);
      return Response.json([pid === 'projD' ? projDFile(game) : versionFor(pid, game)]);
    }
    if (u.pathname === '/v2/projects') {
      return Response.json(JSON.parse(u.searchParams.get('ids')).map((id) => ({ id, title: `Title ${id}`, slug: id })));
    }
    if (u.host === 'cdn.modrinth.com') {
      const [, , pid, file] = u.pathname.split('/');
      const v = file.slice(pid.length + 1, -'.jar'.length);
      return new Response(Buffer.from(`${pid}@${v}`));
    }
    throw new Error(`unexpected fetch ${url}`);
  });
}

function makeInstance(name, version) {
  const dir = path.join(dataDir, 'instances', name);
  const mods = path.join(dir, 'mods');
  fs.mkdirSync(mods, { recursive: true });
  fs.mkdirSync(path.join(dir, 'saves', 'World'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'saves', 'World', 'level.dat'), 'world-bytes');
  fs.writeFileSync(path.join(dir, 'instance.json'), JSON.stringify({
    name, version, loader: 'fabric', loader_version: null, memory_mb: null, memory_auto_migrated: true,
  }));
  return mods;
}

test('downgrade 26.3 -> 26.2: updates, keeps, disables, preserves disabled state, backs up worlds', async (t) => {
  mockFetch(t);
  const mods = makeInstance('vc-down', '26.3');
  fs.writeFileSync(path.join(mods, fileFor('projA', '26.3')), 'projA@26.3');
  fs.writeFileSync(path.join(mods, `${fileFor('projB', '26.3')}.disabled`), 'projB@26.3');
  fs.writeFileSync(path.join(mods, fileFor('projC', '26.3')), 'projC@26.3');
  fs.writeFileSync(path.join(mods, 'projD-26.3.jar'), 'projD@26.3');
  fs.writeFileSync(path.join(mods, 'handmade.jar'), 'not on modrinth');

  const plan = await vc.planVersionChange('vc-down', { version: '26.2' });
  assert.equal(plan.downgrade, true);
  assert.equal(plan.backup_worlds, true);
  const actions = Object.fromEntries(plan.mods.map((m) => [m.filename, m.action]));
  assert.deepEqual(actions, {
    'projA-26.3.jar': 'update',
    'projB-26.3.jar': 'update',
    'projC-26.3.jar': 'disable',
    'projD-26.3.jar': 'keep',
    'handmade.jar': 'disable',
  });
  assert.equal(plan.mods.find((m) => m.filename === 'handmade.jar').reason, 'unknown');
  assert.equal(plan.mods.find((m) => m.filename === 'projC-26.3.jar').reason, 'no_build');
  // Planning is read-only.
  assert.ok(fs.existsSync(path.join(mods, 'projA-26.3.jar')));

  const res = await vc.applyVersionChange('vc-down', { version: '26.2' });
  assert.deepEqual(res.mods.errors, []);
  const inst = JSON.parse(fs.readFileSync(path.join(dataDir, 'instances', 'vc-down', 'instance.json'), 'utf8'));
  assert.equal(inst.version, '26.2');
  assert.equal(inst.loader, 'fabric');

  const onDisk = fs.readdirSync(mods).filter((f) => !f.startsWith('espectral-')).sort();
  assert.deepEqual(onDisk, [
    'handmade.jar.disabled', // unknown: parked, never deleted
    'projA-26.2.jar', // replaced
    'projB-26.2.jar.disabled', // replaced, still disabled as the player left it
    'projC-26.3.jar.disabled', // no 26.2 build: parked
    'projD-26.3.jar', // already fits
  ]);
  assert.equal(fs.readFileSync(path.join(mods, 'projA-26.2.jar'), 'utf8'), 'projA@26.2');

  // Worlds were copied before the switch; the live saves are untouched.
  assert.ok(res.backup && res.backup.includes(path.join('vc-down', 'backups', 'saves-26.3-')));
  assert.equal(fs.readFileSync(path.join(res.backup, 'World', 'level.dat'), 'utf8'), 'world-bytes');
  assert.equal(fs.readFileSync(path.join(dataDir, 'instances', 'vc-down', 'saves', 'World', 'level.dat'), 'utf8'), 'world-bytes');
});

test('upgrade needs no backup; same target is rejected; Vanilla leaves mods alone', async (t) => {
  mockFetch(t);
  const mods = makeInstance('vc-up', '26.2');
  fs.writeFileSync(path.join(mods, fileFor('projA', '26.2')), 'projA@26.2');

  const up = await vc.planVersionChange('vc-up', { version: '26.3' });
  assert.equal(up.downgrade, false);
  assert.equal(up.backup_worlds, false);

  await assert.rejects(vc.planVersionChange('vc-up', { version: '26.2', loader: 'fabric' }), (e) => e.code === 'NO_CHANGE');

  const vanilla = await vc.planVersionChange('vc-up', { loader: 'vanilla' });
  assert.deepEqual(vanilla.mods.map((m) => m.action), ['idle']);
  await vc.applyVersionChange('vc-up', { loader: 'vanilla' });
  assert.ok(fs.existsSync(path.join(mods, 'projA-26.2.jar')), 'mods untouched on Vanilla');
});
