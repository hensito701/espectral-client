import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import zlib from 'node:zlib';

// Sandbox the data dir before importing so tests never touch real data.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const tmpDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-skin-avatar-test-'));
process.env.ESPECTRAL_DATA_DIR = tmpDataDir;

const skins = await import('../src/engine/skins.mjs');
const accounts = await import('../src/engine/accounts.mjs');
const { register } = await import('../src/engine/routes/misc.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(tmpDataDir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// Manual PNG fixture builder (own chunks + own CRC32 + node:zlib deflate —
// deliberately NOT the skins.mjs encoder under test).
// ---------------------------------------------------------------------------

const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const CRC_T = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32Range(buf, s, e) {
  let c = 0xffffffff;
  for (let i = s; i < e; i++) c = CRC_T[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const out = Buffer.alloc(8 + data.length + 4);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'ascii');
  data.copy(out, 8);
  out.writeUInt32BE(crc32Range(out, 4, 8 + data.length), 8 + data.length);
  return out;
}

function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

/** rows: [{ filter, data }] scanlines; depth/color/interlace injectable. */
function encodePng({ width, height, rows, depth = 8, color = 6, interlace = 0 }) {
  const raw = Buffer.concat(rows.map(({ filter, data }) => Buffer.concat([Buffer.from([filter]), data])));
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = depth;
  ihdr[9] = color;
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = interlace;
  return Buffer.concat([
    PNG_MAGIC,
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** 64-wide RGBA skin canvas; paints base + face (8,8,8,8) + hat (40,8,8,8). */
function skinPixels({ face = [255, 0, 0, 255], hat = [0, 0, 0, 0], base = [0, 0, 255, 255], height = 64 } = {}) {
  const W = 64;
  const px = Buffer.alloc(W * height * 4);
  const fill = (x0, y0, w, h, c) => {
    for (let y = y0; y < y0 + h && y < height; y++) {
      for (let x = x0; x < x0 + w; x++) {
        const o = (y * W + x) * 4;
        px[o] = c[0]; px[o + 1] = c[1]; px[o + 2] = c[2]; px[o + 3] = c[3];
      }
    }
  };
  fill(0, 0, W, height, base);
  fill(8, 8, 8, 8, face);
  fill(40, 8, 8, 8, hat);
  return px;
}

function filterEncodeRow(row, prev, f) {
  const out = Buffer.alloc(row.length);
  for (let i = 0; i < row.length; i++) {
    const a = i >= 4 ? row[i - 4] : 0;
    const b = prev[i];
    const c = i >= 4 ? prev[i - 4] : 0;
    const pred = f === 1 ? a : f === 2 ? b : f === 3 ? (a + b) >> 1 : paeth(a, b, c);
    out[i] = (row[i] - pred) & 0xff;
  }
  return out;
}

/** Skin PNG with per-row filter bytes (filterFor(y) in 0..4), properly encoded. */
function skinPngFiltered({ height = 64, filterFor = () => 0, ...colors } = {}) {
  const px = skinPixels({ ...colors, height });
  const stride = 64 * 4;
  const prev = Buffer.alloc(stride);
  const rows = [];
  for (let y = 0; y < height; y++) {
    const row = px.subarray(y * stride, (y + 1) * stride);
    const f = filterFor(y);
    rows.push({ filter: f, data: f === 0 ? Buffer.from(row) : filterEncodeRow(row, prev, f) });
    prev.set(row);
  }
  return encodePng({ width: 64, height, rows });
}

const skinPng = (colors) => skinPngFiltered({ ...colors, filterFor: () => 0 });

/** Minimal decoder for assertions (expects filter-0 RGBA rows). */
function decodeRgba(png) {
  let pos = 8;
  let width = 0;
  let height = 0;
  const parts = [];
  while (pos + 8 <= png.length) {
    const len = png.readUInt32BE(pos);
    const type = png.toString('ascii', pos + 4, pos + 8);
    const ds = pos + 8;
    const de = ds + len;
    if (type === 'IHDR') {
      width = png.readUInt32BE(ds);
      height = png.readUInt32BE(ds + 4);
    } else if (type === 'IDAT') {
      parts.push(png.subarray(ds, de));
    } else if (type === 'IEND') {
      break;
    }
    pos = de + 4;
  }
  const raw = zlib.inflateSync(Buffer.concat(parts));
  const stride = width * 4;
  const px = Buffer.alloc(width * height * 4);
  for (let y = 0; y < height; y++) {
    assert.equal(raw[y * (stride + 1)], 0, 'expected filter-0 avatar rows');
    raw.copy(px, y * stride, y * (stride + 1) + 1, (y + 1) * (stride + 1));
  }
  return { width, height, px };
}

const pixel = (d, x, y) => {
  const o = (y * d.width + x) * 4;
  return [d.px[o], d.px[o + 1], d.px[o + 2], d.px[o + 3]];
};

/** Fake express-ish app capturing route handlers by "METHOD path". */
function makeApp() {
  const routes = new Map();
  const app = {
    get: (p, h) => routes.set(`GET ${p}`, h),
    post: (p, h) => routes.set(`POST ${p}`, h),
    put: (p, h) => routes.set(`PUT ${p}`, h),
    patch: (p, h) => routes.set(`PATCH ${p}`, h),
    delete: (p, h) => routes.set(`DELETE ${p}`, h),
  };
  return { app, routes };
}

const { app, routes } = makeApp();
await register(app);
const postAvatar = routes.get('POST /api/accounts/:username/avatar');

// 1x1 PNG for the custom-avatar side (content never inspected, magic only).
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  'base64',
);

// ---------------------------------------------------------------------------
// extractHeadPng
// ---------------------------------------------------------------------------

test('extractHeadPng: transparent hat shows the face, scaled to 64x64', () => {
  const head = skins.extractHeadPng(skinPng({ face: [255, 0, 0, 255], hat: [0, 0, 0, 0] }));
  const dims = skins.skinPngDimensions(head);
  assert.deepEqual(dims, { width: 64, height: 64 });
  const d = decodeRgba(head);
  assert.deepEqual(pixel(d, 32, 32), [255, 0, 0, 255]);
  assert.deepEqual(pixel(d, 0, 0), [255, 0, 0, 255]);
  assert.deepEqual(pixel(d, 63, 63), [255, 0, 0, 255]);
});

test('extractHeadPng: opaque hat covers the face', () => {
  const head = skins.extractHeadPng(skinPng({ face: [255, 0, 0, 255], hat: [0, 255, 0, 255] }));
  const d = decodeRgba(head);
  assert.deepEqual(pixel(d, 32, 32), [0, 255, 0, 255]);
});

test('extractHeadPng: semi-transparent hat blends over the face', () => {
  const head = skins.extractHeadPng(skinPng({ face: [255, 0, 0, 255], hat: [0, 0, 255, 128] }));
  const d = decodeRgba(head);
  assert.deepEqual(pixel(d, 32, 32), [127, 0, 128, 255]);
});

test('extractHeadPng: decodes all five PNG filter types', () => {
  const plain = skins.extractHeadPng(skinPng({ face: [255, 0, 0, 255], hat: [0, 255, 0, 255] }));
  const filtered = skins.extractHeadPng(
    skinPngFiltered({
      face: [255, 0, 0, 255],
      hat: [0, 255, 0, 255],
      filterFor: (y) => (y % 4) + 1, // cycles Sub, Up, Average, Paeth
    }),
  );
  assert.ok(filtered.equals(plain), 'filtered source must decode to identical head bytes');
});

test('extractHeadPng: accepts 64x32 legacy skins', () => {
  const head = skins.extractHeadPng(skinPng({ height: 32, face: [255, 0, 0, 255], hat: [0, 0, 0, 0] }));
  const d = decodeRgba(head);
  assert.deepEqual([d.width, d.height], [64, 64]);
  assert.deepEqual(pixel(d, 32, 32), [255, 0, 0, 255]);
});

test('extractHeadPng: rejects non-RGBA, interlaced, and wrong-size art with BAD_IMAGE', () => {
  const px64 = skinPixels({ height: 64 });
  const rows64 = [];
  for (let y = 0; y < 64; y++) rows64.push({ filter: 0, data: px64.subarray(y * 256, (y + 1) * 256) });
  // Truecolor (color type 2) with matching RGB row bytes.
  const rgbRows = [];
  for (let y = 0; y < 64; y++) {
    const rgb = Buffer.alloc(64 * 3);
    for (let x = 0; x < 64; x++) {
      const s = (y * 64 + x) * 4;
      rgb[x * 3] = px64[s];
      rgb[x * 3 + 1] = px64[s + 1];
      rgb[x * 3 + 2] = px64[s + 2];
    }
    rgbRows.push({ filter: 0, data: rgb });
  }
  // 16x16 RGBA (valid PNG, wrong dims).
  const tiny = Buffer.alloc(16 * 16 * 4, 0x80);
  const tinyRows = [];
  for (let y = 0; y < 16; y++) tinyRows.push({ filter: 0, data: tiny.subarray(y * 64, (y + 1) * 64) });
  const bad = [
    ['rgb-not-rgba', encodePng({ width: 64, height: 64, rows: rgbRows, color: 2 })],
    ['interlaced', encodePng({ width: 64, height: 64, rows: rows64, interlace: 1 })],
    ['wrong-size', encodePng({ width: 16, height: 16, rows: tinyRows })],
  ];
  for (const [label, buf] of bad) {
    assert.throws(() => skins.extractHeadPng(buf), (e) => e.status === 400 && e.code === 'BAD_IMAGE', label);
  }
  assert.throws(() => skins.extractHeadPng(Buffer.from('not a png')), (e) => e.code === 'BAD_IMAGE', 'junk');
});

// ---------------------------------------------------------------------------
// sync / no-clobber / marker lifecycle
// ---------------------------------------------------------------------------

test('syncAccountAvatarFromSkin: stores the head and marks avatar_auto; re-sync overwrites auto', async () => {
  accounts.createAccount('HeadUser');
  const first = await skins.syncAccountAvatarFromSkin(
    'HeadUser',
    skinPng({ face: [255, 0, 0, 255], hat: [0, 0, 0, 0] }),
  );
  assert.deepEqual(first, { ok: true, auto: true });
  assert.equal(accounts.getAccount('HeadUser').avatar_auto, true);
  assert.equal(accounts.publicAccount(accounts.getAccount('HeadUser')).has_avatar, true);
  assert.deepEqual(pixel(decodeRgba(await accounts.readAccountAvatar('HeadUser')), 32, 32), [255, 0, 0, 255]);

  const second = await skins.syncAccountAvatarFromSkin(
    'HeadUser',
    skinPng({ face: [0, 255, 0, 255], hat: [0, 0, 0, 0] }),
  );
  assert.deepEqual(second, { ok: true, auto: true });
  assert.deepEqual(pixel(decodeRgba(await accounts.readAccountAvatar('HeadUser')), 32, 32), [0, 255, 0, 255]);
});

test('syncAccountAvatarFromSkin: never clobbers a user-uploaded avatar', async () => {
  accounts.createAccount('CustomUser');
  await accounts.writeAccountAvatar('CustomUser', TINY_PNG);
  assert.equal(accounts.getAccount('CustomUser').avatar_auto, undefined);
  const res = await skins.syncAccountAvatarFromSkin(
    'CustomUser',
    skinPng({ face: [255, 0, 0, 255], hat: [0, 0, 0, 0] }),
  );
  assert.deepEqual(res, { skipped: true, reason: 'custom' });
  assert.ok((await accounts.readAccountAvatar('CustomUser')).equals(TINY_PNG), 'custom bytes intact');
  assert.equal(accounts.getAccount('CustomUser').avatar_auto, undefined);
});

test('syncAccountAvatarFromSkin: 404s on unknown accounts', async () => {
  await assert.rejects(
    skins.syncAccountAvatarFromSkin('NobodyHere', skinPng({})),
    (e) => e.status === 404,
  );
});

test('custom avatar upload clears the marker; publicAccount keeps the has_avatar-only contract', async () => {
  accounts.createAccount('MarkerUser');
  await skins.syncAccountAvatarFromSkin('MarkerUser', skinPng({ face: [255, 0, 0, 255], hat: [0, 0, 0, 0] }));
  assert.equal(accounts.getAccount('MarkerUser').avatar_auto, true);
  await accounts.writeAccountAvatar('MarkerUser', TINY_PNG);
  assert.equal(accounts.getAccount('MarkerUser').avatar_auto, undefined);
  const pub = accounts.publicAccount(accounts.getAccount('MarkerUser'));
  assert.equal(pub.has_avatar, true);
  assert.ok(!('avatar_auto' in pub), 'internal marker must not leak to the UI');
});

test('clearAccountAutoAvatar: drops auto avatars, keeps custom ones', async () => {
  accounts.createAccount('AutoDrop');
  await skins.syncAccountAvatarFromSkin('AutoDrop', skinPng({ face: [255, 0, 0, 255], hat: [0, 0, 0, 0] }));
  assert.deepEqual(await skins.clearAccountAutoAvatar('AutoDrop'), { ok: true });
  assert.equal(accounts.publicAccount(accounts.getAccount('AutoDrop')).has_avatar, false);
  assert.equal(accounts.getAccount('AutoDrop').avatar_auto, undefined);

  accounts.createAccount('CustomKeep');
  await accounts.writeAccountAvatar('CustomKeep', TINY_PNG);
  assert.deepEqual(await skins.clearAccountAutoAvatar('CustomKeep'), { skipped: true });
  assert.ok((await accounts.readAccountAvatar('CustomKeep')).equals(TINY_PNG), 'custom avatar kept');
});

// ---------------------------------------------------------------------------
// Hook level (one route file: misc.mjs avatar upload clears the auto marker)
// ---------------------------------------------------------------------------

test('POST /api/accounts/:username/avatar clears avatar_auto (route level)', async () => {
  accounts.createAccount('HookUser');
  await skins.syncAccountAvatarFromSkin('HookUser', skinPng({ face: [255, 0, 0, 255], hat: [0, 0, 0, 0] }));
  assert.equal(accounts.getAccount('HookUser').avatar_auto, true);
  const res = await postAvatar({}, {}, { username: 'HookUser' }, { image_base64: `data:image/png;base64,${TINY_PNG.toString('base64')}` });
  assert.deepEqual(res, { ok: true, has_avatar: true });
  assert.equal(accounts.getAccount('HookUser').avatar_auto, undefined);
  assert.ok((await accounts.readAccountAvatar('HookUser')).equals(TINY_PNG));
});
