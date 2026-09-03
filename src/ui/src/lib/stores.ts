import { get, writable } from 'svelte/store';
import {
  getHealth,
  getLaunches,
  getLaunchLog,
  getServers,
  getVersions,
  listInstances,
  subscribeEvents,
} from './api';
import type { HealthInfo, InstanceSummary, LiveLaunch, ServerStatus, VersionManifest } from './types';
import { startEngine, suppressEngineRestart } from './tauri';
import { theme } from './theme.svelte';
import { t } from './i18n.svelte';
import { pushToast } from './toast.svelte';
export { theme };

/* ==========================================================================
   App-level stores (classic svelte/store — stores.ts is plain TS, so runes
   are unavailable here; consume in .svelte files via $store auto-subscription
   e.g. `$servers.value`, or `get(store)` in .ts).
   ========================================================================== */

const SERVER_POLL_MS = 60_000;
const MAX_LOG_LINES = 2000;

function errMsg(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/* ---------- shared polling-store factory ----------
   One generic implementation for every "GET an endpoint, expose
   { value, loading, error, refresh }" store. `pollMs` opts into a background
   interval; without it the store fetches once on start. */

interface PollingState<T> {
  value: T;
  loading: boolean;
  error: string | null;
  lastFetched: number | null;
  refresh: () => Promise<void>;
}

function createPollingStore<T>(
  initial: T,
  fetcher: () => Promise<T>,
  opts: { pollMs?: number; onError?: (err: unknown) => void; pauseWhenRunning?: boolean } = {},
) {
  const { subscribe, set, update } = writable<PollingState<T>>({
    value: initial,
    loading: true,
    error: null,
    lastFetched: null,
    refresh: async () => {},
  });

  let timer: ReturnType<typeof setInterval> | null = null;

  // Check whether any live launch is currently running — used to quiet
  // polling during boot (C13). Lazy reference to liveLaunches so the factory
  // can be defined before the store itself. Evaluated only inside timer
  // callbacks, i.e. after module init.
  function shouldPausePolling(): boolean {
    if (!opts.pauseWhenRunning) return false;
    try {
      const state = get(liveLaunches as unknown as { subscribe: (fn: (v: unknown) => void) => () => void }) as unknown as PollingState<LiveLaunch[]>;
      const arr = state?.value;
      return Array.isArray(arr) && arr.some((l) => l.running);
    } catch {
      return false;
    }
  }

  const refresh = async (): Promise<void> => {
    update((s) => ({ ...s, loading: true, error: null, refresh }));
    try {
      const value = await fetcher();
      set({ value, loading: false, error: null, lastFetched: Date.now(), refresh });
    } catch (err) {
      update((s) => ({ ...s, loading: false, error: errMsg(err), refresh }));
      opts.onError?.(err);
    }
  };

  return {
    subscribe,
    refresh,
    start: () => {
      void refresh();
      if (opts.pollMs)
        timer = setInterval(() => {
          if (shouldPausePolling()) return;
          void refresh();
        }, opts.pollMs);
    },
    stop: () => {
      clearInterval(timer ?? undefined);
      timer = null;
    },
  };
}

/* ---------- servers: GET /api/servers every 60 s ---------- */

export const servers = createPollingStore<ServerStatus[]>([], getServers, { pollMs: SERVER_POLL_MS, pauseWhenRunning: true });
servers.start();

/* ---------- health: GET /api/health every 30 s (single poller) ----------
   P2-3: Topbar and StatusBar each used to run their own identical 30 s
   getHealth() poller — two always-on duplicate requests. One shared store,
   both components subscribe. */

const HEALTH_POLL_MS = 30_000;

export const health = createPollingStore<HealthInfo | null>(null, getHealth, {
  pollMs: HEALTH_POLL_MS,
  pauseWhenRunning: true,
  // Self-heal: ask the Tauri shell to (re)start the engine. The Rust side is
  // idempotent — it reclaims the port if a stale process squats on it and
  // skips when a healthy engine already answers. Without this, an engine that
  // dies or fails to boot leaves the app permanently offline ("failed to
  // fetch everywhere") until a manual relaunch.
  onError: () => {
    // Updater-owned shutdown (update installing): respawning here would
    // re-lock node.exe and fail the NSIS install — stay down until relaunch.
    if (!suppressEngineRestart) void startEngine();
  },
});
health.start();

/* ---------- versions: version manifest (cached 6 h server-side) ---------- */

export const versions = createPollingStore<VersionManifest | null>(null, getVersions);
versions.start();

/* ---------- instances ---------- */

export const instances = createPollingStore<InstanceSummary[]>([], listInstances);
instances.start();

/* ---------- launch log: keyed ring buffers fed by SSE (launch-log / launch-exit) ----------
   Contract C.1: one buffer per launch key. The old single-buffer store let ANY
   incoming SSE event overwrite the displayed key — with concurrent games each
   key now keeps its own ring buffer, and the UI picks which one is shown via
   `activeKey`. A thin compat surface (key/lines/cursor/running/append/start/
   clear) projects the ACTIVE buffer so existing $launchLog.* callers keep
   working unchanged. */

const MAX_BUFFERS = 8;

export interface LaunchBuffer {
  key: string;
  instance: string;
  account: string;
  lines: string[];
  cursor: number;
  running: boolean;
}

export interface LaunchLogState {
  /** launch key → buffer (insertion order: oldest first). */
  buffers: Record<string, LaunchBuffer>;
  /** Key whose buffer the log viewers display. */
  activeKey: string | null;
  /** Running buffers, newest first — drives the per-game log selector. */
  live: LaunchBuffer[];
  /* Compat projection of the active buffer (old single-buffer shape). */
  key: string | null;
  lines: string[];
  cursor: number;
  running: boolean;
  append: (line: string) => void;
  start: (key: string, meta?: { instance?: string; account?: string }) => Promise<void>;
  setActive: (key: string) => void;
  clear: () => void;
}

function emptyBuffer(key: string, instance = '', account = ''): LaunchBuffer {
  return { key, instance, account, lines: [], cursor: 0, running: true };
}

function liveList(buffers: Record<string, LaunchBuffer>): LaunchBuffer[] {
  return Object.values(buffers)
    .filter((b) => b.running)
    .reverse();
}

function prune(buffers: Record<string, LaunchBuffer>, activeKey: string | null): Record<string, LaunchBuffer> {
  if (Object.keys(buffers).length <= MAX_BUFFERS) return buffers;
  const next = { ...buffers };
  // Drop oldest finished buffers first …
  for (const k of Object.keys(next)) {
    if (Object.keys(next).length <= MAX_BUFFERS) break;
    if (k === activeKey || next[k].running) continue;
    delete next[k];
  }
  // … then oldest running ones (never the active one).
  for (const k of Object.keys(next)) {
    if (Object.keys(next).length <= MAX_BUFFERS) break;
    if (k === activeKey) continue;
    delete next[k];
  }
  return next;
}
function createLaunchLogStore() {
  const { subscribe, update } = writable<LaunchLogState>({
    buffers: {},
    activeKey: null,
    live: [],
    key: null,
    lines: [],
    cursor: 0,
    running: false,
    append: () => {},
    start: async () => {},
    setActive: () => {},
    clear: () => {},
  });

  // Rebuild the compat projection + live list from the keyed map.
  function project(s: LaunchLogState): LaunchLogState {
    const active = s.activeKey !== null ? s.buffers[s.activeKey] : undefined;
    return {
      ...s,
      live: liveList(s.buffers),
      key: active ? active.key : null,
      lines: active ? active.lines : [],
      cursor: active ? active.cursor : 0,
      running: active ? active.running : false,
      append,
      start,
      setActive,
      clear,
    };
  }

  function applyLog(
    buffers: Record<string, LaunchBuffer>,
    activeKey: string | null,
    key: string,
    incoming: string[],
    cursor?: number,
    running?: boolean,
  ): Record<string, LaunchBuffer> {
    const prev = buffers[key] ?? emptyBuffer(key);
    const nextBuf: LaunchBuffer = { ...prev };
    if (incoming.length > 0) {
      // NOTE: keep this returning a NEW array ref — LogViewer receives `lines`
      // as a prop and Svelte 5 memoizes {#each} on reference identity; an
      // in-place mutation would freeze the log view. The O(ring) copy here is
      // the price of prop-based reactivity and is bounded by MAX_LOG_LINES.
      nextBuf.lines = [...prev.lines, ...incoming].slice(-MAX_LOG_LINES);
    }
    if (typeof cursor === 'number') nextBuf.cursor = cursor;
    if (typeof running === 'boolean') nextBuf.running = running;
    return prune({ ...buffers, [key]: nextBuf }, activeKey);
  }

  // --- Coalesce launch-log line appends (C13): accumulate per-key lines and
  // flush once per animation frame (or 16ms fallback). Cursor/running track
  // latest value per key so semantics stay intact.
  const pending = new Map<string, { lines: string[]; cursor?: number; running?: boolean }>();
  let raf: ReturnType<typeof requestAnimationFrame> | null = null;
  let fallbackTimer: ReturnType<typeof setTimeout> | null = null;

  function flushPending(): void {
    if (pending.size === 0) return;
    const snapshot = new Map(pending);
    pending.clear();
    update((s) => {
      let buffers: Record<string, LaunchBuffer> = s.buffers;
      const aKey = s.activeKey;
      for (const [key, p] of snapshot) {
        buffers = applyLog(buffers, aKey, key, p.lines, p.cursor, p.running);
      }
      return project({ ...s, buffers });
    });
  }

  function scheduleFlush(): void {
    if (raf !== null || fallbackTimer !== null) return;
    if (typeof requestAnimationFrame === 'function') {
      raf = requestAnimationFrame(() => {
        raf = null;
        flushPending();
      });
    } else {
      fallbackTimer = setTimeout(() => {
        fallbackTimer = null;
        flushPending();
      }, 16);
    }
  }

  /** Route a launch-log SSE event to its own key's buffer (coalesced). */
  const appendFor = (key: string, incoming: string[], cursor?: number, running?: boolean): void => {
    if (incoming.length === 0 && typeof cursor !== 'number' && typeof running !== 'boolean') return;
    const cur = pending.get(key);
    if (cur) {
      if (incoming.length) cur.lines.push(...incoming);
      if (typeof cursor === 'number') cur.cursor = cursor;
      if (typeof running === 'boolean') cur.running = running;
    } else {
      pending.set(key, { lines: [...incoming], cursor, running });
    }
    scheduleFlush();
  };

  /** Route a launch-exit SSE event: mark that key's buffer finished. */
  const markExited = (key: string, meta?: { instance?: string; account?: string }): void => {
    update((s) => {
      const prev = s.buffers[key] ?? emptyBuffer(key);
      const nextBuf: LaunchBuffer = {
        ...prev,
        instance: meta?.instance ?? prev.instance,
        account: meta?.account ?? prev.account,
        running: false,
      };
      return project({ ...s, buffers: prune({ ...s.buffers, [key]: nextBuf }, s.activeKey) });
    });
  };

  const setActive = (key: string): void => {
    update((s) => (s.buffers[key] ? project({ ...s, activeKey: key }) : s));
  };

  const append = (line: string): void => {
    update((s) =>
      s.activeKey === null
        ? s
        : project({ ...s, buffers: applyLog(s.buffers, s.activeKey, s.activeKey, [line]) }),
    );
  };

  const clear = (): void => {
    update((s) => {
      if (s.activeKey === null || !s.buffers[s.activeKey]) return project(s);
      const buf = s.buffers[s.activeKey];
      const buffers = { ...s.buffers, [s.activeKey]: { ...buf, lines: [], cursor: 0 } };
      return project({ ...s, buffers });
    });
  };

  const start = async (key: string, meta?: { instance?: string; account?: string }): Promise<void> => {
    update((s) => {
      const prev = s.buffers[key];
      const buf: LaunchBuffer = {
        ...(prev ?? emptyBuffer(key)),
        instance: meta?.instance ?? prev?.instance ?? '',
        account: meta?.account ?? prev?.account ?? '',
        lines: [],
        cursor: 0,
        running: true,
      };
      return project({ ...s, activeKey: key, buffers: prune({ ...s.buffers, [key]: buf }, key) });
    });
    try {
      const chunk = await getLaunchLog(key);
      update((s) => {
        const prev = s.buffers[key];
        if (!prev) return s;
        const buf: LaunchBuffer = {
          ...prev,
          lines: chunk.lines.slice(-MAX_LOG_LINES),
          cursor: chunk.cursor,
          running: chunk.running,
        };
        return project({ ...s, buffers: { ...s.buffers, [key]: buf } });
      });
    } catch {
      // SSE stream will populate the buffer
    }
  };

  return { subscribe, update, append, appendFor, markExited, clear, start, setActive };
}

export const launchLog = createLaunchLogStore();

/* ---------- live launches: GET /api/launches every 15 s (Contract C.2) ----------
   Feeds the "running games" indicator and the per-game log selector. The
   engine list is authoritative (it survives UI reloads); SSE launch-exit
   triggers an immediate refresh so the badge clears without waiting a tick. */

const LAUNCHES_POLL_MS = 15_000;

export const liveLaunches = createPollingStore<LiveLaunch[]>([], getLaunches, { pollMs: LAUNCHES_POLL_MS, pauseWhenRunning: true });
liveLaunches.start();

subscribeEvents((ev) => {
  if (ev.type === 'launch-log') {
    const d = ev.data as {
      key?: string;
      line?: string;
      lines?: string[];
      cursor?: number;
      running?: boolean;
    } | null;
    if (!d || typeof d.key !== 'string') return;
    const incoming = Array.isArray(d.lines)
      ? d.lines
      : typeof d.line === 'string'
        ? [d.line]
        : [];
    launchLog.appendFor(d.key, incoming, d.cursor, d.running);
  } else if (ev.type === 'launch-exit') {
    const d = ev.data as { key?: string; instance?: string; account?: string } | null;
    if (!d || typeof d.key !== 'string') return;
    launchLog.markExited(d.key, { instance: d.instance, account: d.account });
    // A game just closed — refresh the authoritative live list immediately so
    // the running-games badge and the per-game selector update without
    // waiting for the next 15 s poll, and refresh instances for running state.
    void liveLaunches.refresh();
    void instances.refresh();
  } else if (ev.type === 'import-done') {
    // A modpack import finished (from any entry point — Importar card,
    // Instancias modal, or a double-clicked .mrpack). Keep the instance
    // list fresh so the new instance shows up without a manual reload, and
    // surface the outcome as a toast.
    const d = ev.data as {
      instance?: string;
      ok?: boolean;
      error?: string;
      already_exists?: boolean;
    } | null;
    const name = d?.instance ?? '';
    if (d?.ok && !d.already_exists) {
      pushToast({
        kind: 'ok',
        text: t('toast.mrpackInstalled', { name }),
        href: `#/instances/${encodeURIComponent(name)}`,
      });
    } else if (d?.ok && d.already_exists) {
      pushToast({ kind: 'info', text: t('toast.mrpackAlready', { name }) });
    } else if (d && !d.ok) {
      pushToast({ kind: 'err', text: t('toast.mrpackError', { error: d.error ?? '' }) });
    }
  } else if (ev.type === 'train-done') {
    // Training finished → the AOT cache may now exist. Refresh the instances
    // list so the per-instance cache-ready badge updates everywhere (the list
    // isn't polled; without this the badge would stay stale after a train).
    void instances.refresh();
  }
});
