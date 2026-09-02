/**
 * Server-Sent Events registry for the engine.
 *
 * - `subscribe(res)`: register an open SSE response (headers already written by
 *   the caller). The connection is kept alive with a comment heartbeat.
 * - `emit(event, payload)`: push one event frame to every subscriber. Payloads
 *   that are not strings are JSON-serialized (single-line, so the SSE framing
 *   stays valid).
 * - `register(app)`: wires `GET /api/events` (text/event-stream + origin-echoing
 *   CORS consistent with server.mjs).
 *
 * Named events (contract): launch-log, launch-exit, mod-progress,
 * train-progress, train-done, import-progress, import-done. The event name
 * is carried in the `event:` line so an EventSource
 * `addEventListener('mod-progress', ...)` dispatches correctly.
 */
import { CORS_ORIGINS } from './server.mjs';

const clients = new Set();
let heartbeat = null;
const HEARTBEAT_MS = 25_000;

export const EVENT_NAMES = [
  'launch-log',
  'launch-exit',
  'mod-progress',
  'train-progress',
  'train-done',
  'import-progress',
  'import-done',
];

/** Register an open SSE response. The caller must have called res.writeHead(...). */
export function subscribe(res) {
  clients.add(res);
  // Tell the browser to reconnect in 3s if the connection drops.
  res.write('retry: 3000\n\n');
  res.on('close', () => {
    clients.delete(res);
    if (clients.size === 0 && heartbeat) {
      clearInterval(heartbeat);
      heartbeat = null;
    }
  });
  if (clients.size === 1 && !heartbeat) {
    heartbeat = setInterval(() => {
      for (const c of clients) {
        try {
          c.write(': ping\n\n');
        } catch {
          /* dead socket; removed on its 'close' event */
        }
      }
    }, HEARTBEAT_MS);
    if (heartbeat.unref) heartbeat.unref();
  }
}

/** Push an event to every subscriber. Payload may be any JSON-serializable value. */
export function emit(event, payload) {
  const data = typeof payload === 'string' ? payload : JSON.stringify(payload);
  const frame = `event: ${event}\ndata: ${data}\n\n`;
  for (const res of clients) {
    try {
      res.write(frame);
    } catch {
      /* dead socket; removed on its 'close' event */
    }
  }
}

/** Wire the SSE endpoint onto an app object (see server.mjs). */
export function register(app) {
  app.get('/api/events', (req, res) => {
    const headers = {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
    };
    // Same CORS rule as the REST routes (server.mjs): echo the request origin
    // when it is a known dev/Tauri origin; otherwise omit the header — same-origin
    // clients that load the UI from the engine itself don't need it.
    const reqOrigin = req.headers.origin ?? null;
    if (reqOrigin && CORS_ORIGINS.has(reqOrigin)) {
      headers['Access-Control-Allow-Origin'] = reqOrigin;
      headers['Vary'] = 'Origin';
    }
    res.writeHead(200, headers);
    subscribe(res);
  });
}
