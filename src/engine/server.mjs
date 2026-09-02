/**
 * Engine HTTP server + routing core (node:http, no deps) — port 4199 on 127.0.0.1.
 */
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as events from './events.mjs';
import { ApiError, httpError, normalizeError } from './error.mjs';


const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = path.resolve(__dirname, '..', '..');
export const UI_DIST = path.join(REPO_ROOT, 'src', 'ui', 'dist');

/** Origins that may call the REST API cross-origin: Vite dev + Tauri webview. */
export const CORS_ORIGINS = new Set([
  'http://localhost:5173', // Vite dev server (hostname form)
  'http://127.0.0.1:5173', // Vite dev server (vite.config pins 127.0.0.1)
  'tauri://localhost', // Tauri v2 (Windows/Linux webview origin)
  'http://tauri.localhost', // Tauri v2 (macOS)
  'https://tauri.localhost',
]);

// Custom header required on every state-changing (non-GET) request (see the
// 403 guard in makeRequestHandler). The engine's own UI sets it — see
// src/ui/src/lib/api.ts. This is the primary CSRF defense; CORS headers are
// for the dev flow only.
export const CLIENT_HEADER = 'x-espectral-client';

// L18: cap request bodies (1 MiB). No endpoint legitimately exceeds this; the
// guard turns the unbounded-buffer DoS (a single big POST exhausting memory)
// into a clean 413.
export const MAX_BODY_BYTES = 1024 * 1024;

// H1 (DNS-rebinding completion): the only valid Hosts are 127.0.0.1 and
// localhost (with an optional port). An attacker's page served from its own
// domain after a rebind to 127.0.0.1 sends Host: <attacker-domain>:<port>,
// which this rejects — closing the same-origin rebinding vector the custom
// header guard (non-GET) can't reach.
const HOST_RE = /^(127\.0\.0\.1|localhost)(?::\d+)?$/;

export const DEFAULT_PORT = 4199;
// Single source of truth: package.json version (was a hardcoded '1.2.0' that
// drifted behind releases — /api/health reported 1.2.0 while shipping 1.3.0).
// Layouts differ: repo = <root>/src/engine/, bundle = <install>/engine/, so
// package.json sits a different number of levels up. Probe both; never throw
// at import time over a version string.
function readVersion() {
  for (const p of [path.join(__dirname, '..', '..', 'package.json'), path.join(__dirname, '..', 'package.json')]) {
    try {
      return JSON.parse(fs.readFileSync(p, 'utf8')).version;
    } catch { /* try next layout */ }
  }
  return '0.0.0-unknown';
}
export const VERSION = readVersion();

const BODY_METHODS = new Set(['POST', 'PUT', 'PATCH']);

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.ico': 'image/x-icon',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.ttf': 'font/ttf',
  '.map': 'application/json',
  '.txt': 'text/plain; charset=utf-8',
};


// ---------------------------------------------------------------------------
// App (route registry + response helper)
// ---------------------------------------------------------------------------

function compilePattern(routePath) {
  return {
    segs: routePath
      .split('/')
      .filter(Boolean)
      .map((s) => (s.startsWith(':') ? { param: s.slice(1) } : { literal: s })),
  };
}

export function createApp() {
  const app = {
    routes: [],

    get(p, h) { app.routes.push({ method: 'GET', pattern: compilePattern(p), handler: h }); return app; },
    post(p, h) { app.routes.push({ method: 'POST', pattern: compilePattern(p), handler: h }); return app; },
    put(p, h) { app.routes.push({ method: 'PUT', pattern: compilePattern(p), handler: h }); return app; },
    patch(p, h) { app.routes.push({ method: 'PATCH', pattern: compilePattern(p), handler: h }); return app; },
    delete(p, h) { app.routes.push({ method: 'DELETE', pattern: compilePattern(p), handler: h }); return app; },

    sendJson(res, status, obj) {
      const payload = obj === undefined ? '' : JSON.stringify(obj);
      res.writeHead(status, {
        'Content-Type': 'application/json; charset=utf-8',
        'Content-Length': Buffer.byteLength(payload),
      });
      res.end(payload);
    },
  };
  return app;
}

function matchRoute(route, method, pathname) {
  if (route.method !== method) return null;
  const segs = pathname.split('/').filter(Boolean);
  if (segs.length !== route.pattern.segs.length) return null;
  const params = {};
  for (let i = 0; i < segs.length; i++) {
    const pat = route.pattern.segs[i];
    if (pat.param) {
      let decoded;
      try {
        decoded = decodeURIComponent(segs[i]);
      } catch {
        return null; // malformed percent-encoding
      }
      params[pat.param] = decoded;
    } else if (pat.literal !== segs[i]) {
      return null;
    }
  }
  return params;
}

async function readJsonBody(req) {
  // Pre-check Content-Length so a large declared body is rejected before any
  // bytes are read (L18).
  const declared = Number(req.headers['content-length'] ?? 0);
  if (Number.isFinite(declared) && declared > MAX_BODY_BYTES) {
    throw httpError(413, 'PAYLOAD_TOO_LARGE', `request body exceeds ${MAX_BODY_BYTES} bytes`);
  }
  const chunks = [];
  let total = 0;
  for await (const chunk of req) {
    total += chunk.length;
    if (total > MAX_BODY_BYTES) {
      // Streaming over-cap (no/lying Content-Length): throw; node closes the
      // connection when a response is sent with the request body unconsumed,
      // so no keep-alive corruption and no need to destroy the socket.
      throw httpError(413, 'PAYLOAD_TOO_LARGE', `request body exceeds ${MAX_BODY_BYTES} bytes`);
    }
    chunks.push(chunk);
  }
  if (chunks.length === 0) return null;
  const text = Buffer.concat(chunks).toString('utf8');
  if (!text.trim()) return null;
  try {
    return JSON.parse(text);
  } catch {
    throw httpError(400, 'BAD_JSON', 'request body is not valid JSON');
  }
}


export function makeRequestHandler(app) {
  return async (req, res) => {
    let pathname;
    try {
      pathname = new URL(req.url, 'http://localhost').pathname;
    } catch {
      return app.sendJson(res, 400, { error: { code: 'BAD_URL', message: 'malformed request URL' } });
    }

    // H1 (DNS-rebinding completion): only accept requests whose Host is the
    // local loopback (127.0.0.1 or localhost, optional port). A page served
    // from its own domain after a rebind to 127.0.0.1 sends a foreign Host —
    // rejected here regardless of method. Requests without a Host header are
    // allowed (node always supplies it for HTTP/1.1; some raw clients omit it).
    const hostHeader = (req.headers.host ?? '').toLowerCase();
    if (hostHeader && !HOST_RE.test(hostHeader)) {
      res.writeHead(403, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: { code: 'FORBIDDEN', message: 'unexpected Host header' } }));
      return;
    }

    // CORS on every response; OPTIONS preflight short-circuits. Echo the
    // request origin when it is one of the known ones (dev Vite, Tauri
    // webview) so credentialed cross-origin calls work; otherwise no header.
    // Allow-Credentials is required for the Discord gate calls — they are the
    // only fetch()es with credentials:'include', and a browser rejects a
    // credentialed CORS response without it ("TypeError: Failed to fetch").
    const reqOrigin = req.headers.origin ?? null;
    if (reqOrigin && CORS_ORIGINS.has(reqOrigin)) {
      res.setHeader('Access-Control-Allow-Origin', reqOrigin);
      res.setHeader('Access-Control-Allow-Credentials', 'true');
      res.setHeader('Vary', 'Origin');
    }
    if (req.method === 'OPTIONS') {
      if (reqOrigin && CORS_ORIGINS.has(reqOrigin)) {
        res.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, PATCH, DELETE, OPTIONS');
        // Must include the engine's custom header (CLIENT_HEADER): the UI sets
        // it on EVERY request (see api.ts), and a browser refuses any request
        // carrying a header that the preflight did not allow. Missing it here
        // made every cross-origin call from the Tauri webview fail preflight
        // with "TypeError: Failed to fetch" (regression in the H1 hardening).
        res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization, x-espectral-client');
        res.setHeader('Access-Control-Max-Age', '86400');
      }
      res.writeHead(204);
      return res.end();
    }

    // H1 mitigation: reject state-changing (non-GET) requests that arrive
    // without the engine's custom header. A browser "simple" request (HTML
    // form, or fetch with Content-Type text/plain) CANNOT set custom headers
    // without a CORS preflight, and the preflight rejects because the origin
    // isn't allowlisted — so this blocks CSRF while keeping the Tauri webview
    // and the Vite dev page working (they set the header, see api.ts).
    if (req.method !== 'GET') {
      const h = req.headers['x-espectral-client'] ?? '';
      if (h !== '1') {
        res.writeHead(403, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { code: 'FORBIDDEN', message: 'missing x-espectral-client header' } }));
        return;
      }
    }
    try {
      for (const route of app.routes) {
        const params = matchRoute(route, req.method, pathname);
        if (!params) continue;
        const body = BODY_METHODS.has(req.method) ? await readJsonBody(req) : null;
        const result = await route.handler(req, res, params, body);
        if (result !== undefined && !res.headersSent) {
          app.sendJson(res, 200, result);
        } else if (!res.headersSent) {
          // Handler wrote nothing and returned undefined (e.g. SSE that
          // never produced headers) — don't leave the socket hanging.
          res.writeHead(204);
          res.end();
        }
        return;
      }
      // M11: unmatched /api/* paths must NOT fall through to the SPA fallback
      // (which would answer a typo'd API call with index.html + HTTP 200).
      // Guard serveStatic to non-API paths so unknown API GETs get a JSON 404.
      if (req.method === 'GET' && !pathname.startsWith('/api/') && serveStatic(req, res, pathname)) return;
      app.sendJson(res, 404, { error: { code: 'NOT_FOUND', message: `no route for ${req.method} ${pathname}` } });
    } catch (err) {
      const { status, code, message } = normalizeError(err);
      if (status >= 500) console.error(`[server] ${req.method} ${pathname} ->`, err);
      if (!res.headersSent) {
        app.sendJson(res, status, { error: { code, message } });
      } else {
        res.end();
      }
    }
  };
}

// ---------------------------------------------------------------------------
// Static UI serving (src/ui/dist when present)
// ---------------------------------------------------------------------------

function serveStatic(req, res, pathname) {
  if (!fs.existsSync(UI_DIST)) return false;
  const rel = pathname === '/' ? 'index.html' : pathname.replace(/^\/+/, '');
  const root = UI_DIST.endsWith(path.sep) ? UI_DIST : UI_DIST + path.sep;
  const file = path.normalize(path.join(UI_DIST, rel));
  // Traversal guard, sep-anchored: a bare-prefix startsWith would let a
  // sibling dir (/foo-bar) slip past a /foo check.
  if (!file.startsWith(root)) return false;
  let target = file;
  if (!fs.existsSync(target) || fs.statSync(target).isDirectory()) {
    target = path.join(UI_DIST, 'index.html'); // SPA fallback
  }
  if (!fs.existsSync(target)) return false;
  const ext = path.extname(target).toLowerCase();
  const base = path.basename(target);
  const isIndex = base === 'index.html';
  // Vite emits content-hashed bundle filenames (assets/*-<8char base36>.(js|css|woff2))
  // — those are safe to serve as immutable. index.html must revalidate so new
  // builds are picked up; everything else gets a short shared cache.
  const hashed = /-[A-Za-z0-9]{8,16}\.(?:js|css|woff2?|png|svg|jpg|webp)$/.test(base);
  const cacheControl = isIndex
    ? 'no-cache'
    : hashed
      ? 'public, max-age=31536000, immutable'
      : 'public, max-age=3600';
  res.writeHead(200, {
    'Content-Type': MIME[ext] || 'application/octet-stream',
    'Cache-Control': cacheControl,
  });
  fs.createReadStream(target).pipe(res);
  return true;
}

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

/** Create app, register events + route modules, and listen. */
export async function start(port = DEFAULT_PORT) {
  const app = createApp();
  events.register(app);
  const { register } = await import('./routes/register.mjs');
  await register(app);

  const server = http.createServer(makeRequestHandler(app));
  server.on('error', (err) => {
    console.error('[server] fatal:', err.message);
    process.exitCode = 1;
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', resolve);
  });
  console.log(`[server] EspectralClient engine v${VERSION} on http://127.0.0.1:${port}`);
  return server;
}

// Bootstrap lives in cli.mjs (NOT here): running the server from this module's
// top level deadlocked the ESM graph (server -> register -> misc -> accounts ->
// server while this module was still evaluating). Importing this module must
// never have side effects.

