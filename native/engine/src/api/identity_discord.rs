//! Facade — cluster **identity-discord** (routes/identity.mjs, routes/discord.mjs).
//!
//! | JS route                                   | method |
//! |--------------------------------------------|--------|
//! | GET    /api/identity                       | [`Engine::identity_state`] |
//! | POST   /api/identity/:uuid/verify          | [`Engine::verify_identity`] |
//! | GET    /api/identity/:uuid/discord/start   | [`Engine::identity_discord_start`] |
//! | DELETE /api/identity/:uuid/discord         | [`Engine::identity_discord_unlink`] |
//! | POST   /api/identity/:uuid/refresh         | [`Engine::identity_refresh`] |
//! | GET    /api/discord/auth/start             | [`Engine::discord_auth_start`] |
//! | GET    /api/discord/claim?state=           | [`Engine::discord_claim`] |
//! | GET    /api/discord/callback               | [`Engine::discord_callback`] (served by crate::loopback) |
//! | GET    /api/discord/me                     | [`Engine::discord_me`] |
//! | POST   /api/discord/logout                 | [`Engine::discord_logout`] |
//!
//! The remaining routes/misc.mjs routes (health, shutdown, theme, accent,
//! config, open-folder) are implemented in api/core.rs; GET /api/jvm is in
//! game_resolve.rs; pick-file / pick-folder are not ported (UI dialogs).
//!
//! [`IdentityDiscordState`] holds the identity.json cache, OAuth states and
//! Discord sessions, the Discord IPC connection, the bridge launch registry +
//! presence cache, presence publishers and the loopback address. Startup:
//! loopback listener (bridge + OAuth callback) and, when `discord_enabled`,
//! the launcher presence `{ state: 'En el launcher', image: 'espectral_logo',
//! startTimestamp: now }`. Shutdown: close the Discord IPC socket, stop the
//! presence publishers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{Account, IdentityAccount, IdentityState, OkReply};
use crate::{accounts, bridge, discord, discord_auth, identity, loopback, presence};

use super::UrlReply;

/// Per-engine overrides of the environment-driven endpoints (tests point
/// them at loopback mocks instead of mutating process-global env vars).
#[derive(Debug, Clone, Default)]
pub struct IdentityDiscordOverrides {
    /// Wins over ESPECTRAL_ID_URL / config `identity_url`.
    pub identity_url: Option<String>,
    /// Wins over ESPECTRAL_SESSION_SERVER.
    pub session_server: Option<String>,
    /// Wins over ESPECTRAL_DISCORD_MOCK.
    pub discord_mock: Option<bool>,
    /// Replaces the Discord IPC socket candidates (no abstract-socket fallback).
    pub ipc_candidates: Option<Vec<PathBuf>>,
}

/// identity-discord in-memory state. Add fields freely — only this cluster owns it.
#[derive(Debug, Default)]
pub struct IdentityDiscordState {
    pub(crate) identity: identity::IdentityStore,
    pub(crate) discord_auth: discord_auth::DiscordAuthState,
    pub(crate) discord: discord::DiscordIpcState,
    pub(crate) bridge: bridge::BridgeState,
    pub(crate) presence: presence::PresenceState,
    pub(crate) loopback: loopback::LoopbackState,
    overrides: Mutex<IdentityDiscordOverrides>,
}

impl IdentityDiscordState {
    /// Snapshot of the endpoint overrides.
    pub fn overrides(&self) -> IdentityDiscordOverrides {
        self.overrides.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Edit the endpoint overrides (tests, embedding).
    pub fn set_overrides(&self, f: impl FnOnce(&mut IdentityDiscordOverrides)) {
        f(&mut self.overrides.lock().unwrap_or_else(|p| p.into_inner()));
    }
}

impl Engine {
    /// This cluster's in-memory state.
    pub fn identity_discord_state(&self) -> &IdentityDiscordState {
        &self.inner.identity_discord
    }
}

/// Engine start hook: loopback listener + launcher Rich Presence.
pub(crate) async fn on_engine_start(engine: &Engine) -> EngineResult<()> {
    let port = loopback::configured_port();
    if let Err(e) = loopback::start(engine, port).await {
        // Another engine (or the legacy Node one) may hold the port: the mod
        // bridge and the Discord callback are unavailable, the launcher is not.
        tracing::warn!("{e}");
    }
    if engine.config().discord_enabled() {
        let e = engine.clone();
        engine.spawn_background("discord", async move {
            discord::set_presence(
                &e,
                discord::Presence {
                    state: Some("En el launcher".into()),
                    image: Some("espectral_logo".into()),
                    start_timestamp: Some(crate::platform::now_ms()),
                    ..Default::default()
                },
            )
            .await
        });
    }
    Ok(())
}

/// Engine shutdown hook: discord.shutdown() + stop presence publishers.
pub(crate) async fn on_engine_shutdown(engine: &Engine) {
    presence::reset(engine);
    discord::shutdown(engine).await;
}

/// A Discord user as the launcher shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordUser {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// GET /api/discord/auth/start -> `{ url, state }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordAuthStart {
    pub url: String,
    pub state: String,
}

/// GET /api/discord/claim -> `{ status: 'pending'|'invalid'|'ok', sessionId?, linked?, uuid?, user? }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordClaim {
    pub status: String,
    #[serde(rename = "sessionId", default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<DiscordUser>,
}

/// discordAuth.handleCallback result; the loopback renders the HTML page
/// (success page with postMessage, or a 302 to tauri://localhost/#/ for
/// `redirect=tauri|deep`) — not needed by the native UI itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordCallbackOutcome {
    /// None for identity-link callbacks.
    pub session_id: Option<String>,
    pub user: Option<DiscordUser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked: Option<bool>,
}

/// GET /api/discord/me -> `{ user }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordMeReply {
    pub user: Option<DiscordUser>,
}

/// `:uuid` param -> (flat uuid, account): 400 BAD_UUID / 404 NOT_FOUND.
fn find_account_by_uuid(engine: &Engine, uuid: &str) -> EngineResult<(String, Account)> {
    let Some(flat) = identity::normalize_uuid(uuid) else {
        return Err(EngineError::bad_request("BAD_UUID", "uuid must be a 32-char hex string"));
    };
    accounts::list_accounts(engine)
        .into_iter()
        .find(|a| identity::normalize_uuid(&a.uuid).as_deref() == Some(flat.as_str()))
        .map(|a| (flat.clone(), a))
        .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("no account with uuid {uuid}")))
}

impl Engine {
    /// GET /api/identity -> `{ service_url, accounts, error? }`.
    pub async fn identity_state(&self) -> EngineResult<IdentityState> {
        let accounts =
            accounts::list_accounts(self).iter().map(|a| identity::account_identity_entry(self, a)).collect();
        Ok(IdentityState { service_url: identity::identity_url(self), accounts, error: None })
    }

    /// POST /api/identity/:uuid/verify (400 BAD_UUID, 404 NOT_FOUND).
    pub async fn verify_identity(&self, uuid: &str) -> EngineResult<IdentityAccount> {
        let (_flat, account) = find_account_by_uuid(self, uuid)?;
        identity::verify_account(self, &account).await?;
        Ok(identity::account_identity_entry(self, &account))
    }

    /// GET /api/identity/:uuid/discord/start -> `{ url }` (409 NOT_VERIFIED).
    pub async fn identity_discord_start(&self, uuid: &str) -> EngineResult<UrlReply> {
        let start = self.identity_discord_start_with_state(uuid).await?;
        Ok(UrlReply { url: start.url })
    }

    /// Like [`Engine::identity_discord_start`] but also returns the OAuth
    /// `state`, so the native UI can poll [`Engine::discord_claim`] for the
    /// link outcome (the JS UI parsed it out of the URL).
    pub async fn identity_discord_start_with_state(&self, uuid: &str) -> EngineResult<DiscordAuthStart> {
        let (flat, _account) = find_account_by_uuid(self, uuid)?;
        if identity::token_for(self, &flat).is_none() {
            return Err(EngineError::conflict("NOT_VERIFIED", "verify this account before linking Discord"));
        }
        let (url, state) = discord_auth::auth_start_url(self, Some(&flat));
        Ok(DiscordAuthStart { url, state })
    }

    /// DELETE /api/identity/:uuid/discord.
    pub async fn identity_discord_unlink(&self, uuid: &str) -> EngineResult<IdentityAccount> {
        let (flat, account) = find_account_by_uuid(self, uuid)?;
        identity::discord_unlink(self, &flat).await?;
        Ok(identity::account_identity_entry(self, &account))
    }

    /// POST /api/identity/:uuid/refresh (also rewrites cosmetics for running games).
    pub async fn identity_refresh(&self, uuid: &str) -> EngineResult<IdentityAccount> {
        let (flat, account) = find_account_by_uuid(self, uuid)?;
        identity::refresh_grant(self, &flat, 5000).await?;
        // cosmetics.rewriteForRunning(flat)
        for launch in bridge::launches_for(self, &flat) {
            if let Err(e) = crate::cosmetics::write_cosmetics_file(self, &flat, &launch.game_dir, None) {
                tracing::warn!("[cosmetics] rewrite for running instance failed: {e}");
            }
        }
        Ok(identity::account_identity_entry(self, &account))
    }

    /// GET /api/discord/auth/start.
    pub async fn discord_auth_start(&self) -> EngineResult<DiscordAuthStart> {
        let (url, state) = discord_auth::auth_start_url(self, None);
        Ok(DiscordAuthStart { url, state })
    }

    /// GET /api/discord/claim?state= (one-time).
    pub async fn discord_claim(&self, state: &str) -> EngineResult<DiscordClaim> {
        Ok(discord_auth::claim_session_user(self, state))
    }

    /// GET /api/discord/callback (query: code, state, error, error_description, redirect).
    pub async fn discord_callback(&self, query: HashMap<String, String>) -> EngineResult<DiscordCallbackOutcome> {
        discord_auth::handle_callback(self, &query).await
    }

    /// GET /api/discord/me — `session_id` is what the UI got from discord_claim
    /// (replaces the cookie / Authorization / x-discord-session headers).
    pub async fn discord_me(&self, session_id: Option<&str>, mock: bool) -> EngineResult<DiscordMeReply> {
        Ok(DiscordMeReply { user: discord_auth::get_session_user(self, session_id, mock) })
    }

    /// POST /api/discord/logout -> `{ ok: true }`.
    pub async fn discord_logout(&self, session_id: Option<&str>) -> EngineResult<OkReply> {
        discord_auth::destroy_session(self, session_id);
        Ok(OkReply::TRUE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::tests::{point_at, seed_token, UUID};
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use crate::types::TokenKind;
    use serde_json::json;

    fn add_account(e: &Engine) {
        let acc = Account {
            username: "TestMsa".into(),
            uuid: "aabbccdd-eeff-0011-2233-445566778899".into(),
            token_kind: TokenKind::Msa,
            ..Default::default()
        };
        e.config().set_accounts(&[acc], None);
    }

    #[tokio::test]
    async fn identity_state_lists_accounts() {
        let (_d, e) = temp_engine();
        e.identity_discord_state().set_overrides(|o| o.identity_url = Some("http://127.0.0.1:9/".into()));
        add_account(&e);
        let s = e.identity_state().await.unwrap();
        assert_eq!(s.service_url, "http://127.0.0.1:9");
        assert_eq!(s.accounts.len(), 1);
        assert_eq!(s.accounts[0].uuid, UUID);
        assert!(!s.accounts[0].verified);
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("error").is_none());
        assert_eq!(v["accounts"][0]["discord"], serde_json::Value::Null);
        assert_eq!(v["accounts"][0]["grant_exp"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn uuid_param_errors() {
        let (_d, e) = temp_engine();
        add_account(&e);
        assert!(e.verify_identity("xyz").await.unwrap_err().has_code("BAD_UUID"));
        let nf = e.verify_identity(&"f".repeat(32)).await.unwrap_err();
        assert_eq!((nf.status(), nf.raw_code()), (404, Some("NOT_FOUND")));
        let nv = e.identity_discord_start(UUID).await.unwrap_err();
        assert_eq!((nv.status(), nv.raw_code()), (409, Some("NOT_VERIFIED")));
        assert!(e.identity_discord_unlink(UUID).await.unwrap_err().has_code("NOT_VERIFIED"));
    }

    #[tokio::test]
    async fn link_start_unlink_and_refresh() {
        let (_d, e) = temp_engine();
        add_account(&e);
        let g = crate::grant::tests::sign_test_grant(UUID, &["supporter"], 3600);
        let server = MockServer::start(move |r| match (r.method.as_str(), r.path.as_str()) {
            ("GET", "/v1/grant") => MockResponse::json(&json!({ "grant": g, "flags": ["supporter"], "exp": 1 })),
            ("DELETE", "/v1/discord/link") => MockResponse::status(204),
            _ => MockResponse::status(404),
        })
        .await;
        point_at(&e, &server);
        e.identity_discord_state().set_overrides(|o| o.discord_mock = Some(false));
        seed_token(&e, UUID, "tok");
        let r = e.identity_discord_start(UUID).await.unwrap();
        assert!(r.url.starts_with("https://discord.com/oauth2/authorize?"), "{}", r.url);
        let with_state = e.identity_discord_start_with_state(UUID).await.unwrap();
        assert_eq!(e.discord_claim(&with_state.state).await.unwrap().status, "pending");

        crate::identity::store_discord(&e, UUID, &crate::discord_auth::mock_user()).unwrap();
        let before = e.identity_state().await.unwrap();
        assert_eq!(before.accounts[0].discord.as_ref().unwrap().username, "TestPlayer");
        let after = e.identity_discord_unlink(UUID).await.unwrap();
        assert_eq!(after.discord, None);
        assert_eq!(after.flags, vec!["supporter"]);
        assert!(after.grant_exp.unwrap() > crate::grant::now_s(), "valid grant is the authority on expiry");

        let refreshed = e.identity_refresh(UUID).await.unwrap();
        assert!(refreshed.verified);
        assert!(server.requests().iter().filter(|r| r.path == "/v1/grant").count() >= 2);
    }

    #[tokio::test]
    async fn discord_session_facade() {
        let (_d, e) = temp_engine();
        e.identity_discord_state().set_overrides(|o| o.discord_mock = Some(true));
        let start = e.discord_auth_start().await.unwrap();
        let mut q = HashMap::new();
        q.insert("code".to_string(), "mock_code".to_string());
        q.insert("state".to_string(), start.state.clone());
        let out = e.discord_callback(q).await.unwrap();
        let claim = e.discord_claim(&start.state).await.unwrap();
        assert_eq!(claim.status, "ok");
        assert_eq!(claim.session_id, out.session_id);
        let sid = claim.session_id.unwrap();
        let v = serde_json::to_value(e.discord_claim(&start.state).await.unwrap()).unwrap();
        assert_eq!(v, json!({ "status": "invalid" }));
        assert_eq!(e.discord_me(Some(&sid), false).await.unwrap().user.unwrap().name, "TestPlayer");
        assert_eq!(e.discord_logout(Some(&sid)).await.unwrap(), OkReply::TRUE);
        assert_eq!(e.discord_me(Some(&sid), false).await.unwrap().user, None);
    }

    #[tokio::test]
    async fn start_hook_never_fails_and_shutdown_stops_it() {
        let (d, e) = temp_engine();
        e.config().set("discord_enabled", json!(false));
        e.identity_discord_state().set_overrides(|o| o.ipc_candidates = Some(vec![d.path().join("none")]));
        // The configured port may be taken on a dev box; the hook must not fail either way.
        on_engine_start(&e).await.unwrap();
        on_engine_shutdown(&e).await;
        e.shutdown().await;
    }
}
