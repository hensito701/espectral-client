//! Port of `src/engine/onboarding.mjs` — first-launch setup.
//!
//! Creates the ready-to-play default instance ("Espectral default", 26.3,
//! Fabric, automatic memory) importing settings from the most recently active
//! detected launcher, then — in the background — installs the performance +
//! branding presets and queues AOT training so the cache is warm before the
//! first play. Runs in [`crate::Engine::start`] before the UI's first call.
//!
//! The `first_launch_setup_complete` config marker prevents a later manual
//! deletion of every instance from silently recreating the default. A failed
//! creation is NOT marked complete (the next launch retries).
//!
//! Depends on content (import / instances / mods) and game-launch (aot)
//! contract functions; until those are ported this logs and returns
//! `creation-failed` without touching the marker.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::engine::Engine;
use crate::types::{CreateInstanceRequest, ImportSource, ImportSourceKind, Loader, ModPreset};

pub const DEFAULT_INSTANCE_NAME: &str = "Espectral default";
pub const DEFAULT_INSTANCE_VERSION: &str = "26.3";
pub const DEFAULT_INSTANCE_LOADER: Loader = Loader::Fabric;

/// ensureFirstLaunch() result (`{ created, reason?, source?, error? }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnboardingResult {
    pub created: bool,
    /// 'already-complete' | 'instances-exist' | 'creation-failed'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Latest mtime (ms) of the files that show a source was actually used.
pub fn source_activity(source: &ImportSource) -> f64 {
    ["options.txt", "optionsLC.txt", "servers.dat"]
        .iter()
        .filter_map(|f| std::fs::metadata(Path::new(&source.path).join(f)).ok())
        .map(|m| crate::platform::mtime_ms(&m))
        .fold(0.0, f64::max)
}

/// Choose the source whose settings were changed most recently, preferring
/// other launchers over plain vanilla `.minecraft`.
pub fn choose_active_source(sources: &[ImportSource]) -> Option<&ImportSource> {
    let usable: Vec<&ImportSource> = sources.iter().filter(|s| s.options_exists || s.servers_exists).collect();
    let others: Vec<&ImportSource> = usable.iter().copied().filter(|s| s.kind != ImportSourceKind::Vanilla).collect();
    let mut candidates: Vec<(&ImportSource, f64)> =
        if others.is_empty() { usable } else { others }.into_iter().map(|s| (s, source_activity(s))).collect();
    // stable sort, newest first (ties keep detection order, like Array#sort)
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    candidates.first().map(|(s, _)| *s)
}

fn marker_patch(source: Option<&str>) -> Map<String, Value> {
    match json!({ "first_launch_setup_complete": true, "first_launch_import_source": source }) {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// One-time first-launch setup (JS ensureFirstLaunch).
pub async fn ensure_first_launch(engine: &Engine) -> OnboardingResult {
    let cfg = engine.config();
    if cfg.is_true("first_launch_setup_complete") {
        return OnboardingResult { created: false, reason: Some("already-complete".into()), source: None, error: None };
    }

    match crate::instances::list_instances(engine).await {
        Ok(existing) if !existing.is_empty() => {
            cfg.set("first_launch_setup_complete", Value::Bool(true));
            return OnboardingResult {
                created: false,
                reason: Some("instances-exist".into()),
                source: None,
                error: None,
            };
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("[onboarding] instance listing failed: {e}"),
    }

    let sources = match crate::import::detect_import_sources(engine).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[onboarding] import source detection failed: {e}");
            vec![]
        }
    };
    let source = choose_active_source(&sources).cloned();
    let memory = source.as_ref().and_then(|s| s.lunar.as_ref()).and_then(|l| l.allocated_memory).filter(|m| *m >= 512);

    let req = CreateInstanceRequest {
        name: DEFAULT_INSTANCE_NAME.into(),
        version: DEFAULT_INSTANCE_VERSION.into(),
        loader: Some(DEFAULT_INSTANCE_LOADER),
        memory_mb: memory,
        import_from: source.as_ref().map(|s| s.id.clone()),
        merge_optionslc: source.as_ref().map(|s| s.kind == ImportSourceKind::Lunar).unwrap_or(false),
        defer_auto_train: true,
        ..Default::default()
    };

    match crate::instances::create_instance(engine, req).await {
        Ok(_) => {
            // Presets + AOT training, fire-and-forget (JS `void (async () => …)()`).
            let bg = engine.clone();
            engine.spawn_background("onboarding", async move {
                for preset in [ModPreset::Performance, ModPreset::Branding] {
                    if let Err(e) = crate::mods::install_preset(&bg, DEFAULT_INSTANCE_NAME, preset).await {
                        tracing::warn!("[onboarding] {} preset install failed: {e}", preset.as_str());
                    }
                }
                let instance = crate::instances::get_instance(&bg, DEFAULT_INSTANCE_NAME).await?;
                crate::aot::queue_train_instance(&bg, &instance, crate::aot::TrainOptions::default()).await?;
                Ok(())
            });
            let source_id = source.as_ref().map(|s| s.id.clone());
            cfg.save(marker_patch(source_id.as_deref()));
            match &source {
                Some(s) => tracing::info!("[onboarding] created '{DEFAULT_INSTANCE_NAME}' from {}", s.label),
                None => tracing::info!("[onboarding] created '{DEFAULT_INSTANCE_NAME}'"),
            }
            OnboardingResult { created: true, reason: None, source: source_id, error: None }
        }
        Err(e) => {
            tracing::warn!("[onboarding] default instance creation failed: {e}");
            OnboardingResult {
                created: false,
                reason: Some("creation-failed".into()),
                source: None,
                error: Some(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    fn src(id: &str, kind: ImportSourceKind, path: &Path, options: bool) -> ImportSource {
        ImportSource {
            id: id.into(),
            kind,
            label: id.into(),
            path: path.to_string_lossy().into_owned(),
            options_exists: options,
            servers_exists: false,
            options_key_count: 0,
            servers_entry_count: 0,
            lunar: None,
        }
    }

    fn touch(p: &Path, secs_ago: u64) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn prefers_other_launchers_then_most_recent_activity() {
        let d = tempfile::tempdir().unwrap();
        let (v, f, l) = (d.path().join("v"), d.path().join("f"), d.path().join("l"));
        touch(&v.join("options.txt"), 1); // vanilla is the freshest…
        touch(&f.join("options.txt"), 500);
        touch(&l.join("optionsLC.txt"), 100);
        let sources = vec![
            src("vanilla", ImportSourceKind::Vanilla, &v, true),
            src("fastclient", ImportSourceKind::Fastclient, &f, true),
            src("lunar", ImportSourceKind::Lunar, &l, true),
            src("unused", ImportSourceKind::Lunar, &d.path().join("u"), false),
        ];
        // …but other launchers win over vanilla; lunar is newer than fastclient.
        assert_eq!(choose_active_source(&sources).unwrap().id, "lunar");
        // only vanilla usable -> vanilla
        assert_eq!(choose_active_source(&sources[..1]).unwrap().id, "vanilla");
        assert!(choose_active_source(&sources[3..]).is_none());
    }

    #[tokio::test]
    async fn already_complete_is_a_no_op() {
        let (_d, e) = temp_engine();
        e.config().set("first_launch_setup_complete", Value::Bool(true));
        let r = ensure_first_launch(&e).await;
        assert_eq!(r.reason.as_deref(), Some("already-complete"));
    }
}
