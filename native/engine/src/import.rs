//! Port of `src/engine/import.mjs` — detect other launchers (vanilla
//! `.minecraft`, FastClient, Lunar) and import options.txt / servers.dat /
//! Lunar settings into an instance.
//!
//! **Owner: content.** Status: STUB.
//!
//! JS exports to port: ImportError pathEscapes parseOptionsTxt
//! writeOptionsTxt setOptionKey countOptionsKeys readOptionsTxt
//! makeLunarSource detectImportSources importProfile.
//! Tests: tests/import.test.mjs. Home / APPDATA lookups: crate::paths::{home_dir, appdata_dir}
//! — tests must point those at a tempdir (inject roots; never read the real home).
//!
//! CONTRACT (called by onboarding, instances.createInstance): detect_import_sources, import_profile.

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{ImportResult, ImportSource, InstanceMeta, OverwritePolicy};

/// Detected import sources (ImportSource rows).
pub async fn detect_import_sources(engine: &Engine) -> EngineResult<Vec<ImportSource>> {
    let _ = engine;
    crate::not_ported!("import::detect_import_sources")
}

/// Import a source's profile files into `instance`.
pub async fn import_profile(
    engine: &Engine,
    instance: &InstanceMeta,
    source_id: &str,
    policy: OverwritePolicy,
    merge_optionslc: bool,
) -> EngineResult<ImportResult> {
    let _ = (engine, instance, source_id, policy, merge_optionslc);
    crate::not_ported!("import::import_profile")
}
