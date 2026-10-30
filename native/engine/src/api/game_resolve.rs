//! Facade — cluster **game-resolve** (routes/versions.mjs + GET /api/jvm).
//!
//! | JS route                        | method |
//! |---------------------------------|--------|
//! | GET /api/versions               | [`Engine::list_versions`] |
//! | GET /api/versions/:id/resolve   | [`Engine::resolve_version_info`] |
//! | GET /api/jvm                    | [`Engine::jvm_info`] |
//!
//! Domain modules: crate::{resolver, runtimes, jvm, neoforge}.

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{JvmInfo, VersionInfo, VersionManifest};

/// game-resolve in-memory state (manifest memo, fabric loader cache, JVM
/// probe memo / test override, …). Add fields freely — only this cluster owns it.
#[derive(Debug, Default)]
pub struct GameResolveState {}

impl Engine {
    /// This cluster's in-memory state.
    #[allow(dead_code)]
    pub(crate) fn game_resolve_state(&self) -> &GameResolveState {
        &self.inner.game_resolve
    }
}

/// Engine start hook (nothing to do at startup in the JS).
pub(crate) async fn on_engine_start(_engine: &Engine) -> EngineResult<()> {
    Ok(())
}

pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

impl Engine {
    /// GET /api/versions -> `{ latest_release, latest_snapshot, versions }`
    /// (6h cache). Any failure -> 502 MANIFEST_FETCH_FAILED
    /// "version manifest fetch failed: <msg>".
    pub async fn list_versions(&self) -> EngineResult<VersionManifest> {
        crate::not_ported!("Engine::list_versions")
    }

    /// GET /api/versions/:id/resolve -> VersionInfo. 404 VERSION_NOT_FOUND
    /// (resolver code `version_not_found`), else 502 VERSION_RESOLVE_FAILED.
    pub async fn resolve_version_info(&self, id: &str) -> EngineResult<VersionInfo> {
        let _ = id;
        crate::not_ported!("Engine::resolve_version_info")
    }

    /// GET /api/jvm -> JvmInfo + `supported_majors: [25, 21, 17, 16, 8]`.
    /// 503 UNAVAILABLE when discovery fails to load.
    pub async fn jvm_info(&self) -> EngineResult<JvmInfo> {
        crate::not_ported!("Engine::jvm_info")
    }
}
