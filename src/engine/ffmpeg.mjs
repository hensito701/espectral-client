/**
 * ffmpeg resolution for agent captures.
 *
 * Live recording and Replay Mod clip rendering both hand raw frames to
 * ffmpeg, and the game (not the engine) spawns it — so the engine must tell
 * the JVM where the binary is via `-Despectral.ffmpeg=<abs path>`.
 *
 * - Linux / macOS: the system `ffmpeg` on PATH (resolved to an absolute path;
 *   the game's PATH is not guaranteed to match the launcher's, and a Windows
 *   install often has none at all).
 * - Windows: a pinned static build (gyan.dev essentials, GPL) downloaded on
 *   demand into <dataDir>/runtimes/ffmpeg-<version>-win64/ — same shape as the
 *   Adoptium JDK provisioning (download → sha256 verify → extract → locate the
 *   exe), except the extraction commits with one rename so a failed attempt
 *   never leaves a half-populated install dir behind.
 *
 * `pruneAotProofLogs`-style laziness: nothing here runs unless an agent launch
 * or `capture_tools` needs it.
 */
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { dataDir } from './config.mjs';
import { downloadFile, fetchText, USER_AGENT } from './download.mjs';

/**
 * Pinned Windows build. gyan.dev keeps every release under a versioned URL, so
 * the pin never rots the way the `latest` aliases do.
 *
 * sha256 = the value the publisher's `<zip>.sha256` sidecar carries
 * (read 2026-10-30). Provisioning re-fetches that sidecar and refuses the
 * archive unless it matches BOTH the pin and the bytes on disk: the pin alone
 * would go stale silently if the release were ever rebuilt in place, and the
 * sidecar alone would trust whatever the same origin happened to serve.
 */
export const FFMPEG_WIN_PIN = Object.freeze({
  version: '8.1.2',
  url: 'https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip',
  sha256Url: 'https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip.sha256',
  sha256: 'db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec',
  size: 109_728_040,
});

/** Download the pinned archive once (exported for the provisioning test). */
export async function downloadPinnedFfmpeg(archive) {
  await downloadFile(FFMPEG_WIN_PIN.url, archive, {
    timeoutMs: 30 * 60_000,
    headers: { 'User-Agent': USER_AGENT },
  });
  return archive;
}

/** The publisher's published sha256 for the pinned archive. */
export async function publishedFfmpegSha256() {
  const text = await fetchText(FFMPEG_WIN_PIN.sha256Url, { timeoutMs: 30_000 });
  const hash = String(text).trim().split(/\s+/)[0].toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(hash)) throw new Error('publisher published no usable sha256 for the ffmpeg archive');
  return hash;
}

/** Directory the Windows build is provisioned into. */
export function ffmpegInstallDir() {
  return path.join(dataDir(), 'runtimes', `ffmpeg-${FFMPEG_WIN_PIN.version}-win64`);
}

/** Absolute path of the provisioned Windows ffmpeg.exe. */
export function provisionedFfmpegPath() {
  return path.join(ffmpegInstallDir(), 'bin', 'ffmpeg.exe');
}

/**
 * Marker written LAST inside a staged tree, before the one rename that makes it
 * the install dir. Its presence is what makes an install dir trustworthy:
 * `bin/ffmpeg.exe` alone exists part-way through an extraction, so an
 * "exe exists" test would accept a tree whose ffprobe/sidecars never landed.
 */
const COMPLETE_MARKER = '.espectral-install-complete';

/** True only for a fully committed install dir (marker + exe). */
export function ffmpegInstallComplete(dir = ffmpegInstallDir()) {
  return fs.existsSync(path.join(dir, COMPLETE_MARKER)) && fs.existsSync(path.join(dir, 'bin', 'ffmpeg.exe'));
}

/**
 * First `ffmpeg` on PATH as an absolute path, else null. Resolution differs
 * per platform (`which` / `where`), matching runtimes.pathJava().
 */
export function pathFfmpeg() {
  const cmd = process.platform === 'win32' ? 'where' : 'which';
  try {
    const r = spawnSync(cmd, ['ffmpeg'], { encoding: 'utf8', shell: false, windowsHide: true });
    if (r.status !== 0) return null;
    const first = (r.stdout || '')
      .split(/\r?\n/)
      .map((x) => x.trim())
      .find(Boolean);
    if (!first) return null;
    const abs = path.resolve(first);
    return fs.existsSync(abs) ? abs : null;
  } catch {
    return null;
  }
}

/**
 * Best-effort ffmpeg path, synchronously — what a launch wants (buildArgv is
 * pure and cannot download). Windows: the already-provisioned build only;
 * a missing one is provisioned by `ensureFfmpeg()` ahead of the launch.
 */
export function resolveFfmpegSync() {
  if (process.platform === 'win32') {
    // Only a committed tree counts: a half-extracted install dir (marker
    // absent) must fall through to PATH rather than hand the game a truncated
    // binary, and be re-provisioned by ensureFfmpeg().
    if (ffmpegInstallComplete()) return provisionedFfmpegPath();
    return pathFfmpeg();
  }
  return pathFfmpeg();
}

/**
 * ffmpeg path for a launch that NEEDS it, provisioning on Windows when the
 * machine has none. Serialized so two concurrent agent launches download the
 * ~110 MB build once. Returns null when provisioning fails (callers treat a
 * missing ffmpeg as "captures unavailable", never as a launch error).
 */
let provisionPromise = null;

export function ensureFfmpeg() {
  const existing = resolveFfmpegSync();
  if (existing) return Promise.resolve(existing);
  if (process.platform !== 'win32') return Promise.resolve(null);
  if (!provisionPromise) {
    provisionPromise = provisionFfmpeg()
      .catch((e) => {
        console.warn(`[ffmpeg] provisioning failed: ${e.message}`);
        return null;
      })
      .finally(() => {
        provisionPromise = null;
      });
  }
  return provisionPromise;
}

/** Extraction is one ~110 MB unzip; a stalled lazy-entry chain must fail, not hang. */
const EXTRACT_TIMEOUT_MS = 10 * 60_000;

/** Download + extract the pinned Windows build; idempotent. */
async function provisionFfmpeg() {
  const dir = ffmpegInstallDir();
  const runtimes = path.dirname(dir); // <data>/runtimes
  fs.mkdirSync(runtimes, { recursive: true });
  // The archive lives BESIDE the install dir: the install dir only ever
  // appears through one rename (below), so it must never hold transient files.
  // Kept only for the length of this call (an already-cached, sha256-verified
  // archive saves the 110 MB download) and removed afterwards.
  const archive = path.join(runtimes, `ffmpeg-${FFMPEG_WIN_PIN.version}-win64.zip`);
  try {
    await ensurePinnedArchive(archive);
    await installFfmpegArchive(archive, dir);
  } finally {
    // The pin makes the archive re-downloadable, so never leave 110 MB in the
    // data dir. A concurrent provisioner may still hold it open (Windows
    // EBUSY): that is cosmetic — the next run re-verifies the same bytes.
    try {
      fs.rmSync(archive, { force: true });
    } catch {
      /* still in use by a sibling provisioning run */
    }
  }
  if (!fs.existsSync(provisionedFfmpegPath())) {
    throw new Error(`ffmpeg extracted but no bin/ffmpeg.exe under ${dir}`);
  }
  return provisionedFfmpegPath();
}

let stageSeq = 0;

/**
 * Extract a verified archive and commit it as `dir`, so `dir` only ever
 * exists complete. Exported for the provisioning test (no network).
 *
 * Staging + one rename: a killed or failed extraction leaves `dir` absent
 * instead of half-populated. Concurrent provisioners race on two private
 * staging dirs and the rename; the loser finds the winner's `dir` in place and
 * keeps it.
 */
export async function installFfmpegArchive(archive, dir) {
  if (ffmpegInstallComplete(dir)) return dir;
  stageSeq += 1;
  const stage = `${dir}.staging-${process.pid}-${stageSeq}`;
  fs.rmSync(stage, { recursive: true, force: true });
  fs.mkdirSync(stage, { recursive: true });
  try {
    await extractFfmpegArchive(archive, stage);
    fs.writeFileSync(path.join(stage, COMPLETE_MARKER), `${FFMPEG_WIN_PIN.sha256}\n`);
    for (let attempt = 1; ; attempt += 1) {
      try {
        fs.renameSync(stage, dir);
        return dir; // stage is gone: the tree is the install dir now
      } catch (err) {
        // A concurrent provisioner may have committed a usable tree first.
        if (ffmpegInstallComplete(dir)) return dir;
        // Otherwise an unusable leftover is in the way; replace it once, then
        // give up rather than deleting forever.
        if (attempt >= 2) throw err;
        fs.rmSync(dir, { recursive: true, force: true });
      }
    }
  } finally {
    // No-op after a successful commit; removes the staged tree otherwise.
    fs.rmSync(stage, { recursive: true, force: true });
  }
}

/**
 * Ensure `archive` holds exactly the pinned bytes, downloading it when not.
 * The publisher's sidecar is consulted first so a silently rebuilt release is
 * reported as a stale pin instead of a download failure.
 */
async function ensurePinnedArchive(archive) {
  if (fs.existsSync(archive) && (await sha256File(archive)) === FFMPEG_WIN_PIN.sha256) return archive;
  fs.rmSync(archive, { force: true });
  let published = null;
  try {
    published = await publishedFfmpegSha256();
  } catch (e) {
    console.warn(`[ffmpeg] could not read the publisher checksum: ${e.message}`);
  }
  if (published && published !== FFMPEG_WIN_PIN.sha256) {
    throw new Error(
      `pinned ffmpeg checksum no longer matches the publisher (pin ${FFMPEG_WIN_PIN.sha256}, published ${published}); ` +
        'update FFMPEG_WIN_PIN after re-verifying the archive'
    );
  }
  await downloadPinnedFfmpeg(archive);
  const got = await sha256File(archive);
  if (got !== FFMPEG_WIN_PIN.sha256) {
    fs.rmSync(archive, { force: true });
    throw new Error(`sha256 mismatch for pinned ffmpeg build (got ${got}, expected ${FFMPEG_WIN_PIN.sha256})`);
  }
  return archive;
}

/**
 * Unpack `archive` into the (empty) `dest` so `dest/bin/ffmpeg.exe` exists.
 * Exported for the provisioning test (no network).
 *
 * Zip-slip: yauzl already refuses absolute paths and `..` segments, but it
 * says nothing about symlinks and extract-zip creates them — a `bin -> /`
 * entry plus a later `bin/x` write would land outside `dest`. Entries are
 * therefore also rejected on symlink mode and on a containment check of their
 * resolved path. Only a pinned, sha256-verified archive is ever extracted; this
 * is the second lock on that door.
 */
export async function extractFfmpegArchive(archive, dest) {
  const root = path.resolve(dest);
  const guard = (entry) => {
    const name = String(entry.fileName ?? '');
    const mode = (entry.externalFileAttributes >> 16) & 0xffff;
    if ((mode & 0o170000) === 0o120000) throw new Error(`ffmpeg archive entry is a symlink: ${name}`);
    const abs = path.resolve(root, name);
    if (
      name.includes('\\') ||
      path.isAbsolute(name) ||
      name.split('/').includes('..') ||
      (abs !== root && !abs.startsWith(root + path.sep))
    ) {
      throw new Error(`ffmpeg archive entry escapes the install dir: ${name}`);
    }
  };
  const { default: extract } = await import('extract-zip');
  let timer;
  try {
    await Promise.race([
      extract(archive, { dir: root, onEntry: guard }),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('ffmpeg extraction timed out')), EXTRACT_TIMEOUT_MS);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
  // gyan.dev zips nest everything under `ffmpeg-<version>-essentials_build/`;
  // hoist that folder so bin\ffmpeg.exe is canonical (same trick as the JDK).
  if (!fs.existsSync(path.join(root, 'bin', 'ffmpeg.exe'))) {
    const nested = fs
      .readdirSync(root, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .find((e) => fs.existsSync(path.join(root, e.name, 'bin', 'ffmpeg.exe')));
    if (nested) {
      const inner = path.join(root, nested.name);
      for (const entry of fs.readdirSync(inner)) {
        fs.renameSync(path.join(inner, entry), path.join(root, entry));
      }
      fs.rmSync(inner, { recursive: true, force: true });
    }
  }
  if (!fs.existsSync(path.join(root, 'bin', 'ffmpeg.exe'))) {
    throw new Error('ffmpeg archive has no bin/ffmpeg.exe');
  }
}

/** sha256 of a file (streamed). */
export function sha256File(file) {
  return new Promise((resolve, reject) => {
    const h = createHash('sha256');
    const s = fs.createReadStream(file);
    s.on('data', (d) => h.update(d));
    s.on('end', () => resolve(h.digest('hex')));
    s.on('error', reject);
  });
}
