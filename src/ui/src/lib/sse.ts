/**
 * Canonical engine API base resolution — the single source of truth for the
 * whole UI (api.ts imports this; do not duplicate it).
 *
 * Dev talks to the engine on 127.0.0.1:4199. In the browser prod build the
 * engine serves the UI, so relative URLs hit the same origin. In the Tauri
 * desktop shell the window loads the bundled dist (custom protocol), so the
 * API must be reached explicitly at the engine's origin — Tauri exposes it on
 * window.__TAURI_INTERNALS__.
 */
export function resolveApiBase(): string {
  if (import.meta.env.DEV) return 'http://127.0.0.1:4199';
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    // The engine binds IPv4-only (server.listen(port, '127.0.0.1')); WebView2
    // may resolve `localhost` to ::1 first and refuse the connection, so the
    // packaged app must target the literal loopback address.
    return 'http://127.0.0.1:4199';
  }
  return '';
}

export const API_BASE: string = resolveApiBase();


export interface SseEvent {
  type: string;
  data: unknown;
}

export const SSE_EVENT_NAMES = [
  'launch-log',
  'launch-exit',
  'mod-progress',
  'train-progress',
  'train-done',
  'import-progress',
  'import-done',
] as const;

/* ---------- one shared EventSource for the entire app ----------
   Every page used to open its own EventSource to /api/events (N connections,
   N reconnect loops). subscribeEvents() now registers listeners on a single
   lazily-created connection that fans out to all subscribers and stays warm
   for the session so page navigations attach instantly. */

type EventsListener = (ev: SseEvent) => void;

const listeners = new Set<EventsListener>();
let shared: EventSource | null = null;
let failures = 0;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

function ensureSharedConnection(): void {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  if (shared) return;
  const es = new EventSource(`${API_BASE}/api/events`);
  shared = es;

  for (const name of SSE_EVENT_NAMES) {
    es.addEventListener(name, (e: MessageEvent<string>) => {
      let data: unknown = null;
      try {
        data = JSON.parse(e.data);
      } catch {
        data = e.data;
      }
      for (const listener of listeners) listener({ type: name, data });
    });
  }

  es.onopen = () => {
    failures = 0;
  };

  es.onerror = () => {
    // CONNECTING means the built-in reconnect is already retrying — leave it.
    if (!shared || shared.readyState === EventSource.CONNECTING) return;
    es.close();
    shared = null;
    const delay = Math.min(1000 * 2 ** Math.min(failures, 5), 15_000);
    failures += 1;
    clearTimeout(reconnectTimer ?? undefined);
    reconnectTimer = setTimeout(ensureSharedConnection, delay);
  };
}

/**
 * Subscribe to engine events over the shared connection. Returns an
 * unsubscribe function.
 */
export function subscribeEvents(onEvent: EventsListener): () => void {
  listeners.add(onEvent);
  ensureSharedConnection();
  return () => {
    listeners.delete(onEvent);
  };
}

