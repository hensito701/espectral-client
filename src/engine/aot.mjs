/**
 * AOT cache lifecycle (B4 contract): key derivation, training, proof.
 *
 * key = sha256(version | javaBuild | os-arch)
 *   version   = Minecraft version id (e.g. '1.21.11').
 *   javaBuild = java.runtime.version from the probe (e.g. '25.0.4+7-LTS'),
 *               null/undefined -> 'unknown'.
 *   osArch    = process.arch (e.g. 'x64'); null/undefined -> process.arch.
 *   mods are off-classpath — Fabric discovers mods from mods/ at runtime;
 *   JEP 483 validates classpath identity (JDK build + arch/OS + classpath),
 *   not the mods dir, so modSetHash is NOT a key input. meta.json still
 *   records mod_set_hash for diagnostics.
 *
 * Cache location: <dataDir>/cache/aot/<key>/game.aot + meta.json.
 *
 * Training: spawn the JVM with -XX:AOTCacheOutput=<cache>/game.aot, wait for
 * 'Sound engine started' (size-tracked read of <gameDir>/logs/latest.log,
 * 180s cap), then ask the JVM to close GRACEFULLY via PowerShell
 * `(Get-Process -Id <pid>).CloseMainWindow()` (WM_CLOSE) and wait out the full
 * ~150s cache dump. NEVER TerminateProcess before the cache write: the AOT
 * configuration is recorded on NORMAL JVM exit only.
 *
 * Proof: the training run itself cannot log 'Using AOT-linked classes: true'
 * (that line is emitted when a cache is READ at startup, not written). The
 * proof is parsed from the newest <gameDir>/aot-<pid>.log produced by any
 * real -XX:AOTCache launch; train-done and the read-side aotStatus
 * (instances.mjs) expose it.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { dataDir, loadConfig } from './config.mjs';
import { computeModSetHash, instanceDir } from './instances.mjs';
import { getJvmInfo } from './jvm.mjs';
import { emit } from './events.mjs';
import { resolveLaunch, buildArgv, spawnJava } from './launch.mjs';

/** Root of all AOT caches: <dataDir>/cache/aot. */
export function aotRootDir() {
  return path.join(dataDir(), 'cache', 'aot');
}

/** Directory for one cache key. */
export function cacheDirFor(key) {
  return path.join(aotRootDir(), key);
}

/** The game.aot file for a key. */
export function cacheFilePath(key) {
  return path.join(cacheDirFor(key), 'game.aot');
}

/** meta.json for a key. */
export function metaPathFor(key) {
  return path.join(cacheDirFor(key), 'meta.json');
}

/** AOT cache key: sha256(version|javaBuild|os-arch). Null-safe. */
export function cacheKey(version, javaBuild, osArch) {
  const b = javaBuild ?? 'unknown';
  const a = osArch ?? process.arch;
  return crypto.createHash('sha256').update(`${version}|${b}|${a}`).digest('hex');
}

/**
 * Identity stamp of a classpath, in JEP 483's terms: the JVM records path +
 * size + timestamp for every entry when the cache is WRITTEN and re-checks all
 * three when it is READ. Any drift makes it refuse the cache wholesale
 * ("shared class paths mismatch" -> "Unable to map shared spaces") and the game
 * silently boots the slow path. Persisting the stamp at train time lets the
 * launcher detect that itself, BEFORE spending a boot on a cache the JVM will
 * reject. Unstattable entries record -1/-1 so a missing jar reads as drift, not
 * as a match.
 */
export function classpathStamp(classpath) {
  const out = [];
  for (const entry of classpath ?? []) {
    let size = -1;
    let mtimeMs = -1;
    try {
      const st = fs.statSync(entry);
      size = st.size;
      mtimeMs = st.mtimeMs;
    } catch {
      /* absent/locked — recorded as drift */
    }
    out.push({ path: entry, size, mtime_ms: mtimeMs });
  }
  return out;
}

/**
 * Compare a stored stamp against a freshly computed one. A missing or empty
 * stored stamp returns false: without evidence the cache cannot be proven
 * valid, and claiming a match would resurrect the exact silent-slow-boot bug
 * this guards against. mtime is compared with a 2s tolerance — FAT/network
 * volumes quantize timestamps, and the JVM's own check is equally coarse.
 */
export function stampMatches(stored, current) {
  if (!Array.isArray(stored) || stored.length === 0) return false;
  if (!Array.isArray(current) || stored.length !== current.length) return false;
  for (let i = 0; i < stored.length; i++) {
    const a = stored[i];
    const b = current[i];
    if (!a || !b) return false;
    if (a.path !== b.path) return false;
    if (a.size !== b.size || a.size < 0) return false;
    if (a.mtime_ms < 0 || b.mtime_ms < 0) return false;
    if (Math.abs(a.mtime_ms - b.mtime_ms) > 2000) return false;
  }
  return true;
}

/** Stored classpath stamp for a key, or null when meta.json has none. */
export function readCacheStamp(key) {
  try {
    const meta = JSON.parse(fs.readFileSync(metaPathFor(key), 'utf8'));
    return Array.isArray(meta?.classpath_stamp) ? meta.classpath_stamp : null;
  } catch {
    return null;
  }
}

/**
 * True when a cache exists but its recorded classpath identity no longer
 * matches disk — i.e. the JVM will reject it. Also true when the cache exists
 * with no stamp at all (pre-1.3.6 caches): unverifiable is treated as stale so
 * one retrain fixes it permanently instead of every boot paying the mismatch.
 */
export function isCacheStale(key, classpath) {
  return !stampMatches(readCacheStamp(key), classpathStamp(classpath));
}

/**
 * Key from the CURRENT config state (no JVM discovery/download — safe for
 * instance summaries). Falls back to javaBuild 'unknown' until a JVM has been
 * probed; after the first launch/train config.jvm is populated and the key
 * converges on the real javaBuild.
 */
export async function instanceKey(instance) {
  const build = loadConfig().jvm?.build ?? null;
  return cacheKey(instance.version, build, process.arch);
}

/** Newest aot-<pid>.log inside the instance dir (gameDir), or null. */
export function latestAotProofLog(instance) {
  const dir = instanceDir(instance.name ?? instance);
  let best = null;
  try {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (!entry.isFile() || !/^aot-\d+\.log$/.test(entry.name)) continue;
      const full = path.join(dir, entry.name);
      const st = fs.statSync(full);
      if (!best || st.mtimeMs > best.mtimeMs) best = { path: full, mtimeMs: st.mtimeMs };
    }
  } catch {
    return null;
  }
  return best ? best.path : null;
}

/**
 * Proof object: parse the newest aot-<pid>.log for
 * 'Using AOT-linked classes: true'. Null when no AOT boot log exists.
 */
export function aotProof(instance) {
  const logPath = latestAotProofLog(instance);
  if (!logPath) return null;
  let using = false;
  try {
    using = /Using AOT-linked classes: true/.test(fs.readFileSync(logPath, 'utf8'));
  } catch {
    /* log may be locked mid-write; report false */
  }
  return { log_path: logPath, using_aot_linked_classes: using };
}

/**
 * Prune AOT proof logs in the instance dir: keep at most 8 newest
 * aot-<pid>.log files and delete any older than 24h. Mirrors the sweep
 * semantics in instances.mjs aotStatus (~:660-674) but actually deletes
 * >24h files (aotStatus only filters them for proof selection).
 */
export async function pruneAotProofLogs(instance) {
  const name = instance?.name ?? instance;
  if (!name || typeof name !== 'string') return;
  const dir = instanceDir(name);
  let entries;
  try {
    entries = await fs.promises.readdir(dir);
  } catch {
    return;
  }
  const logs = [];
  for (const f of entries) {
    if (!/^aot-\d+\.log$/.test(f)) continue;
    const full = path.join(dir, f);
    let m = 0;
    try {
      m = (await fs.promises.stat(full)).mtimeMs;
    } catch {
      continue;
    }
    logs.push({ f, full, m });
  }
  logs.sort((a, b) => a.m - b.m); // oldest first
  const nowMs = Date.now();
  const HOUR = 3600_000;
  const MAX_AOT_LOGS = 8;
  // Delete any older than 24h
  const fresh = [];
  for (const entry of logs) {
    if (nowMs - entry.m > 24 * HOUR) {
      try {
        await fs.promises.unlink(entry.full);
      } catch {
        /* ignore */
      }
    } else {
      fresh.push(entry);
    }
  }
  // Keep at most 8 newest
  if (fresh.length > MAX_AOT_LOGS) {
    const toDelete = fresh.length - MAX_AOT_LOGS;
    for (let i = 0; i < toDelete; i++) {
      try {
        await fs.promises.unlink(fresh[i].full);
      } catch {
        /* ignore */
      }
    }
  }
}

/**
 * Bounded cache eviction: keep at most 4 keys in cache/aot, delete the
 * oldest by meta.json trained_at (dir mtime fallback), never the key just
 * written. Exported for tests; trainInstance calls it after a successful
 * cache write.
 */
export async function evictOldAotCaches(currentKey) {
  try {
    const root = aotRootDir();
    const entries = await fs.promises.readdir(root, { withFileTypes: true });
    const dirNames = entries.filter((e) => e.isDirectory()).map((e) => e.name);
    if (dirNames.length <= 4) return;
    const candidates = [];
    for (const k of dirNames) {
      if (k === currentKey) continue;
      const metaPath = path.join(root, k, 'meta.json');
      let t = null;
      try {
        const raw = await fs.promises.readFile(metaPath, 'utf8');
        const parsed = JSON.parse(raw);
        if (parsed && typeof parsed.trained_at === 'string') {
          const ms = new Date(parsed.trained_at).getTime();
          if (!Number.isNaN(ms)) t = ms;
        }
      } catch {
        /* no meta */
      }
      if (t === null) {
        try {
          const st = await fs.promises.stat(path.join(root, k));
          t = st.mtimeMs;
        } catch {
          t = 0;
        }
      }
      candidates.push({ k, t });
    }
    candidates.sort((a, b) => a.t - b.t); // oldest first
    const toDelete = dirNames.length - 4;
    for (let i = 0; i < toDelete && i < candidates.length; i++) {
      try {
        await fs.promises.rm(path.join(root, candidates[i].k), { recursive: true, force: true });
      } catch {
        /* ignore */
      }
    }
  } catch {
    /* readdir failed — ignore */
  }
}

/** Send WM_CLOSE to the process's main window (graceful close path). */
function closeWindowGracefully(pid) {
  return new Promise((resolve) => {
    try {
      const ps = spawn(
        'powershell.exe',
        ['-NoProfile', '-NonInteractive', '-Command',
          `(Get-Process -Id ${pid} -ErrorAction SilentlyContinue).CloseMainWindow() | Out-Null`],
        { windowsHide: true, stdio: 'ignore' }
      );
      ps.on('error', () => resolve(false));
      ps.on('exit', () => resolve(true));
    } catch {
      resolve(false);
    }
  });
}

/**
 * NOTE — there is no jcmd-based graceful exit. `jcmd <pid> VM.exit` was used
 * here until 2026-09-01; verified against a live JDK 25 JVM it answers
 * `java.lang.IllegalArgumentException: Unknown diagnostic command` (it is absent
 * from `jcmd help`), so that rung ALWAYS failed and every training run has in
 * fact exited through the WM_CLOSE path below. Keeping it only produced a
 * misleading 'jcmd attach failed' progress line and an extra process spawn.
 *
 * WM_CLOSE is safe for training because the branding mod cancels MC 26.2's
 * ClientShutdownWatchdog when `-Despectral.aot-training=true` is set (see
 * ClientShutdownWatchdogMixin + buildArgv); without that, the watchdog's
 * post-main budget expires during the ~150 s AOT dump and force-crashes the JVM
 * mid-write. Older versions (1.21.11) have no such watchdog class at all.
 */

/** Size-tracked latest.log watcher resolving on the menu marker.
 * `gameInitTimeoutMs` arms only on FIRST log activity (file creation/rotation/
 * growth): cold-path pre-log work (asset verification, natives extraction —
 * minutes on a big instance) is pipeline latency, not game-init time, and must
 * not burn the init budget. `hardTimeoutMs` bounds the total wait regardless,
 * so a silent JVM can never wedge the training queue slot. */
function waitForMarker(logFile, gameInitTimeoutMs = 180_000, hardTimeoutMs = 600_000) {
  return new Promise((resolve) => {
    let seenSize = fs.existsSync(logFile) ? fs.statSync(logFile).size : -1;
    const started = Date.now();
    let armedAt = null; // first log activity (creation, rotation, or growth)
    const iv = setInterval(() => {
      const now = Date.now();
      // Hard cap: unconditionally bounded, logged or not.
      if (now - started > hardTimeoutMs) {
        clearInterval(iv);
        console.warn(
          `[aot] hard cap ${hardTimeoutMs / 1000}s reached (log activity: ${armedAt !== null ? 'yes' : 'none'}); aborting marker wait`
        );
        resolve({ ok: false, at: now });
        return;
      }
      // Game-init cap: armed at first log activity, not at spawn.
      if (armedAt !== null && now - armedAt > gameInitTimeoutMs) {
        clearInterval(iv);
        console.warn(`[aot] game reached no menu within ${gameInitTimeoutMs / 1000}s of first log line; aborting marker wait`);
        resolve({ ok: false, at: now });
        return;
      }
      try {
        if (!fs.existsSync(logFile)) return;
        const size = fs.statSync(logFile).size;
        if (seenSize === -1 || size < seenSize) seenSize = 0; // appeared/truncated -> fresh
        if (size > seenSize) {
          if (armedAt === null) armedAt = now; // first activity arms the init clock
          const fd = fs.openSync(logFile, 'r');
          const buf = Buffer.alloc(size - seenSize);
          fs.readSync(fd, buf, 0, buf.length, seenSize);
          fs.closeSync(fd);
          seenSize = size;
          if (/Sound engine started/.test(buf.toString('utf8'))) {
            clearInterval(iv);
            resolve({ ok: true, at: now });
          }
        }
      } catch {
        /* log locked mid-write; retry */
      }
    }, 100);
  });
}

function waitForExit(child, timeoutMs) {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) return resolve(true);
    const timer = setTimeout(() => resolve(false), timeoutMs);
    child.once('exit', () => {
      clearTimeout(timer);
      resolve(true);
    });
    child.once('error', () => {
      clearTimeout(timer);
      resolve(true);
    });
  });
}
function formatResolveError(err) {
  if (!err) return 'resolve failed: unknown error';
  const causeMsg = err.cause?.message || (err.cause ? String(err.cause) : null);
  const detail = causeMsg && !err.message.includes(causeMsg) ? `${err.message} (${causeMsg})` : err.message;
  const urlMatch = err.url ? err.url : (detail.match(/https?:\/\/[^\s'")]+/)?.[0] ?? null);
  if (urlMatch && !detail.startsWith(urlMatch) && !detail.includes(`<${urlMatch}>`)) {
    return `resolve failed: <${urlMatch}>: ${detail}`;
  }
  return `resolve failed: ${detail}`;
}

/**
 * Background AOT training for an instance.
 * Returns { key, ok, cache_size_bytes?, proof?, error? }.
 * Emits train-progress / train-done events (contract).
 */
export async function trainInstance(instance, { key = null, onProgress, force = false } = {}) {
  // H3: respect the opt-in flag. Auto-train (creation / first launch) only runs
  // when the instance opts in; the explicit POST /train route passes force:true
  // so a user who clicks "train" always gets it.
  if (force !== true && instance?.aot_auto_train === false) {
    return { key, ok: false, skipped: true, error: 'aot_auto_train is off for this instance' };
  }
  if (instance?.loader === 'neoforge') {
    console.log('[aot] skipped for NeoForge instance ' + instance?.name);
    return { key: key ?? null, ok: false, error: 'AOT unavailable for NeoForge (requires a JDK 25-tier runtime)' };
  }
  const progress = (phase, message) => {
    const payload = { instance: instance.name, key, phase, message, at: Date.now() };
    if (onProgress) onProgress(payload);
    try {
      emit('train-progress', payload);
    } catch {
      /* SSE registry unavailable */
    }
  };
  const done = (payload) => {
    const tagged = { instance: instance.name, ...payload };
    try {
      emit('train-done', tagged);
    } catch {
      /* ignore */
    }
    return tagged;
  };

  let resolved = null;
  let lastError = null;
  const maxAttempts = 3; // 1 initial attempt + 2 retries
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      progress('resolve', `resolving launch configuration (${attempt}/${maxAttempts})…`);
      resolved = await resolveLaunch(instance, { mode: 'train' });
      lastError = null;
      break;
    } catch (e) {
      lastError = e;
      if (attempt < maxAttempts) {
        const delayMs = attempt * 1200;
        progress('retry', `resolve attempt ${attempt} failed (${e?.message ?? e}); retrying in ${delayMs}ms…`);
        await new Promise((r) => setTimeout(r, delayMs));
      }
    }
  }
  if (!resolved) {
    return done({ key, ok: false, error: formatResolveError(lastError) });
  }
  if (Number(resolved.java?.major ?? 0) < 25) {
    return done({ key, ok: false, error: 'AOT training requires a JDK 25-tier runtime' });
  }

  const modSetHash = await computeModSetHash(instance);
  const resolvedKey = cacheKey(resolved.version.id, resolved.java.build, process.arch);
  if (key !== null && key !== undefined && key !== resolvedKey) {
    // Caller supplied a key estimate (route pre-return); the authoritative key
    // comes from the JVM actually used. Prefer the resolved key and keep going.
    progress('key', `key drifted from estimate ${key} -> ${resolvedKey}`);
    key = resolvedKey;
  } else {
    key = resolvedKey;
  }
  const cacheDir = cacheDirFor(key);
  fs.mkdirSync(cacheDir, { recursive: true });
  const cacheFile = cacheFilePath(key);
  resolved.aotCachePath = cacheFile;
  resolved.aotCacheExists = false;

  const gameDir = resolved.gameDir;
  fs.mkdirSync(path.join(gameDir, 'logs'), { recursive: true });
  const logFile = path.join(gameDir, 'logs', 'latest.log');

  progress('spawn', `train JVM (${resolved.java.build}) -> ${cacheFile}`);
  const argv = buildArgv(instance, resolved); // mode 'train' -> -XX:AOTCacheOutput

  const child = spawnJava(resolved.java.path, argv, {
    cwd: gameDir,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  const pid = child.pid;
  progress('running', `pid ${pid}`);

  // Relay JVM/game output to the console so training is observable.
  const relay = (chunk) => {
    try {
      process.stdout.write(chunk);
    } catch {
      /* stdout closed */
    }
  };
  if (child.stdout) child.stdout.on('data', relay);
  if (child.stderr) child.stderr.on('data', relay);

  // The cap must measure GAME INIT, not the launch pipeline: on a cold train
  // (fingerprint miss -> thousands of asset sha1s + natives re-extraction) the
  // resolve+verify work inside the spawned JVM ran ~3 minutes before the first
  // game log line (measured 2026-08-30: 169s pre-log + 24s init), exhausting a
  // spawn-anchored 180s cap before the game even started logging. The timeout
  // clock arms on FIRST LOG ACTIVITY (creation/rotation/change of latest.log);
  // a totally silent JVM still hits the hard cap above.
  const CAP_MS = 180_000;
  const HARD_CAP_MS = 600_000; // never wait longer than 10 min, logged or not
  const marker = await waitForMarker(logFile, CAP_MS, HARD_CAP_MS);

  if (!marker.ok) {
    progress('timeout', `game did not reach the menu within ${CAP_MS / 1000}s`);
    // Still try a graceful close so a partial AOT config is not corrupted.
    await closeWindowGracefully(pid);
    await waitForExit(child, 60_000);
    return done({
      key,
      ok: false,
      error: `timeout waiting for 'Sound engine started' (${CAP_MS / 1000}s cap)`,
    });
  }

  progress('menu', 'Sound engine started; requesting graceful close');

  // Close ladder for TRAINING: WM_CLOSE, then wait out the whole AOT dump.
  //
  // WM_CLOSE runs Minecraft's own shutdown path, whose ClientShutdownWatchdog
  // (MC 26.2) force-crashes the JVM when post-main shutdown exceeds its budget —
  // and the AOT dump is exactly such a step (147 MB, ~150 s measured on 26.2;
  // crash-2026-08-31_00.02.14 'Client shutdown from post-main' was the JVM being
  // killed mid-write). The branding mod cancels that watchdog when
  // `-Despectral.aot-training=true` is present (ClientShutdownWatchdogMixin),
  // which buildArgv sets for every train run, so the dump can finish.
  //
  // The wait must cover the WHOLE dump: measured 2026-09-01 on 26.2-fabric the
  // JVM logged 'AOTCache creation is complete' ~165 s after the close request. A
  // 120 s budget expired mid-dump, so training reported failure and never wrote
  // meta.json — leaving a cache with no classpath stamp, which every later
  // launch then (correctly) treated as stale and retrained, forever.
  const EXIT_WAIT_MS = 600_000;
  await closeWindowGracefully(pid);
  await waitForExit(child, EXIT_WAIT_MS);
  if (child.exitCode === null && child.signalCode === null) {
    // NEVER TerminateProcess before the cache write. Leave the game running
    // and report the failure instead of corrupting the cache.
    return done({
      key,
      ok: false,
      error: `JVM did not exit within ${EXIT_WAIT_MS / 1000}s of WM_CLOSE; refusing to force-kill (the AOT cache is written on normal exit only)`,
    });
  }

  // Verify the cache and write meta.json in the same step.
  let cacheSize = 0;
  try {
    if (fs.existsSync(cacheFile)) cacheSize = fs.statSync(cacheFile).size;
  } catch {
    /* absent */
  }
  const ok = cacheSize > 0;
  if (ok) {
    const meta = {
      key,
      trained_at: new Date().toISOString(),
      // Full path+size+mtime identity of the classpath the JVM archived, so a
      // later launch can tell a valid cache from one the JVM will reject.
      // (Replaces the old basename-only classpath_manifest, which nothing read
      // and which could not detect the mtime drift that breaks JEP 483.)
      classpath_stamp: classpathStamp(resolved.classpath),
      java_build: resolved.java.build,
      game_version: resolved.version.id,
      mod_set_hash: modSetHash,
    };
    try {
      const tmp = metaPathFor(key) + '.tmp';
      fs.writeFileSync(tmp, JSON.stringify(meta, null, 2), 'utf8');
      fs.renameSync(tmp, metaPathFor(key));
    } catch (e) {
      progress('warn', `meta.json write failed: ${e.message}`);
    }
    await evictOldAotCaches(key);
  }

  const payload = { key, ok, cache_size_bytes: cacheSize, proof: aotProof(instance) };
  if (!ok) payload.error = 'game.aot missing or empty after training exit';
  progress(ok ? 'done' : 'failed', ok ? `cache written (${cacheSize} bytes)` : payload.error);
  return done(payload);
}
const activeTrainings = new Map();

/**
 * Queue one training run per instance. Creation and launch can both request
 * the cache; sharing the promise prevents two JVMs from training it at once.
 */
export function queueTrainInstance(instance, options = {}) {
  if (instance?.loader === 'neoforge') {
    console.log('[aot] skipped for NeoForge instance ' + instance?.name);
    return null;
  }
  const name = instance?.name;
  if (!name) return Promise.reject(new Error('instance name is required'));
  const running = activeTrainings.get(name);
  if (running) return running;
  // options.force flows to trainInstance (explicit POST /train always trains).
  const promise = trainInstance(instance, options).finally(() => {
    if (activeTrainings.get(name) === promise) activeTrainings.delete(name);
  });
  activeTrainings.set(name, promise);
  return promise;
}
