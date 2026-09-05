// @ts-nocheck — svelte 5 compiles .svelte.ts modules with a JS-only parser
// (analyze_module hardcodes typescript=false), so this file cannot contain TS
// syntax/annotations; tsc would flag implicit anys, hence the file-level opt-out.
import { es, en } from './i18n-dicts';
import { createStorageState } from './storageState.svelte';

/* ==========================================================================
   i18n store (runes — this file MUST keep the .svelte.ts extension so the
   Svelte compiler rewrites $state; .svelte.ts modules are parsed as plain JS).
   - `lang.value`: 'es' | 'en' (reactive; read directly).
   - `lang.set(l)`: applies + persists to localStorage 'espectral_lang'.
   - `t(key, params)`: returns the string for the active language (reads the
     reactive state, so calls inside templates / $derived re-render on toggle).
   The <html lang> attribute is driven by a module-level $effect via
   initLangEffect() (must be called from a component scope, like the theme).
   ========================================================================== */

const dicts = { es, en };

function isLang(v) {
  return v === 'es' || v === 'en';
}

const _storage = createStorageState('espectral_lang', 'es', isLang);

export const lang = {
  get value() {
    return _storage.value;
  },
  /** @param {'es' | 'en'} next */
  set(next) {
    _storage.value = next;
  },
};

/**
 * Translate a key for the active language. Reads `lang.value` reactively, so
 * callers get re-renders when the language changes. Unknown keys fall back to
 * the key itself, or to a string fallback when one is passed as 2nd arg
 * (e.g. server-provided English text). Optional params replace `{name}`
 * placeholders.
 * @param {string} key
 * @param {Record<string, string | number> | string} [paramsOrFallback]
 */
export function t(key, paramsOrFallback?) {
  const table = dicts[lang.value];
  const fallback = typeof paramsOrFallback === 'string' ? paramsOrFallback : key;
  let str = table && Object.prototype.hasOwnProperty.call(table, key) ? table[key] : fallback;
  if (paramsOrFallback && typeof paramsOrFallback === 'object') {
    for (const k of Object.keys(paramsOrFallback)) {
      str = str.split(`{${k}}`).join(String(paramsOrFallback[k]));
    }
  }
  return str;
}

/**
 * MUST be called once from a component's init scope (App.svelte) — a module-
 * level $effect here is an orphan and crashes mount. Keeps <html lang> in
 * sync with the store.
 */
export function initLangEffect() {
  $effect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.lang = lang.value;
  });
}
