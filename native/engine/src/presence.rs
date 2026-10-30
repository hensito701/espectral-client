//! Port of `src/engine/presence.mjs` — ties espectral-id PUT/DELETE
//! /v1/presence to running launches (republish every 240 s while sharing).
//!
//! **Owner: identity-discord.** Status: STUB.
//!
//! JS exports to port: REPUBLISH_MS launchStarted launchExited syncShare
//! republishNow _reset. Tests: tests/bridge.test.mjs (presence parts).
//!
//! CONTRACT (called by game-launch on spawn/exit/adopt, accounts-skins on
//! cosmetics share changes): launch_started, launch_exited, sync_share.

use std::path::PathBuf;

use crate::engine::Engine;
use crate::error::EngineResult;

pub const REPUBLISH_MS: u64 = 240_000;

/// A spawned game the bridge + presence publisher should know about.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPresence {
    pub instance: String,
    /// any uuid form
    pub uuid: Option<String>,
    pub game_dir: PathBuf,
    /// per-launch bridge token (also on the JVM command line)
    pub token: String,
}

pub fn launch_started(engine: &Engine, launch: LaunchPresence) -> EngineResult<()> {
    let _ = (engine, launch);
    crate::not_ported!("presence::launch_started")
}

/// `token`: only tear down the registration made with this token.
pub fn launch_exited(engine: &Engine, instance: &str, token: Option<&str>) -> EngineResult<()> {
    let _ = (engine, instance, token);
    crate::not_ported!("presence::launch_exited")
}

/// Start/stop the publisher to match the current share flag.
pub fn sync_share(engine: &Engine, flat_uuid: &str) -> EngineResult<()> {
    let _ = (engine, flat_uuid);
    crate::not_ported!("presence::sync_share")
}
