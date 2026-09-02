// @ts-nocheck — svelte 5 compiles .svelte.ts modules with a JS-only parser
// (analyze_module hardcodes typescript=false), so this file cannot contain TS
// syntax/annotations; tsc would flag implicit anys, hence the file-level opt-out.
// clipboard helper — MUST keep .svelte.ts so the Svelte compiler rewrites
// $state; .svelte.ts modules are parsed as plain JS (typescript=false).

/**
 * Reactive clipboard helper.
 * @param {number} [ms=1600] - ms until `copied` resets
 * @returns {{ get copied(): boolean, copy: (text: string) => void }}
 */
export function useCopy(ms = 1600) {
  let copied = $state(false);
  /** @type {ReturnType<typeof setTimeout> | undefined} */
  let timer;

  /**
   * @param {string} text
   */
  function copy(text) {
    if (!text) return;
    try {
      const p = navigator.clipboard?.writeText(text);
      if (p && typeof p.then === 'function') {
        p.then(() => {
          copied = true;
          clearTimeout(timer);
          timer = setTimeout(() => (copied = false), ms);
        }).catch(() => {
          copied = false;
        });
      } else {
        // sync fallback (clipboard unavailable — still show feedback)
        copied = true;
        clearTimeout(timer);
        timer = setTimeout(() => (copied = false), ms);
      }
    } catch {
      copied = false;
    }
  }

  return {
    get copied() {
      return copied;
    },
    copy,
  };
}
