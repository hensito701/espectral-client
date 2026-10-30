//! Facade — cluster **game-resolve** (routes/versions.mjs + GET /api/jvm).
//!
//! | JS route                        | method |
//! |---------------------------------|--------|
//! | GET /api/versions               | [`Engine::list_versions`] |
//! | GET /api/versions/:id/resolve   | [`Engine::resolve_version_info`] |
//! | GET /api/jvm                    | [`Engine::jvm_info`] |
//!
//! Domain modules: crate::{resolver, runtimes, jvm, neoforge}.
//!
//! [`GameResolveState`] also carries the cluster's **test seams**: every
//! upstream base URL ([`ResolveEndpoints`]) and every probe of the user's
//! machine ([`ResolveHooks`]: java probe, PATH `java`, FastClient JDK,
//! FastClient seed root, NeoForge installer JVM). Tests — including other
//! clusters' tests that call into game-resolve — point the endpoints at a
//! `MockServer` and call [`GameResolveState::isolate`] so nothing touches the
//! real network or the real home dir.

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{JavaProbe, JvmInfo, VersionInfo, VersionManifest};

/// Replacement for every java probe (JS runtimes `__setProbeOverride`).
pub type ProbeFn = Arc<dyn Fn(&Path) -> JavaProbe + Send + Sync>;

/// Upstream base URLs (the JS module constants). Injectable for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveEndpoints {
    /// resolver.MANIFEST_URL
    pub manifest_url: String,
    /// resolver.FABRIC_META_BASE (no trailing slash)
    pub fabric_meta_base: String,
    /// resolver.FABRIC_MAVEN (trailing slash)
    pub fabric_maven: String,
    /// resolver.ASSET_BASE (no trailing slash)
    pub asset_base: String,
    /// neoforge.NEOFORGE_MAVEN (no trailing slash)
    pub neoforge_maven: String,
    /// `https://api.adoptium.net/v3` (runtimes.downloadTemurin)
    pub adoptium_api: String,
}

impl Default for ResolveEndpoints {
    fn default() -> Self {
        ResolveEndpoints {
            manifest_url: crate::resolver::MANIFEST_URL.to_string(),
            fabric_meta_base: crate::resolver::FABRIC_META_BASE.to_string(),
            fabric_maven: crate::resolver::FABRIC_MAVEN.to_string(),
            asset_base: crate::resolver::ASSET_BASE.to_string(),
            neoforge_maven: crate::neoforge::NEOFORGE_MAVEN.to_string(),
            adoptium_api: crate::runtimes::ADOPTIUM_API.to_string(),
        }
    }
}

impl ResolveEndpoints {
    /// Every endpoint on one base URL (a `MockServer`): `<base>/manifest.json`,
    /// `<base>/fabric-meta/v2`, `<base>/fabric-maven/`, `<base>/assets`,
    /// `<base>/neoforge`, `<base>/adoptium/v3`.
    pub fn all_on(base: &str) -> Self {
        let b = base.trim_end_matches('/');
        ResolveEndpoints {
            manifest_url: format!("{b}/manifest.json"),
            fabric_meta_base: format!("{b}/fabric-meta/v2"),
            fabric_maven: format!("{b}/fabric-maven/"),
            asset_base: format!("{b}/assets"),
            neoforge_maven: format!("{b}/neoforge"),
            adoptium_api: format!("{b}/adoptium/v3"),
        }
    }
}

/// Machine probes. `None` = the real thing; `Some(x)` = pretend the probe answered `x`.
#[derive(Clone, Default)]
pub struct ResolveHooks {
    /// Replaces `java -XshowSettings:properties -version` for every probe.
    pub probe: Option<ProbeFn>,
    /// Result of the PATH `java` lookup (`which` / `where`).
    pub path_java: Option<Option<PathBuf>>,
    /// Location of FastClient's bundled Temurin 25 (both jvm.mjs and runtimes.mjs flavours).
    pub fastclient_java: Option<Option<PathBuf>>,
    /// `%APPDATA%/.fastclient` seed root (resolver.fastClientRoot).
    pub fast_client_root: Option<Option<PathBuf>>,
    /// JVM used to run the official NeoForge installer (`--installClient`).
    pub installer_java: Option<PathBuf>,
}

impl std::fmt::Debug for ResolveHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolveHooks")
            .field("probe", &self.probe.as_ref().map(|_| "<fn>"))
            .field("path_java", &self.path_java)
            .field("fastclient_java", &self.fastclient_java)
            .field("fast_client_root", &self.fast_client_root)
            .field("installer_java", &self.installer_java)
            .finish()
    }
}

/// Per-key async serialization (JS `withMajorLock` / `withEnsureLock`).
#[derive(Debug)]
pub(crate) struct KeyedLocks<K: Eq + Hash> {
    map: Mutex<HashMap<K, Arc<tokio::sync::Mutex<()>>>>,
}

impl<K: Eq + Hash> Default for KeyedLocks<K> {
    fn default() -> Self {
        KeyedLocks { map: Mutex::new(HashMap::new()) }
    }
}

impl<K: Eq + Hash + Clone> KeyedLocks<K> {
    pub(crate) async fn lock(&self, key: &K) -> tokio::sync::OwnedMutexGuard<()> {
        let m = {
            let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
            map.entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
        };
        m.lock_owned().await
    }
}

/// `(mtime:size of versions/manifest.json, id -> entry)`.
pub(crate) type ManifestMemo = (String, Arc<HashMap<String, crate::resolver::ManifestEntry>>);

/// game-resolve in-memory state: manifest id->entry memo, JVM probe memo,
/// per-major / per-NeoForge-version locks, endpoints and test hooks.
#[derive(Debug, Default)]
pub struct GameResolveState {
    endpoints: RwLock<ResolveEndpoints>,
    hooks: RwLock<ResolveHooks>,
    /// `(mtime:size of versions/manifest.json, id -> entry)` (JS `_manifestEntryMemo`).
    pub(crate) manifest_memo: Mutex<Option<ManifestMemo>>,
    /// `path:mtimeMs:size` -> probe (JS jvm.mjs `probeMemo`).
    pub(crate) probe_memo: Mutex<HashMap<String, JavaProbe>>,
    pub(crate) major_locks: KeyedLocks<u32>,
    pub(crate) neoforge_locks: KeyedLocks<String>,
}

impl GameResolveState {
    pub fn endpoints(&self) -> ResolveEndpoints {
        self.endpoints.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_endpoints(&self, endpoints: ResolveEndpoints) {
        *self.endpoints.write().unwrap_or_else(|p| p.into_inner()) = endpoints;
    }

    pub fn hooks(&self) -> ResolveHooks {
        self.hooks.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Mutate the hooks in place.
    pub fn update_hooks(&self, f: impl FnOnce(&mut ResolveHooks)) {
        f(&mut self.hooks.write().unwrap_or_else(|p| p.into_inner()));
    }

    /// Test isolation: no PATH java, no FastClient JDK, no FastClient seed
    /// root (the probe override, if any, is kept).
    pub fn isolate(&self) {
        self.update_hooks(|h| {
            h.path_java = Some(None);
            h.fastclient_java = Some(None);
            h.fast_client_root = Some(None);
        });
    }

    /// JS `__setProbeOverride` / `__clearProbeOverride`.
    pub fn set_probe_override(&self, probe: Option<ProbeFn>) {
        self.update_hooks(|h| h.probe = probe);
    }

    /// JS `__clearManifestMemo`.
    pub fn clear_manifest_memo(&self) {
        *self.manifest_memo.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// JS `__clearProbeMemo`.
    pub fn clear_probe_memo(&self) {
        self.probe_memo.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    /// JS `__probeMemoSize`.
    pub fn probe_memo_size(&self) -> usize {
        self.probe_memo.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

impl Engine {
    /// This cluster's in-memory state (endpoints + test hooks are public so
    /// other clusters' tests can isolate game-resolve).
    pub fn game_resolve_state(&self) -> &GameResolveState {
        &self.inner.game_resolve
    }
}

/// Engine start hook (nothing to do at startup in the JS).
pub(crate) async fn on_engine_start(_engine: &Engine) -> EngineResult<()> {
    Ok(())
}

pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

/// GET /api/jvm `supported_majors`.
pub const JVM_SUPPORTED_MAJORS: [u32; 5] = [25, 21, 17, 16, 8];

impl Engine {
    /// GET /api/versions -> `{ latest_release, latest_snapshot, versions }`
    /// (6h cache). Any failure -> 502 MANIFEST_FETCH_FAILED
    /// "version manifest fetch failed: <msg>".
    pub async fn list_versions(&self) -> EngineResult<VersionManifest> {
        crate::resolver::get_version_manifest(self, false)
            .await
            .map_err(|e| EngineError::api(502, "MANIFEST_FETCH_FAILED", format!("version manifest fetch failed: {e}")))
    }

    /// GET /api/versions/:id/resolve -> VersionInfo. 404 VERSION_NOT_FOUND
    /// (resolver code `version_not_found`), else 502 VERSION_RESOLVE_FAILED.
    pub async fn resolve_version_info(&self, id: &str) -> EngineResult<VersionInfo> {
        crate::resolver::resolve_version(self, id).await.map_err(|e| {
            if e.has_code("version_not_found") {
                EngineError::api(404, "VERSION_NOT_FOUND", e.to_string())
            } else {
                EngineError::api(502, "VERSION_RESOLVE_FAILED", format!("version resolve failed: {e}"))
            }
        })
    }

    /// GET /api/jvm -> JvmInfo + `supported_majors: [25, 21, 17, 16, 8]`.
    /// Like the JS route, any discovery failure answers 503 UNAVAILABLE (the
    /// cause is logged).
    pub async fn jvm_info(&self) -> EngineResult<JvmInfo> {
        match crate::jvm::get_jvm_info(self, None).await {
            Ok(runtime) => Ok(JvmInfo { runtime, supported_majors: Some(JVM_SUPPORTED_MAJORS.to_vec()) }),
            Err(e) => {
                tracing::warn!("[jvm] discovery failed: {e}");
                Err(EngineError::api(503, "UNAVAILABLE", "JVM discovery not loaded (launch slice pending)"))
            }
        }
    }
}
