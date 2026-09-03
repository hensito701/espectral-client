/**
 * Tauri bootstrap — runs only inside the desktop shell (window.__TAURI_INTERNALS__).
 *
 * In the browser flow the engine is already running (started by the user). In
 * the Tauri app the shell spawns the engine as a child process, so we ask it
 * to start before the UI's first API call. The call is best-effort: the app
 * still works in a plain browser tab.
 */
import { API_BASE } from './api';

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
/**
 * Set while the in-app updater owns the engine lifecycle (shutdown → install):
 * polling stores must NOT self-heal respawn the engine or node.exe stays
 * locked and the NSIS installer fails with "Error opening file for writing".
 */
export let suppressEngineRestart: boolean = false;
export function setSuppressEngineRestart(v: boolean): void {
  suppressEngineRestart = v;
}

/** Ask the Rust shell to start the engine child process, then wait until the
 *  engine answers /api/health (bounded retry) so the UI's first fetch lands. */
export async function startEngine(): Promise<void> {
  if (!isTauri()) return;
  try {
    // Injected by @tauri-apps/api in the Tauri context (invoke lives on
    // window.__TAURI_INTERNALS__.invoke when not bundled with the npm pkg).
    const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as {
      invoke?: (cmd: string, args?: unknown) => Promise<unknown>;
    };
    await internals.invoke?.('start_engine', {});
  } catch {
    /* non-fatal — the engine may already be up */
  }
  // Bounded wait for the engine's HTTP server (starts fast, but the first
  // launch can be slow while the JDK is resolved).
  const base = API_BASE;
  for (let i = 0; i < 20; i++) {
    try {
      const res = await fetch(`${base}/api/health`, { signal: AbortSignal.timeout(1000) });
      if (res.ok) return;
    } catch {
      /* engine still booting */
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}
