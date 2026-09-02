/**
 * Launch-timing statistics (v0.5.0).
 *
 * One JSON object per line in <data>/launch-stats.jsonl, newest appended last:
 *
 *   { key, instance, version, started_at, menu_at, menu_ms, played_ms, spawn_ms, boot_ms, phases }
 *
 *   - started_at  ms epoch when the request was received (pre-resolve)
 *   - menu_at     ms epoch when the game reached the menu marker (null if never)
 *   - menu_ms     ms from launch to menu (null when menu_at is null)
 *   - played_ms   ms from launch to process exit
 *   - spawn_ms    ms from request receipt to java spawn (null if never spawned)
 *   - boot_ms     ms from java spawn to menu (null if never reached menu)
 *   - phases      object of phase name -> ms epoch (empty if no phases recorded)
 *
 * Writes are best-effort: fs failures are logged and never thrown into the
 * launch flow; reads never throw on a missing or corrupt file.
 */
import fs from 'node:fs';
import path from 'node:path';
import { dataDir } from './config.mjs';

/** Absolute path of the launch-stats JSONL document. */
function statsPath() {
  return path.join(dataDir(), 'launch-stats.jsonl');
}

// L7: bound the JSONL file. Trim() reads the whole file, so keep the cap
// modest (500 launches ≈ ~120 KB). Rotate by rewriting the tail after an
// append when the file exceeds the cap — the file never grows unbounded.
const MAX_STATS_RECORDS = 500;

/** Append one launch record (one JSON line). Never throws. */
export function recordLaunchStat(stat) {
  try {
    const file = statsPath();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.appendFileSync(file, JSON.stringify(stat) + '\n', 'utf8');
    // Cheap rotation check: only rewrite when the file is clearly oversized
    // (avoids reading on every launch). Re-parse is bounded by MAX_STATS_RECORDS.
    const size = fs.statSync(file).size;
    if (size > 256 * 1024) {
      const lines = fs.readFileSync(file, 'utf8').split('\n').filter((l) => l.trim() !== '');
      if (lines.length > MAX_STATS_RECORDS) {
        const kept = lines.slice(-MAX_STATS_RECORDS);
        fs.writeFileSync(file, kept.join('\n') + '\n', 'utf8');
      }
    }
  } catch (e) {
    console.warn('[stats] could not record launch stat:', e.message);
  }
}

/** Read the newest `limit` records (newest first). Never throws. */
export async function readLaunchStats(limit = 10) {
  try {
    const file = statsPath();
    if (!fs.existsSync(file)) return [];
    const records = [];
    // Read at most the last ~256 KB so an oversized file can't balloon memory
    // on the stats endpoint (L7: the file is rotated, but stay defensive).
    const stat = fs.statSync(file);
    const readSize = Math.min(stat.size, 256 * 1024);
    const readOffset = stat.size - readSize;
    const buf = Buffer.alloc(readSize);
    const fd = fs.openSync(file, 'r');
    try {
      fs.readSync(fd, buf, 0, readSize, readOffset);
    } finally {
      fs.closeSync(fd);
    }
    const text = buf.toString('utf8');
    // Only a tail-read (we seeked past the start of the file) can begin mid-
    // line — drop that first fragment then. A full-file read starts on a line
    // boundary and must keep its first record (a stale drop lost the oldest
    // launch stat on every read).
    let slice = text;
    if (readOffset > 0) {
      const firstNewline = text.indexOf('\n');
      if (firstNewline !== -1) slice = text.slice(firstNewline + 1);
    }
    for (const line of slice.split('\n')) {
      if (line.trim() === '') continue;
      try {
        records.push(JSON.parse(line));
      } catch {
        // skip garbage lines
      }
    }
    return records.slice(-limit).reverse();
  } catch {
    return [];
  }
}
