//! Port of `src/engine/version-change.mjs` — change an instance's game
//! version and/or loader: plan (per-mod keep/update/disable/idle/add, world
//! backup, downgrade flag) and apply.
//!
//! **Owner: game-launch.** Status: STUB.
//!
//! JS exports to port: planVersionChange applyVersionChange.
//! Tests: tests/version-change.test.mjs.
//! Uses content's mods planModsForTarget / applyModsPlan (crate::mods) and
//! game-resolve's fabric/neoforge lookups.

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{VersionChangePlan, VersionChangeResult, VersionTarget};

pub async fn plan_version_change(
    engine: &Engine,
    name: &str,
    target: &VersionTarget,
) -> EngineResult<VersionChangePlan> {
    let _ = (engine, name, target);
    crate::not_ported!("version_change::plan_version_change")
}

pub async fn apply_version_change(
    engine: &Engine,
    name: &str,
    target: &VersionTarget,
) -> EngineResult<VersionChangeResult> {
    let _ = (engine, name, target);
    crate::not_ported!("version_change::apply_version_change")
}
