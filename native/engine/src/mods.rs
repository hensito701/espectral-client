//! Port of `src/engine/mods.mjs` — mod listing / enable-disable, pinned
//! presets (performance / QoL / branding), Modrinth search + dependency
//! planning + install, version-change mod planning.
//!
//! **Owner: content.** Status: STUB.
//!
//! JS exports to port: MODRINTH_API PERFORMANCE_PRESET BRANDING_PRESET
//! QOL_PRESET PINNED_VERSIONS PINS_BY_VERSION PINS_QOL_BY_VERSION
//! qolPinsForVersion brandingVersions supportsBranding brandingPinForVersion
//! supportsPerformanceBundle pinsForVersion pinNoteForVersion listMods
//! setModEnabled installPreset ensureBrandingSeeded isFilenameForSlug
//! slugifyTitle installedModFilenames searchModrinth planModInstall
//! summarizePlan installModrinthMod planModsForTarget applyModsPlan.
//! Tests: tests/mods.test.mjs. Bundled branding jars live in the repo's
//! `assets/` (resolve relative to the install / CARGO_MANIFEST_DIR in dev).
//! Error codes from routes/mods.mjs are lowercase and VERBATIM: use
//! EngineError::api_verbatim.
//!
//! CONTRACT (called by onboarding, game-launch launch/version-change):
//! install_preset, ensure_branding_seeded, plan_mods_for_target, apply_mods_plan.

use serde_json::Value;

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{InstanceMeta, ModPreset, PresetInstallResult, VersionTarget};

pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";

/// Install a preset into the instance's effective mods dir (mod-progress events).
pub async fn install_preset(
    engine: &Engine,
    instance_name: &str,
    preset: ModPreset,
) -> EngineResult<PresetInstallResult> {
    let _ = (engine, instance_name, preset);
    crate::not_ported!("mods::install_preset")
}

/// Retro-seed the branding jar for instances that predate the preset (best-effort).
pub async fn ensure_branding_seeded(engine: &Engine, instance: &InstanceMeta) -> EngineResult<()> {
    let _ = (engine, instance);
    crate::not_ported!("mods::ensure_branding_seeded")
}

/// Per-jar plan for a version/loader change (JS planModsForTarget). JSON
/// shape: version-change.mjs consumes it; keep it a Value until both sides land.
pub async fn plan_mods_for_target(
    engine: &Engine,
    instance: &InstanceMeta,
    target: &VersionTarget,
) -> EngineResult<Value> {
    let _ = (engine, instance, target);
    crate::not_ported!("mods::plan_mods_for_target")
}

/// Apply a plan from plan_mods_for_target.
pub async fn apply_mods_plan(engine: &Engine, instance_name: &str, plan: &Value) -> EngineResult<Value> {
    let _ = (engine, instance_name, plan);
    crate::not_ported!("mods::apply_mods_plan")
}
