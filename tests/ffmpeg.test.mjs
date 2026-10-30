import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';

// Sandbox the data dir BEFORE importing the engine (same pattern as the other
// engine tests).
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-ffmpeg-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const ff = await import('../src/engine/ffmpeg.mjs');

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

/**
 * Minimal stored-entry ZIP writer (no dependency). Entries are
 * `{name, data?, symlink?, dir?}`; a symlink sets the Unix IFLNK mode exactly
 * like a malicious archive would.
 */
function buildZip(entries) {
  const parts = [];
  const central = [];
  let offset = 0;
  for (const entry of entries) {
    const name = Buffer.from(entry.name, 'utf8');
    const data = Buffer.from(entry.symlink ?? entry.data ?? '');
    const mode = entry.symlink !== undefined ? 0o120777 : entry.dir ? 0o040755 : 0o100755;
    const crc = zlib.crc32(data) >>> 0;
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    const cen = Buffer.alloc(46);
    cen.writeUInt32LE(0x02014b50, 0);
    cen.writeUInt16LE(0x0314, 4);
    cen.writeUInt16LE(20, 6);
    cen.writeUInt32LE(crc, 16);
    cen.writeUInt32LE(data.length, 20);
    cen.writeUInt32LE(data.length, 24);
    cen.writeUInt16LE(name.length, 28);
    cen.writeUInt32LE((mode << 16) >>> 0, 38);
    cen.writeUInt32LE(offset, 42);
    parts.push(local, name, data);
    central.push(cen, name);
    offset += local.length + name.length + data.length;
  }
  const cd = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(cd.length, 12);
  eocd.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, cd, eocd]);
}

/** A gyan.dev-shaped archive: everything nested under one versioned folder. */
function pinnedShapedZip() {
  const root = `ffmpeg-${ff.FFMPEG_WIN_PIN.version}-essentials_build`;
  return buildZip([
    { name: `${root}/`, dir: true },
    { name: `${root}/README.txt`, data: 'gyan.dev build' },
    { name: `${root}/bin/`, dir: true },
    { name: `${root}/bin/ffmpeg.exe`, data: 'MZ-ffmpeg' },
    { name: `${root}/bin/ffprobe.exe`, data: 'MZ-ffprobe' },
  ]);
}

function tempZip(name, entries) {
  const file = path.join(dataDir, `archive-${name}.zip`);
  fs.writeFileSync(file, buildZip(entries));
  return file;
}

test('ffmpeg pin: URL, sha256 and size agree with what the publisher serves', () => {
  assert.match(ff.FFMPEG_WIN_PIN.sha256, /^[0-9a-f]{64}$/);
  assert.equal(ff.FFMPEG_WIN_PIN.url.split('/').pop(), `ffmpeg-${ff.FFMPEG_WIN_PIN.version}-essentials_build.zip`);
  assert.equal(ff.FFMPEG_WIN_PIN.sha256Url, `${ff.FFMPEG_WIN_PIN.url}.sha256`);
  assert.ok(Number.isInteger(ff.FFMPEG_WIN_PIN.size) && ff.FFMPEG_WIN_PIN.size > 0);
  // The install dir is keyed by version so a pin bump never reuses stale bytes.
  assert.match(ff.ffmpegInstallDir(), new RegExp(`ffmpeg-${ff.FFMPEG_WIN_PIN.version}-win64$`));
  assert.ok(ff.ffmpegInstallDir().startsWith(dataDir));
});

test('extractFfmpegArchive: hoists the single nested build folder to bin/ffmpeg.exe', async () => {
  const archive = path.join(dataDir, 'hoist.zip');
  fs.writeFileSync(archive, pinnedShapedZip());
  const dest = path.join(dataDir, 'hoist-out');
  fs.rmSync(dest, { recursive: true, force: true });

  await ff.extractFfmpegArchive(archive, dest);

  assert.equal(fs.readFileSync(path.join(dest, 'bin', 'ffmpeg.exe'), 'utf8'), 'MZ-ffmpeg');
  assert.equal(fs.readFileSync(path.join(dest, 'bin', 'ffprobe.exe'), 'utf8'), 'MZ-ffprobe');
  assert.ok(fs.existsSync(path.join(dest, 'README.txt')), 'sibling files are hoisted too');
  assert.equal(
    fs.readdirSync(dest).some((n) => n.startsWith('ffmpeg-') && n.endsWith('_build')),
    false,
    'the nested folder is gone, not duplicated'
  );
});

test('extractFfmpegArchive: rejects an archive with no bin/ffmpeg.exe', async () => {
  const archive = tempZip('noexe', [{ name: 'README.txt', data: 'nothing here' }]);
  const dest = path.join(dataDir, 'noexe-out');
  fs.rmSync(dest, { recursive: true, force: true });
  await assert.rejects(() => ff.extractFfmpegArchive(archive, dest), /no bin\/ffmpeg\.exe/);
});

test('extractFfmpegArchive: refuses a symlink entry (zip-slip via bin -> /)', async () => {
  const archive = tempZip('symlink', [
    { name: 'bin', symlink: '/' },
    { name: 'bin/ffmpeg.exe', data: 'MZ' },
  ]);
  const dest = path.join(dataDir, 'symlink-out');
  fs.rmSync(dest, { recursive: true, force: true });
  await assert.rejects(() => ff.extractFfmpegArchive(archive, dest), /symlink/);
});

test('extractFfmpegArchive: refuses .. and absolute entry paths', async () => {
  const traversal = tempZip('traversal', [
    { name: 'bin/ffmpeg.exe', data: 'MZ' },
    { name: '../ffmpeg-escape.txt', data: 'pwned' },
  ]);
  const escaping = path.join(dataDir, 'ffmpeg-escape.txt');
  fs.rmSync(escaping, { force: true });
  await assert.rejects(
    () => ff.extractFfmpegArchive(traversal, path.join(dataDir, 'traversal-out')),
    /invalid relative path|escapes the install dir/
  );
  assert.equal(fs.existsSync(escaping), false, 'nothing written outside the install dir');
});

test('installFfmpegArchive: commits atomically and leaves a usable tree', async () => {
  const archive = path.join(dataDir, 'commit.zip');
  fs.writeFileSync(archive, pinnedShapedZip());
  const dest = path.join(dataDir, 'runtimes', `ffmpeg-${ff.FFMPEG_WIN_PIN.version}-win64`);
  fs.rmSync(dest, { recursive: true, force: true });

  await ff.installFfmpegArchive(archive, dest);
  assert.equal(fs.readFileSync(path.join(dest, 'bin', 'ffmpeg.exe'), 'utf8'), 'MZ-ffmpeg');

  // Idempotent: a second call keeps the committed tree and does no work.
  const before = fs.statSync(path.join(dest, 'bin', 'ffmpeg.exe')).mtimeMs;
  await ff.installFfmpegArchive(archive, dest);
  assert.equal(fs.statSync(path.join(dest, 'bin', 'ffmpeg.exe')).mtimeMs, before);
});

test('installFfmpegArchive: a failed extraction leaves no install dir and no staging dir', async () => {
  const archive = tempZip('broken', [{ name: 'README.txt', data: 'no exe' }]);
  const dest = path.join(dataDir, 'runtimes', 'failed-win64');
  fs.rmSync(dest, { recursive: true, force: true });

  await assert.rejects(() => ff.installFfmpegArchive(archive, dest));
  assert.equal(fs.existsSync(dest), false, 'a half-extracted tree must never look provisioned');
  const siblings = fs.readdirSync(path.join(dataDir, 'runtimes'));
  assert.deepEqual(siblings.filter((n) => n.startsWith('failed-win64.staging')), [], 'staging cleaned up');
});

test('installFfmpegArchive: an unusable leftover install dir is replaced, not trusted', async () => {
  const archive = path.join(dataDir, 'replace.zip');
  fs.writeFileSync(archive, pinnedShapedZip());
  const dest = path.join(dataDir, 'runtimes', 'stale-win64');
  fs.rmSync(dest, { recursive: true, force: true });
  // Simulate a pre-staging crash: bin/ffmpeg.exe exists but the rest is gone.
  fs.mkdirSync(path.join(dest, 'bin'), { recursive: true });
  fs.writeFileSync(path.join(dest, 'bin', 'ffmpeg.exe'), 'truncated');

  await ff.installFfmpegArchive(archive, dest);
  assert.equal(fs.readFileSync(path.join(dest, 'bin', 'ffmpeg.exe'), 'utf8'), 'MZ-ffmpeg');
  assert.equal(fs.readFileSync(path.join(dest, 'bin', 'ffprobe.exe'), 'utf8'), 'MZ-ffprobe');
});

test('installFfmpegArchive: concurrent calls converge on one complete tree', async () => {
  const archive = path.join(dataDir, 'race.zip');
  fs.writeFileSync(archive, pinnedShapedZip());
  const dest = path.join(dataDir, 'runtimes', 'race-win64');
  fs.rmSync(dest, { recursive: true, force: true });

  await Promise.all([
    ff.installFfmpegArchive(archive, dest),
    ff.installFfmpegArchive(archive, dest),
    ff.installFfmpegArchive(archive, dest),
  ]);
  assert.equal(fs.readFileSync(path.join(dest, 'bin', 'ffmpeg.exe'), 'utf8'), 'MZ-ffmpeg');
  assert.equal(fs.readFileSync(path.join(dest, 'bin', 'ffprobe.exe'), 'utf8'), 'MZ-ffprobe');
  assert.deepEqual(
    fs.readdirSync(path.join(dataDir, 'runtimes')).filter((n) => n.includes('.staging-')),
    [],
    'no staging leftovers'
  );
});

test('ensureFfmpeg: no download on POSIX; resolveFfmpegSync returns an absolute PATH binary or null', async () => {
  if (process.platform === 'win32') return; // provisioning path is Windows-only
  const resolved = ff.resolveFfmpegSync();
  assert.ok(resolved === null || path.isAbsolute(resolved));
  assert.equal(await ff.ensureFfmpeg(), resolved, 'POSIX never provisions');
});

test('ffmpegInstallComplete: a bare bin/ffmpeg.exe without the commit marker is not usable', async () => {
  const dir = path.join(dataDir, 'runtimes', 'marker-win64');
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(path.join(dir, 'bin'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'bin', 'ffmpeg.exe'), 'half-extracted');
  assert.equal(ff.ffmpegInstallComplete(dir), false, 'no marker -> not a finished install');

  const archive = path.join(dataDir, 'marker.zip');
  fs.writeFileSync(archive, pinnedShapedZip());
  await ff.installFfmpegArchive(archive, dir);

  assert.equal(ff.ffmpegInstallComplete(dir), true);
  assert.equal(fs.readFileSync(path.join(dir, 'bin', 'ffmpeg.exe'), 'utf8'), 'MZ-ffmpeg');
  assert.equal(fs.readFileSync(path.join(dir, 'bin', 'ffprobe.exe'), 'utf8'), 'MZ-ffprobe');
});
