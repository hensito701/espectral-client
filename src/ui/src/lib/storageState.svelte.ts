// @ts-nocheck — svelte 5 compiles .svelte.ts modules with a JS-only parser
// (analyze_module hardcodes typescript=false), so this file cannot contain TS
// syntax/annotations; tsc would flag implicit anys, hence the file-level opt-out.
// shared localStorage + $state helper — MUST keep .svelte.ts so the Svelte
// compiler rewrites $state; .svelte.ts modules are parsed as plain JS
// (typescript=false), so this file keeps JS-only syntax + JSDoc.

/**
 * Reactive localStorage wrapper.
 * - reads from localStorage once (with try/catch for SSR/storage-unavailable)
 * - validates the stored value, falling back when invalid
 * - exposes `{ value }` with a setter that persists + optionally notifies
 *
 * @template T
 * @param {string} key - localStorage key
 * @param {T} fallback - value when nothing stored or validation fails
 * @param {(v: unknown) => boolean} validate - predicate for stored / incoming values
 * @param {(next: T) => void} [onSet] - called after a successful set (e.g. mirror to backend)
 * @returns {{ get value(): T, set value(v: T): void }}
 */
export function createStorageState(key, fallback, validate, onSet) {
  let initial = fallback;
  if (typeof localStorage !== 'undefined') {
    try {
      const saved = localStorage.getItem(key);
      if (validate(saved)) initial = /** @type {T} */ (saved);
    } catch {
      /* storage unavailable */
    }
  }

  const state = $state({ value: initial });

  return {
    get value() {
      return state.value;
    },
    /** @param {T} next */
    set value(next) {
      apply(next);
    },
    /** @param {T} next */
    set(next) {
      apply(next);
    },
  };

  /** @param {T} next */
  function apply(next) {
    if (!validate(next)) return;
    state.value = next;
    try {
      localStorage.setItem(key, next);
    } catch {
      /* storage unavailable — in-memory only */
    }
    if (typeof onSet === 'function') {
      try {
        onSet(next);
      } catch {
        /* onSet is best-effort */
      }
    }
  }
}
