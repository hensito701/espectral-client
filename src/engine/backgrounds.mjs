/**
 * Menu background packs (Workstream 4 — free personalization).
 *
 * Store layout under <dataDir>/backgrounds/<packId>/:
 *   pack.json   { id, name, type: "image"|"frames", fps, frame_count, width, height }
 *   image.png   (type "image")
 *   frames/NNNN.png  (type "frames", 1-based zero-padded)
 *
 * Import limits (contract):
 *   - image packs: PNG magic + IHDR dims, <= 8192px per side, <= 64 MB decoded
 *   - frame packs: <= 300 frames, every frame <= 1280x720 and sharing one
 *     resolution, total decoded bytes <= 80 MB, fps clamped to [1, 60]
 *
 * Uploads follow the icon-upload pattern (routes/misc.mjs decodePngImage):
 * a `data:image/png;base64,…` URL or raw base64 for `image_base64`/`frames[]`,
 * or an on-disk file/dir for `source_path`/`frame_paths` (same trust level as
 * the existing pick-file endpoints — a local desktop app must accept local
 * paths; every byte is still PNG-validated before it lands in the store).
 *
 * Selection writes `personalization.background` into the instance's
 * espectral-client.json through patchClientConfig (the THEMES slice owns the
 * `personalization` whitelist inside validatePatch); the routes layer adds
 * the pack-existence check.
 *
 * At launch the selected pack is staged into
 * <gameDir>/config/espectral-menu-bg/<packId>/ so the mod can read it from the
 * per-account profile gameDir (Contract C) — the single on-disk convention the
 * mod knows about.
 */
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { dataDir } from './config.mjs';
import { httpError } from './error.mjs';
import { loadClientConfig, patchClientConfig } from './client.mjs';
import { skinPngDimensions } from './skins.mjs';
import { decodePngImage } from './routes/misc.mjs';

// --- limits (contract) ------------------------------------------------------
export const MAX_FRAMES = 300;
export const MAX_FRAME_W = 1280;
export const MAX_FRAME_H = 720;
export const MAX_FRAME_PACK_BYTES = 80 * 1024 * 1024;
export const MAX_IMAGE_DIM = 8192;
export const MAX_IMAGE_FILE_BYTES = 64 * 1024 * 1024;
export const MAX_NAME = 60;
const DEFAULT_FPS = 24;
const MIN_FPS = 1;
const MAX_FPS = 60;

const PACK_ID_RE = /^[a-z0-9][a-z0-9_-]{0,39}$/;
const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

export function backgroundsRoot() {
  return path.join(dataDir(), 'backgrounds');
}

export function packDir(id) {
  return path.join(backgroundsRoot(), id);
}

export function isValidPackId(id) {
  return typeof id === 'string' && PACK_ID_RE.test(id);
}

function assertPackId(id) {
  if (!isValidPackId(id)) {
    throw httpError(400, 'BAD_PACK_ID', 'pack id must match ^[a-z0-9][a-z0-9_-]{0,39}$');
  }
  return id;
}

function assertName(name) {
  if (typeof name !== 'string' || name.trim().length === 0 || name.trim().length > MAX_NAME) {
    throw httpError(400, 'BAD_NAME', `name must be 1-${MAX_NAME} characters`);
  }
  return name.trim();
}

/** id derived from the pack name, uniquified against the store. */
function uniquePackId(name) {
  let base = name
    .toLowerCase()
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32);
  if (!PACK_ID_RE.test(base)) base = 'pack';
  if (!fs.existsSync(packDir(base))) return base;
  for (let i = 2; i < 1000; i++) {
    const id = `${base}-${i}`;
    if (!fs.existsSync(packDir(id))) return id;
  }
  return `${base}-${crypto.randomBytes(3).toString('hex')}`;
}

/** Parse + normalize a pack.json inside a pack dir; null when unreadable/invalid. */
export function readPackMeta(dir) {
  try {
    const raw = fs.readFileSync(path.join(dir, 'pack.json'), 'utf8');
    const j = JSON.parse(raw);
    if (!j || typeof j !== 'object') return null;
    if (j.type !== 'image' && j.type !== 'frames') return null;
    return {
      id: typeof j.id === 'string' ? j.id : path.basename(dir),
      name: typeof j.name === 'string' && j.name ? j.name : path.basename(dir),
      type: j.type,
      fps: j.type === 'frames' ? Number(j.fps) || DEFAULT_FPS : 0,
      frame_count: j.type === 'frames' ? Math.max(0, Math.floor(Number(j.frame_count) || 0)) : 1,
      width: Math.max(0, Math.floor(Number(j.width) || 0)),
      height: Math.max(0, Math.floor(Number(j.height) || 0)),
    };
  } catch {
    return null;
  }
}

/** All importable packs (store scan, name-sorted). */
export function listPacks() {
  let entries;
  try {
    entries = fs.readdirSync(backgroundsRoot(), { withFileTypes: true });
  } catch {
    return [];
  }
  const packs = [];
  for (const e of entries) {
    if (!e.isDirectory() || e.name.startsWith('.')) continue;
    const meta = readPackMeta(packDir(e.name));
    if (!meta) continue;
    if (previewFile(meta) === null) continue; // half-written or broken pack
    packs.push(meta);
  }
  packs.sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
  return packs;
}

/** Preview PNG path for a pack meta (first frame for animated packs). */
function previewFile(meta) {
  const dir = packDir(meta.id);
  const p = meta.type === 'frames'
    ? path.join(dir, 'frames', '0001.png')
    : path.join(dir, 'image.png');
  return fs.existsSync(p) ? p : null;
}

/** Absolute path of the pack preview PNG for the GET route; 404 otherwise. */
export function packPreviewPath(id) {
  assertPackId(id);
  const meta = readPackMeta(packDir(id));
  const file = meta && previewFile(meta);
  if (!file) throw httpError(404, 'PACK_NOT_FOUND', `no such background pack '${id}'`);
  return file;
}

function assertPng(buf, what) {
  if (!Buffer.isBuffer(buf) || buf.length < 8 || !buf.subarray(0, 8).equals(PNG_MAGIC)) {
    throw httpError(400, 'BAD_IMAGE', `${what} must be a PNG`);
  }
  const dims = skinPngDimensions(buf);
  if (!dims) {
    throw httpError(400, 'BAD_IMAGE', `${what} is not a readable PNG (bad IHDR)`);
  }
  return dims;
}

/** Read a PNG file from a local path; validates magic + IHDR dims. */
function readPngFile(p) {
  if (typeof p !== 'string' || p.trim().length === 0) {
    throw httpError(400, 'BAD_PATH', 'source path must be a non-empty string');
  }
  const resolved = path.resolve(p);
  let st;
  try {
    st = fs.statSync(resolved);
  } catch {
    throw httpError(400, 'BAD_PATH', `cannot read ${p}`);
  }
  if (!st.isFile()) throw httpError(400, 'BAD_PATH', `${p} is not a file`);
  if (st.size > MAX_IMAGE_FILE_BYTES) {
    throw httpError(413, 'IMAGE_TOO_LARGE', `file exceeds ${MAX_IMAGE_FILE_BYTES} bytes`);
  }
  return fs.readFileSync(resolved);
}

/**
 * Decode one base64 PNG upload. Unlike decodePngImage (icon cap) the byte
 * limit is a parameter so frame packs can carry per-frame limits.
 */
function decodePngBytes(imageBase64, maxBytes, what) {
  if (typeof imageBase64 !== 'string' || imageBase64.trim().length === 0) {
    throw httpError(400, 'BAD_IMAGE', `${what} is required`);
  }
  let b64 = imageBase64.trim();
  if (b64.startsWith('data:')) {
    const m = b64.match(/^data:image\/png;base64,(.*)$/s);
    if (!m) throw httpError(400, 'BAD_IMAGE', `${what} must be a PNG data URL or raw base64 PNG`);
    b64 = m[1];
  }
  const buf = Buffer.from(b64, 'base64');
  if (buf.length > maxBytes) {
    throw httpError(413, 'IMAGE_TOO_LARGE', `${what} exceeds ${maxBytes} bytes decoded`);
  }
  assertPng(buf, what);
  return buf;
}

/** Write a pack dir via a temp dir + rename so a failed import leaves nothing behind. */
function commitPack(id, write) {
  const root = backgroundsRoot();
  fs.mkdirSync(root, { recursive: true });
  const tmp = path.join(root, `.tmp-${crypto.randomBytes(6).toString('hex')}`);
  fs.mkdirSync(tmp, { recursive: true });
  try {
    write(tmp);
    fs.renameSync(tmp, packDir(id));
  } catch (e) {
    try { fs.rmSync(tmp, { recursive: true, force: true }); } catch {}
    throw e;
  }
}

/**
 * Import a still-image pack. Source: `{ image_base64 }` (icon-upload pattern,
 * capped at decodePngImage's 500 KB) or `{ source_path }` (PNG file on disk,
 * up to 64 MB / 8192px per side).
 */
export async function importImagePack({ name, image_base64, source_path } = {}) {
  const cleanName = assertName(name);
  let buf;
  if (source_path !== undefined && source_path !== null) {
    buf = readPngFile(source_path);
    assertPng(buf, 'image');
  } else {
    // The icon-upload decoder handles the data-URL/raw-base64 shapes.
    buf = decodePngImage(image_base64);
  }
  const dims = skinPngDimensions(buf);
  if (!dims || dims.width > MAX_IMAGE_DIM || dims.height > MAX_IMAGE_DIM) {
    throw httpError(400, 'BAD_IMAGE', `image must be a PNG no larger than ${MAX_IMAGE_DIM}px per side`);
  }
  const id = uniquePackId(cleanName);
  const meta = {
    id,
    name: cleanName,
    type: 'image',
    fps: 0,
    frame_count: 1,
    width: dims.width,
    height: dims.height,
  };
  commitPack(id, (tmp) => {
    fs.writeFileSync(path.join(tmp, 'image.png'), buf);
    fs.writeFileSync(path.join(tmp, 'pack.json'), JSON.stringify(meta, null, 2) + '\n', 'utf8');
  });
  return meta;
}

/** Natural sort for frame filenames (frame2 < frame10). */
function naturalSort(a, b) {
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
}

/**
 * Collect frame sources from a directory (top-level *.png) or an explicit
 * path list. Returns `[{ label, buf }]` — buffers read lazily by the caller
 * is overkill for <=80MB; validated eagerly here instead.
 */
function frameSources({ source_path, frame_paths, frames }) {
  const out = [];
  if (Array.isArray(frame_paths)) {
    if (frame_paths.length === 0) throw httpError(400, 'NO_FRAMES', 'frame_paths is empty');
    if (frame_paths.length > MAX_FRAMES) {
      throw httpError(400, 'TOO_MANY_FRAMES', `at most ${MAX_FRAMES} frames per pack`);
    }
    for (const p of frame_paths) {
      out.push({ label: path.basename(String(p)), read: () => readPngFile(p) });
    }
  } else if (source_path !== undefined && source_path !== null) {
    if (typeof source_path !== 'string' || source_path.trim().length === 0) {
      throw httpError(400, 'BAD_PATH', 'source_path must be a non-empty string');
    }
    const dir = path.resolve(source_path);
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      throw httpError(400, 'BAD_PATH', `cannot read ${source_path}`);
    }
    const pngs = entries
      .filter((e) => e.isFile() && /\.png$/i.test(e.name))
      .map((e) => e.name)
      .sort(naturalSort);
    if (pngs.length === 0) {
      throw httpError(400, 'NO_FRAMES', `no PNG files in ${source_path}`);
    }
    if (pngs.length > MAX_FRAMES) {
      throw httpError(400, 'TOO_MANY_FRAMES', `at most ${MAX_FRAMES} frames per pack`);
    }
    for (const n of pngs) {
      const p = path.join(dir, n);
      out.push({ label: n, read: () => readPngFile(p) });
    }
  } else if (Array.isArray(frames)) {
    if (frames.length === 0) throw httpError(400, 'NO_FRAMES', 'frames is empty');
    if (frames.length > MAX_FRAMES) {
      throw httpError(400, 'TOO_MANY_FRAMES', `at most ${MAX_FRAMES} frames per pack`);
    }
    frames.forEach((f, i) => {
      out.push({
        label: `frames[${i}]`,
        read: () => decodePngBytes(f, MAX_FRAME_PACK_BYTES, `frames[${i}]`),
      });
    });
  } else {
    throw httpError(400, 'NO_FRAMES', 'provide frames[] (base64), frame_paths[] or source_path');
  }
  return out;
}

/**
 * Import an animated frame pack: <=300 PNG frames, all <=1280x720 sharing one
 * resolution, total decoded bytes <=80MB. `fps` defaults to 24 (clamped
 * 1..60); a pack.json alongside a directory import supplies fps/name defaults.
 */
export async function importFramesPack({ name, fps, source_path, frame_paths, frames } = {}) {
  // Optional pack.json next to a directory import provides defaults.
  let embedded = null;
  if (source_path) {
    try {
      const j = JSON.parse(
        fs.readFileSync(path.join(path.resolve(source_path), 'pack.json'), 'utf8'),
      );
      if (j && typeof j === 'object') embedded = j;
    } catch { /* no pack.json — fine */ }
  }
  const cleanName = assertName(
    name !== undefined && name !== null && name !== '' ? name : embedded?.name,
  );
  let fpsVal = fps !== undefined && fps !== null ? Number(fps)
    : embedded?.fps !== undefined ? Number(embedded.fps) : DEFAULT_FPS;
  if (!Number.isFinite(fpsVal)) fpsVal = DEFAULT_FPS;
  fpsVal = Math.min(MAX_FPS, Math.max(MIN_FPS, fpsVal));

  const sources = frameSources({ source_path, frame_paths, frames });
  const bufs = [];
  let total = 0;
  let width = 0;
  let height = 0;
  for (let i = 0; i < sources.length; i++) {
    const buf = sources[i].read();
    total += buf.length;
    if (total > MAX_FRAME_PACK_BYTES) {
      throw httpError(413, 'PACK_TOO_LARGE', `frame pack exceeds ${MAX_FRAME_PACK_BYTES} bytes decoded`);
    }
    const dims = assertPng(buf, sources[i].label);
    if (dims.width > MAX_FRAME_W || dims.height > MAX_FRAME_H) {
      throw httpError(400, 'FRAME_TOO_LARGE', `${sources[i].label}: frames must be <= ${MAX_FRAME_W}x${MAX_FRAME_H}`);
    }
    if (i === 0) {
      width = dims.width;
      height = dims.height;
    } else if (dims.width !== width || dims.height !== height) {
      throw httpError(400, 'FRAME_DIMS', `${sources[i].label}: all frames must share one resolution (${width}x${height})`);
    }
    bufs.push(buf);
  }

  const id = uniquePackId(cleanName);
  const meta = {
    id,
    name: cleanName,
    type: 'frames',
    fps: fpsVal,
    frame_count: bufs.length,
    width,
    height,
  };
  commitPack(id, (tmp) => {
    const fdir = path.join(tmp, 'frames');
    fs.mkdirSync(fdir, { recursive: true });
    bufs.forEach((buf, i) => {
      fs.writeFileSync(path.join(fdir, `${String(i + 1).padStart(4, '0')}.png`), buf);
    });
    fs.writeFileSync(path.join(tmp, 'pack.json'), JSON.stringify(meta, null, 2) + '\n', 'utf8');
  });
  return meta;
}

/** Delete a pack from the store. 404 when it does not exist. */
export function deletePack(id) {
  assertPackId(id);
  const dir = packDir(id);
  if (!fs.existsSync(dir)) {
    throw httpError(404, 'PACK_NOT_FOUND', `no such background pack '${id}'`);
  }
  fs.rmSync(dir, { recursive: true, force: true });
  return { ok: true };
}

// --- personalization write --------------------------------------------------

/**
 * Merge `personalization` fields into the instance's espectral-client.json
 * and return the ClientInfo payload (same shape as PATCH .../client).
 *
 * The write goes through `patchClientConfig` — the THEMES slice owns the
 * `personalization` whitelist inside `validatePatch` (keys theme|hud_theme|
 * background|reduced_motion, enum + type checks). This route's one extra
 * check: a non-null `background` must name a pack that actually exists in
 * the store (validatePatch only checks the id shape).
 */
export async function writePersonalization(instanceName, personalization) {
  if (personalization === null || typeof personalization !== 'object'
      || Array.isArray(personalization)) {
    throw httpError(400, 'BAD_PERSONALIZATION', 'personalization must be an object');
  }
  if ('background' in personalization) {
    const v = personalization.background;
    if (v !== null) {
      if (typeof v !== 'string' || !isValidPackId(v)) {
        throw httpError(400, 'BAD_PACK_ID', 'background must be a pack id or null');
      }
      if (readPackMeta(packDir(v)) === null) {
        throw httpError(404, 'PACK_NOT_FOUND', `no such background pack '${v}'`);
      }
    }
  }
  return await patchClientConfig(instanceName, { personalization });
}

/** Selected background id from the canonical client config, or null. */
export function selectedBackground(instanceName) {
  try {
    const bg = loadClientConfig(instanceName).personalization?.background;
    return typeof bg === 'string' && isValidPackId(bg) ? bg : null;
  } catch {
    return null;
  }
}

/**
 * One-time cutover from the pre-pack convention: a hand-placed
 * `config/espectral-menu-background.png` in the instance dir (or a gameDir
 * mirror of it) becomes a regular image pack, selected when the instance has
 * no selection yet. The legacy files are removed once their bytes are safe
 * in the store; the mod no longer reads that path. Never throws.
 */
export async function migrateLegacyBackground(instanceName, dirs) {
  const legacy = dirs
    .map((d) => path.join(d, 'config', 'espectral-menu-background.png'))
    .filter((p, i, all) => all.indexOf(p) === i && fs.existsSync(p));
  if (legacy.length === 0) return null;
  try {
    const meta = await importImagePack({ name: 'Mi fondo', source_path: legacy[0] });
    if (selectedBackground(instanceName) === null) {
      await writePersonalization(instanceName, { background: meta.id });
    }
    for (const p of legacy) fs.rmSync(p, { force: true });
    return meta;
  } catch {
    return null;
  }
}

// --- launch staging ---------------------------------------------------------

/**
 * Stage the selected pack into `<gameDir>/config/espectral-menu-bg/<packId>/`
 * (Contract C mirror: the game reads only gameDir). Stale packs are pruned;
 * `packId === null` removes the whole staging dir. Never throws — launch must
 * not fail over background staging.
 */
export function stageBackgroundPack(packId, gameDir) {
  try {
    const destRoot = path.join(gameDir, 'config', 'espectral-menu-bg');
    if (packId === null || packId === undefined) {
      fs.rmSync(destRoot, { recursive: true, force: true });
      return;
    }
    if (!isValidPackId(packId)) return;
    const src = packDir(packId);
    if (readPackMeta(src) === null) {
      // Selected pack vanished from the store — stage nothing (mod falls
      // back to the branded default) and prune any stale copies.
      fs.rmSync(destRoot, { recursive: true, force: true });
      return;
    }
    fs.mkdirSync(destRoot, { recursive: true });
    for (const e of fs.readdirSync(destRoot, { withFileTypes: true })) {
      if (e.name !== packId) {
        fs.rmSync(path.join(destRoot, e.name), { recursive: true, force: true });
      }
    }
    const dest = path.join(destRoot, packId);
    // Packs are immutable once imported; an identical pack.json means the
    // whole pack is already staged.
    try {
      if (fs.readFileSync(path.join(dest, 'pack.json'), 'utf8')
          === fs.readFileSync(path.join(src, 'pack.json'), 'utf8')) {
        return;
      }
    } catch { /* dest missing/partial — restage below */ }
    fs.rmSync(dest, { recursive: true, force: true });
    fs.cpSync(src, dest, { recursive: true });
  } catch {
    /* ignore staging errors — launch must not fail over the background */
  }
}
