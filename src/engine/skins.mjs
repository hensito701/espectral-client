/**
 * Skin library (Skin Atelier gallery) + Mojang-skin helpers shared by routes.
 *
 * Library layout (global per installation, like the vanilla launcher):
 *   <dataDir>/skin-library.json        — index [{ id, name, variant, created_at, source }]
 *   <dataDir>/skin-library/<id>.png    — art bytes
 * Mojang texture cache (per account, TTL 1h):
 *   <dataDir>/skins/<uuid>.png
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import { promises as fsp } from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { dataDir } from './config.mjs';
import { httpError } from './error.mjs';
import * as accounts from './accounts.mjs';

export const SKIN_VARIANTS = ['classic', 'slim'];
const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const SKIN_CACHE_TTL_MS = 60 * 60 * 1000;
const MAX_NAME_LEN = 40;

export function skinsDir() {
  return path.join(dataDir(), 'skins');
}

export function libraryDir() {
  return path.join(dataDir(), 'skin-library');
}

export function libraryIndexPath() {
  return path.join(dataDir(), 'skin-library.json');
}

function skinCachePath(uuid) {
  return path.join(skinsDir(), `${uuid}.png`);
}

function libraryPngPath(id) {
  return path.join(libraryDir(), `${id}.png`);
}

/**
 * Parse PNG IHDR dimensions: width = bytes 16-19, height = bytes 20-23
 * (big-endian, after the 8-byte signature + length/type). Returns
 * { width, height } or null when the buffer is too short or not a PNG.
 */
export function skinPngDimensions(buf) {
  try {
    if (!Buffer.isBuffer(buf) || buf.length < 24) return null;
    if (!buf.subarray(0, 8).equals(PNG_MAGIC)) return null;
    // Chunk type must be IHDR ('I','H','D','R').
    if (buf[12] !== 0x49 || buf[13] !== 0x48 || buf[14] !== 0x44 || buf[15] !== 0x52) return null;
    const width = buf.readUInt32BE(16);
    const height = buf.readUInt32BE(20);
    if (!Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0) return null;
    return { width, height };
  } catch {
    return null;
  }
}

/** True for buffers Mojang accepts as Java skins: PNG 64x64 (64x32 legacy). */
export function isValidSkinPng(buf) {
  const dims = skinPngDimensions(buf);
  return !!dims && dims.width === 64 && (dims.height === 64 || dims.height === 32);
}

export function assertSkinVariant(variant) {
  if (variant !== 'classic' && variant !== 'slim') {
    throw httpError(400, 'BAD_SKIN_VARIANT', 'variant must be classic|slim');
  }
  return variant;
}

/**
 * Decode a `data:image/png;base64,…` URL (or raw base64 PNG) into bytes,
 * enforcing PNG magic. Anything else is a 400 BAD_IMAGE. Mirrors the avatar
 * upload contract without importing route code.
 */
export function decodeSkinPng(imageBase64) {
  if (typeof imageBase64 !== 'string' || imageBase64.length === 0) {
    throw httpError(400, 'BAD_IMAGE', 'image_base64 is required');
  }
  let b64 = imageBase64;
  const comma = imageBase64.indexOf(',');
  if (comma >= 0) {
    const header = imageBase64.slice(0, comma);
    if (!/^data:image\/png;base64$/i.test(header)) {
      throw httpError(400, 'BAD_IMAGE', 'image must be a PNG');
    }
    b64 = imageBase64.slice(comma + 1);
  }
  let buf;
  try {
    buf = Buffer.from(b64, 'base64');
  } catch {
    throw httpError(400, 'BAD_IMAGE', 'image must be a PNG');
  }
  if (buf.length === 0 || !buf.subarray(0, 8).equals(PNG_MAGIC)) {
    throw httpError(400, 'BAD_IMAGE', 'image must be a PNG');
  }
  return buf;
}

function assertSkinName(name) {
  const clean = typeof name === 'string' ? name.trim() : '';
  if (!clean || clean.length > MAX_NAME_LEN) {
    throw httpError(400, 'BAD_SKIN_NAME', `name must be 1-${MAX_NAME_LEN} characters`);
  }
  return clean;
}

// ---------------------------------------------------------------------------
// Mojang texture cache (<dataDir>/skins/<uuid>.png, TTL 1h)
// ---------------------------------------------------------------------------

/** Fresh (<1h by mtime) cached skin bytes, or null. Never throws. */
export async function readSkinCache(uuid) {
  try {
    const p = skinCachePath(uuid);
    const st = await fsp.stat(p);
    if (Date.now() - st.mtimeMs > SKIN_CACHE_TTL_MS) return null;
    return await fsp.readFile(p);
  } catch {
    return null;
  }
}

/** Store skin bytes in the cache (mkdir -p). Best-effort: never throws. */
export async function writeSkinCache(uuid, png) {
  try {
    await fsp.mkdir(skinsDir(), { recursive: true });
    await fsp.writeFile(skinCachePath(uuid), png);
  } catch {
    /* cache is best-effort */
  }
}

/** Drop a cached skin (missing file is a no-op). Never throws. */
export async function clearSkinCache(uuid) {
  try {
    await fsp.unlink(skinCachePath(uuid));
  } catch {
    /* already absent */
  }
}

/**
 * Resolve the MSA account + live Minecraft token for a skin endpoint.
 * 404 UNKNOWN_ACCOUNT when missing, 404 OFFLINE_ACCOUNT when token_kind
 * is not 'msa'. Dynamic msauth import mirrors the microsoft/poll route.
 */
export async function resolveSkinAccount(username) {
  const account = accounts.getAccount(username);
  if (!account) throw httpError(404, 'UNKNOWN_ACCOUNT', `account '${username}' does not exist`);
  if (account.token_kind !== 'msa') {
    throw httpError(404, 'OFFLINE_ACCOUNT', `account '${username}' is offline — skins need a Microsoft sign-in`);
  }
  const msauth = await import('./msauth.mjs');
  const token = await msauth.ensureMinecraftToken(account);
  if (!token || !token.accessToken) {
    throw httpError(401, 'MSA_REFRESH_FAILED', 'session expired — sign in with Microsoft again');
  }
  return { account, accessToken: token.accessToken, msauth };
}

// ---------------------------------------------------------------------------
// Library index (<dataDir>/skin-library.json)
// ---------------------------------------------------------------------------

/** Read the index; missing/corrupt file -> []. Never throws. */
export async function loadLibraryIndex() {
  try {
    const raw = await fsp.readFile(libraryIndexPath(), 'utf8');
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((e) => e && typeof e.id === 'string' && typeof e.name === 'string');
  } catch {
    return [];
  }
}

async function saveLibraryIndex(entries) {
  await fsp.mkdir(dataDir(), { recursive: true });
  const tmp = libraryIndexPath() + '.tmp';
  await fsp.writeFile(tmp, JSON.stringify(entries, null, 2), 'utf8');
  await fsp.rename(tmp, libraryIndexPath());
}

export function publicSkinEntry(e) {
  return { id: e.id, name: e.name, variant: e.variant, created_at: e.created_at, source: e.source };
}

function newSkinId() {
  return `skin_${Date.now().toString(36)}_${crypto.randomBytes(3).toString('hex')}`;
}

/**
 * Save PNG bytes as a named library entry. Validates variant + dims.
 * Returns the public entry.
 */
export async function saveLibrarySkin({ name, variant, png, source = 'upload', createdAt = null }) {
  const cleanName = assertSkinName(name);
  assertSkinVariant(variant);
  if (!isValidSkinPng(png)) {
    throw httpError(400, 'BAD_SKIN_DIMS', 'skin must be a 64x64 PNG (64x32 legacy accepted)');
  }
  const entry = {
    id: newSkinId(),
    name: cleanName,
    variant,
    created_at: createdAt ?? new Date().toISOString(),
    source,
  };
  await fsp.mkdir(libraryDir(), { recursive: true });
  await fsp.writeFile(libraryPngPath(entry.id), png);
  const index = await loadLibraryIndex();
  index.push(entry);
  await saveLibraryIndex(index);
  return publicSkinEntry(entry);
}

/** 404 UNKNOWN_SKIN when the id is not in the index. */
export async function getLibraryEntry(id) {
  const index = await loadLibraryIndex();
  const found = index.find((e) => e.id === id);
  if (!found) throw httpError(404, 'UNKNOWN_SKIN', `skin '${id}' does not exist`);
  return found;
}

/** Rename / re-variant an entry. Returns the public entry. */
export async function updateLibrarySkin(id, patch = {}) {
  const index = await loadLibraryIndex();
  const found = index.find((e) => e.id === id);
  if (!found) throw httpError(404, 'UNKNOWN_SKIN', `skin '${id}' does not exist`);
  if (patch.name !== undefined) found.name = assertSkinName(patch.name);
  if (patch.variant !== undefined) {
    found.variant = assertSkinVariant(patch.variant);
  }
  await saveLibraryIndex(index);
  return publicSkinEntry(found);
}

/** Remove an entry + its PNG (missing file is a no-op). */
export async function deleteLibrarySkin(id) {
  const index = await loadLibraryIndex();
  const at = index.findIndex((e) => e.id === id);
  if (at < 0) throw httpError(404, 'UNKNOWN_SKIN', `skin '${id}' does not exist`);
  index.splice(at, 1);
  await saveLibraryIndex(index);
  try {
    await fsp.unlink(libraryPngPath(id));
  } catch {
    /* already absent */
  }
  return { removed: true };
}

/** Raw PNG bytes for a library entry. 404 UNKNOWN_SKIN when absent. */
export async function readLibraryPng(id) {
  await getLibraryEntry(id); // 404 when unknown
  try {
    return await fsp.readFile(libraryPngPath(id));
  } catch {
    throw httpError(404, 'UNKNOWN_SKIN', `skin '${id}' has no image`);
  }
}

// ---------------------------------------------------------------------------
// Vanilla launcher import (%APPDATA%/.minecraft/launcher_custom_skins.json)
// ---------------------------------------------------------------------------

export function defaultVanillaSkinsPath() {
  const roaming = process.env.APPDATA
    || (process.platform === 'win32' ? path.join(process.env.USERPROFILE ?? '', 'AppData', 'Roaming') : null);
  const home = roaming ?? process.env.HOME ?? os_homedir();
  return path.join(home, '.minecraft', 'launcher_custom_skins.json');
}

function os_homedir() {
  try {
    return fs.realpathSync(process.env.HOME ?? '.');
  } catch {
    return '.';
  }
}

/**
 * Import named skins from the vanilla launcher's launcher_custom_skins.json.
 * Idempotent: entries whose name (case-insensitive) already exists are
 * skipped. The vanilla file carries no model variant, so imports land as
 * 'classic' — the user can flip individual entries to slim in the gallery.
 * `filePath` is injectable for tests.
 */
export async function importVanillaSkins(filePath = defaultVanillaSkinsPath()) {
  let doc;
  try {
    doc = JSON.parse(await fsp.readFile(filePath, 'utf8'));
  } catch (e) {
    if (e && e.code === 'ENOENT') {
      throw httpError(404, 'NO_VANILLA_SKINS', 'vanilla launcher has no saved skins on this PC');
    }
    throw httpError(400, 'BAD_VANILLA_SKINS', 'could not read the vanilla launcher skins file');
  }
  const raw = doc && typeof doc === 'object' && doc.customSkins && typeof doc.customSkins === 'object'
    ? Object.values(doc.customSkins)
    : [];
  const index = await loadLibraryIndex();
  const known = new Set(index.map((e) => String(e.name).toLowerCase()));
  let imported = 0;
  let skipped = 0;
  for (const s of raw) {
    try {
      const name = assertSkinName(s && s.name);
      if (known.has(name.toLowerCase())) {
        skipped += 1;
        continue;
      }
      const png = decodeSkinPng(s && s.skinImage);
      if (!isValidSkinPng(png)) {
        skipped += 1;
        continue;
      }
      const created = typeof (s && s.created) === 'string' ? s.created : new Date().toISOString();
      const entry = {
        id: newSkinId(),
        name,
        variant: 'classic',
        created_at: created,
        source: 'vanilla',
      };
      await fsp.mkdir(libraryDir(), { recursive: true });
      await fsp.writeFile(libraryPngPath(entry.id), png);
      index.push(entry);
      known.add(name.toLowerCase());
      imported += 1;
    } catch {
      skipped += 1;
    }
  }
  if (imported > 0) await saveLibraryIndex(index);
  return { imported, skipped, total: raw.length };
}

// ---------------------------------------------------------------------------
// MSA avatar auto = skin head (64x64 PNG derived from the Mojang skin)
// ---------------------------------------------------------------------------
// Pure-JS, zero-dependency PNG head extractor. Mojang faces live at (8,8,8,8)
// with the hat overlay at (40,8,8,8); the avatar is the source-over composite
// of the two, nearest-neighbor scaled to 64x64.

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf, start, end) {
  let c = 0xffffffff;
  for (let i = start; i < end; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
  const out = Buffer.alloc(8 + data.length + 4);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'ascii');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out, 4, 8 + data.length), 8 + data.length);
  return out;
}

function paethPredictor(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

/**
 * Decode skin bytes to raw RGBA pixels. Validates dims with
 * skinPngDimensions first, then requires 8-bit RGBA (color type 6),
 * non-interlaced. Anything else is a 400 BAD_IMAGE.
 */
function decodeSkinRgba(skinBytes) {
  const dims = skinPngDimensions(skinBytes);
  if (!dims || dims.width !== 64 || (dims.height !== 64 && dims.height !== 32)) {
    throw httpError(400, 'BAD_IMAGE', 'skin must be a 64x64 PNG (64x32 legacy accepted)');
  }
  let pos = 8;
  let seenIhdr = false;
  let width = 0;
  let height = 0;
  const idatParts = [];
  while (pos + 8 <= skinBytes.length) {
    const len = skinBytes.readUInt32BE(pos);
    const type = skinBytes.toString('ascii', pos + 4, pos + 8);
    const dataStart = pos + 8;
    const dataEnd = dataStart + len;
    if (dataEnd + 4 > skinBytes.length) {
      throw httpError(400, 'BAD_IMAGE', 'truncated skin PNG');
    }
    if (!seenIhdr) {
      if (type !== 'IHDR' || len !== 13) {
        throw httpError(400, 'BAD_IMAGE', 'skin PNG is missing its IHDR');
      }
      width = skinBytes.readUInt32BE(dataStart);
      height = skinBytes.readUInt32BE(dataStart + 4);
      const depth = skinBytes[dataStart + 8];
      const color = skinBytes[dataStart + 9];
      const comp = skinBytes[dataStart + 10];
      const filter = skinBytes[dataStart + 11];
      const interlace = skinBytes[dataStart + 12];
      if (depth !== 8 || color !== 6) {
        throw httpError(400, 'BAD_IMAGE', 'skin PNG must be 8-bit RGBA');
      }
      if (comp !== 0 || filter !== 0) {
        throw httpError(400, 'BAD_IMAGE', 'unsupported skin PNG encoding');
      }
      if (interlace !== 0) {
        throw httpError(400, 'BAD_IMAGE', 'interlaced skin PNGs are not supported');
      }
      seenIhdr = true;
    } else if (type === 'IDAT') {
      idatParts.push(skinBytes.subarray(dataStart, dataEnd));
    } else if (type === 'IEND') {
      break;
    }
    pos = dataEnd + 4;
  }
  if (!seenIhdr || idatParts.length === 0) {
    throw httpError(400, 'BAD_IMAGE', 'skin PNG has no image data');
  }
  let raw;
  try {
    raw = zlib.inflateSync(Buffer.concat(idatParts));
  } catch {
    throw httpError(400, 'BAD_IMAGE', 'could not decode skin PNG image data');
  }
  const stride = width * 4;
  if (raw.length !== height * (stride + 1)) {
    throw httpError(400, 'BAD_IMAGE', 'corrupt skin PNG image data');
  }
  const px = Buffer.alloc(width * height * 4);
  const prev = Buffer.alloc(stride);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)];
    const row = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const out = px.subarray(y * stride, (y + 1) * stride);
    if (f === 0) {
      row.copy(out);
    } else if (f === 1 || f === 2 || f === 3 || f === 4) {
      for (let i = 0; i < stride; i++) {
        const a = i >= 4 ? out[i - 4] : 0;
        const b = prev[i];
        const c = i >= 4 ? prev[i - 4] : 0;
        const pred = f === 1 ? a : f === 2 ? b : f === 3 ? (a + b) >> 1 : paethPredictor(a, b, c);
        out[i] = (row[i] + pred) & 0xff;
      }
    } else {
      throw httpError(400, 'BAD_IMAGE', 'corrupt skin PNG filter');
    }
    prev.set(out);
  }
  return { width, height, px };
}

/**
 * Extract the 64x64 avatar head PNG from Mojang skin bytes: crop the face
 * (8,8,8,8), source-over composite the hat overlay (40,8,8,8), then
 * nearest-neighbor scale to 64x64. Rejects non-RGBA / interlaced / wrongly
 * sized art with 400 BAD_IMAGE.
 */
export function extractHeadPng(skinBytes) {
  if (!Buffer.isBuffer(skinBytes)) {
    throw httpError(400, 'BAD_IMAGE', 'skin must be PNG bytes');
  }
  const { width, px } = decodeSkinRgba(skinBytes);
  const at = (x, y) => (y * width + x) * 4;
  const head = Buffer.alloc(8 * 8 * 4);
  for (let hy = 0; hy < 8; hy++) {
    for (let hx = 0; hx < 8; hx++) {
      const f = at(8 + hx, 8 + hy);
      const h = at(40 + hx, 8 + hy);
      const fa = px[f + 3];
      const ha = px[h + 3];
      const base = (fa * (255 - ha)) / 255;
      const outA = Math.round(ha + base);
      const o = (hy * 8 + hx) * 4;
      if (outA === 0) {
        head[o] = head[o + 1] = head[o + 2] = head[o + 3] = 0;
      } else {
        head[o] = Math.round((px[h] * ha + px[f] * base) / outA);
        head[o + 1] = Math.round((px[h + 1] * ha + px[f + 1] * base) / outA);
        head[o + 2] = Math.round((px[h + 2] * ha + px[f + 2] * base) / outA);
        head[o + 3] = outA;
      }
    }
  }
  // Nearest-neighbor scale 8x8 -> 64x64 (each head pixel is an 8x8 block).
  const S = 64;
  const rs = S * 4;
  const raw = Buffer.alloc(S * (rs + 1));
  for (let y = 0; y < S; y++) {
    raw[y * (rs + 1)] = 0; // filter type 0 (None)
    const hy = y >> 3;
    for (let x = 0; x < S; x++) {
      const src = (hy * 8 + (x >> 3)) * 4;
      const dst = y * (rs + 1) + 1 + x * 4;
      raw[dst] = head[src];
      raw[dst + 1] = head[src + 1];
      raw[dst + 2] = head[src + 2];
      raw[dst + 3] = head[src + 3];
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(S, 0);
  ihdr.writeUInt32BE(S, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  const png = Buffer.concat([
    PNG_MAGIC,
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', zlib.deflateSync(raw)),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
  return png;
}

/**
 * True when the account's avatar may be (re)derived from its skin: no avatar
 * file yet, or the current one is itself skin-derived (avatar_auto marker).
 * A user-uploaded avatar (file present, marker absent) is never clobbered.
 */
export function shouldSyncAvatarFromSkin(username) {
  const account = accounts.getAccount(username);
  if (!account) return false;
  return !accounts.hasAvatar(account) || account.avatar_auto === true;
}

/**
 * Derive the avatar head from skin bytes and store it via
 * accounts.writeAccountAvatar, marking the account `avatar_auto: true`.
 * No-op (`{ skipped: true }`) when the user has a custom avatar — those are
 * never clobbered. 404 UNKNOWN_ACCOUNT when the account is missing.
 */
export async function syncAccountAvatarFromSkin(username, skinBytes) {
  const account = accounts.getAccount(username);
  if (!account) {
    throw httpError(404, 'UNKNOWN_ACCOUNT', `account '${username}' does not exist`);
  }
  if (accounts.hasAvatar(account) && account.avatar_auto !== true) {
    return { skipped: true, reason: 'custom' };
  }
  const head = extractHeadPng(skinBytes); // throws BAD_IMAGE on junk
  await accounts.writeAccountAvatar(username, head); // clears any stale marker
  accounts.setAvatarAuto(username, true);
  return { ok: true, auto: true };
}

/**
 * Drop a skin-derived (avatar_auto) avatar, e.g. after a Mojang skin reset.
 * Keeps user-uploaded avatars: no-op (`{ skipped: true }`) unless the marker
 * is set. Never throws for a missing account.
 */
export async function clearAccountAutoAvatar(username) {
  const account = accounts.getAccount(username);
  if (!account || account.avatar_auto !== true) return { skipped: true };
  await accounts.removeAccountAvatar(username); // clears the marker too
  return { ok: true };
}
