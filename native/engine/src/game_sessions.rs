//! Port of `src/engine/game-sessions.mjs` — `<data>/running-games.json`:
//! every spawned (detached) game is recorded so a fresh engine can re-adopt
//! games still running from an earlier launcher session.
//!
//! **Owner: game-launch.** Status: STUB.
//!
//! JS exports to port: sessionsPath readSessions recordSession removeSession
//! isPidAlive liveSessions. Tests: tests/game-sessions.test.mjs.
//! Use crate::platform::is_pid_alive (foundation) and types::GameSession.
//! The file is removed when the last session goes (JS writeSessions).

use indexmap::IndexMap;

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::GameSession;

/// Sessions keyed by instance name ({} when none / unreadable).
pub fn read_sessions(engine: &Engine) -> IndexMap<String, GameSession> {
    std::fs::read_to_string(engine.paths().running_games())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Live recorded sessions (dead pids pruned).
pub async fn live_sessions(engine: &Engine) -> EngineResult<Vec<(String, GameSession)>> {
    let _ = engine;
    crate::not_ported!("game_sessions::live_sessions")
}
