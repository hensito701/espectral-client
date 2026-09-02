#!/usr/bin/env node
/* ==========================================================================
 * make-icons.mjs — PNG-in-ICO writer (Node builtins only, no ImageMagick).
 *
 * Builds src-tauri/icons/icon.ico with three PNG-compressed ICONDIRENTRYs:
 *   32px  <- assets/site/favicon.png
 *   180px <- assets/site/apple-touch-icon.png
 *   256px <- assets/site/logo_circle.png  (entry width/height byte = 0)
 *
 * Also stages plain copies for Tauri window/tray use:
 *   icons/32x32.png  (favicon.png), icons/128x128.png + icons/icon.png (logo_circle.png)
 *
 * ICO layout: 6-byte ICONDIR (reserved=0, type=1, count=N) + N x 16-byte
 * ICONDIRENTRY (width, height, 0=256, colors=0, reserved=0, planes=1, bpp=32,
 * bytesInRes, imageOffset) + raw PNG bytes.
 *
 * Run: node scripts/make-icons.mjs
 * ========================================================================== */

import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SITE_DIR = join(ROOT, 'assets', 'site');
const OUT_DIR = join(ROOT, 'src-tauri', 'icons');

const ENTRIES = [
  { file: 'favicon.png', size: 32 },
  { file: 'apple-touch-icon.png', size: 180 },
  { file: 'logo_circle.png', size: 256 },
];

/** Reads a PNG's width/height from the IHDR chunk (bytes 16..24). */
function pngSize(buf) {
  if (buf.length < 24 || buf.toString('latin1', 1, 4) !== 'PNG') {
    throw new Error(`no es un PNG válido (${buf.length} bytes)`);
  }
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

/** Builds a complete .ico buffer embedding the given PNGs. */
function buildIco(images) {
  const count = images.length;

  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: 1 = icon
  header.writeUInt16LE(count, 4);

  const entries = Buffer.alloc(16 * count);
  const body = [];
  let offset = header.length + entries.length;

  images.forEach(({ png, size }, i) => {
    const e = entries.subarray(i * 16, i * 16 + 16);
    const dim = size === 256 ? 0 : size; // 0 encodes 256
    e.writeUInt8(dim, 0); // width
    e.writeUInt8(dim, 1); // height
    e.writeUInt8(0, 2); // palette count
    e.writeUInt8(0, 3); // reserved
    e.writeUInt16LE(1, 4); // color planes
    e.writeUInt16LE(32, 6); // bits per pixel
    e.writeUInt32LE(png.length, 8); // bytes in resource
    e.writeUInt32LE(offset, 12); // image offset
    body.push(png);
    offset += png.length;
  });

  return Buffer.concat([header, entries, ...body]);
}

function main() {
  mkdirSync(OUT_DIR, { recursive: true });

  const images = ENTRIES.map(({ file, size }) => {
    const png = readFileSync(join(SITE_DIR, file));
    const { width, height } = pngSize(png);
    if (width !== size || height !== size) {
      console.warn(
        `[make-icons] aviso: ${file} es ${width}x${height}, se esperaba ${size}x${size} — se incrusta tal cual (entrada forzada a ${size}).`,
      );
    } else {
      console.log(`[make-icons] ${file}: ${width}x${height} (${png.length} bytes)`);
    }
    return { png, size };
  });

  const ico = buildIco(images);
  writeFileSync(join(OUT_DIR, 'icon.ico'), ico);
  console.log(`[make-icons] escribió icon.ico (${ico.length} bytes, ${images.length} entradas)`);

  // Plain copies for Tauri window/tray config.
  copyFileSync(join(SITE_DIR, 'favicon.png'), join(OUT_DIR, '32x32.png'));
  copyFileSync(join(SITE_DIR, 'logo_circle.png'), join(OUT_DIR, '128x128.png'));
  copyFileSync(join(SITE_DIR, 'logo_circle.png'), join(OUT_DIR, 'icon.png'));
  console.log('[make-icons] copió 32x32.png, 128x128.png, icon.png');
}

main();
