/**
 * download.mjs — concurrent, sha1-verified HTTP downloads with .part resume.
 *
 * Zero-dependency (node:http-free; uses the global fetch). Every file lands in
 * `<dest>.part` first and is atomically renamed to `dest` only after the whole
 * body has been written AND verified (sha1 when known, size otherwise). A
 * pre-existing `.part` resumes with a `Range: bytes=N-` request; servers that
 * ignore Range get a fresh truncated write.
 *
 * Progress is surfaced two ways:
 *   - `downloadAll(items, { onItem })` callback per item completion, and
 *   - SSE events via events.mjs (B1 slice) — items may carry `eventName`
 *     ('mod-progress' | 'asset-progress') + `context`; the emitted payload is
 *     `{ instance, phase: 'start'|'done'|'error', message?, filename?,
 *       index?, total?, kind?, ...context }`.
 * The event bridge is defensive: if events.mjs is absent or mid-write the
 * module still loads and downloads still work.
 */
import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream, readFileSync } from 'node:fs';
import { mkdir, rename, stat, unlink } from 'node:fs/promises';
import path from 'node:path';
import { Transform, Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';

export const USER_AGENT = 'EspectralClient/1.1.1';

// ---------------------------------------------------------------------------
// Event sink bridge (defensive; events.mjs is owned by the B1 slice)
// ---------------------------------------------------------------------------
let _events = null;
try {
  _events = await import('./events.mjs');
} catch {
  _events = null; // events.mjs not built yet (parallel slices) — retried lazily
}

export function emitEvent(name, payload) {
  try {
    if (_events && typeof _events.emit === 'function') {
      _events.emit(name, payload);
      return;
    }
  } catch {
    /* the event sink must never break a download */
  }
  // events.mjs was missing at load time — re-import once (module may exist now).
  if (_events === null) {
    import('./events.mjs')
      .then((mod) => {
        if (mod && typeof mod.emit === 'function') {
          _events = mod;
          mod.emit(name, payload);
        }
      })
      .catch(() => {});
  }
}

// ---------------------------------------------------------------------------
// Data directory + app config (B1's config.mjs may not exist yet; self-read)
// ---------------------------------------------------------------------------
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

export function dataDir() {
  return process.env.ESPECTRAL_DATA_DIR
    ? path.resolve(process.env.ESPECTRAL_DATA_DIR)
    : path.join(repoRoot, 'data');
}

export function readAppConfig() {
  try {
    return JSON.parse(readFileSync(path.join(dataDir(), 'config.json'), 'utf8'));
  } catch {
    return {};
  }
}

export function downloadConcurrency() {
  const n = readAppConfig().download_concurrency;
  return Number.isInteger(n) && n > 0 ? n : 6;
}

// ---------------------------------------------------------------------------
// Hashing / small fs helpers
// ---------------------------------------------------------------------------
export function sha1Hex(buf) {
  return createHash('sha1').update(buf).digest('hex');
}

export async function sha1File(file) {
  const h = createHash('sha1');
  for await (const chunk of createReadStream(file)) h.update(chunk);
  return h.digest('hex');
}

async function sha1FileSafe(file) {
  try {
    return await sha1File(file);
  } catch {
    return null;
  }
}

async function fileSize(file) {
  try {
    return (await stat(file)).size;
  } catch {
    return -1;
  }
}

/**
 * Move a finished .part into place. If the rename races a concurrent writer
 * that already landed a verified dest (asset indexes list duplicate hashes),
 * accept the existing file when it verifies instead of failing the install.
 *
 * IDENTITY PRESERVATION (no-hash callers): a download with neither sha1 nor
 * size cannot be short-circuited by the checks in downloadFileInner, so it
 * re-downloads on EVERY verification pass — Fabric's launcherMeta publishes no
 * hash for the intermediary/fabric-loader jars, so those two were re-fetched
 * each launch. Renaming byte-identical content over the destination still
 * gives it a NEW mtime, and JEP 483 records path+size+TIMESTAMP for every
 * classpath entry: one such touch permanently invalidates the AOT cache
 * ("This file is not the one used while building the AOT cache … timestamp has
 * changed" -> "Unable to map shared spaces"), silently costing the whole AOT
 * boot win. When the fresh bytes equal what is already on disk, drop the .part
 * and keep the original file — mtime included. Only the hashless path pays the
 * extra hash: a sha1 caller never reaches a download when the file matches.
 */
async function finalizePart(part, dest, sha1, size) {
  if (!sha1) {
    const [fresh, current] = await Promise.all([sha1FileSafe(part), sha1FileSafe(dest)]);
    if (fresh !== null && fresh === current) {
      await unlink(part).catch(() => {});
      return dest;
    }
  }
  try {
    await rename(part, dest);
  } catch {
    if (sha1 && (await sha1FileSafe(dest)) === sha1) return dest;
    if (!sha1 && size != null && (await fileSize(dest)) === size) return dest;
    throw new Error(`rename failed for ${dest} and no verified file in place`);
  }
  return dest;
}

// ---------------------------------------------------------------------------
// JSON / text fetches (small payloads, generous timeout)
// ---------------------------------------------------------------------------
export async function fetchText(url, { headers = {}, timeoutMs = 60000 } = {}) {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(new Error('request timeout')), timeoutMs);
  let res;
  try {
    res = await fetch(url, {
      headers: { 'User-Agent': USER_AGENT, ...headers },
      signal: ac.signal,
      redirect: 'follow',
    });
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) {
    try {
      await res.body?.cancel(); // release the undici connection; never swallow the HTTP error
    } catch {
      /* best-effort cleanup */
    }
    throw Object.assign(new Error(`GET ${url} -> HTTP ${res.status}`), { status: res.status });
  }
  return await res.text();
}

export async function fetchJson(url, opts) {
  const text = await fetchText(url, opts);
  try {
    return JSON.parse(text);
  } catch (err) {
    throw Object.assign(new Error(`invalid JSON from ${url}`), { cause: err });
  }
}

// ---------------------------------------------------------------------------
// Single file download with .part resume + verification
// ---------------------------------------------------------------------------

// Per-destination lock. Several callers can reach the same dest concurrently:
// createInstance's background library seeding (seedLibraries → installLibraries),
// AOT training's resolveLaunch and a user launch all resolve the same library
// set. Without serialization, two downloadFile calls to one dest interleave on
// `<dest>.part` (`flags:'w'` truncates while the other resumes/appends) → torn
// .part → sha1 mismatch → mutual retry/fail. With the lock the second caller
// waits, then re-runs the already-present/sha1 checks once the first finalizes
// and short-circuits.
const destLocks = new Map();

function withDestLock(dest, fn) {
  const prev = destLocks.get(dest) ?? Promise.resolve();
  const next = prev.then(() => fn(), () => fn());
  destLocks.set(dest, next);
  // Cleanup must not introduce an unhandled rejection when `next` rejects —
  // the caller handles that; the cleanup chain swallows it.
  void next
    .catch(() => {})
    .then(() => {
      if (destLocks.get(dest) === next) destLocks.delete(dest);
    });
  return next;
}

export function downloadFile(url, dest, opts = {}) {
  return withDestLock(dest, () => downloadFileInner(url, dest, opts));
}

async function downloadFileInner(url, dest, opts = {}) {
  const {
    sha1 = null,
    size = null,
    headers = {},
    timeoutMs = 120000,
    onProgress = null,
  } = opts;
  await mkdir(path.dirname(dest), { recursive: true });

  // Already present and verified -> nothing to do.
  if (sha1 && (await sha1FileSafe(dest)) === sha1) return dest;
  if (!sha1 && size != null && (await fileSize(dest)) === size) return dest;

  const part = `${dest}.part`;

  // A completed .part that verifies is just a finished download.
  let partSize = await fileSize(part);
  if (sha1 && partSize > 0 && (await sha1FileSafe(part)) === sha1) {
    return finalizePart(part, dest, sha1, size);
  }

  for (let attempt = 0; attempt < 5; attempt++) {
    // Re-read the .part each attempt: a previous stalled/failed attempt may
    // have written more bytes, and the Range must resume from the ACTUAL
    // size or the append corrupts the file.
    partSize = await fileSize(part);
    const hdrs = { 'User-Agent': USER_AGENT, ...headers };
    if (partSize > 0) hdrs['Range'] = `bytes=${partSize}-`;

    const ac = new AbortController();
    const timer = setTimeout(() => ac.abort(new Error('download timeout')), timeoutMs);
    let res;
    try {
      res = await fetch(url, { headers: hdrs, signal: ac.signal, redirect: 'follow' });
    } finally {
      clearTimeout(timer);
    }

    if (res.status === 416) {
      // Server considers the range complete: verify the .part, else restart.
      if (sha1 && (await sha1FileSafe(part)) === sha1) {
        return finalizePart(part, dest, sha1, size);
      }
      if (!sha1 && size != null && (await fileSize(part)) === size) {
        return finalizePart(part, dest, sha1, size);
      }
      await unlink(part).catch(() => {});
      continue;
    }
    if (!res.ok) {
      throw Object.assign(new Error(`GET ${url} -> HTTP ${res.status}`), { status: res.status });
    }

    const append = res.status === 206 && partSize > 0;
    const out = createWriteStream(part, { flags: append ? 'a' : 'w' });
    const hinted = Number(res.headers.get('content-length') ?? 0) + (append ? partSize : 0);
    const total = size != null ? size : hinted > 0 ? hinted : null;
    let received = append ? partSize : 0;
    let stallTimer = null;
    const armStall = () => {
      if (stallTimer) clearTimeout(stallTimer);
      // A stream that delivers no bytes for timeoutMs is dead (proxy hang,
      // half-open socket) — abort it so the retry resumes from the .part.
      // Resumed downloads keep the same watchdog per chunk.
      stallTimer = setTimeout(() => out.destroy(new Error('download stalled (no data)')), timeoutMs);
    };
    const counter = new Transform({
      transform(chunk, _enc, cb) {
        received += chunk.length;
        if (onProgress) {
          try {
            onProgress({ received, total });
          } catch {
            /* progress callback must never break the download */
          }
        }
        cb(null, chunk);
      },
    });
    counter.on('data', () => armStall());

    try {
      armStall();
      if (res.body) {
        await pipeline(Readable.fromWeb(res.body), counter, out);
      } else {
        await new Promise((resolve, reject) => out.end((e) => (e ? reject(e) : resolve())));
      }
      if (stallTimer) clearTimeout(stallTimer);
    } catch (err) {
      out.destroy();
      if (stallTimer) clearTimeout(stallTimer);
      if (err?.code === 'ERR_STREAM_PREMATURE_CLOSE' || String(err?.message).includes('stalled')) {
        // Interrupted/resumed download — retry from the .part.
        continue;
      }
      throw Object.assign(new Error(`download interrupted: ${url} (${err.message})`), { cause: err });
    }

    if (sha1) {
      const actual = await sha1FileSafe(part);
      if (actual !== sha1) {
        await unlink(part).catch(() => {});
        throw Object.assign(new Error(`sha1 mismatch for ${url}: expected ${sha1}, got ${actual}`), {
          code: 'sha1_mismatch',
          status: 502,
        });
      }
    } else if (size != null && (await fileSize(part)) !== size) {
      const actual = await fileSize(part);
      await unlink(part).catch(() => {});
      throw new Error(`size mismatch for ${url}: expected ${size}, got ${actual}`);
    }
    return finalizePart(part, dest, sha1, size);
  }
  throw new Error(`download failed after retries: ${url}`);
}

// ---------------------------------------------------------------------------
// Concurrent batch download (worker pool, bounded concurrency)
// ---------------------------------------------------------------------------
export async function downloadAll(items, opts = {}) {
  const concurrency = Math.max(1, opts.concurrency ?? downloadConcurrency());
  const onItem = opts.onItem ?? null;
  const queue = items.map((item, index) => ({ ...item, index }));
  const results = new Array(queue.length);
  const failures = [];
  let cursor = 0;

  const worker = async () => {
    for (;;) {
      const i = cursor++;
      if (i >= queue.length) return;
      const item = queue[i];
      const context = item.eventName ? { ...(item.context ?? {}) } : null;

      if (context) {
        emitEvent(item.eventName, { instance: context.instance, phase: 'start', index: i, total: queue.length, ...context });
      }
      try {
        const dest = await downloadFile(item.url, item.dest, item);
        results[i] = { dest, ok: true };
        if (context) {
          emitEvent(item.eventName, {
            instance: context.instance,
            phase: 'done',
            index: i,
            total: queue.length,
            ...context,
            dest,
            sha1: item.sha1 ?? null,
          });
        }
        if (onItem) {
          try {
            onItem({ index: i, ok: true, dest, item });
          } catch {
            /* ignore */
          }
        }
      } catch (err) {
        results[i] = { dest: item.dest, ok: false, error: err.message };
        failures.push({ item, error: err.message });
        if (context) {
          emitEvent(item.eventName, {
            instance: context.instance,
            phase: 'error',
            index: i,
            total: queue.length,
            ...context,
            error: err.message,
            message: err.message,
          });
        }
        if (onItem) {
          try {
            onItem({ index: i, ok: false, error: err.message, item });
          } catch {
            /* ignore */
          }
        }
      }
    }
  };

  await Promise.all(Array.from({ length: concurrency }, worker));

  if (failures.length > 0) {
    const err = new Error(`downloadAll: ${failures.length}/${queue.length} item(s) failed`);
    err.details = failures;
    err.results = results;
    throw err;
  }
  return results.map((r) => r.dest);
}
