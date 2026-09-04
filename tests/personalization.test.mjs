import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir BEFORE importing the engine so module evaluation sees
// it too (same pattern as tests/launch.test.mjs).
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-personalization-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const instances = await import('../src/engine/instances.mjs');
const accounts = await import('../src/engine/accounts.mjs');
const launch = await import('../src/engine/launch.mjs');
const resolver = await import('../src/engine/resolver.mjs');
const { decodePngImage } = await import('../src/engine/routes/misc.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

// 1x1 transparent PNG (68 bytes decoded).
const TINY_PNG_B64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const TINY_PNG = Buffer.from(TINY_PNG_B64, 'base64');
const TINY_DATA_URL = `data:image/png;base64,${TINY_PNG_B64}`;

function circularDistance(a, b) {
  const d = Math.abs(a - b) % 360;
  return Math.min(d, 360 - d);
}

function assertHue(h) {
  assert.ok(Number.isInteger(h) && h >= 0 && h <= 359, `hue ${h} is an int in 0-359`);
}

// --- hue auto-assign + validation -------------------------------------------

test('createInstance auto-assigns spaced hues and stores explicit hue', async () => {
  const a = await instances.createInstance({ name: 'hue-a', version: '1.21.11' });
  const b = await instances.createInstance({ name: 'hue-b', version: '1.21.11' });
  assertHue(a.hue);
  assertHue(b.hue);
  assert.ok(
    circularDistance(a.hue, b.hue) >= 30,
    `auto hues ${a.hue}/${b.hue} must be >= 30 apart`,
  );
  const c = await instances.createInstance({ name: 'hue-c', version: '1.21.11', hue: 123 });
  assert.equal(c.hue, 123);
});

test('createInstance rejects invalid hue and relative game_dir', async () => {
  for (const hue of [-1, 360, 1.5, 'red', NaN]) {
    await assert.rejects(
      instances.createInstance({ name: 'hue-bad', version: '1.21.11', hue }),
      (e) => e.status === 400 && e.code === 'BAD_HUE',
      `hue ${String(hue)}`,
    );
  }
  await assert.rejects(
    instances.createInstance({ name: 'dir-bad', version: '1.21.11', game_dir: 'relative/path' }),
    (e) => e.status === 400 && e.code === 'BAD_GAME_DIR',
  );
  await assert.rejects(
    instances.createInstance({ name: 'dir-bad2', version: '1.21.11', game_dir: '' }),
    (e) => e.status === 400 && e.code === 'BAD_GAME_DIR',
  );
});

// --- game_dir lifecycle ------------------------------------------------------

test('game_dir is created on disk, summarized, and cleared by null patch', async () => {
  const custom = path.join(dataDir, 'custom-game-dir');
  const created = await instances.createInstance({
    name: 'dir-inst',
    version: '1.21.11',
    game_dir: custom,
  });
  assert.equal(created.game_dir, custom);
  assert.ok(fs.existsSync(custom), 'custom game_dir is mkdir -p on create');

  const patched = await instances.patchInstance('dir-inst', { game_dir: null });
  assert.equal(patched.game_dir, null);

  const custom2 = path.join(dataDir, 'custom-game-dir-2');
  const patched2 = await instances.patchInstance('dir-inst', { game_dir: custom2 });
  assert.equal(patched2.game_dir, custom2);
  assert.ok(fs.existsSync(custom2), 'custom game_dir is mkdir -p on patch');

  await assert.rejects(
    instances.patchInstance('dir-inst', { game_dir: 'relative/again' }),
    (e) => e.status === 400 && e.code === 'BAD_GAME_DIR',
  );
});

test('patch hue updates and clears; summary exposes hue/game_dir/has_icon', async () => {
  await instances.createInstance({ name: 'sum-inst', version: '1.21.11' });
  const patched = await instances.patchInstance('sum-inst', { hue: 200 });
  assert.equal(patched.hue, 200);
  assert.equal(patched.game_dir, null);
  assert.equal(patched.has_icon, false);
  await assert.rejects(
    instances.patchInstance('sum-inst', { hue: 999 }),
    (e) => e.status === 400 && e.code === 'BAD_HUE',
  );
  const cleared = await instances.patchInstance('sum-inst', { hue: null });
  assert.equal(cleared.hue, null);
});

// --- instance icon roundtrip -------------------------------------------------

test('instance icon roundtrip flips has_icon; missing icon reads 404', async () => {
  await instances.createInstance({ name: 'icon-inst', version: '1.21.11' });
  assert.equal((await instances.getSummary('icon-inst')).has_icon, false);
  await assert.rejects(instances.readInstanceIcon('icon-inst'), (e) => e.status === 404);

  const written = await instances.writeInstanceIcon('icon-inst', TINY_PNG);
  assert.deepEqual(written, { ok: true, has_icon: true });
  assert.equal((await instances.getSummary('icon-inst')).has_icon, true);
  assert.ok((await instances.readInstanceIcon('icon-inst')).equals(TINY_PNG));

  const removed = await instances.removeInstanceIcon('icon-inst');
  assert.deepEqual(removed, { ok: true, has_icon: false });
  assert.equal((await instances.getSummary('icon-inst')).has_icon, false);
  // Removing twice is a no-op, not an error.
  await instances.removeInstanceIcon('icon-inst');
  await assert.rejects(instances.writeInstanceIcon('no-such-instance', TINY_PNG), (e) => e.status === 404);
});

// --- PNG validator -----------------------------------------------------------

test('decodePngImage accepts data URL + raw base64, rejects non-PNG and oversize', () => {
  assert.ok(decodePngImage(TINY_DATA_URL).equals(TINY_PNG));
  assert.ok(decodePngImage(TINY_PNG_B64).equals(TINY_PNG));
  assert.throws(() => decodePngImage('not-base64!!!'), (e) => e.status === 400 && e.code === 'BAD_IMAGE');
  assert.throws(
    () => decodePngImage(`data:image/jpeg;base64,${TINY_PNG_B64}`),
    (e) => e.status === 400 && e.code === 'BAD_IMAGE',
  );
  assert.throws(() => decodePngImage(''), (e) => e.status === 400 && e.code === 'BAD_IMAGE');
  assert.throws(() => decodePngImage(null), (e) => e.status === 400 && e.code === 'BAD_IMAGE');
  // 500KB + 1 byte with a PNG header must trip the size cap.
  const big = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    Buffer.alloc(500 * 1024 + 1 - 8),
  ]);
  assert.throws(() => decodePngImage(big.toString('base64')), (e) => e.status === 400 && e.code === 'BAD_IMAGE');
  // Exactly 500KB with a PNG header passes.
  const edge = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    Buffer.alloc(500 * 1024 - 8),
  ]);
  assert.equal(decodePngImage(edge.toString('base64')).length, 500 * 1024);
});

// --- accounts ----------------------------------------------------------------

test('createAccount auto-assigns spaced avatar colors; publicAccount exposes flags', () => {
  const a = accounts.createAccount('AvatarAl');
  const b = accounts.createAccount('AvatarBo');
  assertHue(a.avatar_color);
  assertHue(b.avatar_color);
  assert.ok(
    circularDistance(a.avatar_color, b.avatar_color) >= 30,
    `avatar colors ${a.avatar_color}/${b.avatar_color} must be >= 30 apart`,
  );
  const pub = accounts.publicAccount(accounts.getAccount('AvatarAl'));
  assert.equal(pub.avatar_color, a.avatar_color);
  assert.equal(pub.has_avatar, false);
});

test('setAvatarColor validates, clears, and 404s on unknown accounts', () => {
  accounts.createAccount('ColorUser');
  const set = accounts.setAvatarColor('ColorUser', 42);
  assert.equal(set.avatar_color, 42);
  assert.equal(accounts.publicAccount(accounts.getAccount('ColorUser')).avatar_color, 42);
  const cleared = accounts.setAvatarColor('ColorUser', null);
  assert.equal(cleared.avatar_color, null);
  assert.throws(
    () => accounts.setAvatarColor('ColorUser', 400),
    (e) => e.status === 400 && e.code === 'BAD_AVATAR_COLOR',
  );
  assert.throws(
    () => accounts.setAvatarColor('NobodyHere', 10),
    (e) => e.status === 404,
  );
});

test('account avatar roundtrip flips has_avatar', async () => {
  accounts.createAccount('AvatarUser');
  assert.equal(accounts.publicAccount(accounts.getAccount('AvatarUser')).has_avatar, false);
  await assert.rejects(accounts.readAccountAvatar('AvatarUser'), (e) => e.status === 404);

  const written = await accounts.writeAccountAvatar('AvatarUser', TINY_PNG);
  assert.deepEqual(written, { ok: true, has_avatar: true });
  assert.equal(accounts.publicAccount(accounts.getAccount('AvatarUser')).has_avatar, true);
  assert.ok((await accounts.readAccountAvatar('AvatarUser')).equals(TINY_PNG));

  const removed = await accounts.removeAccountAvatar('AvatarUser');
  assert.deepEqual(removed, { ok: true, has_avatar: false });
  assert.equal(accounts.publicAccount(accounts.getAccount('AvatarUser')).has_avatar, false);
  await assert.rejects(accounts.readAccountAvatar('NobodyHere'), (e) => e.status === 404);
});

// --- launch gameDir selection (no network: pure helper) -----------------------

test('selectGameDir: custom game_dir wins, mkdir -p, skips profile split', () => {
  const custom = path.join(dataDir, 'launch-custom');
  const other = { username: 'Other', uuid: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee' };
  const warnings = [];
  const sel = launch.selectGameDir({ name: 'whatever', game_dir: custom }, other, { warnings });
  assert.equal(sel.gameDir, custom);
  assert.equal(sel.nativesDir, resolver.instanceNativesDir('whatever'));
  assert.ok(fs.existsSync(custom), 'custom game_dir is mkdir -p');
  assert.ok(
    !fs.existsSync(path.join(resolver.instanceDir('whatever'), 'profiles')),
    'profile split is skipped when game_dir is set',
  );

  const dry = path.join(dataDir, 'launch-custom-dry');
  const selDry = launch.selectGameDir({ name: 'whatever', game_dir: dry }, other, { dryRun: true });
  assert.equal(selDry.gameDir, dry);
  assert.ok(!fs.existsSync(dry), 'dryRun never touches the filesystem');
});

test('selectGameDir: default split unchanged (active -> base, other -> profile)', () => {
  const active = accounts.createAccount('LaunchActive');
  accounts.setActiveAccount('LaunchActive');
  const inst = { name: 'split-inst', game_dir: null };
  const base = launch.selectGameDir(inst, active, {});
  assert.equal(base.gameDir, resolver.instanceDir('split-inst'));

  const alt = { username: 'LaunchAlt', uuid: '11111111-2222-3333-4444-555555555555' };
  const split = launch.selectGameDir(inst, alt, { dryRun: true });
  assert.equal(split.gameDir, path.join(resolver.instanceDir('split-inst'), 'profiles', alt.uuid));
  assert.equal(split.nativesDir, path.join(split.gameDir, 'natives'));
});
