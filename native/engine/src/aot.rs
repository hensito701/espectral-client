//! Port of `src/engine/aot.mjs` — JEP 483 AOT cache: keys, classpath stamps,
//! staleness, proof logs, eviction, training runs (WM_CLOSE close ladder),
//! the training queue.
//!
//! **Owner: game-launch.** Status: STUB (cache_key / paths implemented).
//!
//! JS exports to port: classpathStamp stampMatches readCacheStamp
//! isCacheStale instanceKey latestAotProofLog aotRefusalLine aotProof
//! pruneAotProofLogs evictOldAotCaches waitForMarker trainInstance
//! queueTrainInstance. Tests: tests/aot.test.mjs.
//! Paths: Paths::aot_root / aot_cache_dir / aot_cache_file / aot_meta.
//! Events: train-progress / train-done (crate::events).
//! Windows graceful close: PostMessage(WM_CLOSE) to the game's windows via
//! windows-sys (Win32_UI_WindowsAndMessaging is enabled).
//!
//! CONTRACT (called by content instances.getSummary/aotStatus and onboarding):
//! cache_key, queue_train_instance, TrainOptions.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::events::TrainDoneEvent;
use crate::types::InstanceMeta;

/// AOT cache key: `sha256("<version>|<javaBuild>|<osArch>")`, null-safe
/// (`unknown` build, Node `process.arch` default). Existing caches stay valid.
pub fn cache_key(version: &str, java_build: Option<&str>, os_arch: Option<&str>) -> String {
    let b = java_build.unwrap_or("unknown");
    let a = os_arch.unwrap_or(crate::platform::node_arch());
    hex::encode(Sha256::digest(format!("{version}|{b}|{a}").as_bytes()))
}

/// queueTrainInstance() options.
#[derive(Clone, Default)]
pub struct TrainOptions {
    /// Key estimate (train-done carries the authoritative one).
    pub key: Option<String>,
    /// Explicit user action: train even when aot_auto_train is off.
    pub force: bool,
    /// Never spawn the trainer while this returns true (a live game shares the gameDir).
    pub is_blocked: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

/// Queue (deduped per instance) a background training run; resolves with the
/// train-done payload.
pub async fn queue_train_instance(
    engine: &Engine,
    instance: &InstanceMeta,
    opts: TrainOptions,
) -> EngineResult<TrainDoneEvent> {
    let _ = (engine, instance, opts);
    crate::not_ported!("aot::queue_train_instance")
}

/// AOT key for an instance with its current JVM (aot.instanceKey).
pub async fn instance_key(engine: &Engine, instance: &InstanceMeta) -> EngineResult<String> {
    let _ = (engine, instance);
    crate::not_ported!("aot::instance_key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_matches_js_sha256() {
        let k = cache_key("1.21.11", Some("25.0.4+7-LTS"), Some("x64"));
        assert_eq!(k, hex::encode(Sha256::digest(b"1.21.11|25.0.4+7-LTS|x64")));
        assert_eq!(k, cache_key("1.21.11", Some("25.0.4+7-LTS"), Some("x64")));
        assert_ne!(k, cache_key("26.2", Some("25.0.4+7-LTS"), Some("x64")));
        assert_ne!(k, cache_key("1.21.11", Some("21.0.1+12-LTS"), Some("x64")));
        assert_ne!(k, cache_key("1.21.11", Some("25.0.4+7-LTS"), Some("arm64")));
        assert_eq!(cache_key("v", None, Some("x64")), hex::encode(Sha256::digest(b"v|unknown|x64")));
        assert_eq!(cache_key("v", Some("b"), None), cache_key("v", Some("b"), Some(crate::platform::node_arch())));
    }
}
