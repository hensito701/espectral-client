// dialog.ts — Tauri native file/folder pickers (Option A: Linux compat).
//
// On Linux the engine cannot show a PowerShell file picker, so the UI opens
// the Tauri dialog plugin directly when running inside Tauri. Outside Tauri
// (plain browser / dev preview) every function degrades to a thrown
// DialogUnavailable sentinel so callers in lib/api.ts can fall back to the
// existing engine POST endpoints byte-identical to the Windows path.
//
// The plugin module is loaded via dynamic import only — never a top-level
// static import — so the vite browser build cannot break when
// @tauri-apps/plugin-dialog is absent or unusable outside Tauri.

/** Sentinel thrown when no native dialog is available; callers fall back. */
export class DialogUnavailable extends Error {
  constructor(message = 'Native dialog unavailable') {
    super(message);
    this.name = 'DialogUnavailable';
  }
}

/** True only inside a Tauri webview — same check as lib/tauri.ts. */
export const isTauri = (): boolean => {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
};

/**
 * True on Windows (Chromium userAgentData with legacy userAgent fallback).
 * Used to keep the battle-tested engine PowerShell picker on Windows while
 * the native Tauri dialog serves Linux, where the engine has no picker.
 */
export const isWindows = (): boolean => {
  try {
    if (typeof navigator === 'undefined') return false;
    const withData = navigator as Navigator & { userAgentData?: { platform?: string } };
    const plat = withData.userAgentData?.platform ?? withData.userAgent ?? '';
    return /windows/i.test(plat);
  } catch {
    return false;
  }
};

/** Normalize plugin-dialog open() results (string | string[] | null) to string | null. */
const normalizeSelection = (selection: unknown): string | null => {
  if (typeof selection === 'string') return selection;
  if (Array.isArray(selection)) {
    const first = selection[0];
    return typeof first === 'string' ? first : null;
  }
  return null;
};

/** Native .mrpack file picker; throws DialogUnavailable when unusable. */
export const openMrpackDialog = async (): Promise<string | null> => {
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const selection = await open({
      multiple: false,
      filters: [{ name: 'Modrinth modpack', extensions: ['mrpack'] }],
    });
    return normalizeSelection(selection);
  } catch {
    throw new DialogUnavailable('Native mrpack dialog unavailable');
  }
};

/** Native folder picker; throws DialogUnavailable when unusable. */
export const openFolderDialog = async (title?: string): Promise<string | null> => {
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const selection = await open({ directory: true, title });
    return normalizeSelection(selection);
  } catch {
    throw new DialogUnavailable('Native folder dialog unavailable');
  }
};
