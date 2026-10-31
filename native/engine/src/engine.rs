//! The [`Engine`] facade: one cheap-to-clone handle the UI holds.
//!
//! Replaces `server.mjs` + `cli.mjs`: there is no HTTP server any more — the
//! UI calls the async methods in `src/api/*.rs` (one per JS route) directly.
//! What the JS bootstrap did is [`Engine::start`]:
//!
//! 1. resolve + create the data dir (`paths::resolve_data_dir`);
//! 2. first-launch onboarding (`onboarding::ensure_first_launch`) — before
//!    the UI's first request, like the JS (which ran it before `listen`);
//! 3. per-cluster start hooks (`on_engine_start` in each `api/<cluster>.rs`):
//!    MSA token prewarm, Discord "En el launcher" presence, adopting games
//!    still running from a previous session, the loopback bridge listener…
//!
//! [`Engine::shutdown`] runs the per-cluster shutdown hooks (Discord IPC
//! close, …) and cancels background tasks. Running games are independent OS
//! processes and are never touched (same as the JS engine restart).

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::api;
use crate::config::ConfigStore;
use crate::download::{HttpClient, HttpOptions};
use crate::error::EngineResult;
use crate::events::{EngineEvent, EventBus};
use crate::paths::{self, Paths};

/// Engine version (mirrors package.json; reported by `health()`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Construction options.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// None = [`paths::resolve_data_dir`] (ESPECTRAL_DATA_DIR / LOCALAPPDATA / XDG).
    pub data_dir: Option<PathBuf>,
    pub http: HttpOptions,
    /// Run first-launch onboarding in [`Engine::start`].
    pub run_onboarding: bool,
    /// Run the per-cluster start hooks in [`Engine::start`].
    pub start_background_tasks: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            data_dir: None,
            http: HttpOptions::default(),
            run_onboarding: true,
            start_background_tasks: true,
        }
    }
}

/// Instances with a launch currently preparing or running (JS
/// routes/launch.mjs `activeInstances`). Owned by game-launch; read by
/// content (rename / version-change guards, `running` flags). In-memory only.
#[derive(Debug, Default)]
pub struct ActiveInstances {
    set: Mutex<std::collections::HashSet<String>>,
}

impl ActiveInstances {
    pub fn contains(&self, name: &str) -> bool {
        self.set.lock().unwrap_or_else(|p| p.into_inner()).contains(name)
    }
    /// Returns false when already present.
    pub fn insert(&self, name: &str) -> bool {
        self.set.lock().unwrap_or_else(|p| p.into_inner()).insert(name.to_string())
    }
    pub fn remove(&self, name: &str) -> bool {
        self.set.lock().unwrap_or_else(|p| p.into_inner()).remove(name)
    }
    pub fn snapshot(&self) -> Vec<String> {
        self.set.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect()
    }
    pub fn clear(&self) {
        self.set.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
}

pub(crate) struct Inner {
    pub(crate) paths: Paths,
    pub(crate) config: Arc<ConfigStore>,
    pub(crate) events: EventBus,
    pub(crate) http: HttpClient,
    pub(crate) active_instances: ActiveInstances,
    pub(crate) cancel: CancellationToken,
    // Per-cluster in-memory state. Each struct lives in (and is owned by)
    // its cluster's api file; add fields there, not here.
    pub(crate) game_resolve: api::game_resolve::GameResolveState,
    pub(crate) game_launch: api::game_launch::GameLaunchState,
    pub(crate) content: api::content::ContentState,
    pub(crate) accounts_skins: api::accounts_skins::AccountsSkinsState,
    pub(crate) identity_discord: api::identity_discord::IdentityDiscordState,
}

/// The engine. `Clone` is an `Arc` bump; share it freely across tasks.
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").field("data_dir", &self.inner.paths.data_dir()).finish()
    }
}

impl Engine {
    /// Build an engine without bootstrapping (no onboarding, no hooks).
    /// Creates the data dir. Tests use this (see `test_support::temp_engine`).
    pub fn new(opts: EngineOptions) -> EngineResult<Engine> {
        let root = opts.data_dir.clone().unwrap_or_else(paths::resolve_data_dir);
        std::fs::create_dir_all(&root)?;
        let paths = Paths::new(root);
        let events = EventBus::new();
        let config = Arc::new(ConfigStore::new(&paths));
        let http = HttpClient::new(opts.http.clone(), events.clone(), Some(config.clone()))?;
        Ok(Engine {
            inner: Arc::new(Inner {
                paths,
                config,
                events,
                http,
                active_instances: ActiveInstances::default(),
                cancel: CancellationToken::new(),
                game_resolve: Default::default(),
                game_launch: Default::default(),
                content: Default::default(),
                accounts_skins: Default::default(),
                identity_discord: Default::default(),
            }),
        })
    }

    /// Build + bootstrap (the JS `cli.mjs`): onboarding, then cluster start hooks.
    pub async fn start(opts: EngineOptions) -> EngineResult<Engine> {
        let run_onboarding = opts.run_onboarding;
        let start_tasks = opts.start_background_tasks;
        let engine = Engine::new(opts)?;
        tracing::info!("[engine] EspectralClient engine v{VERSION} — data dir {}", engine.paths().data_dir().display());
        // Loads (and migrates) config.json up front, like the JS first import.
        let _ = engine.config().load();
        if run_onboarding {
            let r = crate::onboarding::ensure_first_launch(&engine).await;
            tracing::debug!("[onboarding] {r:?}");
        }
        if start_tasks {
            engine.run_start_hooks().await;
        }
        Ok(engine)
    }

    async fn run_start_hooks(&self) {
        macro_rules! hook {
            ($name:literal, $f:expr) => {
                if let Err(e) = $f.await {
                    if matches!(e, crate::EngineError::NotImplemented(_)) {
                        tracing::debug!("[engine] {} start hook not ported yet", $name);
                    } else {
                        tracing::warn!("[engine] {} start hook failed: {e}", $name);
                    }
                }
            };
        }
        hook!("game-resolve", api::game_resolve::on_engine_start(self));
        hook!("content", api::content::on_engine_start(self));
        hook!("accounts-skins", api::accounts_skins::on_engine_start(self));
        hook!("identity-discord", api::identity_discord::on_engine_start(self));
        hook!("game-launch", api::game_launch::on_engine_start(self));
    }

    /// Graceful shutdown: cluster hooks (Discord IPC close, …) then cancel
    /// background tasks. Running games keep running.
    pub async fn shutdown(&self) {
        api::identity_discord::on_engine_shutdown(self).await;
        api::accounts_skins::on_engine_shutdown(self).await;
        api::content::on_engine_shutdown(self).await;
        api::game_launch::on_engine_shutdown(self).await;
        api::game_resolve::on_engine_shutdown(self).await;
        self.inner.cancel.cancel();
    }

    // ---- shared services -------------------------------------------------

    /// On-disk layout under the data dir.
    pub fn paths(&self) -> &Paths {
        &self.inner.paths
    }

    /// `<data>/config.json`.
    pub fn config(&self) -> &ConfigStore {
        &self.inner.config
    }

    /// HTTP client (fetch / verified downloads / mirrors).
    pub fn http(&self) -> &HttpClient {
        &self.inner.http
    }

    /// Event bus (emit side). UI code uses [`Engine::subscribe`].
    pub fn events(&self) -> &EventBus {
        &self.inner.events
    }

    /// Emit one event (shorthand for `events().emit`).
    pub fn emit(&self, event: EngineEvent) {
        self.inner.events.emit(event);
    }

    /// Replaces `GET /api/events` (SSE): every event emitted from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// Instances with a launch preparing/running.
    pub fn active_instances(&self) -> &ActiveInstances {
        &self.inner.active_instances
    }

    /// Cancelled by [`Engine::shutdown`]; background loops should select on it.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.inner.cancel
    }

    /// True once [`Engine::shutdown`] ran (or a shutdown was requested).
    pub fn is_shutting_down(&self) -> bool {
        self.inner.cancel.is_cancelled()
    }

    /// Fire-and-forget background task (JS `void (async () => …)()`): errors
    /// are logged; the task is dropped at engine shutdown.
    pub fn spawn_background<F>(&self, name: &'static str, fut: F)
    where
        F: Future<Output = EngineResult<()>> + Send + 'static,
    {
        let cancel = self.inner.cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel.cancelled() => {}
                r = fut => {
                    if let Err(e) = r {
                        tracing::warn!("[{name}] background task failed: {e}");
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    #[tokio::test]
    async fn engine_is_cheap_clone_and_creates_data_dir() {
        let (dir, e) = temp_engine();
        assert!(dir.path().exists());
        let e2 = e.clone();
        assert_eq!(e.paths().data_dir(), e2.paths().data_dir());
        assert!(Arc::ptr_eq(&e.inner, &e2.inner));
        assert!(e.active_instances().insert("a"));
        assert!(e2.active_instances().contains("a"));
        assert!(!e.active_instances().insert("a"));
    }

    #[tokio::test]
    async fn start_bootstraps_and_shutdown_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let opts = crate::test_support::test_options(dir.path().join("data"));
        // Onboarding and the cluster hooks touch the network / the real home
        // dir once ported, so unit tests keep them off (test_options default).
        let e = Engine::start(opts).await.unwrap();
        assert!(e.paths().config_json().exists());
        assert!(!e.is_shutting_down());
        e.shutdown().await;
        assert!(e.is_shutting_down());
    }
}
