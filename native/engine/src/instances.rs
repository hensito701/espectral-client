//! Port of `src/engine/instances.mjs` — instance CRUD (instance.json),
//! summaries, mod-set hash, icons, servers.dat / options.txt access, AOT
//! status read side, background seeding at creation.
//!
//! **Owner: content.** Status: STUB (path helpers implemented).
//!
//! JS exports to port: isValidHue hueDistance pickDistinctHue normalizeGameDir
//! getInstance (+ one-shot 3072/4096 memory migration, see
//! config::LEGACY_DEFAULT_MEMORY_MB) listInstances computeModSetHash
//! getSummary getInstanceDetail createInstance deleteInstance (trash rename
//! `<name>.deleted.<ms>`) patchInstance readInstanceIcon writeInstanceIcon
//! removeInstanceIcon readServersDat writeServers parseOptionsTxt readOptions
//! importOptionsFromInstance aotStatus.
//! Tests: tests/multi-instance.test.mjs, personalization.test.mjs (instances
//! parts), backgrounds.test.mjs (instance parts).
//!
//! Already in the foundation: name / trash regexes (crate::util),
//! servers.dat codec (crate::nbt), Paths::instance_dir / instance_json /
//! instance_icon, atomic JSON writes (paths::write_json_atomic), aot::cache_key.
//!
//! CONTRACT (called by game-launch, onboarding, accounts-skins, identity-discord):
//! get_instance, list_instances, create_instance, effective_game_dir,
//! effective_mods_dir, aot_status, compute_mod_set_hash.

use std::path::PathBuf;

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{AotStatus, CreateInstanceRequest, InstanceMeta, InstanceSummary};

/// Effective game dir: the custom absolute `game_dir` when set, else
/// `<data>/instances/<name>` (instances.effectiveGameDir).
pub fn effective_game_dir(engine: &Engine, instance: &InstanceMeta) -> PathBuf {
    match instance.game_dir.as_deref() {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => engine.paths().instance_dir(&instance.name),
    }
}

/// effectiveGameDir(name): reads instance.json best-effort.
pub fn effective_game_dir_by_name(engine: &Engine, name: &str) -> PathBuf {
    std::fs::read_to_string(engine.paths().instance_json(name))
        .ok()
        .and_then(|s| serde_json::from_str::<InstanceMeta>(&s).ok())
        .and_then(|m| m.game_dir.filter(|d| !d.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| engine.paths().instance_dir(name))
}

/// `<effective game dir>/mods` — the ONE mods dir every reader/writer uses.
pub fn effective_mods_dir(engine: &Engine, instance: &InstanceMeta) -> PathBuf {
    effective_game_dir(engine, instance).join("mods")
}

/// Load an instance (409 INVALID_NAME, 404 NOT_FOUND; applies the memory migration).
pub async fn get_instance(engine: &Engine, name: &str) -> EngineResult<InstanceMeta> {
    let _ = (engine, name);
    crate::not_ported!("instances::get_instance")
}

/// Every instance summary, sorted by name (trash dirs skipped). Never fails on a bad entry.
pub async fn list_instances(engine: &Engine) -> EngineResult<Vec<InstanceSummary>> {
    let _ = engine;
    crate::not_ported!("instances::list_instances")
}

/// Create an instance (validation, hue, game dirs, optional two-hop import,
/// background seeding, Fabric performance preset).
pub async fn create_instance(engine: &Engine, req: CreateInstanceRequest) -> EngineResult<InstanceSummary> {
    let _ = (engine, req);
    crate::not_ported!("instances::create_instance")
}

/// AotStatus snapshot for an instance (read side).
pub async fn aot_status(engine: &Engine, instance: &InstanceMeta) -> EngineResult<AotStatus> {
    let _ = (engine, instance);
    crate::not_ported!("instances::aot_status")
}

/// Canonical sha256 over the enabled jars' `name:sha1` lines (memoized by stat).
pub async fn compute_mod_set_hash(engine: &Engine, instance: &InstanceMeta) -> EngineResult<String> {
    let _ = (engine, instance);
    crate::not_ported!("instances::compute_mod_set_hash")
}
