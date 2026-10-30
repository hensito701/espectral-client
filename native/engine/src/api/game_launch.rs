//! Facade — cluster **game-launch** (routes/launch.mjs + the version-change
//! routes of routes/instances.mjs, whose logic is version-change.mjs).
//!
//! | JS route                                        | method |
//! |-------------------------------------------------|--------|
//! | POST /api/instances/:name/launch                | [`Engine::launch_instance`] |
//! | POST /api/instances/:name/stop                  | [`Engine::stop_instance`] |
//! | POST /api/instances/:name/train                 | [`Engine::train_instance`] |
//! | GET  /api/instances/:name/aot                   | [`Engine::instance_aot_status`] |
//! | GET  /api/launch/:key/log?cursor=N              | [`Engine::launch_log`] |
//! | GET  /api/launches                              | [`Engine::list_launches`] |
//! | GET  /api/stats/launches?limit=N                | [`Engine::launch_stats`] (implemented) |
//! | POST /api/instances/:name/version-change/plan   | [`Engine::plan_version_change`] |
//! | POST /api/instances/:name/version-change        | [`Engine::apply_version_change`] |
//!
//! State to port into [`GameLaunchState`]: launch ring buffers (2000 lines,
//! 10-min TTL, LRU cap 8), liveLaunches, runningProcesses (child + pid +
//! single-flight finalizer), pendingStops. `activeInstances` is shared and
//! lives on the Engine (`Engine::active_instances`). Startup: adopt games
//! still running from a previous session (game_sessions::live_sessions).
//! Presence helpers from routes/launch.mjs (presenceForServer,
//! presenceEventFromLog, SERVER_STATE …) belong to this cluster too.

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{
    AotStatus, DryRunResult, LaunchHandle, LaunchReply, LaunchStat, LogChunk, VersionChangePlan, VersionChangeResult,
    VersionTarget,
};

/// game-launch in-memory state. Add fields freely — only this cluster owns it.
#[derive(Debug, Default)]
pub struct GameLaunchState {}

impl Engine {
    /// This cluster's in-memory state.
    #[allow(dead_code)]
    pub(crate) fn game_launch_state(&self) -> &GameLaunchState {
        &self.inner.game_launch
    }
}

/// Engine start hook: adopt running games (JS adoptRunningGames at route registration).
pub(crate) async fn on_engine_start(_engine: &Engine) -> EngineResult<()> {
    crate::not_ported!("game_launch::on_engine_start (adoptRunningGames)")
}

pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

/// POST /api/instances/:name/launch body.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LaunchRequest {
    #[serde(default)]
    pub dry_run: bool,
    /// Launch under this (non-active) account; 400 BAD_ACCOUNT when empty,
    /// 404 NOT_FOUND when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// Launch reply: a started launch or a dry-run preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LaunchOutcome {
    Started(LaunchReply),
    DryRun(DryRunResult),
}

/// POST /api/instances/:name/stop -> `{ ok, instance, cancelled?, already_stopped? }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopReply {
    pub ok: bool,
    pub instance: String,
    /// 'starting' when the stop landed during the prepare phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub already_stopped: Option<bool>,
}

/// POST /api/instances/:name/train -> `{ key, deferred? }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainReply {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred: Option<bool>,
}

/// GET /api/launches -> `{ launches }` (newest first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchesReply {
    pub launches: Vec<LaunchHandle>,
}

/// GET /api/stats/launches -> `{ launches }` (newest first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchStatsReply {
    pub launches: Vec<LaunchStat>,
}

impl Engine {
    /// POST /api/instances/:name/launch. Dry run -> DryRunResult. Otherwise
    /// 409 ALREADY_RUNNING when active; answers immediately with
    /// `{ key, pid: null, preparing: true, instance, version, account }` and
    /// resolves + spawns in the background (launch-log / launch-ready /
    /// launch-exit events, stats row, presence, deferred auto-train).
    pub async fn launch_instance(&self, name: &str, req: LaunchRequest) -> EngineResult<LaunchOutcome> {
        let _ = (name, req);
        crate::not_ported!("Engine::launch_instance")
    }

    /// POST /api/instances/:name/stop — idempotent force-kill (taskkill /T /F
    /// on Windows, SIGKILL elsewhere); a stop during prepare cancels the launch.
    pub async fn stop_instance(&self, name: &str) -> EngineResult<StopReply> {
        let _ = name;
        crate::not_ported!("Engine::stop_instance")
    }

    /// POST /api/instances/:name/train — 400 AOT_UNAVAILABLE on NeoForge;
    /// deferred while the game runs; else background train (train-* events).
    pub async fn train_instance(&self, name: &str) -> EngineResult<TrainReply> {
        let _ = name;
        crate::not_ported!("Engine::train_instance")
    }

    /// GET /api/instances/:name/aot -> AotStatus (instances::aot_status).
    pub async fn instance_aot_status(&self, name: &str) -> EngineResult<AotStatus> {
        let _ = name;
        crate::not_ported!("Engine::instance_aot_status")
    }

    /// GET /api/launch/:key/log?cursor=N -> `{ lines, cursor, running }`;
    /// 404 NOT_FOUND for an unknown key. Negative/invalid cursor -> 0.
    pub async fn launch_log(&self, key: &str, cursor: u64) -> EngineResult<LogChunk> {
        let _ = (key, cursor);
        crate::not_ported!("Engine::launch_log")
    }

    /// GET /api/launches — reconciles dead pids first.
    pub async fn list_launches(&self) -> EngineResult<LaunchesReply> {
        crate::not_ported!("Engine::list_launches")
    }

    /// GET /api/stats/launches?limit=N (default 10, clamped 1..=100).
    pub async fn launch_stats(&self, limit: Option<i64>) -> EngineResult<LaunchStatsReply> {
        let limit = match limit {
            Some(l) if l >= 1 => l.min(100) as usize,
            _ => 10,
        };
        Ok(LaunchStatsReply { launches: crate::stats::read_launch_stats(self.paths(), limit) })
    }

    /// POST /api/instances/:name/version-change/plan `{ version?, loader? }`.
    pub async fn plan_version_change(&self, name: &str, target: VersionTarget) -> EngineResult<VersionChangePlan> {
        let _ = (name, target);
        crate::not_ported!("Engine::plan_version_change")
    }

    /// POST /api/instances/:name/version-change — 409 ALREADY_RUNNING while
    /// active; marks the instance active while applying.
    pub async fn apply_version_change(&self, name: &str, target: VersionTarget) -> EngineResult<VersionChangeResult> {
        let _ = (name, target);
        crate::not_ported!("Engine::apply_version_change")
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::temp_engine;
    use crate::types::LaunchStat;

    #[tokio::test]
    async fn launch_stats_clamps_limit() {
        let (_d, e) = temp_engine();
        for i in 0..120 {
            crate::stats::record_launch_stat(e.paths(), &LaunchStat { key: format!("k{i}"), ..Default::default() });
        }
        assert_eq!(e.launch_stats(None).await.unwrap().launches.len(), 10);
        assert_eq!(e.launch_stats(Some(0)).await.unwrap().launches.len(), 10);
        assert_eq!(e.launch_stats(Some(500)).await.unwrap().launches.len(), 100);
        assert_eq!(e.launch_stats(Some(3)).await.unwrap().launches[0].key, "k119");
    }
}
