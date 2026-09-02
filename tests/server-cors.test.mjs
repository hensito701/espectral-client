import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';

// Sandbox the data dir before importing so module evaluation and the route
// modules (which read config at import/boot time) never touch real data.
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-cors-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const { start } = await import('../src/engine/server.mjs');

let server;
let base;

test.before(async () => {
  server = await start(0); // ephemeral port
  base = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  if (server) await new Promise((r) => server.close(r));
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const TAURI_ORIGIN = 'http://tauri.localhost'; // packaged webview origin
const VITE_ORIGIN = 'http://localhost:5173'; // dev server origin

test('preflight allows the engine client header (x-espectral-client)', async () => {
  // The UI sets x-espectral-client on EVERY request (see src/ui/src/lib/api.ts).
  // A browser refuses any request whose headers were not granted by the
  // preflight — if this header is missing from Access-Control-Allow-Headers,
  // every cross-origin call from the Tauri webview fails with
  // "TypeError: Failed to fetch" (regression shipped with the H1 hardening).
  const res = await fetch(`${base}/api/health`, {
    method: 'OPTIONS',
    headers: {
      Origin: TAURI_ORIGIN,
      'Access-Control-Request-Method': 'GET',
      'Access-Control-Request-Headers': 'x-espectral-client',
    },
  });
  assert.equal(res.status, 204);
  const allowed = res.headers.get('access-control-allow-headers') ?? '';
  const granted = allowed.split(',').map((h) => h.trim().toLowerCase());
  assert.ok(
    granted.includes('x-espectral-client'),
    `Access-Control-Allow-Headers must include x-espectral-client (got: "${allowed}")`,
  );
  assert.equal(res.headers.get('access-control-allow-origin'), TAURI_ORIGIN);
});

test('actual cross-origin request with the client header succeeds', async () => {
  const res = await fetch(`${base}/api/health`, {
    headers: {
      Origin: TAURI_ORIGIN,
      'x-espectral-client': '1',
    },
  });
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('access-control-allow-origin'), TAURI_ORIGIN);
  const body = await res.json();
  assert.equal(body.ok, true);
});

test('non-allowlisted origins get no CORS grant (CSRF posture intact)', async () => {
  // The H1 hardening rejects foreign origins. The preflight must not grant
  // the custom header to them (no Allow-Headers), and any ACAO echoed must
  // never match the request origin — so the browser blocks the response.
  const res = await fetch(`${base}/api/health`, {
    method: 'OPTIONS',
    headers: {
      Origin: 'https://evil.example',
      'Access-Control-Request-Method': 'GET',
      'Access-Control-Request-Headers': 'x-espectral-client',
    },
  });
  assert.equal(res.status, 204);
  assert.equal(res.headers.get('access-control-allow-headers'), null);
  const acao = res.headers.get('access-control-allow-origin');
  assert.notEqual(acao, 'https://evil.example');
});
