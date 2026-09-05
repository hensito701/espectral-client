import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir before importing so tests never touch real data.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const tmpDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-skins-test-'));
process.env.ESPECTRAL_DATA_DIR = tmpDataDir;

const skins = await import('../src/engine/skins.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(tmpDataDir, { recursive: true, force: true });
});

/** Minimal PNG: real magic + IHDR with the given dims (no IDAT needed for validation). */
function pngHeader(width, height) {
  const buf = Buffer.alloc(30);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(buf, 0);
  buf.writeUInt32BE(13, 8);
  buf.write('IHDR', 12);
  buf.writeUInt32BE(width, 16);
  buf.writeUInt32BE(height, 20);
  buf[24] = 8; // bit depth
  buf[25] = 6; // color type RGBA
  return buf;
}

const dataUrl = (buf) => `data:image/png;base64,${buf.toString('base64')}`;

test('skinPngDimensions parses IHDR dims and rejects junk', () => {
  assert.deepEqual(skins.skinPngDimensions(pngHeader(64, 64)), { width: 64, height: 64 });
  assert.deepEqual(skins.skinPngDimensions(pngHeader(64, 32)), { width: 64, height: 32 });
  assert.equal(skins.skinPngDimensions(Buffer.from('hello')), null);
  assert.equal(skins.skinPngDimensions(Buffer.alloc(10)), null);
  assert.equal(skins.skinPngDimensions(null), null);
});

test('isValidSkinPng accepts 64x64 and 64x32 only', () => {
  assert.equal(skins.isValidSkinPng(pngHeader(64, 64)), true);
  assert.equal(skins.isValidSkinPng(pngHeader(64, 32)), true);
  assert.equal(skins.isValidSkinPng(pngHeader(16, 16)), false);
  assert.equal(skins.isValidSkinPng(pngHeader(128, 128)), false);
});

test('decodeSkinPng accepts data URLs and raw base64, rejects non-PNG', () => {
  const buf = skins.decodeSkinPng(dataUrl(pngHeader(64, 64)));
  assert.ok(Buffer.isBuffer(buf));
  assert.throws(() => skins.decodeSkinPng('aGVsbG8='), (e) => e.code === 'BAD_IMAGE');
  assert.throws(() => skins.decodeSkinPng('data:image/jpeg;base64,aGVsbG8='), (e) => e.code === 'BAD_IMAGE');
  assert.throws(() => skins.decodeSkinPng(''), (e) => e.code === 'BAD_IMAGE');
});

test('library CRUD roundtrip', async () => {
  const saved = await skins.saveLibrarySkin({
    name: 'hen chad',
    variant: 'slim',
    png: pngHeader(64, 64),
    source: 'upload',
  });
  assert.equal(saved.name, 'hen chad');
  assert.equal(saved.variant, 'slim');
  const list = await skins.loadLibraryIndex();
  assert.equal(list.length, 1);

  const art = await skins.readLibraryPng(saved.id);
  assert.deepEqual(art.subarray(0, 8), pngHeader(64, 64).subarray(0, 8));

  const renamed = await skins.updateLibrarySkin(saved.id, { name: 'hen chad v2', variant: 'classic' });
  assert.equal(renamed.name, 'hen chad v2');
  assert.equal(renamed.variant, 'classic');

  assert.deepEqual(await skins.deleteLibrarySkin(saved.id), { removed: true });
  assert.deepEqual(await skins.loadLibraryIndex(), []);
});

test('library rejects bad names, variants, dims, unknown ids', async () => {
  await assert.rejects(
    skins.saveLibrarySkin({ name: '  ', variant: 'classic', png: pngHeader(64, 64) }),
    (e) => e.code === 'BAD_SKIN_NAME',
  );
  await assert.rejects(
    skins.saveLibrarySkin({ name: 'x'.repeat(41), variant: 'classic', png: pngHeader(64, 64) }),
    (e) => e.code === 'BAD_SKIN_NAME',
  );
  await assert.rejects(
    skins.saveLibrarySkin({ name: 'ok', variant: 'wide', png: pngHeader(64, 64) }),
    (e) => e.code === 'BAD_SKIN_VARIANT',
  );
  await assert.rejects(
    skins.saveLibrarySkin({ name: 'ok', variant: 'classic', png: pngHeader(16, 16) }),
    (e) => e.code === 'BAD_SKIN_DIMS',
  );
  await assert.rejects(skins.getLibraryEntry('nope'), (e) => e.code === 'UNKNOWN_SKIN');
  await assert.rejects(skins.deleteLibrarySkin('nope'), (e) => e.code === 'UNKNOWN_SKIN');
  await assert.rejects(skins.updateLibrarySkin('nope', { name: 'x' }), (e) => e.code === 'UNKNOWN_SKIN');
});

test('importVanillaSkins imports, skips dups and bad art, is idempotent', async () => {
  const fixture = {
    version: 1,
    customSkins: {
      a: { id: 'a', name: 'hen mujer', created: '2023-12-24T04:14:18.586Z', skinImage: dataUrl(pngHeader(64, 64)) },
      b: { id: 'b', name: 'HEN MUJER', created: '2024-01-01T00:00:00.000Z', skinImage: dataUrl(pngHeader(64, 64)) },
      c: { id: 'c', name: 'broken', created: '2024-01-01T00:00:00.000Z', skinImage: dataUrl(pngHeader(16, 16)) },
      d: { id: 'd', name: 'garbage', created: '2024-01-01T00:00:00.000Z', skinImage: 'not-an-image' },
    },
  };
  const fp = path.join(tmpDataDir, 'launcher_custom_skins.json');
  fs.writeFileSync(fp, JSON.stringify(fixture));

  const first = await skins.importVanillaSkins(fp);
  assert.deepEqual(first, { imported: 1, skipped: 3, total: 4 });

  const index = await skins.loadLibraryIndex();
  assert.equal(index.length, 1);
  assert.equal(index[0].name, 'hen mujer');
  assert.equal(index[0].variant, 'classic');
  assert.equal(index[0].source, 'vanilla');

  const second = await skins.importVanillaSkins(fp);
  assert.deepEqual(second, { imported: 0, skipped: 4, total: 4 });
});

test('importVanillaSkins 404s when the vanilla file is absent', async () => {
  await assert.rejects(
    skins.importVanillaSkins(path.join(tmpDataDir, 'does-not-exist.json')),
    (e) => e.code === 'NO_VANILLA_SKINS',
  );
});
