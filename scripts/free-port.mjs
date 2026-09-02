#!/usr/bin/env node
// predev hook: free port 4199 before the dev engine starts. A leftover engine
// (desktop app or a previous dev run) silently serves STALE code, which looks
// exactly like "my change broke the API". Kill the listener first.
import { spawnSync } from 'node:child_process';

const PORT = process.env.ESPECTRAL_PORT || 4199;

function win32Pids(port) {
  const r = spawnSync('netstat', ['-ano'], { encoding: 'utf8', shell: true });
  if (r.status !== 0) return [];
  return (r.stdout || '')
    .split('\n')
    .filter((l) => l.includes(`:${port} `) && l.includes('LISTENING'))
    .map((l) => l.trim().split(/\s+/).pop())
    .filter((p) => /^\d+$/.test(p));
}

const pids = win32Pids(PORT);
if (!pids.length) process.exit(0);

console.warn(`[predev] port ${PORT} in use by pid(s) ${pids.join(', ')} — killing (stale engines serve old code)`);
for (const pid of pids) {
  spawnSync('taskkill', ['/PID', pid, '/T', '/F'], { shell: true, stdio: 'ignore' });
}
