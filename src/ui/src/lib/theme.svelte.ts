// @ts-nocheck — svelte 5 compiles .svelte.ts modules with a JS-only parser
// (analyze_module hardcodes typescript=false), so this file cannot contain TS
// syntax/annotations; tsc would flag implicit anys, hence the file-level opt-out.
import { setTheme as apiSetTheme } from './api';
import { createStorageState } from './storageState.svelte';

/* ==========================================================================
   Theme store (runes — this file MUST keep the .svelte.ts extension so the
   Svelte compiler rewrites $state/$effect; .svelte.ts modules are parsed as
   plain JS by the compiler, so no TypeScript annotations here).
   - `theme.value`: 'dark' | 'light' | 'system' (reactive; read directly).
   - `theme.set(t)`: applies + persists to localStorage 'espectral_theme' +
     mirrors to the backend (PUT /api/theme).
   The data-theme attribute on <html> is driven by a module-level $effect,
   resolving 'system' through prefers-color-scheme (default dark).
   ========================================================================== */

function isTheme(v) {
  return v === 'dark' || v === 'light' || v === 'system';
}

function mirrorTheme(next) {
  apiSetTheme(next).catch(() => {
    /* backend offline: rendering stays correct from localStorage */
  });
}

const _storage = createStorageState('espectral_theme', 'system', isTheme, mirrorTheme);

const media =
  typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-color-scheme: dark)')
    : null;

let systemPrefersDark = $state(media ? media.matches : true);

media?.addEventListener('change', (e) => {
  systemPrefersDark = e.matches;
});

/**
 * @param {'dark' | 'light' | 'system'} t
 * @returns {'dark' | 'light'}
 */
export function resolveTheme(t) {
  if (t === 'system') return systemPrefersDark ? 'dark' : 'light';
  return t;
}

export const theme = {
  get value() {
    return _storage.value;
  },
  /** @param {'dark' | 'light' | 'system'} next */
  set(next) {
    _storage.value = next;
  },
};

/**
 * MUST be called once from a component's init scope (App.svelte) — a module-
 * level $effect here is an orphan (svelte.dev/e/effect_orphan) and crashes
 * mount. The effect keeps <html data-theme> in sync with the store.
 */
export function initThemeEffect() {
  $effect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.dataset.theme = resolveTheme(_storage.value);
  });
}
