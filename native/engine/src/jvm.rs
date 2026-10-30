//! Port of `src/engine/jvm.mjs` — JVM discovery / probing (`java
//! -XshowSettings:properties -version`), primary JVM cache in `config.jvm`.
//!
//! **Owner: game-resolve.** Status: STUB.
//!
//! JS exports to port: fastClientJava pathJava probeJava parseMajor
//! discoverJvm getJvmInfo downloadAdoptium jcmdPath (+ probe memo).
//! Tests: tests/jvm.test.mjs, tests/linux-compat.test.mjs.
//! Spawn the probe with `CREATE_NO_WINDOW` on Windows (JS windowsHide).
//!
//! CONTRACT (called by game-launch / api): get_jvm_info, probe_java, jcmd_path.

use std::path::{Path, PathBuf};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{JavaProbe, JavaRuntime};

/// Cached-if-possible primary JVM; with `override_path`, probe exactly that java.
pub async fn get_jvm_info(engine: &Engine, override_path: Option<&str>) -> EngineResult<JavaRuntime> {
    let _ = (engine, override_path);
    crate::not_ported!("jvm::get_jvm_info")
}

/// Probe one java binary (memoized by path+size+mtime).
pub async fn probe_java(engine: &Engine, java_path: &Path) -> EngineResult<JavaProbe> {
    let _ = (engine, java_path);
    crate::not_ported!("jvm::probe_java")
}

/// `jcmd` next to a java binary (AOT training uses it).
pub fn jcmd_path(java_path: &Path) -> PathBuf {
    let name = if cfg!(windows) { "jcmd.exe" } else { "jcmd" };
    java_path.parent().map(|p| p.join(name)).unwrap_or_else(|| PathBuf::from(name))
}
