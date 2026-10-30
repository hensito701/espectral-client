//! Port of `src/engine/discordAuth.mjs` — Discord OAuth2 (launcher login and
//! identity link), state registry, in-memory sessions, `<data>/discord-users.json`.
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! The authorization-code exchange happens SERVER-SIDE on espectral-id — the
//! launcher holds only the public client_id. Plain logins forward the code to
//! /v1/discord/identify; callbacks whose state was minted for an identity link
//! forward to /v1/discord/link and bind Discord to that verified uuid.
//!
//! - OAuth artifacts are NEVER persisted to disk.
//! - Sessions use random opaque UUIDs (memory only, 30 days).
//! - Plain logins upsert `{id, name, avatar, mc_accounts}` into discord-users.json.
//!
//! IMPORTANT: the redirect URI registered with Discord is
//! `http://127.0.0.1:4199/api/discord/callback`, so the native app still
//! needs a loopback listener for that one path (crate::loopback). Cookies are
//! not needed natively: the UI holds the session id from `discord_claim`
//! (the cookie helpers are ported for completeness).

use std::collections::HashMap;
use std::sync::Mutex;

use rand::RngCore;
use serde_json::{json, Map, Value};

use crate::api::identity_discord::{DiscordCallbackOutcome, DiscordClaim, DiscordUser};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::identity;

pub const REDIRECT_URI: &str = "http://127.0.0.1:4199/api/discord/callback";
/// Public Discord application id of the Espectral identity app (the secret
/// stays on espectral-id, which exchanges the code).
pub const DEFAULT_DISCORD_CLIENT_ID: &str = "1454618588659843072";

pub const STATE_TTL_MS: i64 = 15 * 60 * 1000;
pub const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Session cookie name (`espectral_session`).
pub const SESSION_COOKIE: &str = "espectral_session";

#[derive(Debug, Clone, Default)]
struct OAuthState {
    created_at: i64,
    link_uuid: Option<String>,
    session_id: Option<String>,
    linked: bool,
    user: Option<DiscordUser>,
    processing: bool,
}

#[derive(Debug, Clone)]
struct Session {
    user: DiscordUser,
    created_at: i64,
}

/// In-memory OAuth states + launcher sessions.
#[derive(Debug, Default)]
pub struct DiscordAuthState {
    states: Mutex<HashMap<String, OAuthState>>,
    sessions: Mutex<HashMap<String, Session>>,
}

fn st(engine: &Engine) -> &DiscordAuthState {
    &engine.identity_discord_state().discord_auth
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The synthetic user mock mode answers with.
pub fn mock_user() -> DiscordUser {
    DiscordUser {
        id: "718293847561829304".into(),
        name: "TestPlayer".into(),
        avatar: Some("https://cdn.discordapp.com/embed/avatars/0.png".into()),
        extra: Map::new(),
    }
}

/// ESPECTRAL_DISCORD_MOCK=1|true (or the test override).
pub fn is_mock_mode(engine: &Engine) -> bool {
    if let Some(m) = engine.identity_discord_state().overrides().discord_mock {
        return m;
    }
    matches!(std::env::var("ESPECTRAL_DISCORD_MOCK").as_deref(), Ok("1") | Ok("true"))
}

fn sweep_expired_states(engine: &Engine) {
    let now = crate::platform::now_ms();
    lock(&st(engine).states).retain(|_, s| now - s.created_at <= STATE_TTL_MS);
}

fn truthy_id(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) if n.as_f64() != Some(0.0) => Some(n.to_string()),
        _ => None,
    }
}

/// Client id precedence: config `discord.client_id` → `discord.client_id` key
/// → `discord_client_id` → env DISCORD_CLIENT_ID → env
/// ESPECTRAL_DISCORD_CLIENT_ID → [`DEFAULT_DISCORD_CLIENT_ID`].
pub fn discord_client_id_from(config: &Map<String, Value>, env: impl Fn(&str) -> Option<String>) -> String {
    truthy_id(config.get("discord").and_then(|d| d.get("client_id")))
        .or_else(|| truthy_id(config.get("discord.client_id")))
        .or_else(|| truthy_id(config.get("discord_client_id")))
        .or_else(|| env("DISCORD_CLIENT_ID").filter(|s| !s.is_empty()))
        .or_else(|| env("ESPECTRAL_DISCORD_CLIENT_ID").filter(|s| !s.is_empty()))
        .unwrap_or_else(|| DEFAULT_DISCORD_CLIENT_ID.to_string())
}

/// JS getDiscordCredentials().clientId.
pub fn discord_client_id(engine: &Engine) -> String {
    engine.config().with(|cfg| discord_client_id_from(cfg, |k| std::env::var(k).ok()))
}

fn random_state() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Discord authorization URL with a fresh random state. `link_uuid` (flat
/// 32-hex) marks the state as an identity link. Mock mode short-circuits to
/// the local callback with a synthetic code.
pub fn auth_start_url(engine: &Engine, link_uuid: Option<&str>) -> (String, String) {
    sweep_expired_states(engine);
    let state = random_state();
    let entry = OAuthState {
        created_at: crate::platform::now_ms(),
        link_uuid: link_uuid.map(str::to_string),
        ..Default::default()
    };
    if is_mock_mode(engine) {
        lock(&st(engine).states).insert(state.clone(), entry);
        return (format!("http://127.0.0.1:4199/api/discord/callback?code=mock_code&state={state}"), state);
    }
    let client_id = discord_client_id(engine);
    lock(&st(engine).states).insert(state.clone(), entry);
    let mut url = url::Url::parse("https://discord.com/oauth2/authorize").expect("static url");
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", "identify")
        .append_pair("state", &state)
        .append_pair("prompt", "consent");
    (url.to_string(), state)
}

/// One-time claim of the outcome of an OAuth flow the app started:
/// pending | invalid | ok (session or link).
pub fn claim_session_user(engine: &Engine, state: &str) -> DiscordClaim {
    sweep_expired_states(engine);
    let invalid = DiscordClaim { status: "invalid".into(), session_id: None, linked: None, uuid: None, user: None };
    let mut states = lock(&st(engine).states);
    let Some(entry) = (!state.is_empty()).then(|| states.get(state).cloned()).flatten() else {
        return invalid;
    };
    if let Some(sid) = &entry.session_id {
        states.remove(state);
        drop(states);
        let user = lock(&st(engine).sessions).get(sid).map(|s| s.user.clone());
        return match user {
            None => invalid,
            Some(user) => DiscordClaim {
                status: "ok".into(),
                session_id: Some(sid.clone()),
                linked: None,
                uuid: None,
                user: Some(user),
            },
        };
    }
    if entry.linked {
        states.remove(state);
        return DiscordClaim {
            status: "ok".into(),
            session_id: None,
            linked: Some(true),
            uuid: entry.link_uuid.clone(),
            user: entry.user.clone(),
        };
    }
    DiscordClaim { status: "pending".into(), session_id: None, linked: None, uuid: None, user: None }
}

fn q<'a>(query: &'a HashMap<String, String>, k: &str) -> Option<&'a str> {
    query.get(k).map(String::as_str).filter(|s| !s.is_empty())
}

/// Handle Discord's OAuth2 redirect: validate the state, forward the code to
/// espectral-id (link or identify), create the launcher session for plain
/// logins and re-attach the outcome to the state for [`claim_session_user`].
pub async fn handle_callback(engine: &Engine, query: &HashMap<String, String>) -> EngineResult<DiscordCallbackOutcome> {
    let is_mock = is_mock_mode(engine);
    if q(query, "error").is_some() {
        let msg = q(query, "error_description")
            .or_else(|| q(query, "error"))
            .unwrap_or("Discord authorization was cancelled or denied");
        return Err(EngineError::bad_request("DISCORD_OAUTH_FAILED", msg));
    }
    let Some(code) = q(query, "code") else {
        return Err(EngineError::bad_request("MISSING_CODE", "No authorization code received from Discord"));
    };
    let state = q(query, "state").map(str::to_string);

    let (had_entry, link_uuid) = {
        let mut states = lock(&st(engine).states);
        match state.as_deref().and_then(|s| states.get_mut(s)) {
            None => {
                if !is_mock {
                    return Err(EngineError::bad_request(
                        "INVALID_STATE",
                        "OAuth state is invalid or expired. Please try logging in again.",
                    ));
                }
                (false, None)
            }
            Some(e) if e.processing || e.session_id.is_some() || e.linked => {
                // Replay protection: a consumed state or one with an exchange
                // in flight must not run a second code exchange.
                return Err(EngineError::bad_request(
                    "INVALID_STATE",
                    "OAuth state was already used. Please try logging in again.",
                ));
            }
            Some(e) => {
                // Keep the entry while the exchange runs: a claim poll
                // mid-flight must answer 'pending', not 'invalid'.
                e.processing = true;
                (true, e.link_uuid.as_deref().and_then(identity::normalize_uuid))
            }
        }
    };

    let exchange = async {
        if is_mock && code.starts_with("mock_") {
            let user = mock_user();
            if let Some(u) = &link_uuid {
                identity::store_discord(engine, u, &user)?;
                let _ = identity::refresh_grant(engine, u, 5000).await;
            }
            Ok(user)
        } else if let Some(u) = &link_uuid {
            identity::discord_link(engine, u, code, REDIRECT_URI).await
        } else {
            identity::discord_identify(engine, code, REDIRECT_URI).await
        }
    };
    let user = match exchange.await {
        Ok(u) => u,
        Err(e) => {
            // The flow is dead: the next claim must see 'invalid', not 'pending'.
            if let Some(s) = &state {
                lock(&st(engine).states).remove(s);
            }
            return Err(e);
        }
    };
    let reattach = state.as_ref().filter(|_| had_entry || is_mock);

    if let Some(u) = link_uuid {
        if let Some(s) = reattach {
            lock(&st(engine).states).insert(
                s.clone(),
                OAuthState {
                    created_at: crate::platform::now_ms(),
                    link_uuid: Some(u),
                    linked: true,
                    user: Some(user.clone()),
                    ..Default::default()
                },
            );
        }
        return Ok(DiscordCallbackOutcome { session_id: None, user: Some(user), linked: Some(true) });
    }

    upsert_discord_user(engine, &user);
    let session_id = uuid::Uuid::new_v4().to_string();
    lock(&st(engine).sessions)
        .insert(session_id.clone(), Session { user: user.clone(), created_at: crate::platform::now_ms() });
    if let Some(s) = reattach {
        lock(&st(engine).states).insert(
            s.clone(),
            OAuthState {
                created_at: crate::platform::now_ms(),
                session_id: Some(session_id.clone()),
                ..Default::default()
            },
        );
    }
    Ok(DiscordCallbackOutcome { session_id: Some(session_id), user: Some(user), linked: None })
}

/// Upsert `{id, name, avatar, mc_accounts}` into `<data>/discord-users.json`
/// (array; a legacy object map is read as its values). Never fails.
pub fn upsert_discord_user(engine: &Engine, user: &DiscordUser) {
    let path = engine.paths().discord_users_json();
    let mut list: Vec<Value> = match std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()) {
        Some(Value::Array(a)) => a,
        Some(Value::Object(m)) => m.into_iter().map(|(_, v)| v).collect(),
        _ => vec![],
    };
    let idx = list.iter().position(|u| u.get("id").and_then(Value::as_str) == Some(user.id.as_str()));
    let mc_accounts =
        idx.and_then(|i| list[i].get("mc_accounts").filter(|v| v.is_array()).cloned()).unwrap_or_else(|| json!([]));
    let rec = json!({ "id": user.id, "name": user.name, "avatar": user.avatar, "mc_accounts": mc_accounts });
    match idx {
        Some(i) => list[i] = rec,
        None => list.push(rec),
    }
    if let Err(e) = crate::paths::write_json_atomic(&path, &list) {
        tracing::warn!("[discordAuth] could not upsert discord-users.json: {e}");
    }
}

/// Active session user, or the mock user when `mock_requested` in mock mode.
pub fn get_session_user(engine: &Engine, session_id: Option<&str>, mock_requested: bool) -> Option<DiscordUser> {
    if mock_requested && is_mock_mode(engine) {
        return Some(mock_user());
    }
    let sid = session_id.filter(|s| !s.is_empty())?;
    let mut sessions = lock(&st(engine).sessions);
    let sess = sessions.get(sid)?;
    if crate::platform::now_ms() - sess.created_at > SESSION_TTL_MS {
        sessions.remove(sid);
        return None;
    }
    Some(sess.user.clone())
}

/// Invalidate a session.
pub fn destroy_session(engine: &Engine, session_id: Option<&str>) {
    if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
        lock(&st(engine).sessions).remove(sid);
    }
}

/// Session id from `x-discord-session`, `Authorization: Bearer`, or the
/// `espectral_session` / `session` cookie (in that order).
pub fn extract_session_id(cookie: Option<&str>, auth: Option<&str>, custom: Option<&str>) -> Option<String> {
    if let Some(c) = custom.map(str::trim).filter(|c| !c.is_empty()) {
        return Some(c.to_string());
    }
    if let Some(a) = auth.and_then(|a| a.strip_prefix("Bearer ")) {
        return Some(a.trim().to_string());
    }
    for part in cookie?.split(';') {
        let t = part.trim();
        if let Some(eq) = t.find('=').filter(|i| *i > 0) {
            let (k, v) = (t[..eq].trim(), t[eq + 1..].trim());
            if k == SESSION_COOKIE || k == "session" {
                return Some(
                    url::form_urlencoded::parse(format!("v={}", v.replace('+', "%2B")).as_bytes())
                        .next()
                        .map(|(_, v)| v.into_owned())
                        .unwrap_or_else(|| v.to_string()),
                );
            }
        }
    }
    None
}

/// `Set-Cookie` value for an opaque session id.
pub fn create_session_cookie(session_id: &str) -> String {
    let enc: String = url::form_urlencoded::byte_serialize(session_id.as_bytes()).collect();
    format!("{SESSION_COOKIE}={enc}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000")
}

/// `Set-Cookie` value that clears the session cookie.
pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::tests::{point_at, seed_token};
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use std::time::Duration;

    const LINK_UUID: &str = "aabbccddeeff00112233445566778899";

    async fn stub(delay_ms: u64) -> MockServer {
        MockServer::start(move |r| {
            let body = r.body_json().unwrap_or_default();
            let d = Duration::from_millis(delay_ms);
            match (r.method.as_str(), r.path.as_str()) {
                ("POST", "/v1/discord/identify") => {
                    if body["code"] == "fail-code" {
                        return MockResponse::json(
                            &json!({ "error": { "code": "DISCORD_IDENTIFY_FAILED", "message": "exchange failed" } }),
                        )
                        .with_status(502);
                    }
                    MockResponse::json(&json!({ "discord": { "id": "9999", "username": "ServiceUser", "avatar": null } }))
                        .with_delay(d)
                }
                ("POST", "/v1/discord/link") => MockResponse::json(
                    &json!({ "discord": { "id": "8888", "username": "LinkUser", "avatar": null }, "flags": ["supporter"] }),
                )
                .with_delay(d),
                ("GET", "/v1/grant") => {
                    MockResponse::json(&json!({ "error": { "code": "UNAUTHORIZED", "message": "bad token" } })).with_status(401)
                }
                _ => MockResponse::json(&json!({ "error": { "code": "NOT_FOUND", "message": "stub" } })).with_status(404),
            }
        })
        .await
    }

    fn qs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn setup() -> (tempfile::TempDir, Engine) {
        let (d, e) = temp_engine();
        e.identity_discord_state().set_overrides(|o| o.discord_mock = Some(false));
        (d, e)
    }

    #[tokio::test]
    async fn claim_stays_pending_while_exchange_in_flight() {
        let (_d, e) = setup();
        let server = stub(200).await;
        point_at(&e, &server);
        let (_url, state) = auth_start_url(&e, None);
        let e2 = e.clone();
        let s2 = state.clone();
        let cb = tokio::spawn(async move { handle_callback(&e2, &qs(&[("code", "real-code"), ("state", &s2)])).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(claim_session_user(&e, &state).status, "pending");
        let r = cb.await.unwrap().unwrap();
        assert_eq!(r.user.as_ref().unwrap().id, "9999");
        let claim = claim_session_user(&e, &state);
        assert_eq!(claim.status, "ok");
        assert_eq!(claim.user.unwrap().id, "9999");
        assert_eq!(get_session_user(&e, claim.session_id.as_deref(), false).unwrap().id, "9999");
        assert_eq!(claim_session_user(&e, &state).status, "invalid");
        // discord-users.json upserted (no tokens).
        let users: Value =
            serde_json::from_str(&std::fs::read_to_string(e.paths().discord_users_json()).unwrap()).unwrap();
        assert_eq!(
            users,
            json!([{ "id": "9999", "name": "ServiceUser",
            "avatar": "https://cdn.discordapp.com/embed/avatars/0.png", "mc_accounts": [] }])
        );
    }

    #[tokio::test]
    async fn duplicate_callback_rejected_while_exchange_runs() {
        let (_d, e) = setup();
        let server = stub(200).await;
        point_at(&e, &server);
        let (_url, state) = auth_start_url(&e, None);
        let e2 = e.clone();
        let s2 = state.clone();
        let cb = tokio::spawn(async move { handle_callback(&e2, &qs(&[("code", "real-code"), ("state", &s2)])).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let dup = handle_callback(&e, &qs(&[("code", "real-code-2"), ("state", &state)])).await.unwrap_err();
        assert!(dup.to_string().to_lowercase().contains("already used"));
        assert!(dup.has_code("INVALID_STATE"));
        assert_eq!(cb.await.unwrap().unwrap().user.unwrap().id, "9999");
        assert_eq!(claim_session_user(&e, &state).status, "ok");
    }

    #[tokio::test]
    async fn link_flow_resolves_linked_for_uuid() {
        let (_d, e) = setup();
        let server = stub(200).await;
        point_at(&e, &server);
        seed_token(&e, LINK_UUID, "sess-1");
        let (url, state) = auth_start_url(&e, Some(LINK_UUID));
        assert!(url.contains(&state));
        let e2 = e.clone();
        let s2 = state.clone();
        let cb = tokio::spawn(async move { handle_callback(&e2, &qs(&[("code", "link-code"), ("state", &s2)])).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(claim_session_user(&e, &state).status, "pending");
        let r = cb.await.unwrap().unwrap();
        assert_eq!(r.linked, Some(true));
        assert_eq!(r.session_id, None);
        let claim = claim_session_user(&e, &state);
        assert_eq!(claim.status, "ok");
        assert_eq!(claim.linked, Some(true));
        assert_eq!(claim.uuid.as_deref(), Some(LINK_UUID));
        assert_eq!(claim.user.unwrap().id, "8888");
        let entry = identity::get_entry(&e, LINK_UUID).unwrap();
        assert_eq!(entry["discord"]["id"], "8888");
        assert_eq!(entry["flags"], json!(["supporter"]));
    }

    #[tokio::test]
    async fn failed_exchange_drops_state() {
        let (_d, e) = setup();
        let server = stub(0).await;
        point_at(&e, &server);
        let (_url, state) = auth_start_url(&e, None);
        assert!(handle_callback(&e, &qs(&[("code", "fail-code"), ("state", &state)])).await.is_err());
        assert_eq!(claim_session_user(&e, &state).status, "invalid");
    }

    #[tokio::test]
    async fn mock_mode_completes_the_flow() {
        let (_d, e) = temp_engine();
        e.identity_discord_state().set_overrides(|o| o.discord_mock = Some(true));
        let (url, state) = auth_start_url(&e, None);
        assert!(url.contains("/api/discord/callback"), "{url}");
        assert!(url.contains("code=mock_code"), "{url}");
        let r = handle_callback(&e, &qs(&[("code", "mock_code"), ("state", &state)])).await.unwrap();
        assert_eq!(r.user.unwrap().name, "TestPlayer");
        assert!(r.session_id.is_some());
        assert_eq!(claim_session_user(&e, &state).status, "ok");
        assert_eq!(get_session_user(&e, None, true).unwrap().name, "TestPlayer");
    }

    #[tokio::test]
    async fn oauth_errors_and_missing_state() {
        let (_d, e) = setup();
        let err =
            handle_callback(&e, &qs(&[("error", "access_denied"), ("error_description", "Nope")])).await.unwrap_err();
        assert!(err.has_code("DISCORD_OAUTH_FAILED"));
        assert_eq!(err.to_string(), "Nope");
        assert!(handle_callback(&e, &qs(&[])).await.unwrap_err().has_code("MISSING_CODE"));
        assert!(handle_callback(&e, &qs(&[("code", "x"), ("state", "zzz")]))
            .await
            .unwrap_err()
            .has_code("INVALID_STATE"));
    }

    #[test]
    fn stock_install_uses_public_app_id() {
        assert_eq!(DEFAULT_DISCORD_CLIENT_ID, "1454618588659843072");
        assert_eq!(discord_client_id_from(&Map::new(), |_| None), "1454618588659843072");
        let (_d, e) = setup();
        if std::env::var("DISCORD_CLIENT_ID").is_err() && std::env::var("ESPECTRAL_DISCORD_CLIENT_ID").is_err() {
            let (url, _state) = auth_start_url(&e, Some(LINK_UUID));
            let parsed = url::Url::parse(&url).unwrap();
            assert_eq!(
                format!("{}{}", parsed.origin().ascii_serialization(), parsed.path()),
                "https://discord.com/oauth2/authorize"
            );
            let p: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
            assert_eq!(p["client_id"], "1454618588659843072");
            assert_eq!(p["redirect_uri"], REDIRECT_URI);
            assert_eq!(p["response_type"], "code");
            assert_eq!(p["scope"], "identify");
            assert_eq!(p["prompt"], "consent");
            assert_eq!(p["state"].len(), 32);
        }
    }

    #[test]
    fn config_client_id_overrides_env_and_default() {
        let env = |k: &str| (k == "DISCORD_CLIENT_ID").then(|| "test-client-id".to_string());
        let mut cfg = Map::new();
        assert_eq!(discord_client_id_from(&cfg, env), "test-client-id");
        cfg.insert("discord".into(), json!({ "client_id": "config-app-id" }));
        assert_eq!(discord_client_id_from(&cfg, env), "config-app-id");
        cfg.insert("discord".into(), Value::Null);
        assert_eq!(discord_client_id_from(&cfg, env), "test-client-id");
        cfg.insert("discord_client_id".into(), json!("flat-id"));
        assert_eq!(discord_client_id_from(&cfg, env), "flat-id");
        let env2 = |k: &str| (k == "ESPECTRAL_DISCORD_CLIENT_ID").then(|| "esp".to_string());
        assert_eq!(discord_client_id_from(&Map::new(), env2), "esp");

        let (_d, e) = setup();
        e.config().set("discord", json!({ "client_id": "config-app-id" }));
        let (url, _) = auth_start_url(&e, None);
        assert!(url.contains("client_id=config-app-id"), "{url}");
    }

    #[test]
    fn unknown_or_missing_state_claims_invalid() {
        let (_d, e) = setup();
        assert_eq!(claim_session_user(&e, "deadbeefcafe").status, "invalid");
        assert_eq!(claim_session_user(&e, "").status, "invalid");
    }

    #[test]
    fn session_id_extraction_and_cookies() {
        assert_eq!(extract_session_id(Some("a=1; espectral_session=abc%20d"), None, None).as_deref(), Some("abc d"));
        assert_eq!(extract_session_id(Some("session=x"), Some("Bearer tok "), None).as_deref(), Some("tok"));
        assert_eq!(extract_session_id(Some("session=x"), Some("Bearer tok"), Some(" c ")).as_deref(), Some("c"));
        assert_eq!(extract_session_id(Some("other=1"), None, None), None);
        assert_eq!(extract_session_id(None, None, None), None);
        assert_eq!(
            create_session_cookie("ab-1"),
            "espectral_session=ab-1; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000"
        );
        assert!(clear_session_cookie().contains("Max-Age=0"));
        let (_d, e) = setup();
        assert_eq!(get_session_user(&e, Some("nope"), false), None);
        assert_eq!(get_session_user(&e, None, true), None, "mock user only in mock mode");
        destroy_session(&e, Some("nope"));
    }
}
