import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir before importing so the live engine never touches real data.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const tmpDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-skin-test-'));
process.env.ESPECTRAL_DATA_DIR = tmpDataDir;

const { getProfileSkin, uploadProfileSkin, resetProfileSkin } = await import('../src/engine/msauth.mjs');
const { register, skinPngDimensions } = await import('../src/engine/routes/misc.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(tmpDataDir, { recursive: true, force: true });
});

/** Minimal fetch-Response stub: ok/status/headers.get + json()/text(). */
function stubResponse({ status = 200, body = {}, headers = {} } = {}) {
  const lower = new Map(Object.entries(headers).map(([k, v]) => [String(k).toLowerCase(), String(v)]));
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (name) => lower.get(String(name).toLowerCase()) ?? null },
    json: async () => body,
    text: async () => JSON.stringify(body),
  };
}

/** Minimal PNG: 8-byte signature + IHDR chunk with the given dimensions. */
function makePng(width, height) {
  const buf = Buffer.alloc(29);
  buf[0] = 0x89; buf[1] = 0x50; buf[2] = 0x4e; buf[3] = 0x47;
  buf[4] = 0x0d; buf[5] = 0x0a; buf[6] = 0x1a; buf[7] = 0x0a;
  buf.writeUInt32BE(13, 8); // IHDR data length
  buf.write('IHDR', 12);
  buf.writeUInt32BE(width, 16);
  buf.writeUInt32BE(height, 20);
  buf[24] = 8; // bit depth
  buf[25] = 2; // color type (truecolor)
  return buf;
}

/** Await a handler that must throw; return the error for assertions. */
async function throwsHttp(fn) {
  try {
    await fn();
  } catch (err) {
    return err;
  }
  assert.fail('expected an httpError to be thrown');
}

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
const postSkin = routes.get('POST /api/accounts/:username/skin');
const getSkin = routes.get('GET /api/accounts/:username/skin');

// ---------------------------------------------------------------------------
// skinPngDimensions vectors
// ---------------------------------------------------------------------------

test('skinPngDimensions: parses IHDR width/height vectors', () => {
  assert.deepEqual(skinPngDimensions(makePng(64, 64)), { width: 64, height: 64 });
  assert.deepEqual(skinPngDimensions(makePng(64, 32)), { width: 64, height: 32 });
  assert.deepEqual(skinPngDimensions(makePng(16, 16)), { width: 16, height: 16 });
});

test('skinPngDimensions: null on short / non-PNG buffers', () => {
  assert.equal(skinPngDimensions(Buffer.alloc(0)), null);
  assert.equal(skinPngDimensions(makePng(64, 64).subarray(0, 10)), null);
  const notIhdr = makePng(64, 64);
  notIhdr.write('IDAT', 12);
  assert.equal(skinPngDimensions(notIhdr), null);
  assert.equal(skinPngDimensions(null), null);
});

// ---------------------------------------------------------------------------
// POST /api/accounts/:username/skin validation (no network: bad bodies throw
// before any account lookup)
// ---------------------------------------------------------------------------

test('POST skin: bad variant -> 400 BAD_SKIN_VARIANT', async () => {
  for (const variant of ['wide', 'CLASSIC', '', undefined]) {
    const err = await throwsHttp(() => postSkin({}, null, { username: 'nobody' }, {
      image_base64: makePng(64, 64).toString('base64'),
      variant,
    }));
    assert.equal(err.status, 400, `variant ${String(variant)}`);
    assert.equal(err.code, 'BAD_SKIN_VARIANT', `variant ${String(variant)}`);
  }
});

test('POST skin: 16x16 PNG -> 400 BAD_SKIN_DIMS', async () => {
  const err = await throwsHttp(() => postSkin({}, null, { username: 'nobody' }, {
    image_base64: makePng(16, 16).toString('base64'),
    variant: 'classic',
  }));
  assert.equal(err.status, 400);
  assert.equal(err.code, 'BAD_SKIN_DIMS');
});

test('POST skin: 64x64 + 64x32 pass validation (fail later on unknown account)', async () => {
  for (const [w, h] of [[64, 64], [64, 32]]) {
    const err = await throwsHttp(() => postSkin({}, null, { username: 'nobody' }, {
      image_base64: makePng(w, h).toString('base64'),
      variant: 'slim',
    }));
    assert.equal(err.code, 'UNKNOWN_ACCOUNT', `${w}x${h} must pass dims validation`);
    assert.equal(err.status, 404, `${w}x${h} must pass dims validation`);
  }
});

test('GET skin: unknown account -> 404 UNKNOWN_ACCOUNT without network', async () => {
  const err = await throwsHttp(() => getSkin({}, null, { username: 'ghost' }));
  assert.equal(err.status, 404);
  assert.equal(err.code, 'UNKNOWN_ACCOUNT');
});

// ---------------------------------------------------------------------------
// getProfileSkin (stub fetchFn)
// ---------------------------------------------------------------------------

test('getProfileSkin: maps active SLIM skin + active cape', async () => {
  const fetchFn = async (url, opts) => {
    assert.equal(url, 'https://api.minecraftservices.com/minecraft/profile');
    assert.equal(opts.headers.Authorization, 'Bearer mc-token-123');
    return stubResponse({
      body: {
        id: 'uuid-1',
        name: 'Steve',
        skins: [
          { id: 's1', state: 'INACTIVE', url: 'https://old-skin', variant: 'CLASSIC' },
          { id: 's2', state: 'ACTIVE', url: 'https://textures.minecraft.net/texture/abc', variant: 'SLIM' },
        ],
        capes: [{ id: 'c1', state: 'ACTIVE', url: 'https://cape', alias: 'Migrator' }],
      },
    });
  };
  const profile = await getProfileSkin('mc-token-123', fetchFn);
  assert.deepEqual(profile, {
    variant: 'slim',
    has_skin: true,
    cape: true,
    skinUrl: 'https://textures.minecraft.net/texture/abc',
    uuid: 'uuid-1',
    name: 'Steve',
  });
});

test('getProfileSkin: no skin defaults to classic, no cape, null skinUrl', async () => {
  const profile = await getProfileSkin('tok', async () => stubResponse({
    body: { id: 'u', name: 'Alex', skins: [], capes: [] },
  }));
  assert.equal(profile.variant, 'classic');
  assert.equal(profile.has_skin, false);
  assert.equal(profile.cape, false);
  assert.equal(profile.skinUrl, null);
  assert.equal(profile.uuid, 'u');
  assert.equal(profile.name, 'Alex');
});

// ---------------------------------------------------------------------------
// uploadProfileSkin multipart (stub fetchFn)
// ---------------------------------------------------------------------------

test('uploadProfileSkin: POST multipart variant + file Blob', async () => {
  let seen;
  const fetchFn = async (url, opts) => {
    seen = { url, opts };
    return stubResponse({ body: {} });
  };
  await uploadProfileSkin('mc-token-123', makePng(64, 64), 'classic', fetchFn);
  assert.equal(seen.url, 'https://api.minecraftservices.com/minecraft/profile/skins');
  assert.equal(seen.opts.method, 'POST');
  assert.equal(seen.opts.headers.Authorization, 'Bearer mc-token-123');
  assert.ok(seen.opts.body instanceof FormData, 'body is multipart FormData');
  assert.deepEqual([...seen.opts.body.keys()], ['variant', 'file']);
  assert.equal(seen.opts.body.get('variant'), 'CLASSIC');
  const file = seen.opts.body.get('file');
  assert.ok(file instanceof Blob, 'file field is a Blob');
  assert.equal(file.type, 'image/png');
  assert.equal(file.name, 'skin.png');
  assert.ok(file.size > 0, 'file field carries the PNG bytes');
});

test('uploadProfileSkin: POST 405 -> single PUT retry, which then succeeds', async () => {
  const methods = [];
  const fetchFn = async (url, opts) => {
    methods.push(opts.method);
    if (opts.method === 'POST') return stubResponse({ status: 405, body: { error: 'METHOD_NOT_ALLOWED' } });
    return stubResponse({ body: { ok: true } });
  };
  const origError = console.error;
  console.error = () => {};
  try {
    const out = await uploadProfileSkin('t', makePng(64, 64), 'classic', fetchFn);
    assert.deepEqual(methods, ['POST', 'PUT']);
    assert.deepEqual(out, { ok: true });
  } finally {
    console.error = origError;
  }
});

test('uploadProfileSkin: no PUT retry on non-405 failure (400 surfaces at once)', async () => {
  const methods = [];
  const fetchFn = async (url, opts) => {
    methods.push(opts.method);
    return stubResponse({ status: 400, body: { error: 'Bad Request' } });
  };
  const origError = console.error;
  console.error = () => {};
  try {
    const err = await throwsHttp(() => uploadProfileSkin('t', makePng(64, 64), 'classic', fetchFn));
    assert.equal(err.status, 400);
    assert.deepEqual(methods, ['POST']);
  } finally {
    console.error = origError;
  }
 });

test('uploadProfileSkin: slim maps to SLIM; bad variant -> BAD_SKIN_VARIANT', async () => {
  let seen;
  const fetchFn = async (url, opts) => {
    seen = { url, opts };
    return stubResponse({ body: {} });
  };
  await uploadProfileSkin('t', makePng(64, 32), 'slim', fetchFn);
  assert.equal(seen.opts.body.get('variant'), 'SLIM');
  const err = await throwsHttp(() => uploadProfileSkin('t', makePng(64, 64), 'wide', fetchFn));
  assert.equal(err.status, 400);
  assert.equal(err.code, 'BAD_SKIN_VARIANT');
});

test('uploadProfileSkin: Mojang errorMessage is surfaced + logged server-side', async () => {
  const logged = [];
  const origError = console.error;
  console.error = (...args) => { logged.push(args.join(' ')); };
  try {
    const mojang405 = async () => stubResponse({
      status: 405,
      body: { error: 'Method Not Allowed', errorMessage: 'The skin upload endpoint rejects this content type' },
    });
    const err = await throwsHttp(() => uploadProfileSkin('SECRET-TOKEN-XYZ', makePng(64, 64), 'classic', mojang405));
    assert.equal(err.status, 405);
    assert.equal(err.code, 'SKIN_UPLOAD_FAILED');
    assert.ok(err.message.includes('405'), 'message carries the HTTP status');
    assert.ok(
      err.message.includes('The skin upload endpoint rejects this content type'),
      'message carries Mojang errorMessage, not just the generic error code',
    );
    assert.ok(!err.message.includes('SECRET-TOKEN-XYZ'), 'token must not leak');
    assert.ok(
      logged.some((line) => line.includes('405') && line.includes('The skin upload endpoint rejects this content type')),
      'server-side log carries status + body snippet',
    );
  } finally {
    console.error = origError;
  }
});

test('uploadProfileSkin: non-JSON rejection body surfaces its text snippet', async () => {
  const origError = console.error;
  console.error = () => {};
  try {
    const text405 = async () => ({
      ok: false,
      status: 405,
      headers: { get: () => null },
      json: async () => { throw new SyntaxError('Unexpected token < in JSON'); },
      text: async () => 'Method Not Allowed',
    });
    const err = await throwsHttp(() => uploadProfileSkin('t', makePng(64, 64), 'classic', text405));
    assert.equal(err.status, 405);
    assert.equal(err.code, 'SKIN_UPLOAD_FAILED');
    assert.ok(err.message.includes('405'), 'message carries the HTTP status');
    assert.ok(err.message.includes('Method Not Allowed'), 'message carries the raw body snippet');
  } finally {
    console.error = origError;
  }
});

// ---------------------------------------------------------------------------
// resetProfileSkin (stub fetchFn)
// ---------------------------------------------------------------------------

test('resetProfileSkin: DELETE .../skins/active, empty body -> { reset: true }', async () => {
  let seen;
  const fetchFn = async (url, opts) => {
    seen = { url, opts };
    // text-only empty body (a real 204 has no JSON): exercises the text path.
    return { ok: true, status: 204, headers: { get: () => null }, text: async () => '' };
  };
  const out = await resetProfileSkin('t', fetchFn);
  assert.equal(seen.url, 'https://api.minecraftservices.com/minecraft/profile/skins/active');
  assert.equal(seen.opts.method, 'DELETE');
  assert.equal(seen.opts.headers.Authorization, 'Bearer t');
  assert.deepEqual(out, { reset: true });
});

// ---------------------------------------------------------------------------
// HTTP error mapping (never leak the token)
// ---------------------------------------------------------------------------

test('skin errors: 401 -> MSA_REFRESH_FAILED on fetch/upload/reset', async () => {
  const unauth = async () => stubResponse({ status: 401, body: { error: 'Unauthorized' } });
  const g = await throwsHttp(() => getProfileSkin('SECRET-TOKEN-XYZ', unauth));
  assert.equal(g.status, 401);
  assert.equal(g.code, 'MSA_REFRESH_FAILED');
  const u = await throwsHttp(() => uploadProfileSkin('SECRET-TOKEN-XYZ', makePng(64, 64), 'classic', unauth));
  assert.equal(u.status, 401);
  assert.equal(u.code, 'MSA_REFRESH_FAILED');
  const r = await throwsHttp(() => resetProfileSkin('SECRET-TOKEN-XYZ', unauth));
  assert.equal(r.status, 401);
  assert.equal(r.code, 'MSA_REFRESH_FAILED');
  for (const e of [g, u, r]) assert.ok(!e.message.includes('SECRET-TOKEN-XYZ'), 'token must not leak');
});

test('skin errors: 429 -> SKIN_RATE_LIMITED with retry_after when header present', async () => {
  const limited = async () => stubResponse({
    status: 429,
    body: { error: 'Too Many Requests' },
    headers: { 'retry-after': '7' },
  });
  const g = await throwsHttp(() => getProfileSkin('SECRET-TOKEN-XYZ', limited));
  assert.equal(g.status, 429);
  assert.equal(g.code, 'SKIN_RATE_LIMITED');
  assert.equal(g.retry_after, 7);
  const u = await throwsHttp(() => uploadProfileSkin('SECRET-TOKEN-XYZ', makePng(64, 64), 'classic', limited));
  assert.equal(u.code, 'SKIN_RATE_LIMITED');
  assert.equal(u.retry_after, 7);
  // Absent header -> no retry_after attached.
  const noHeader = async () => stubResponse({ status: 429, body: {} });
  const r = await throwsHttp(() => resetProfileSkin('SECRET-TOKEN-XYZ', noHeader));
  assert.equal(r.code, 'SKIN_RATE_LIMITED');
  assert.equal(r.retry_after, undefined);
  for (const e of [g, u, r]) assert.ok(!e.message.includes('SECRET-TOKEN-XYZ'), 'token must not leak');
});

test('skin errors: other non-2xx -> SKIN_FETCH_FAILED / SKIN_UPLOAD_FAILED with status snippet', async () => {
  const bad = async () => stubResponse({
    status: 403,
    body: { error: 'ForbiddenOperationException' },
  });
  const g = await throwsHttp(() => getProfileSkin('SECRET-TOKEN-XYZ', bad));
  assert.equal(g.code, 'SKIN_FETCH_FAILED');
  assert.ok(g.message.includes('403'), 'message carries the HTTP status');
  assert.ok(!g.message.includes('SECRET-TOKEN-XYZ'), 'token must not leak');
  const u = await throwsHttp(() => uploadProfileSkin('SECRET-TOKEN-XYZ', makePng(64, 64), 'classic', bad));
  assert.equal(u.code, 'SKIN_UPLOAD_FAILED');
  assert.ok(u.message.includes('403'), 'message carries the HTTP status');
  assert.ok(!u.message.includes('SECRET-TOKEN-XYZ'), 'token must not leak');
  // Transport failure maps to the read/write codes, never a bare throw.
  const netFail = async () => { throw new Error('socket hang up'); };
  const n = await throwsHttp(() => getProfileSkin('SECRET-TOKEN-XYZ', netFail));
  assert.equal(n.code, 'SKIN_FETCH_FAILED');
});
