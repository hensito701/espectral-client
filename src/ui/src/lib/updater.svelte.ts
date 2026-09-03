// @ts-nocheck — svelte 5 compiles .svelte.ts modules with a JS-only parser
// (analyze_module hardcodes typescript=false), so this file cannot contain TS
// syntax/annotations; tsc would flag implicit anys, hence the file-level opt-out.
//
// NOTE on the dynamic imports below: the @tauri-apps/plugin-updater and
// plugin-process modules are only loadable inside the Tauri desktop shell
// (they invoke plugin commands at module init). Importing them statically
// breaks the plain-browser dev flow (`npm run ui` in a browser tab), so they
// are loaded lazily behind isTauri() — a genuine runtime-selected platform
// module, the sanctioned exception to static-import policy.
import { isTauri, setSuppressEngineRestart, startEngine } from './tauri';
import { shutdownEngine, API_BASE } from './api';

/* ==========================================================================
   Updater — Tauri auto-update bridge (desktop shell only).
   Flow: boot check 30 s after launch (once per session) plus a 6 h re-check
   so a long-running session notices a release that drops mid-session; when
   an update exists, the top-bar CTA (TopChrome) offers the install; on user
   accept, downloadAndInstall with progress, then relaunch via the process
   plugin. Plain-browser flow: no-op. Every failure lands back in 'idle' —
   the updater must never block or break the launcher.
   ========================================================================== */

export const updateState = $state({
  status: 'idle', // idle | checking | available | downloading | installing | ready
  version: '', // the version offered by the endpoint
  progress: 0, // 0..1 download progress (0 when no content-length was sent)
  error: '',
});

let updateObj = null; // plugin Update resource (kept for install)
let checkedThisSession = false;
let pollingArmed = false; // startUpdatePolling idempotency guard

const FIRST_CHECK_DELAY_MS = 30_000; // C13: keep the 15 s HTTPS check off the boot path
const RECHECK_INTERVAL_MS = 6 * 60 * 60 * 1000; // 6 h

/**
 * Check for updates. The boot path keeps a once-per-session guard; `force`
 * (the 6 h re-checks) bypasses it. A forced check never clobbers an
 * in-flight download/install: it only runs from 'idle' or 'available'.
 * Safe to call from any component scope.
 */
export async function checkForUpdate(force = false) {
  if (!isTauri()) return;
  if (!force && checkedThisSession) return;
  checkedThisSession = true;
  if (updateState.status !== 'idle' && !(force && updateState.status === 'available')) return;
  updateState.status = 'checking';
  try {
    const mod = await import('@tauri-apps/plugin-updater');
    const update = await mod.check({ timeout: 15000 });
    if (update) {
      updateObj = update;
      updateState.version = update.version;
      updateState.error = '';
      updateState.status = 'available';
    } else {
      updateState.status = 'idle';
    }
  } catch (e) {
    // Offline, endpoint down, unsigned dev build — all non-events.
    updateState.status = 'idle';
    updateState.error = e instanceof Error ? e.message : String(e);
  }
}

/**
 * Arm the update polling: first check 30 s after launch, then re-check every
 * 6 h so a release that drops mid-session is picked up. Idempotent; no-op
 * outside the Tauri desktop shell.
 */
export function startUpdatePolling() {
  if (!isTauri() || pollingArmed) return;
  pollingArmed = true;
  setTimeout(() => void checkForUpdate(), FIRST_CHECK_DELAY_MS);
  setInterval(() => void checkForUpdate(true), RECHECK_INTERVAL_MS);
}

/**
 * Download + install the pending update, then relaunch the app.
 * Resolves when the relaunch is scheduled (the process exits shortly after).
 *
 * Two hard-won rules live here:
 * 1. Exactly ONE installer run: download() then install(). The old code called
 *    downloadAndInstall() (which already installs) followed by install(),
 *    launching the NSIS installer twice over the same files — the second run
 *    fails with "Error opening file for writing".
 * 2. The engine is DOWN before NSIS writes: it runs on the bundled node.exe,
 *    and any live engine keeps that file locked with the same error. The
 *    health store's self-heal is suppressed meanwhile so it can't respawn it.
 */
export async function installUpdate() {
  if (!updateObj || !isTauri()) return;
  try {
    // Dynamic: the process plugin invokes Tauri commands at module init and
    // only exists inside the desktop shell (see header note).
    const proc = await import('@tauri-apps/plugin-process');
    updateState.status = 'downloading';
    updateState.progress = 0;
    let total = 0;
    let received = 0;
    await updateObj.download((event) => {
      if (event.event === 'Started') {
        total = event.data.contentLength ?? 0;
      } else if (event.event === 'Progress') {
        received += event.data.chunkLength;
        if (total > 0) updateState.progress = Math.min(1, received / total);
      } else if (event.event === 'Finished') {
        updateState.progress = 1;
      }
    });
    updateState.status = 'installing';
    setSuppressEngineRestart(true);
    try {
      await shutdownEngine();
    } catch {
      // Engine already dead — nothing locked.
    }
    await waitForEngineExit(API_BASE, 15_000);
    // Single installer run; the process plugin restarts the app afterwards.
    await updateObj.install();
    updateState.status = 'ready';
    await proc.relaunch();
  } catch (e) {
    // Install failed: hand lifecycle back (restart the engine we stopped) so
    // the app isn't left permanently offline.
    setSuppressEngineRestart(false);
    try {
      await startEngine();
    } catch {
      // Self-heal stays on; the next health failure retries.
    }
    updateState.status = updateObj ? 'available' : 'idle';
    updateState.error = e instanceof Error ? e.message : String(e);
  }
}

/**
 * Resolve when the engine stops answering /api/health (connection refused =
 * process gone = node.exe unlocked) or when the timeout elapses — NSIS shows
 * its own retry dialog past that point rather than failing silently.
 */
async function waitForEngineExit(apiBase, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const ctrl = new AbortController();
      const timer = setTimeout(() => ctrl.abort(), 1500);
      await fetch(`${apiBase}/api/health`, { signal: ctrl.signal });
      clearTimeout(timer);
    } catch {
      return;
    }
    const { promise, resolve } = Promise.withResolvers();
    setTimeout(resolve, 250);
    await promise;
  }
}
