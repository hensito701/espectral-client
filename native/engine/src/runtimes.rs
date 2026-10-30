//! Port of `src/engine/runtimes.mjs` — Java tier selection + Temurin
//! provisioning (`<data>/runtimes/jdk-<major>/`, `info.json`).
//!
//! **Owner: game-resolve.** Status: STUB.
//!
//! JS exports to port: SUPPORTED_MAJORS javaMajorForVersionId runtimeJavaPath
//! fastClientJava pathJava cachedMajor cacheMajor downloadTemurin maxMajorFor
//! javaForVersion javaForMajorExact. Tests: tests/runtimes.test.mjs.
//! Archives: zip (Windows) via the `zip` crate, tar.gz (Linux) via `tar` +
//! `flate2`. Paths: Paths::runtime_dir / runtime_info / runtime_java.
//! Probe override for tests (JS __setProbeOverride) -> put a hook in
//! GameResolveState (api/game_resolve.rs).
//!
//! CONTRACT (called by game-launch): java_for_version, java_for_major_exact.

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::JavaRuntime;

/// Tiers, newest first (JS runtimes.SUPPORTED_MAJORS).
pub const SUPPORTED_MAJORS: [u32; 5] = [25, 21, 17, 16, 8];

/// JDK whose major is in [need, maxMajorFor(need)] (override -> config.jvm ->
/// FastClient -> PATH -> downloaded -> Adoptium). Error message `NO_JDK_<major>…`.
pub async fn java_for_version(engine: &Engine, major: u32) -> EngineResult<JavaRuntime> {
    let _ = (engine, major);
    crate::not_ported!("runtimes::java_for_version")
}

/// JDK with EXACTLY `major` (NeoForge needs 21; never upgrades).
pub async fn java_for_major_exact(engine: &Engine, major: u32) -> EngineResult<JavaRuntime> {
    let _ = (engine, major);
    crate::not_ported!("runtimes::java_for_major_exact")
}
