/**
 * Discord Rich Presence via the local RPC pipe.
 *
 * Protocol (frame format):
 *   [ 4 bytes opcode LE ][ 4 bytes length LE ][ JSON payload ]
 *   opcode 0 = HANDSHAKE, 1 = FRAME, 2 = CLOSE, 3 = PING, 4 = PONG
 *   (opcode is a full u32 BEFORE the length; any other layout is rejected
 *   by Discord with protocol error 1003 and the pipe is closed — verified
 *   against the live client.)
 *
 * Windows: named pipe \\.\pipe\discord-ipc-0 (per-instance suffixes -1..-9).
 * macOS/Linux: unix socket $XDG_RUNTIME_DIR/discord-ipc-0 (Linux uses abstract
 * namespace; Node can address it as '@discord-ipc-0' — WSL has no such
 * socket, so presence there is a no-op and we never log noise).
 *
 * This module is best-effort by design: any failure (pipe missing, malformed
 * reply) degrades to a quiet no-op — presence is a cosmetic feature and must
 * never disturb the launcher. No external deps (node:net + node:fs only).
 */
import net from 'node:net';
import fs from 'node:fs';

const OP = { HANDSHAKE: 0, FRAME: 1, CLOSE: 2, PING: 3, PONG: 4 };

const CONNECT_TIMEOUT_MS = 1500;
const FRAME_TIMEOUT_MS = 2000;

/**
 * Application ID on Discord's developer portal (espectral.es launcher).
 * Created at https://discord.com/developers/applications (Rich Presence →
 * Register an application). Rich Presence Art Assets keys used by the
 * launcher, uploaded with EXACTLY these names:
 *   `espectral_logo` — launcher open / singleplayer (generic logo)
 *   `uhc`, `24h`     — Espectral server logos (shown while on that server)
 */
export const DISCORD_CLIENT_ID =
  process.env.ESPECTRAL_DISCORD_CLIENT_ID ?? '1535246763856760874';

let socket = null;
let sequence = 0;
let pending = new Map(); // seq -> { resolve, timer }
let pingTimer = null;
let handshakeDone = false;
/** Buffer for frames split across TCP segments (READY is ~375 bytes). */
let frameBuffer = Buffer.alloc(0);
/** Resolver for the in-flight handshake; completed by the READY dispatch. */
let handshakeWaiter = null;

/** All Win32 spellings of the Discord RPC pipes (aliases of the same pipes). */
function pipeCandidates() {
  const out = [];
  for (let i = 0; i < 10; i++) {
    out.push(`\\\\?\\pipe\\discord-ipc-${i}`);
    out.push(`\\\\.\\pipe\\discord-ipc-${i}`);
  }
  return out;
}

function pickPipe() {
  if (process.platform === 'win32') {
    for (const p of pipeCandidates()) {
      try {
        fs.accessSync(p);
        return p;
      } catch {
        /* not present — try the next */
      }
    }
    return null;
  }
  const base = process.platform === 'darwin' ? process.env.XDG_RUNTIME_DIR ?? '' : process.env.XDG_RUNTIME_DIR ?? '';
  const candidates = base ? [`${base}/discord-ipc-0`, `${base}/discord-ipc-1`] : [];
  for (const p of candidates) {
    try {
      fs.accessSync(p);
      return p;
    } catch {
      /* skip */
    }
  }
  return null;
}

function encode(op, payload) {
  const body = Buffer.from(JSON.stringify(payload ?? {}), 'utf8');
  // Discord RPC frame header is [ opcode:u32 ][ length:u32 ][ payload ]
  // (verified against the live client — length-first is rejected with 1003).
  const frame = Buffer.alloc(4 + 4 + body.length);
  frame.writeUInt32LE(op, 0);
  frame.writeUInt32LE(body.length, 4);
  body.copy(frame, 8);
  return frame;
}

function send(op, payload) {
  if (!socket || !socket.writable) return Promise.resolve(null);
  return new Promise((resolve) => {
    const seq = sequence++;
    const timer = setTimeout(() => {
      pending.delete(seq);
      resolve(null);
    }, FRAME_TIMEOUT_MS);
    pending.set(seq, { resolve, timer });
    socket.write(encode(op, { ...payload, nonce: String(seq) }));
  });
}

/** Reconnect + re-handshake. Returns true when the connection is usable. */
async function ensureConnected() {
  if (socket && socket.writable && handshakeDone) return true;
  await connect();
  return !!(socket && socket.writable && handshakeDone);
}

function connect() {
  return new Promise((resolve) => {
    const pipe = pickPipe();
    if (!pipe) return resolve(null);
    let settled = false;
    const settle = (ok) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(ok ? socket : null);
    };
    let socket2;
    try {
      socket2 = net.createConnection(pipe);
      socket = socket2;
    } catch {
      socket = null;
      return resolve(null);
    }
    const onTimeout = () => {
      destroy('connect timeout');
      settle(null);
    };
    const timer = setTimeout(onTimeout, CONNECT_TIMEOUT_MS);
    socket2.setNoDelay(true);

    socket2.once('connect', async () => {
      const ok = await handshake();
      settle(ok);
    });
    socket2.on('error', () => {
      destroy('socket error');
      settle(null);
    });
    socket2.on('close', () => {
      if (handshakeDone) {
        // Discord went away — clear presence state so a reconnect re-sets it.
        handshakeDone = false;
        if (pingTimer) { clearInterval(pingTimer); pingTimer = null; }
      }
      settle(null);
    });
    socket2.on('data', onFrame);
  });
}

function handshake() {
  return new Promise((resolve) => {
    if (!socket) return resolve(false);
    // Resolved by onFrame when the READY dispatch arrives (opcode FRAME, not
    // HANDSHAKE — verified against the live client). The connect timeout
    // bounds the wait if Discord never answers.
    handshakeWaiter = resolve;
    socket.write(encode(OP.HANDSHAKE, {
      v: 1,
      client_id: DISCORD_CLIENT_ID,
    }));
  });
}

function parseFrame(buf) {
  if (buf.length < 8) return { op: -1, payload: null };
  const op = buf.readUInt32LE(0);
  const len = buf.readUInt32LE(4);
  let payload = null;
  try {
    payload = JSON.parse(buf.subarray(8, 8 + len).toString('utf8'));
  } catch { /* ignore */ }
  return { op, payload };
}

/** Dispatch one complete frame. */
function handleFrame(op, payload) {
  if (op === OP.PONG || op === OP.PING) return;
  if (payload?.cmd === 'DISPATCH' && payload?.evt === 'READY') {
    handshakeDone = true;
    // Discord requires a PING every ~30s or the connection is dropped.
    pingTimer = setInterval(() => {
      try {
        socket?.write(encode(OP.PING, {}));
      } catch { /* ignore */ }
    }, 15_000);
    if (pingTimer.unref) pingTimer.unref();
    const waiter = handshakeWaiter;
    handshakeWaiter = null;
    if (waiter) waiter(true);
    return;
  }
  const nonce = payload?.nonce ?? payload?.evt;
  if (nonce === undefined) return;
  const p = pending.get(nonce);
  if (!p) return;
  clearTimeout(p.timer);
  pending.delete(nonce);
  p.resolve(payload);
}

function onFrame(chunk) {
  frameBuffer = Buffer.concat([frameBuffer, chunk]);
  while (frameBuffer.length >= 8) {
    const { op, payload } = parseFrame(frameBuffer);
    if (op === -1) break; // incomplete header — wait for more bytes
    const len = frameBuffer.readUInt32LE(4);
    if (frameBuffer.length < 8 + len) break; // incomplete payload
    frameBuffer = frameBuffer.subarray(8 + len);
    handleFrame(op, payload);
  }
}

function destroy(reason) {
  if (pingTimer) { clearInterval(pingTimer); pingTimer = null; }
  for (const { timer } of pending.values()) clearTimeout(timer);
  pending.clear();
  handshakeDone = false;
  if (handshakeWaiter) {
    const waiter = handshakeWaiter;
    handshakeWaiter = null;
    waiter(false);
  }
  frameBuffer = Buffer.alloc(0);
  if (socket) {
    try { socket.removeAllListeners(); } catch { /* ignore */ }
    try { socket.destroy(); } catch { /* ignore */ }
  }
  socket = null;
}

/** Rate-limit the "Discord not running" warning so repeated calls stay quiet. */
let lastNoDiscordWarn = 0;

/**
 * Set the presence. `state`/`details` are the two visible text lines;
 * optional `startTimestamp` (ms epoch) renders an elapsed clock and a second
 * timestamp may be passed for "ends". `image`/`smallImage` are Art Asset keys
 * registered on the Discord application (e.g. `espectral_logo`, `uhc`, `24h`).
 *
 * Discord rejects the whole activity when a referenced asset key does not
 * exist, so when SET_ACTIVITY errors and images were requested, this retries
 * once without images — presence always shows, and the logo appears as soon
 * as the asset is uploaded (restart Discord after uploading new assets; the
 * client caches the asset list).
 */
export async function setPresence({ details, state, startTimestamp = null, endTimestamp = null, image = null, smallImage = null }) {
  if (!(await ensureConnected())) {
    if (Date.now() - lastNoDiscordWarn > 60_000) {
      lastNoDiscordWarn = Date.now();
      console.warn('[discord] Discord desktop client not detected — Rich Presence unavailable');
    }
    return { ok: false, error: 'no discord connection' };
  }

  const activity = { type: 0 }; // Playing
  if (details) activity.details = details;
  if (state) activity.state = state;
  const assets = {};
  if (image) assets.large_image = image;
  if (smallImage) assets.small_image = smallImage;
  if (Object.keys(assets).length > 0) {
    activity.assets = { ...assets, large_text: 'Espectral Client' };
  }
  if (startTimestamp) activity.timestamps = { start: startTimestamp };
  else if (endTimestamp) activity.timestamps = { end: endTimestamp };

  const isError = (reply) =>
    !!(reply && (reply.evt === 'ERROR' || (reply.data && typeof reply.data.code === 'number')));
  const doSet = (act) => send(OP.FRAME, { cmd: 'SET_ACTIVITY', args: { pid: process.pid, activity: act } });

  let reply = await doSet(activity);
  if (isError(reply)) {
    const { code = '?', message = 'unknown' } = reply.data ?? {};
    if (Object.keys(assets).length > 0) {
      console.warn(
        `[discord] SET_ACTIVITY rejected (${code}: ${message}) — retrying without images; ` +
          'upload the missing Rich Presence asset (exact key) and restart Discord',
      );
      activity.assets = undefined;
      reply = await doSet(activity);
      if (isError(reply)) {
        return { ok: false, error: `SET_ACTIVITY rejected (${reply.data?.code}: ${reply.data?.message})` };
      }
      return { ok: true, degraded: true };
    }
    return { ok: false, error: `SET_ACTIVITY rejected (${code}: ${message})` };
  }
  return { ok: true };
}

/** Clear presence ("En el launcher" reverts to the idle base state). */
export async function clearPresence() {
  if (!(await ensureConnected())) return;
  await send(OP.FRAME, {
    cmd: 'SET_ACTIVITY',
    args: { pid: process.pid, activity: null },
  });
}

/** Tear down the connection and timers (called on engine shutdown). */
export function shutdown() {
  destroy('shutdown');
}
