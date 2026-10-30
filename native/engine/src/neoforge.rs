//! Port of `src/engine/neoforge.mjs` — NeoForge loader: installer download,
//! profile extraction (crate::ziputil), official installer run
//! (`java -jar <installer> --installClient <data>`), library downloads.
//!
//! **Owner: game-resolve.** Status: STUB.
//!
//! JS exports to port: NEOFORGE_MAVEN installerUrl neoforgePrefixFor
//! latestNeoForgeFor neoforgeJsonPath mcAndNeoFormVersion clientArtifactsDir
//! neoforgeJarPath launchArtifactsReady readNeoForgeJson isNeoForgeInstalled
//! ensureNeoForgeLoader. Tests: tests/neoforge.test.mjs (fixtures:
//! fake-neoforge-installer.jar, neoforge-21.1.242-version.json,
//! vanilla-1.21.1-mini.json). Paths: Paths::neoforge_installer,
//! Paths::launcher_profiles_json, Paths::version_json("neoforge-<v>").
//!
//! CONTRACT (called by game-launch / content): neoforge_version_id,
//! ensure_neoforge_loader, launch_artifacts_ready, latest_neoforge_for.

use serde_json::Value;

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{InstanceMeta, VersionJson};

pub const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";

/// Version id used under `<data>/versions`: `neoforge-<v>`.
pub fn neoforge_version_id(v: &str) -> String {
    format!("neoforge-{v}")
}

/// Install (once) the NeoForge loader `v`; emits mod-progress kind 'neoforge-loader'.
/// Returns the JS `{ json, ...rest }` object.
pub async fn ensure_neoforge_loader(engine: &Engine, v: &str, instance: Option<&InstanceMeta>) -> EngineResult<Value> {
    let _ = (engine, v, instance);
    crate::not_ported!("neoforge::ensure_neoforge_loader")
}

/// True when the installer-generated launch artifacts exist for `v`.
pub fn launch_artifacts_ready(engine: &Engine, v: &str, profile: &VersionJson) -> EngineResult<bool> {
    let _ = (engine, v, profile);
    crate::not_ported!("neoforge::launch_artifacts_ready")
}

/// Newest NeoForge build for a Minecraft version, None when there is none.
pub async fn latest_neoforge_for(engine: &Engine, mc_version: &str) -> EngineResult<Option<String>> {
    let _ = (engine, mc_version);
    crate::not_ported!("neoforge::latest_neoforge_for")
}
