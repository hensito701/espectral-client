/**
 * Running games that must outlive the launcher.
 *
 * The game is spawned detached (launch.mjs), so closing the launcher no longer
 * kills it — players on low-RAM PCs close the ~500 MB launcher while they
 * play. A fresh engine then has to know the game is still running: otherwise
 * Play would start a second game in the same game dir and Stop would do
 * nothing. Every spawned game is recorded in <dataDir>/running-games.json and
 * removed when it exits; a new engine re-adopts the records whose process is
 * still that game.
 *
 * Identity check: a stale record's pid can be reused by an unrelated process
 * after a crash or reboot, and adopting it would block Play and point Stop at
 * a stranger. A record is only adopted when the live process's command line
 * contains the game dir it was launched with (`--gameDir <dir>`).
 */
import fs from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { dataDir } from './config.mjs';

const execFileAsync = promisify(execFile);

export function sessionsPath() {
  return path.join(dataDir(), 'running-games.json');
}

/** Recorded sessions keyed by instance name ({} when none / unreadable). */
export function readSessions() {
  try {
    const raw = JSON.parse(fs.readFileSync(sessionsPath(), 'utf8'));
    return raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {};
  } catch {
    return {};
  }
}

function writeSessions(sessions) {
  try {
    const file = sessionsPath();
    if (Object.keys(sessions).length === 0) {
      fs.rmSync(file, { force: true });
      return;
    }
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const tmp = `${file}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(sessions, null, 2), 'utf8');
    fs.renameSync(tmp, file);
  } catch (e) {
    console.warn('[sessions] could not persist running games:', e.message);
  }
}

/**
 * Record (or update) a running game. Fields: key, pid, version, account,
 * uuid, game_dir, started_at, menu_at, bridge_token.
 */
export function recordSession(instance, fields) {
  const sessions = readSessions();
  sessions[instance] = { ...(sessions[instance] ?? {}), ...fields };
  writeSessions(sessions);
}

export function removeSession(instance) {
  const sessions = readSessions();
  if (!(instance in sessions)) return;
  delete sessions[instance];
  writeSessions(sessions);
}

/** True when the OS still holds this pid (signal-0 probe; EPERM counts as alive). */
export function isPidAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err?.code === 'EPERM';
  }
}

/** pid -> command line for the given pids (missing = not running / unreadable). */
async function commandLines(pids) {
  const out = new Map();
  if (pids.length === 0) return out;
  if (process.platform === 'win32') {
    const filter = pids.map((p) => `ProcessId=${p}`).join(' OR ');
    try {
      const { stdout } = await execFileAsync(
        'powershell',
        ['-NoProfile', '-NonInteractive', '-Command',
          `Get-CimInstance Win32_Process -Filter "${filter}" | ForEach-Object { "$($_.ProcessId)\`t$($_.CommandLine)" }`],
        { windowsHide: true, timeout: 15_000, maxBuffer: 4 * 1024 * 1024 },
      );
      for (const line of stdout.split(/\r?\n/)) {
        const tab = line.indexOf('\t');
        if (tab > 0) out.set(Number(line.slice(0, tab)), line.slice(tab + 1));
      }
    } catch {
      /* unknown -> nothing adopted */
    }
    return out;
  }
  for (const pid of pids) {
    try {
      out.set(pid, fs.readFileSync(`/proc/${pid}/cmdline`, 'utf8').split('\0').join(' '));
    } catch {
      /* gone */
    }
  }
  return out;
}

/**
 * Sessions whose game is verifiably still running; every other record is
 * dropped from the file. Returns [{ instance, ...fields }].
 */
export async function liveSessions() {
  const sessions = readSessions();
  const names = Object.keys(sessions);
  if (names.length === 0) return [];
  const pids = names.map((n) => sessions[n].pid).filter((p) => isPidAlive(p));
  const lines = await commandLines(pids);
  const live = [];
  const kept = {};
  for (const name of names) {
    const s = sessions[name];
    const cmd = lines.get(s.pid);
    if (cmd && typeof s.game_dir === 'string' && s.game_dir && cmd.includes(s.game_dir)) {
      live.push({ instance: name, ...s });
      kept[name] = s;
    }
  }
  writeSessions(kept);
  return live;
}
