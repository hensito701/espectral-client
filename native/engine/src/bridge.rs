//! Port of `src/engine/bridge.mjs` — the in-game mod bridge: per-launch
//! token registry, visible-player cosmetics cache, batched presence queries.
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! The launcher hands every launched game a random per-launch token via the
//! JVM property `-Despectral.bridge.token` (url via `-Despectral.bridge.url`).
//! The mod calls `POST http://127.0.0.1:<port>/api/bridge/visible` with
//! headers `x-espectral-client: 1` + `x-espectral-bridge: <token>` and a
//! `{uuids:[…]}` body — served by crate::loopback, which calls
//! [`visible_for_token`].
//!
//! Answers come from an in-memory cache ONLY — the request path never does
//! network I/O. Uncached uuids are queued and resolved in the background by
//! `POST /v1/presence/query` on espectral-id: at most 1 request / 3 s,
//! batches of ≤100, positive and negative cache TTL 60 s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use indexmap::{IndexMap, IndexSet};
use rand::RngCore;
use serde_json::{json, Value};

use crate::api::accounts_skins::{BridgePlayer, BridgeVisibleReply};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::identity::{self, normalize_uuid};

pub const BRIDGE_HEADER: &str = "x-espectral-bridge";
pub const CACHE_TTL_MS: u64 = 60_000;
pub const NEGATIVE_TTL_MS: u64 = 60_000;
pub const QUERY_BATCH: usize = 100;
pub const QUERY_INTERVAL_MS: u64 = 3_000;
pub const MAX_VISIBLE_INPUT: usize = 1000;

/// Per-launch bridge token: 32 random bytes, hex.
pub fn mint_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// A running launch registered for the bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLaunch {
    pub instance: String,
    /// 32 lowercase hex (or the raw value when it was not a uuid).
    pub uuid: String,
    pub game_dir: PathBuf,
    pub token: String,
    /// ms epoch.
    pub started_at: i64,
}

/// Presence cache entry (`absent` = negative entry).
#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub grant: Option<String>,
    pub loadout: Option<IndexMap<String, Option<String>>>,
    pub absent: bool,
    /// ms epoch.
    pub expires_at: i64,
}

#[derive(Debug, Default)]
struct Inner {
    launches: IndexMap<String, BridgeLaunch>,
    cache: HashMap<String, CacheEntry>,
    /// uuid -> owners needing a background query.
    pending: IndexMap<String, IndexSet<String>>,
    flushing: bool,
    last_flush_at: i64,
    flush_timer: Option<tokio::task::JoinHandle<()>>,
}

/// Bridge registry + presence cache (JS module state).
#[derive(Debug, Default)]
pub struct BridgeState {
    inner: Mutex<Inner>,
}

fn lock(engine: &Engine) -> std::sync::MutexGuard<'_, Inner> {
    engine.identity_discord_state().bridge.inner.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> i64 {
    crate::platform::now_ms()
}

/// Register a running game. A re-register (same instance relaunching) swaps
/// the token — a stale token can never authorize after reattach.
pub fn attach(engine: &Engine, instance: &str, uuid: Option<&str>, game_dir: PathBuf, token: &str) -> BridgeLaunch {
    let raw = uuid.unwrap_or_default();
    let entry = BridgeLaunch {
        instance: instance.to_string(),
        uuid: normalize_uuid(raw).unwrap_or_else(|| raw.to_string()),
        game_dir,
        token: token.to_string(),
        started_at: now_ms(),
    };
    lock(engine).launches.insert(instance.to_string(), entry.clone());
    entry
}

/// Unregister a launch. `expected_token`, when given, scopes the delete to
/// the launch that owns that token (a stale teardown must not evict a newer
/// registration for the same instance name).
pub fn detach(engine: &Engine, instance: &str, expected_token: Option<&str>) -> Option<BridgeLaunch> {
    let mut g = lock(engine);
    let entry = g.launches.get(instance)?;
    if expected_token.is_some_and(|t| t != entry.token) {
        return None;
    }
    g.launches.shift_remove(instance)
}

/// Every live launch owned by a uuid (0..n — concurrent instances).
pub fn launches_for(engine: &Engine, uuid: &str) -> Vec<BridgeLaunch> {
    let Some(flat) = normalize_uuid(uuid) else { return vec![] };
    lock(engine).launches.values().filter(|l| l.uuid == flat).cloned().collect()
}

/// Every live launch.
pub fn all_launches(engine: &Engine) -> Vec<BridgeLaunch> {
    lock(engine).launches.values().cloned().collect()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The launch owning `token` (64 hex, case-insensitive; constant-time compare).
pub fn authenticate(engine: &Engine, token: &str) -> Option<BridgeLaunch> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let probe = token.to_ascii_lowercase();
    let g = lock(engine);
    for l in g.launches.values() {
        let stored = l.token.to_ascii_lowercase();
        // Launches registered without a minted token ('') are skipped.
        if stored.len() != probe.len() {
            continue;
        }
        if ct_eq(stored.as_bytes(), probe.as_bytes()) {
            return Some(l.clone());
        }
    }
    None
}

/// Test hook: direct cache injection.
pub fn seed_cache(engine: &Engine, uuid: &str, entry: CacheEntry) {
    if let Some(flat) = normalize_uuid(uuid) {
        lock(engine).cache.insert(flat, entry);
    }
}

/// Test hook: wipe module state.
pub fn reset(engine: &Engine) {
    let mut g = lock(engine);
    if let Some(t) = g.flush_timer.take() {
        t.abort();
    }
    *g = Inner::default();
}

/// POST /api/bridge/visible: 403 BRIDGE_AUTH for an unknown token, else
/// [`visible`] for the owning launch.
pub fn visible_for_token(engine: &Engine, token: &str, uuids: &[String]) -> EngineResult<BridgeVisibleReply> {
    let Some(owner) = authenticate(engine, token) else {
        return Err(EngineError::api(403, "BRIDGE_AUTH", "missing or invalid x-espectral-bridge token"));
    };
    Ok(visible(engine, &owner, uuids))
}

/// Answer a visible-uuid query from cache; uncached uuids are queued for a
/// background fetch billed to the owner's session.
pub fn visible(engine: &Engine, owner: &BridgeLaunch, uuids: &[String]) -> BridgeVisibleReply {
    let now = now_ms();
    let mut players = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut queued = false;
    {
        let mut g = lock(engine);
        for raw in uuids.iter().take(MAX_VISIBLE_INPUT) {
            let Some(flat) = normalize_uuid(raw) else { continue };
            if !seen.insert(flat.clone()) {
                continue;
            }
            match g.cache.get(&flat) {
                Some(hit) if hit.expires_at > now => {
                    if !hit.absent {
                        players.push(BridgePlayer {
                            uuid: flat,
                            grant: hit.grant.clone(),
                            loadout: hit.loadout.clone(),
                        });
                    }
                    continue; // negative entries are also "cached" — don't requeue
                }
                Some(_) => {
                    g.cache.remove(&flat);
                }
                None => {}
            }
            if !owner.uuid.is_empty() {
                g.pending.entry(flat).or_default().insert(owner.uuid.clone());
                queued = true;
            }
        }
    }
    if queued {
        schedule_flush(engine);
    }
    BridgeVisibleReply { players }
}

fn schedule_flush(engine: &Engine) {
    let mut g = lock(engine);
    if g.flushing || g.flush_timer.is_some() || g.pending.is_empty() {
        return;
    }
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let wait = (QUERY_INTERVAL_MS as i64 - (now_ms() - g.last_flush_at)).max(0) as u64;
    let e = engine.clone();
    let cancel = engine.cancel_token().clone();
    g.flush_timer = Some(tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => {}
            _ = async {
                tokio::time::sleep(Duration::from_millis(wait)).await;
                lock(&e).flush_timer = None;
                flush(&e).await;
            } => {}
        }
    }));
}

/// Drain the pending queue in ≤100-uuid batches, at most one request per
/// QUERY_INTERVAL_MS, grouped by owner (each call is Bearer-authed with the
/// owner's session token).
async fn flush(engine: &Engine) {
    {
        let mut g = lock(engine);
        if g.flushing {
            return;
        }
        g.flushing = true;
    }
    loop {
        let by_owner: IndexMap<String, Vec<String>> = {
            let g = lock(engine);
            if g.pending.is_empty() {
                break;
            }
            let mut m: IndexMap<String, Vec<String>> = IndexMap::new();
            for (uuid, owners) in &g.pending {
                for o in owners {
                    m.entry(o.clone()).or_default().push(uuid.clone());
                }
            }
            m
        };
        let mut did_work = false;
        for (owner, uuids) in by_owner {
            let batch: Vec<String> = uuids.into_iter().take(QUERY_BATCH).collect();
            {
                let mut g = lock(engine);
                for u in &batch {
                    if let Some(owners) = g.pending.get_mut(u) {
                        owners.shift_remove(&owner);
                        if owners.is_empty() {
                            g.pending.shift_remove(u);
                        }
                    }
                }
            }
            let Some(token) = identity::token_for(engine, &owner) else { continue };
            did_work = true;
            query_once(engine, &owner, &token, &batch).await;
            if !lock(engine).pending.is_empty() {
                tokio::time::sleep(Duration::from_millis(QUERY_INTERVAL_MS)).await;
            }
        }
        if !did_work {
            break; // every queued uuid had an unverified owner — dropped
        }
    }
    let again = {
        let mut g = lock(engine);
        g.flushing = false;
        g.last_flush_at = now_ms();
        !g.pending.is_empty()
    };
    if again {
        schedule_flush(engine);
    }
}

fn loadout_of(v: Option<&Value>) -> Option<IndexMap<String, Option<String>>> {
    match v {
        Some(Value::Object(m)) => Some(m.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_string))).collect()),
        _ => None,
    }
}

async fn query_once(engine: &Engine, owner: &str, token: &str, uuids: &[String]) {
    let res = identity::service_fetch(
        engine,
        identity::Method::Post,
        "/v1/presence/query",
        Some(json!({ "uuids": uuids })),
        Some(token),
        8000,
    )
    .await;
    let mut g = lock(engine);
    match res {
        Ok(data) => {
            let expires_at = now_ms() + CACHE_TTL_MS as i64;
            let mut seen = std::collections::HashSet::new();
            if let Some(Value::Array(players)) = data.get("players") {
                for p in players {
                    let Some(flat) = p.get("uuid").and_then(Value::as_str).and_then(normalize_uuid) else { continue };
                    seen.insert(flat.clone());
                    g.cache.insert(
                        flat,
                        CacheEntry {
                            grant: p.get("grant").and_then(Value::as_str).map(str::to_string),
                            loadout: loadout_of(p.get("loadout")),
                            absent: false,
                            expires_at,
                        },
                    );
                }
            }
            // Asked for but absent from the response: no live presence.
            let neg = now_ms() + NEGATIVE_TTL_MS as i64;
            for u in uuids {
                if !seen.contains(u) {
                    g.cache.insert(u.clone(), CacheEntry { grant: None, loadout: None, absent: true, expires_at: neg });
                }
            }
        }
        Err(e) => {
            let code = e.raw_code().map(str::to_string).unwrap_or_else(|| e.to_string());
            tracing::warn!("[bridge] presence/query failed for {}…: {code}", owner.get(..8).unwrap_or(owner));
            // Negatively cache the batch so a failing service isn't hammered.
            let neg = now_ms() + NEGATIVE_TTL_MS as i64;
            for u in uuids {
                g.cache.insert(u.clone(), CacheEntry { grant: None, loadout: None, absent: true, expires_at: neg });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::tests::{point_at, seed_token};
    use crate::test_support::{temp_engine, MockResponse, MockServer};

    const OWNER: &str = "11111111111111111111111111111111";
    const PEER: &str = "22222222222222222222222222222222";
    const GHOST: &str = "33333333333333333333333333333333";

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn tokens_are_64_hex_and_unique() {
        let a = mint_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, mint_token());
    }

    #[tokio::test]
    async fn missing_or_wrong_token_is_403() {
        let (d, e) = temp_engine();
        let err = visible_for_token(&e, "", &s(&[PEER])).unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (403, Some("BRIDGE_AUTH")));
        attach(&e, "inst-a", Some(OWNER), d.path().into(), &"a".repeat(64));
        let err = visible_for_token(&e, &"b".repeat(64), &s(&[PEER])).unwrap_err();
        assert_eq!(err.status(), 403);
        assert!(visible_for_token(&e, &"A".repeat(64), &s(&[])).is_ok(), "case-insensitive");
    }

    #[tokio::test]
    async fn answers_from_cache_only() {
        let (d, e) = temp_engine();
        let token = mint_token();
        attach(&e, "inst-b", Some(OWNER), d.path().into(), &token);
        let mut lo = IndexMap::new();
        lo.insert("espalda".to_string(), Some("alas_eco".to_string()));
        seed_cache(
            &e,
            PEER,
            CacheEntry {
                grant: Some("hdr.pay.sig".into()),
                loadout: Some(lo.clone()),
                absent: false,
                expires_at: now_ms() + 60_000,
            },
        );
        seed_cache(&e, GHOST, CacheEntry { grant: None, loadout: None, absent: true, expires_at: now_ms() + 60_000 });
        let r = visible_for_token(&e, &token, &s(&[PEER, GHOST, "not-a-uuid"])).unwrap();
        assert_eq!(r.players.len(), 1);
        assert_eq!(r.players[0].uuid, PEER);
        assert_eq!(r.players[0].grant.as_deref(), Some("hdr.pay.sig"));
        assert_eq!(r.players[0].loadout, Some(lo));
        assert_eq!(serde_json::to_value(&r).unwrap()["players"][0]["loadout"], json!({ "espalda": "alas_eco" }));
    }

    #[tokio::test]
    async fn expired_entries_are_requeued_not_answered() {
        let (d, e) = temp_engine();
        let token = mint_token();
        attach(&e, "inst-c", Some(OWNER), d.path().into(), &token);
        seed_cache(
            &e,
            PEER,
            CacheEntry { grant: Some("g".into()), loadout: None, absent: false, expires_at: now_ms() - 1 },
        );
        assert!(visible_for_token(&e, &token, &s(&[PEER])).unwrap().players.is_empty());
        reset(&e);
    }

    #[tokio::test]
    async fn uncached_uuids_batched_to_presence_query() {
        let (d, e) = temp_engine();
        let server = MockServer::start(|_| {
            MockResponse::json(
                &json!({ "players": [{ "uuid": PEER, "grant": "g2", "loadout": { "cabeza": "orejas_vulpinas" } }] }),
            )
        })
        .await;
        point_at(&e, &server);
        seed_token(&e, OWNER, "tok-owner");
        let token = mint_token();
        attach(&e, "inst-d", Some(OWNER), d.path().into(), &token);
        assert!(visible_for_token(&e, &token, &s(&[PEER, GHOST])).unwrap().players.is_empty());
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while server.hits() == 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let reqs = server.requests();
        assert_eq!(reqs.len(), 1, "exactly one batched query");
        assert_eq!(reqs[0].path, "/v1/presence/query");
        assert_eq!(reqs[0].header("authorization").as_deref(), Some("Bearer tok-owner"));
        assert_eq!(reqs[0].body_json().unwrap()["uuids"], json!([PEER, GHOST]));
        let r = visible_for_token(&e, &token, &s(&[PEER, GHOST])).unwrap();
        assert_eq!(r.players.len(), 1, "ghost negatively cached");
        assert_eq!(r.players[0].grant.as_deref(), Some("g2"));
        assert_eq!(server.hits(), 1, "negative entries are not requeued");
    }

    #[tokio::test]
    async fn uuid_spellings_fold_to_one() {
        let (d, e) = temp_engine();
        let token = mint_token();
        attach(&e, "inst-uuid", Some(OWNER), d.path().into(), &token);
        seed_cache(
            &e,
            PEER,
            CacheEntry { grant: Some("g3".into()), loadout: None, absent: false, expires_at: now_ms() + 60_000 },
        );
        let dashed = "22222222-2222-2222-2222-222222222222";
        let upper = PEER.to_uppercase();
        let r = visible_for_token(&e, &token, &s(&[dashed, &upper, PEER])).unwrap();
        assert_eq!(r.players.len(), 1);
        assert_eq!(r.players[0].uuid, PEER);
    }

    #[tokio::test]
    async fn empty_token_launch_never_authenticates() {
        let (d, e) = temp_engine();
        attach(&e, "no-token-launch", Some(OWNER), d.path().into(), "");
        assert_eq!(visible_for_token(&e, &mint_token(), &s(&[PEER])).unwrap_err().status(), 403);
        assert_eq!(authenticate(&e, ""), None);
        assert_eq!(authenticate(&e, &"f".repeat(64)), None);
        detach(&e, "no-token-launch", None);
    }

    #[tokio::test]
    async fn detach_scoped_by_token() {
        let (d, e) = temp_engine();
        let t = mint_token();
        attach(&e, "inst-g", Some(OWNER), d.path().into(), &t);
        assert_eq!(detach(&e, "inst-g", Some(&"0".repeat(64))), None);
        assert_eq!(authenticate(&e, &t).unwrap().instance, "inst-g");
        assert_eq!(launches_for(&e, OWNER).len(), 1);
        assert_eq!(all_launches(&e).len(), 1);
        assert_eq!(detach(&e, "inst-g", Some(&t)).unwrap().token, t);
        assert_eq!(authenticate(&e, &t), None);
        assert!(launches_for(&e, OWNER).is_empty());
    }
}
