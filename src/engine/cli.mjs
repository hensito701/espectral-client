/**
 * EspectralClient engine — CLI entry (thin bootstrap).
 *
 * The bootstrap lives HERE, not in server.mjs, so importing server.mjs never
 * starts the server (that side effect created an ESM circular-import deadlock:
 * server -> register -> misc -> accounts -> server while server.mjs was still
 * mid-evaluation). `node src/engine/cli.mjs [--open]`.
 */
import { start, DEFAULT_PORT } from './server.mjs';
import { prewarmMsaToken } from './msauth.mjs';
import { spawn } from 'node:child_process';
import { ensureFirstLaunch } from './onboarding.mjs';
import * as discord from './discord.mjs';
import { loadConfig } from './config.mjs';

function openBrowser(url) {
  let cmd, args;
  if (process.platform === 'win32') {
    cmd = 'cmd';
    args = ['/c', 'start', '', url];
  } else if (process.platform === 'darwin') {
    cmd = 'open';
    args = [url];
  } else {
    cmd = 'xdg-open';
    args = [url];
  }
  const child = spawn(cmd, args, { stdio: 'ignore', detached: true, windowsHide: true });
  child.on('error', () => {
    /* browser missing — ignore */
  });
  child.unref();
}
const port = Number(process.env.ESPECTRAL_PORT) || DEFAULT_PORT;
const wantOpen = process.argv.includes('--open');
await ensureFirstLaunch();
const server = await start(port);
void prewarmMsaToken().catch(() => {});


// Parent watchdog: die with the app that spawned us. When the Tauri shell is
// force-killed (taskkill, Stop-Process), no WindowEvent fires and the engine
// would be orphaned holding port 4199 — the next launch then talks to a zombie
// serving stale code and stale config. Polling ppid liveness closes that hole;
// the interval is unref'd so it never keeps the process alive on its own.
const ppid = process.ppid;
if (ppid > 0) {
  setInterval(() => {
    try {
      process.kill(ppid, 0);
    } catch {
      process.exit(0);
    }
  }, 2000).unref();
}

process.once('exit', () => discord.shutdown());

if (loadConfig().discord_enabled !== false) {
  void discord
    .setPresence({ state: 'En el launcher', image: 'espectral_logo', startTimestamp: Date.now() })
    .catch(() => {
      /* Discord may not be running; game launch will retry the connection. */
    });
}

if (wantOpen) openBrowser(`http://127.0.0.1:${port}`);
