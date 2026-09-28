// @ts-nocheck — svelte 5 compiles .svelte.ts modules with a JS-only parser
// (analyze_module hardcodes typescript=false), so this file cannot contain TS
// syntax/annotations; tsc would flag implicit anys, hence the file-level opt-out.
import { setTheme as apiSetTheme, setAccent as apiSetAccent } from './api';
import { createStorageState } from './storageState.svelte';

/* ==========================================================================
   Theme + accent stores (runes — this file MUST keep the .svelte.ts extension
   so the Svelte compiler rewrites $state/$effect; .svelte.ts modules are
   parsed as plain JS by the compiler, so no TypeScript annotations here).
   - `theme.value`: 'dark' | 'light' | 'system' (reactive; read directly).
   - `theme.set(t)`: applies + persists to localStorage 'espectral_theme' +
     mirrors to the backend (PUT /api/theme).
   - `accent.value`: 'oro' | 'carmesi' | 'amatista' | 'esmeralda' | 'glaciar'
     (Workstream 4). Persists to 'espectral_accent' + mirrors to the backend
     (PUT /api/accent); the engine copies it into personalization.theme at
     launch so the Suite and title menu share the palette.
   The data-theme/data-accent attributes on <html> are driven by module
   effects registered from App.svelte (initThemeEffect).
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

// --- Accent (client colour theme) -------------------------------------------

/** Accent ids in display order; `swatch` feeds pickers (CSS needs no lookup). */
export const ACCENTS = [
  { id: 'oro', swatch: '#d9a93b' },
  { id: 'carmesi', swatch: '#e5484d' },
  { id: 'amatista', swatch: '#bb86fc' },
  { id: 'esmeralda', swatch: '#10b981' },
  { id: 'glaciar', swatch: '#38bdf8' },
];

const ACCENT_IDS = ACCENTS.map((a) => a.id);

function isAccent(v) {
  return typeof v === 'string' && ACCENT_IDS.includes(v);
}

function mirrorAccent(next) {
  apiSetAccent(next).catch(() => {
    /* backend offline: localStorage keeps the choice */
  });
}

const _accent = createStorageState('espectral_accent', 'oro', isAccent, mirrorAccent);

export const accent = {
  get value() {
    return _accent.value;
  },
  /** @param {string} next */
  set(next) {
    _accent.value = next;
  },
};

/**
 * MUST be called once from a component's init scope (App.svelte) — a module-
 * level $effect here is an orphan (svelte.dev/e/effect_orphan) and crashes
 * mount. The effects keep <html data-theme>/<html data-accent> in sync.
 */
export function initThemeEffect() {
  $effect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.dataset.theme = resolveTheme(_storage.value);
  });
  $effect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.dataset.accent = _accent.value;
  });
}
