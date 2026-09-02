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
import { isTauri } from './tauri';

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
 */
export async function installUpdate() {
  if (!updateObj || !isTauri()) return;
  try {
    const proc = await import('@tauri-apps/plugin-process');
    updateState.status = 'downloading';
    updateState.progress = 0;
    let total = 0;
    let received = 0;
    await updateObj.downloadAndInstall((event) => {
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
    // The NSIS installer runs its silent update; the process plugin restarts
    // the app afterwards.
    await updateObj.install();
    updateState.status = 'ready';
    await proc.relaunch();
  } catch (e) {
    updateState.status = updateObj ? 'available' : 'idle';
    updateState.error = e instanceof Error ? e.message : String(e);
  }
}
