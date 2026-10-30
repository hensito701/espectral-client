//! Port of `src/engine/presence.mjs` — ties espectral-id PUT/DELETE
//! /v1/presence to running launches (republish every 240 s while sharing).
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! While an instance launched under account X is running AND X's cosmetics
//! `share` flag is on, the engine PUTs the sanitized loadout now and every
//! [`REPUBLISH_MS`], and DELETEs on exit or when share flips off. Presence
//! is memory-only on the service (TTL 600 s), so a missed DELETE expires.
//!
//! CONTRACT (called by game-launch on spawn/exit/adopt, accounts-skins on
//! cosmetics share changes): launch_started, launch_exited, sync_share
//! (+ republish_now for loadout edits while sharing).
//!
//! Depends on accounts-skins: `cosmetics::share_enabled` (contract) and
//! `cosmetics::presence_loadout` (sanitized loadout for the verified flags —
//! JS `sanitizeLoadout(getCosmeticsState(uuid).loadout, flags)`). Until those
//! land, an error from them reads as "not sharing" / "nothing to publish".

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::bridge;
use crate::engine::Engine;
use crate::error::EngineResult;
use crate::identity::{self, normalize_uuid};

pub const REPUBLISH_MS: u64 = 240_000;

/// A spawned game the bridge + presence publisher should know about.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPresence {
    pub instance: String,
    /// any uuid form
    pub uuid: Option<String>,
    pub game_dir: PathBuf,
    /// per-launch bridge token (also on the JVM command line)
    pub token: String,
}

#[derive(Debug)]
struct Publisher {
    task: Option<tokio::task::JoinHandle<()>>,
    inflight: Arc<AtomicBool>,
}

/// One publisher per account uuid.
#[derive(Debug, Default)]
pub struct PresenceState {
    publishers: Mutex<HashMap<String, Publisher>>,
}

fn publishers(engine: &Engine) -> std::sync::MutexGuard<'_, HashMap<String, Publisher>> {
    engine.identity_discord_state().presence.publishers.lock().unwrap_or_else(|p| p.into_inner())
}

fn sharing(engine: &Engine, flat: &str) -> bool {
    match crate::cosmetics::share_enabled(engine, flat) {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("[presence] share flag for {} unavailable: {e}", flat.get(..8).unwrap_or(flat));
            false
        }
    }
}

fn has_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok()
}

/// Register a spawned game (bridge attach) and start publishing when sharing.
pub fn launch_started(engine: &Engine, launch: LaunchPresence) -> EngineResult<()> {
    let raw = launch.uuid.clone().unwrap_or_default();
    let flat = normalize_uuid(&raw).unwrap_or(raw);
    bridge::attach(engine, &launch.instance, Some(&flat), launch.game_dir, &launch.token);
    if !flat.is_empty() && sharing(engine, &flat) {
        start_publisher(engine, &flat);
    }
    Ok(())
}

/// Unregister a finished launch; stops + DELETEs presence when it was the
/// last launch for that account. `token`: only tear down the registration
/// made with this token.
pub fn launch_exited(engine: &Engine, instance: &str, token: Option<&str>) -> EngineResult<()> {
    if let Some(entry) = bridge::detach(engine, instance, token) {
        if bridge::launches_for(engine, &entry.uuid).is_empty() {
            stop_publisher(engine, &entry.uuid, true);
        }
    }
    Ok(())
}

/// Start/stop the publisher to match the current share flag.
pub fn sync_share(engine: &Engine, flat_uuid: &str) -> EngineResult<()> {
    let Some(flat) = normalize_uuid(flat_uuid) else { return Ok(()) };
    let running = !bridge::launches_for(engine, &flat).is_empty();
    if running && sharing(engine, &flat) {
        start_publisher(engine, &flat);
    } else {
        stop_publisher(engine, &flat, true);
    }
    Ok(())
}

/// Push the current loadout immediately when it changes while sharing.
pub fn republish_now(engine: &Engine, uuid: &str) -> EngineResult<()> {
    let Some(flat) = normalize_uuid(uuid) else { return Ok(()) };
    let running = !bridge::launches_for(engine, &flat).is_empty();
    if running && sharing(engine, &flat) {
        let existed = publishers(engine).contains_key(&flat);
        start_publisher(engine, &flat);
        if existed && has_runtime() {
            let e = engine.clone();
            engine.spawn_background("presence", async move {
                publish_once(&e, &flat).await;
                Ok(())
            });
        }
        Ok(())
    } else {
        sync_share(engine, &flat)
    }
}

/// True while a publisher runs for the uuid (tests / introspection).
pub fn is_publishing(engine: &Engine, uuid: &str) -> bool {
    normalize_uuid(uuid).is_some_and(|f| publishers(engine).contains_key(&f))
}

fn start_publisher(engine: &Engine, uuid: &str) {
    let mut g = publishers(engine);
    if g.contains_key(uuid) {
        return;
    }
    let inflight = Arc::new(AtomicBool::new(false));
    let task = has_runtime().then(|| {
        let e = engine.clone();
        let u = uuid.to_string();
        let cancel = engine.cancel_token().clone();
        tokio::spawn(async move {
            let run = async {
                loop {
                    publish_once(&e, &u).await;
                    tokio::time::sleep(Duration::from_millis(REPUBLISH_MS)).await;
                }
            };
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = run => {}
            }
        })
    });
    g.insert(uuid.to_string(), Publisher { task, inflight });
}

fn stop_publisher(engine: &Engine, uuid: &str, publish: bool) {
    if let Some(p) = publishers(engine).remove(uuid) {
        if let Some(t) = p.task {
            t.abort();
        }
    }
    if publish && has_runtime() {
        let e = engine.clone();
        let u = uuid.to_string();
        engine.spawn_background("presence", async move {
            unpublish(&e, &u).await;
            Ok(())
        });
    }
}

/// Test hook: stop every publisher without touching the bridge registry.
pub fn reset(engine: &Engine) {
    for (_, p) in publishers(engine).drain() {
        if let Some(t) = p.task {
            t.abort();
        }
    }
}

async fn publish_once(engine: &Engine, uuid: &str) {
    let Some(inflight) = publishers(engine).get(uuid).map(|p| p.inflight.clone()) else { return };
    if inflight.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = async {
        let Some(token) = identity::token_for(engine, uuid) else { return Ok(()) }; // unverified
        let flags = identity::flags_info(engine, uuid).flags;
        let loadout = crate::cosmetics::presence_loadout(engine, uuid, &flags)?;
        identity::service_fetch(
            engine,
            identity::Method::Put,
            "/v1/presence",
            Some(json!({ "loadout": loadout })),
            Some(&token),
            8000,
        )
        .await
        .map(|_| ())
    }
    .await;
    if let Err(e) = result {
        let code = e.raw_code().map(str::to_string).unwrap_or_else(|| e.to_string());
        tracing::warn!("[presence] publish for {}… failed: {code}", uuid.get(..8).unwrap_or(uuid));
    }
    inflight.store(false, Ordering::SeqCst);
}

async fn unpublish(engine: &Engine, uuid: &str) {
    let Some(token) = identity::token_for(engine, uuid) else { return };
    if let Err(e) =
        identity::service_fetch(engine, identity::Method::Delete, "/v1/presence", None, Some(&token), 8000).await
    {
        let code = e.raw_code().map(str::to_string).unwrap_or_else(|| e.to_string());
        tracing::warn!("[presence] unpublish for {}… failed: {code}", uuid.get(..8).unwrap_or(uuid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::tests::{point_at, seed_token};
    use crate::test_support::{temp_engine, MockResponse, MockServer};

    #[tokio::test]
    async fn launch_exited_detaches_only_the_owning_registration() {
        let (d, e) = temp_engine();
        let uuid = "99999999999999999999999999999999"; // no identity token -> no network
        let trainer = bridge::mint_token();
        let game = bridge::mint_token();
        let lp = |token: &str| LaunchPresence {
            instance: "inst-h".into(),
            uuid: Some(uuid.into()),
            game_dir: d.path().into(),
            token: token.into(),
        };
        launch_started(&e, lp(&trainer)).unwrap();
        launch_started(&e, lp(&game)).unwrap();
        launch_exited(&e, "inst-h", Some(&trainer)).unwrap();
        assert_eq!(bridge::authenticate(&e, &game).unwrap().instance, "inst-h");
        launch_exited(&e, "inst-h", Some(&game)).unwrap();
        assert_eq!(bridge::authenticate(&e, &game), None);
        reset(&e);
    }

    #[tokio::test]
    async fn publisher_lifecycle_unpublishes_on_last_exit() {
        let (d, e) = temp_engine();
        let server = MockServer::start(|_| MockResponse::status(204)).await;
        point_at(&e, &server);
        let uuid = "aabbccddeeff00112233445566778899";
        seed_token(&e, uuid, "tok");
        let t = bridge::mint_token();
        bridge::attach(&e, "a", Some(uuid), d.path().into(), &t);
        // Force a publisher on (share flag lives in accounts-skins).
        start_publisher(&e, uuid);
        assert!(is_publishing(&e, uuid));
        launch_exited(&e, "a", Some(&t)).unwrap();
        assert!(!is_publishing(&e, uuid));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !server.requests().iter().any(|r| r.method == "DELETE") && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let del = server.requests().into_iter().find(|r| r.method == "DELETE").expect("DELETE /v1/presence");
        assert_eq!(del.path, "/v1/presence");
        assert_eq!(del.header("authorization").as_deref(), Some("Bearer tok"));
    }
}
