/* ==========================================================================
   Toast store (runes — this file MUST keep the .svelte.ts extension so the
   Svelte compiler rewrites $state).
   Items: { id, kind: 'ok' | 'err' | 'info', text, href? }.
   pushToast auto-dismisses after 6 s and returns the id; dismissToast removes
   a toast immediately. The href field makes the toast navigate on click.
   ========================================================================== */

export type ToastKind = 'ok' | 'err' | 'info';

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
  href?: string;
}

export interface PushToastOptions {
  kind: ToastKind;
  text: string;
  href?: string;
}

const TOAST_MS = 6000;

export const toasts = $state<ToastItem[]>([]);

let nextId = 1;

/**
 * Push a toast onto the stack. Auto-dismisses after 6 s.
 */
export function pushToast({ kind, text, href }: PushToastOptions): number {
  const id = nextId++;
  toasts.push({ id, kind, text, href });
  setTimeout(() => dismissToast(id), TOAST_MS);
  return id;
}

/**
 * Remove a toast by id (no-op if already dismissed).
 */
export function dismissToast(id: number): void {
  const i = toasts.findIndex((toast) => toast.id === id);
  if (i !== -1) toasts.splice(i, 1);
}
