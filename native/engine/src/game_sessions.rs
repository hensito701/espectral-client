//! Port of `src/engine/game-sessions.mjs` — `<data>/running-games.json`:
//! every spawned (detached) game is recorded so a fresh engine can re-adopt
//! games still running from an earlier launcher session.
//!
//! **Owner: game-launch.** Status: PORTED.
//!
//! Identity check: a stale record's pid can be reused by an unrelated
//! process after a crash or reboot. A record is only adopted when the live
//! process's command line contains the game dir it was launched with
//! (Linux: `/proc/<pid>/cmdline`; Windows: `Win32_Process.CommandLine` via a
//! hidden PowerShell `Get-CimInstance`, 15 s timeout — same as the JS).
//!
//! Writes are read-modify-write of the raw JSON (unknown keys survive) under
//! a per-engine lock; the file is removed when the last session goes.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::GameSession;

/// `<data>/running-games.json`.
pub fn sessions_path(engine: &Engine) -> std::path::PathBuf {
    engine.paths().running_games()
}

/// Sessions keyed by instance name ({} when none / unreadable).
pub fn read_sessions(engine: &Engine) -> IndexMap<String, GameSession> {
    read_raw(engine)
        .into_iter()
        .filter_map(|(k, v)| serde_json::from_value::<GameSession>(v).ok().map(|s| (k, s)))
        .collect()
}

fn read_raw(engine: &Engine) -> Map<String, Value> {
    std::fs::read_to_string(sessions_path(engine))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

fn write_raw(engine: &Engine, sessions: &Map<String, Value>) {
    let file = sessions_path(engine);
    let r = if sessions.is_empty() {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        crate::paths::write_json_atomic(&file, sessions)
    };
    if let Err(e) = r {
        tracing::warn!("[sessions] could not persist running games: {e}");
    }
}

fn lock(engine: &Engine) -> std::sync::MutexGuard<'_, ()> {
    engine.game_launch_state().sessions_lock.lock().unwrap_or_else(|p| p.into_inner())
}

/// Record (or update) a running game: `fields` (an object: key, pid,
/// version, account, uuid, game_dir, started_at, menu_at, bridge_token) is
/// merged over the existing record.
pub fn record_session(engine: &Engine, instance: &str, fields: Value) {
    let _g = lock(engine);
    let mut sessions = read_raw(engine);
    let entry = sessions.entry(instance.to_string()).or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let (Value::Object(cur), Value::Object(f)) = (entry, fields) {
        for (k, v) in f {
            cur.insert(k, v);
        }
    }
    write_raw(engine, &sessions);
}

pub fn remove_session(engine: &Engine, instance: &str) {
    let _g = lock(engine);
    let mut sessions = read_raw(engine);
    if sessions.shift_remove(instance).is_none() {
        return;
    }
    write_raw(engine, &sessions);
}

/// True when the OS still holds this pid.
pub fn is_pid_alive(pid: u32) -> bool {
    crate::platform::is_pid_alive(pid)
}

/// pid -> command line for the given pids (missing = not running / unreadable).
pub async fn command_lines(pids: &[u32]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    if pids.is_empty() {
        return out;
    }
    if cfg!(windows) {
        let filter = pids.iter().map(|p| format!("ProcessId={p}")).collect::<Vec<_>>().join(" OR ");
        let script = format!(
            "Get-CimInstance Win32_Process -Filter \"{filter}\" | ForEach-Object {{ \"$($_.ProcessId)`t$($_.CommandLine)\" }}"
        );
        let mut cmd = crate::launch::process::hidden_command("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]).stdout(std::process::Stdio::piped());
        if let Ok(Ok(o)) = tokio::time::timeout(std::time::Duration::from_secs(15), cmd.output()).await {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Some(tab) = line.find('\t').filter(|t| *t > 0) {
                    if let Ok(pid) = line[..tab].trim().parse::<u32>() {
                        out.insert(pid, line[tab + 1..].to_string());
                    }
                }
            }
        }
        return out;
    }
    for &pid in pids {
        if let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) {
            out.insert(pid, String::from_utf8_lossy(&raw).split('\0').collect::<Vec<_>>().join(" "));
        }
    }
    out
}

/// Sessions whose game is verifiably still running; every other record is
/// dropped from the file.
pub async fn live_sessions(engine: &Engine) -> EngineResult<Vec<(String, GameSession)>> {
    let sessions = read_sessions(engine);
    if sessions.is_empty() {
        // Unreadable / empty: JS returned [] without touching the file.
        return Ok(vec![]);
    }
    let pids: Vec<u32> = sessions.values().map(|s| s.pid).filter(|p| is_pid_alive(*p)).collect();
    let lines = command_lines(&pids).await;
    let mut live = Vec::new();
    {
        let _g = lock(engine);
        let raw = read_raw(engine);
        let mut kept = Map::new();
        for (name, s) in &sessions {
            let ok = match (lines.get(&s.pid), s.game_dir.as_deref()) {
                (Some(cmd), Some(dir)) if !dir.is_empty() => cmd.contains(dir),
                _ => false,
            };
            if ok {
                live.push((name.clone(), s.clone()));
                if let Some(v) = raw.get(name) {
                    kept.insert(name.clone(), v.clone());
                }
            }
        }
        write_raw(engine, &kept);
    }
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use serde_json::json;

    #[test]
    fn record_merge_remove_and_file_lifecycle() {
        let (_d, e) = temp_engine();
        record_session(&e, "a", json!({ "pid": 12, "game_dir": "/g/a", "custom": 1 }));
        record_session(&e, "a", json!({ "menu_at": 99 }));
        record_session(&e, "b", json!({ "pid": 13 }));
        let s = read_sessions(&e);
        assert_eq!(s["a"].pid, 12);
        assert_eq!(s["a"].menu_at, Some(99));
        assert_eq!(s["a"].extra.get("custom"), Some(&json!(1)));
        assert_eq!(s.keys().collect::<Vec<_>>(), ["a", "b"]);
        remove_session(&e, "a");
        remove_session(&e, "missing");
        assert_eq!(read_sessions(&e).len(), 1);
        remove_session(&e, "b");
        assert!(!sessions_path(&e).exists(), "file removed with the last session");
        std::fs::write(sessions_path(&e), "[1,2]").unwrap();
        assert!(read_sessions(&e).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn live_sessions_keeps_running_game_drops_stale_and_reused() {
        let (_d, e) = temp_engine();
        let dir = e.paths().instance_dir("live-check");
        std::fs::create_dir_all(&dir).unwrap();
        // `sh -c 'sleep 30; :' --gameDir <dir>`: the trailing `:` stops sh from
        // exec'ing sleep, so the cmdline keeps carrying the game dir.
        let mut game =
            std::process::Command::new("sh").args(["-c", "sleep 30; :", "--gameDir"]).arg(&dir).spawn().unwrap();
        let dir_s = dir.to_string_lossy().into_owned();
        record_session(&e, "live-check", json!({ "pid": game.id(), "game_dir": dir_s, "version": "26.3" }));
        record_session(
            &e,
            "reused",
            json!({ "pid": std::process::id(), "game_dir": e.paths().instance_dir("reused").to_string_lossy(), "version": "26.3" }),
        );
        record_session(&e, "dead", json!({ "pid": (1u32 << 22) + 12345, "game_dir": dir_s, "version": "26.3" }));
        let live = live_sessions(&e).await.unwrap();
        assert_eq!(live.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["live-check"]);
        assert_eq!(read_sessions(&e).keys().collect::<Vec<_>>(), ["live-check"]);
        let _ = game.kill();
        let _ = game.wait();
    }
}
