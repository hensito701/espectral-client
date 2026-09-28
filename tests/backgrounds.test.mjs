import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir BEFORE importing the engine so module evaluation sees
// it too (same pattern as tests/personalization.test.mjs).
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-backgrounds-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const backgrounds = await import('../src/engine/backgrounds.mjs');
const instances = await import('../src/engine/instances.mjs');
const { loadClientConfig } = await import('../src/engine/client.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

// Forge a PNG buffer with arbitrary IHDR dims — the validators only inspect
// magic + IHDR, same as decodePngImage/skinPngDimensions.
function fakePng(w = 16, h = 16, payloadLen = 0) {
  const ihdr = Buffer.alloc(24);
  ihdr.writeUInt32BE(13, 0); // length
  ihdr.write('IHDR', 4, 'ascii');
  ihdr.writeUInt32BE(w, 8);
  ihdr.writeUInt32BE(h, 12);
  ihdr[16] = 8; // bit depth
  ihdr[17] = 6; // color type RGBA
  const png = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), ihdr, Buffer.alloc(payloadLen)]);
  return png;
}

const B64 = (buf) => `data:image/png;base64,${buf.toString('base64')}`;

async function rejectsWith(promise, code) {
  await assert.rejects(promise, (e) => e.code === code, `expected ${code}`);
}

// --- image pack import ------------------------------------------------------

test('importImagePack: happy path writes pack.json + image.png', async () => {
  const pack = await backgrounds.importImagePack({ name: 'Fondo Noche', image_base64: B64(fakePng(64, 32)) });
  assert.equal(pack.type, 'image');
  assert.equal(pack.width, 64);
  assert.equal(pack.height, 32);
  assert.match(pack.id, /^[a-z0-9][a-z0-9_-]{0,39}$/);
  const dir = backgrounds.packDir(pack.id);
  assert.ok(fs.existsSync(path.join(dir, 'image.png')));
  const meta = JSON.parse(fs.readFileSync(path.join(dir, 'pack.json'), 'utf8'));
  assert.equal(meta.type, 'image');
  assert.equal(meta.name, 'Fondo Noche');
  // Listed + previewable
  assert.ok(backgrounds.listPacks().some((p) => p.id === pack.id));
  assert.ok(backgrounds.packPreviewPath(pack.id).endsWith('image.png'));
});

test('importImagePack rejects non-PNG and oversized dims', async () => {
  await rejectsWith(backgrounds.importImagePack({ name: 'x', image_base64: B64(Buffer.from('not a png')) }), 'BAD_IMAGE');
  await rejectsWith(backgrounds.importImagePack({ name: 'x', image_base64: B64(fakePng(0, 0)) }), 'BAD_IMAGE');
  await rejectsWith(backgrounds.importImagePack({ name: 'x', image_base64: B64(fakePng(9000, 10)) }), 'BAD_IMAGE');
  await rejectsWith(backgrounds.importImagePack({ name: '', image_base64: B64(fakePng()) }), 'BAD_NAME');
});

test('importImagePack source_path reads a file from disk', async () => {
  const src = path.join(dataDir, 'wall.png');
  fs.writeFileSync(src, fakePng(1920, 1080));
  const pack = await backgrounds.importImagePack({ name: 'wall', source_path: src });
  assert.equal(pack.width, 1920);
  assert.equal(pack.height, 1080);
  // Nonexistent path is a validation error, not a crash.
  await rejectsWith(backgrounds.importImagePack({ name: 'nope', source_path: path.join(dataDir, 'missing.png') }), 'BAD_PATH');
});

// --- frame pack import ------------------------------------------------------

test('importFramesPack: folder import normalizes to frames/NNNN.png + fps', async () => {
  const dir = path.join(dataDir, 'frames-src');
  fs.mkdirSync(dir);
  // Deliberate non-numeric names: natural sort, not lexicographic.
  fs.writeFileSync(path.join(dir, 'f10.png'), fakePng(320, 180));
  fs.writeFileSync(path.join(dir, 'f2.png'), fakePng(320, 180));
  fs.writeFileSync(path.join(dir, 'f1.png'), fakePng(320, 180));
  fs.writeFileSync(path.join(dir, 'notes.txt'), 'ignored');
  const pack = await backgrounds.importFramesPack({ name: 'loop', fps: 30, source_path: dir });
  assert.equal(pack.type, 'frames');
  assert.equal(pack.frame_count, 3);
  assert.equal(pack.fps, 30);
  assert.equal(pack.width, 320);
  assert.equal(pack.height, 180);
  const framesDir = path.join(backgrounds.packDir(pack.id), 'frames');
  assert.deepEqual(fs.readdirSync(framesDir).sort(), ['0001.png', '0002.png', '0003.png']);
  // Order follows natural sort: 0001 == f1.png content.
  assert.equal(
    fs.readFileSync(path.join(framesDir, '0001.png')).length,
    fakePng(320, 180).length,
  );
});

test('importFramesPack: base64 frames[] path validates + clamps fps', async () => {
  const pack = await backgrounds.importFramesPack({
    name: 'b64loop',
    fps: 500,
    frames: [B64(fakePng(8, 8)), B64(fakePng(8, 8))],
  });
  assert.equal(pack.frame_count, 2);
  assert.equal(pack.fps, 60); // clamped to the contract ceiling
});

test('importFramesPack enforces dimension uniformity and per-frame cap', async () => {
  await rejectsWith(
    backgrounds.importFramesPack({
      name: 'mixed',
      frames: [B64(fakePng(100, 100)), B64(fakePng(200, 100))],
    }),
    'FRAME_DIMS',
  );
  await rejectsWith(
    backgrounds.importFramesPack({
      name: 'bigframe',
      frames: [B64(fakePng(1920, 1080))],
    }),
    'FRAME_TOO_LARGE',
  );
  await rejectsWith(
    backgrounds.importFramesPack({ name: 'empty', frames: [] }),
    'NO_FRAMES',
  );
});

test('importFramesPack enforces the 300-frame cap without reading files', async () => {
  const paths = Array.from({ length: 301 }, (_, i) => path.join(dataDir, `nope-${i}.png`));
  await rejectsWith(
    backgrounds.importFramesPack({ name: 'many', frame_paths: paths }),
    'TOO_MANY_FRAMES',
  );
});
test('importFramesPack enforces the 80 MB total cap', async () => {
  // Each file is under the 64 MB per-file read cap, so the cumulative
  // PACK_TOO_LARGE check is what fires.
  const big1 = path.join(dataDir, 'huge-frame-1.png');
  const big2 = path.join(dataDir, 'huge-frame-2.png');
  fs.writeFileSync(big1, fakePng(16, 16));
  fs.writeFileSync(big2, fakePng(16, 16));
  fs.truncateSync(big1, 45 * 1024 * 1024);
  fs.truncateSync(big2, 45 * 1024 * 1024);
  await rejectsWith(
    backgrounds.importFramesPack({ name: 'huge', frame_paths: [big1, big2] }),
    'PACK_TOO_LARGE',
  );
});

// --- pack delete ------------------------------------------------------------

test('deletePack removes the dir; unknown/malformed ids are errors', async () => {
  const pack = await backgrounds.importImagePack({ name: 'bye', image_base64: B64(fakePng()) });
  assert.ok(fs.existsSync(backgrounds.packDir(pack.id)));
  assert.deepEqual(backgrounds.deletePack(pack.id), { ok: true });
  assert.ok(!fs.existsSync(backgrounds.packDir(pack.id)));
  assert.throws(() => backgrounds.deletePack(pack.id), (e) => e.code === 'PACK_NOT_FOUND');
  assert.throws(() => backgrounds.deletePack('../escape'), (e) => e.code === 'BAD_PACK_ID');
  // Deleted pack no longer listed or previewable.
  assert.ok(!backgrounds.listPacks().some((p) => p.id === pack.id));
});

// --- selection (personalization.background) ---------------------------------

test('writePersonalization stores/clears personalization.background', async () => {
  await instances.createInstance({ name: 'bg-sel', version: '1.21.11' });
  const pack = await backgrounds.importImagePack({ name: 'mine', image_base64: B64(fakePng(50, 50)) });

  const r1 = await backgrounds.writePersonalization('bg-sel', { background: pack.id });
  assert.equal(r1.config.personalization.background, pack.id);
  assert.equal(loadClientConfig('bg-sel').personalization.background, pack.id);

  const r2 = await backgrounds.writePersonalization('bg-sel', { background: null });
  assert.equal(r2.config.personalization.background, null);

  // Unknown pack id → 404; unknown personalization key → 400.
  await rejectsWith(backgrounds.writePersonalization('bg-sel', { background: 'nope' }), 'PACK_NOT_FOUND');
  await rejectsWith(backgrounds.writePersonalization('bg-sel', { bogus: 1 }), 'BAD_PERSONALIZATION');
  // hud_theme goes through patchClientConfig's whitelist too.
  const r3 = await backgrounds.writePersonalization('bg-sel', { hud_theme: 'pixel' });
  assert.equal(r3.config.personalization.hud_theme, 'pixel');
  await rejectsWith(backgrounds.writePersonalization('bg-sel', { hud_theme: 'neon' }), 'BAD_PERSONALIZATION');
});

test('migrateLegacyBackground turns the old hand-placed PNG into a selected pack', async () => {
  await instances.createInstance({ name: 'bg-legacy', version: '1.21.11' });
  const instDir = path.join(dataDir, 'legacy-inst');
  const profileDir = path.join(dataDir, 'legacy-profile');
  for (const d of [instDir, profileDir]) {
    fs.mkdirSync(path.join(d, 'config'), { recursive: true });
    fs.writeFileSync(path.join(d, 'config', 'espectral-menu-background.png'), fakePng(40, 20));
  }
  const meta = await backgrounds.migrateLegacyBackground('bg-legacy', [instDir, profileDir]);
  assert.equal(meta.type, 'image');
  assert.equal(loadClientConfig('bg-legacy').personalization.background, meta.id);
  for (const d of [instDir, profileDir]) {
    assert.equal(fs.existsSync(path.join(d, 'config', 'espectral-menu-background.png')), false);
  }
  // Nothing left to migrate; an existing selection is never overridden.
  assert.equal(await backgrounds.migrateLegacyBackground('bg-legacy', [instDir, profileDir]), null);
  const other = await backgrounds.importImagePack({ name: 'keep', image_base64: B64(fakePng(8, 8)) });
  await backgrounds.writePersonalization('bg-legacy', { background: other.id });
  fs.writeFileSync(path.join(instDir, 'config', 'espectral-menu-background.png'), fakePng(40, 20));
  await backgrounds.migrateLegacyBackground('bg-legacy', [instDir]);
  assert.equal(loadClientConfig('bg-legacy').personalization.background, other.id);
});

// --- launch staging ---------------------------------------------------------

test('stageBackgroundPack mirrors the selected pack into the profile gameDir', async () => {
  await instances.createInstance({ name: 'bg-stage', version: '1.21.11' });
  const pack = await backgrounds.importFramesPack({
    name: 'staged',
    frames: [B64(fakePng(8, 8)), B64(fakePng(8, 8))],
  });
  await backgrounds.writePersonalization('bg-stage', { background: pack.id });

  // Contract C per-account profile dir (what launch.mjs does).
  const profileDir = path.join(dataDir, 'instances', 'bg-stage', 'profiles', 'acct1');
  fs.mkdirSync(profileDir, { recursive: true });

  backgrounds.stageBackgroundPack(backgrounds.selectedBackground('bg-stage'), profileDir);
  const staged = path.join(profileDir, 'config', 'espectral-menu-bg', pack.id);
  assert.ok(fs.existsSync(path.join(staged, 'pack.json')));
  assert.ok(fs.existsSync(path.join(staged, 'frames', '0001.png')));
  assert.ok(fs.existsSync(path.join(staged, 'frames', '0002.png')));

  // Restage is a no-op (pack.json identical) — mutating a staged frame does
  // NOT trigger a copy until the pack.json changes (packs are immutable).
  const before = fs.statSync(path.join(staged, 'frames', '0001.png')).mtimeMs;
  backgrounds.stageBackgroundPack(pack.id, profileDir);
  assert.equal(fs.statSync(path.join(staged, 'frames', '0001.png')).mtimeMs, before);

  // A second pack staged alongside gets pruned when it is not selected.
  const stale = path.join(profileDir, 'config', 'espectral-menu-bg', 'stale-pack');
  fs.mkdirSync(stale, { recursive: true });
  fs.writeFileSync(path.join(staged, 'pack.json'), '{}');
  backgrounds.stageBackgroundPack(pack.id, profileDir);
  assert.ok(!fs.existsSync(stale));
  assert.ok(fs.existsSync(staged));

  // Deselect → the whole staging dir is removed (mod falls back).
  await backgrounds.writePersonalization('bg-stage', { background: null });
  backgrounds.stageBackgroundPack(backgrounds.selectedBackground('bg-stage'), profileDir);
  assert.ok(!fs.existsSync(path.join(profileDir, 'config', 'espectral-menu-bg')));
});

test('stageBackgroundPack wipes the staging dir when the store pack vanished', async () => {
  const profileDir = path.join(dataDir, 'profiles-phantom');
  const stale = path.join(profileDir, 'config', 'espectral-menu-bg', 'gone');
  fs.mkdirSync(stale, { recursive: true });
  backgrounds.stageBackgroundPack('gone', profileDir);
  assert.ok(!fs.existsSync(path.join(profileDir, 'config', 'espectral-menu-bg')));
});

// --- HTTP route wiring ------------------------------------------------------

test('background routes: list/import/preview/delete + PATCH personalization', async () => {
  const { start } = await import('../src/engine/server.mjs');
  const server = await start(0);
  const base = `http://127.0.0.1:${server.address().port}`;
  const H = { 'Content-Type': 'application/json', 'x-espectral-client': '1' };
  try {
    await instances.createInstance({ name: 'bg-http', version: '1.21.11' });

    // Import an image pack over HTTP (icon-upload base64 pattern).
    const imp = await fetch(`${base}/api/backgrounds`, {
      method: 'POST',
      headers: H,
      body: JSON.stringify({ name: 'http pack', image_base64: B64(fakePng(20, 10)) }),
    });
    assert.equal(imp.status, 200);
    const { pack } = await imp.json();
    assert.equal(pack.type, 'image');
    assert.equal(pack.width, 20);

    // Listed + preview bytes are a PNG.
    const list = await (await fetch(`${base}/api/backgrounds`, { headers: H })).json();
    assert.ok(list.packs.some((p) => p.id === pack.id));
    const img = await fetch(`${base}/api/backgrounds/${pack.id}/image`, { headers: H });
    assert.equal(img.status, 200);
    assert.equal(img.headers.get('content-type'), 'image/png');
    const bytes = Buffer.from(await img.arrayBuffer());
    assert.ok(bytes.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])));

    // Selection PATCH writes personalization.background and answers ClientInfo.
    const sel = await fetch(`${base}/api/instances/bg-http/personalization`, {
      method: 'PATCH',
      headers: H,
      body: JSON.stringify({ background: pack.id }),
    });
    assert.equal(sel.status, 200);
    const info = await sel.json();
    assert.equal(info.config.personalization.background, pack.id);

    // Unknown pack id → 404 envelope; malformed body → 400 envelope.
    const bad = await fetch(`${base}/api/instances/bg-http/personalization`, {
      method: 'PATCH',
      headers: H,
      body: JSON.stringify({ background: 'missing-pack' }),
    });
    assert.equal(bad.status, 404);
    assert.equal((await bad.json()).error.code, 'PACK_NOT_FOUND');
    const badPatch = await fetch(`${base}/api/instances/bg-http/personalization`, {
      method: 'PATCH',
      headers: H,
      body: JSON.stringify(['not', 'an', 'object']),
    });
    assert.equal(badPatch.status, 400);
    assert.equal((await badPatch.json()).error.code, 'BAD_PERSONALIZATION');

    // Delete removes it from the store and the list.
    const del = await fetch(`${base}/api/backgrounds/${pack.id}`, { method: 'DELETE', headers: H });
    assert.equal(del.status, 200);
    const list2 = await (await fetch(`${base}/api/backgrounds`, { headers: H })).json();
    assert.ok(!list2.packs.some((p) => p.id === pack.id));
  } finally {
    await new Promise((r) => server.close(r));
  }
});
