/**
 * ziputil.mjs — minimal ZIP reader for NeoForge installer jars and mrpack
 * archives (no external dependency).
 *
 * Pure central-directory parsing: locate the End Of Central Directory record,
 * walk the central directory entries, then jump to each entry's local file
 * header to find the payload start. Method 8 (deflate) entries are inflated
 * with zlib.inflateRawSync, method 0 (stored) entries are copied verbatim.
 * Sizes come from the central directory (authoritative even when the local
 * header uses data descriptors), so nothing here ever needs the tail of the
 * archive.
 *
 * All three helpers share one `scanCentralDirectory` pass; reads are
 * synchronous because every caller (NeoForge installer, mrpack import) treats
 * the archive as a small, one-shot file.
 *
 * Errors are plain Error with descriptive messages; the public entry points
 * return null / false for "entry not found" and throw only on a corrupt
 * archive or unsupported compression method.
 */
import fs from 'node:fs';
import zlib from 'node:zlib';

const SIG_EOCD = 0x06054b50; // PK\x05\x06
const SIG_CENTRAL = 0x02014b50; // PK\x01\x02
const SIG_LOCAL = 0x04034b50; // PK\x03\x04

/** Parse the EOCD record; returns { totalEntries, cdOffset, cdSize }. */
function findEndOfCentralDirectory(buf) {
  // The EOCD lives in the last 65557 bytes (22-byte record + 64KB comment).
  const windowStart = Math.max(0, buf.length - 65557);
  let eocd = -1;
  for (let i = buf.length - 22; i >= windowStart; i--) {
    if (buf.readUInt32LE(i) === SIG_EOCD) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) {
    throw new Error('zip corrupt: end-of-central-directory record not found');
  }
  return {
    totalEntries: buf.readUInt16LE(eocd + 10),
    cdOffset: buf.readUInt32LE(eocd + 16),
    cdSize: buf.readUInt32LE(eocd + 12),
  };
}

/**
 * Walk the central directory, returning one entry per file/dir:
 * `{ name, method, compSize, localOffset }`.
 */
function scanCentralDirectory(buf) {
  const eocd = findEndOfCentralDirectory(buf);
  let off = eocd.cdOffset;
  const end = off + eocd.cdSize;
  const entries = [];
  for (let n = 0; n < eocd.totalEntries; n++) {
    if (off + 46 > end || buf.readUInt32LE(off) !== SIG_CENTRAL) {
      throw new Error(`zip corrupt: bad central directory header at offset ${off}`);
    }
    const method = buf.readUInt16LE(off + 10);
    const compSize = buf.readUInt32LE(off + 20);
    const nameLen = buf.readUInt16LE(off + 28);
    const extraLen = buf.readUInt16LE(off + 30);
    const commentLen = buf.readUInt16LE(off + 32);
    const localOffset = buf.readUInt32LE(off + 42);
    const name = buf.toString('utf8', off + 46, off + 46 + nameLen);
    entries.push({ name, method, compSize, localOffset });
    off += 46 + nameLen + extraLen + commentLen;
  }
  return entries;
}

/** Locate the payload bytes for one central-directory entry. */
function entryPayload(buf, entry) {
  const off = entry.localOffset;
  if (off + 30 > buf.length || buf.readUInt32LE(off) !== SIG_LOCAL) {
    throw new Error(`zip corrupt: bad local header for '${entry.name}'`);
  }
  const nameLen = buf.readUInt16LE(off + 26);
  const extraLen = buf.readUInt16LE(off + 28);
  const dataStart = off + 30 + nameLen + extraLen;
  if (dataStart + entry.compSize > buf.length) {
    throw new Error(`zip corrupt: truncated data for '${entry.name}'`);
  }
  return buf.subarray(dataStart, dataStart + entry.compSize);
}

/** Decompress one entry's payload (method 8 deflate, method 0 stored). */
function decompress(entry, data) {
  if (entry.method === 0) return Buffer.from(data);
  if (entry.method === 8) {
    try {
      return zlib.inflateRawSync(data);
    } catch (err) {
      throw new Error(`zip corrupt: bad deflate stream for '${entry.name}': ${err.message}`);
    }
  }
  throw new Error(`zip unsupported: compression method ${entry.method} for '${entry.name}'`);
}

/**
 * Read one entry from a zip archive as a Buffer.
 *
 * @param {string} zipPath path to the archive
 * @param {string} entryName exact entry name (e.g. 'version.json')
 * @returns {Buffer | null} payload, or null when the entry does not exist
 */
export function readZipEntry(zipPath, entryName) {
  const buf = fs.readFileSync(zipPath);
  const entries = scanCentralDirectory(buf);
  const entry = entries.find((e) => e.name === entryName);
  if (!entry) return null;
  return decompress(entry, entryPayload(buf, entry));
}

/**
 * List every entry name in a zip archive (files and directories), in central
 * directory order.
 *
 * @param {string} zipPath path to the archive
 * @returns {string[]}
 */
export function listZipEntries(zipPath) {
  const buf = fs.readFileSync(zipPath);
  return scanCentralDirectory(buf).map((e) => e.name);
}

/**
 * Check whether a zip archive contains an entry.
 *
 * @param {string} zipPath path to the archive
 * @param {string} entryName exact entry name
 * @returns {boolean}
 */
export function zipHasEntry(zipPath, entryName) {
  const buf = fs.readFileSync(zipPath);
  return scanCentralDirectory(buf).some((e) => e.name === entryName);
}
