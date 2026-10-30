//! Port of `src/engine/launch.mjs` — account resolution, JVM/game argument
//! construction, run-dir selection, `resolveLaunch`, dry-run, spawning the
//! game (detached, output piped, latest.log marker watch, boot phases).
//!
//! **Owner: game-launch.** Status: STUB.
//!
//! JS exports to port: INITIAL_HEAP_MB MARKER_RE FIXED_JVM_ARGS memoryArgs
//! defaultAccount pickAccount resolveAccountFor resolveAccount expandJvmArgs
//! expandGameArgs substitutePlaceholders bootFlags effectiveMemoryMb buildArgv
//! applyLaunchPreferences selectGameDir javaForLaunch resolveLaunch
//! dryRunResult libraryStats spawnJava launchInstance formatCommandPreview.
//! Tests: tests/launch.test.mjs, preferences.test.mjs, multi-instance.test.mjs,
//! presence.test.mjs, personalization.test.mjs (launch parts).
//!
//! Notes:
//! - The game must outlive the launcher: spawn detached (Windows:
//!   DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW; unix:
//!   setsid via `CommandExt::process_group(0)`), stdout/stderr piped.
//! - `-Despectral.bridge.url=http://127.0.0.1:<port>/api/bridge` — the bridge
//!   listener port comes from identity-discord's loopback (crate::loopback).
//! - Fingerprint fast path: crate::fingerprint (foundation, already ported).
//! - Uses resolver / runtimes / jvm / neoforge (game-resolve), instances /
//!   mods (content), msauth / accounts / cosmetics / backgrounds
//!   (accounts-skins), identity / client / bridge (identity-discord) contract fns.
//!
//! CONTRACT (called by aot.rs and api/game_launch.rs — same cluster; listed
//! so the shapes in crate::types stay the reference): resolve_launch, dry_run_result.

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{Account, DryRunResult, InstanceMeta, LaunchAccount, LaunchMode, ResolvedLaunch};

/// `Sound engine started` — the main-menu marker.
pub const MARKER: &str = "Sound engine started";
pub const INITIAL_HEAP_MB: u32 = 512;

/// Progress / phase callback (`onProgress(line)`, `onPhase(name)`).
pub type LineFn = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// resolveLaunch() options.
#[derive(Clone, Default)]
pub struct ResolveLaunchOptions {
    pub mode: LaunchMode,
    /// Already-resolved identity (route passes resolveAccountFor's result) or a stored account.
    pub account: Option<LaunchAccountInput>,
    pub dry_run: bool,
    pub force_verify: bool,
    pub on_progress: Option<LineFn>,
    pub on_phase: Option<LineFn>,
}

/// Account given to resolveLaunch: a stored account (token resolved inside)
/// or an already-resolved launch identity.
#[derive(Debug, Clone)]
pub enum LaunchAccountInput {
    Stored(Box<Account>),
    Resolved(LaunchAccount),
}

/// Resolve everything a launch needs (see types::ResolvedLaunch).
pub async fn resolve_launch(
    engine: &Engine,
    instance: &InstanceMeta,
    opts: ResolveLaunchOptions,
) -> EngineResult<ResolvedLaunch> {
    let _ = (engine, instance, opts);
    crate::not_ported!("launch::resolve_launch")
}

/// DryRunResult for a resolved launch (token redacted; never spawns).
pub fn dry_run_result(
    engine: &Engine,
    instance: &InstanceMeta,
    resolved: &ResolvedLaunch,
) -> EngineResult<DryRunResult> {
    let _ = (engine, instance, resolved);
    crate::not_ported!("launch::dry_run_result")
}
