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
//! State to port into [`IdentityDiscordState`]: OAuth states, Discord
//! sessions, Discord IPC connection, bridge launch registry + cache,
//! presence publishers, loopback server handle. Startup: loopback listener
//! (bridge + OAuth callback) and, when `discord_enabled`, the launcher
//! presence `{ state: 'En el launcher', image: 'espectral_logo', startTimestamp: now }`.
//! Shutdown: close the Discord IPC socket.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{IdentityAccount, IdentityState, OkReply};

use super::UrlReply;

/// identity-discord in-memory state. Add fields freely — only this cluster owns it.
#[derive(Debug, Default)]
pub struct IdentityDiscordState {}

impl Engine {
    /// This cluster's in-memory state.
    #[allow(dead_code)]
    pub(crate) fn identity_discord_state(&self) -> &IdentityDiscordState {
        &self.inner.identity_discord
    }
}

/// Engine start hook: loopback listener + launcher Rich Presence.
pub(crate) async fn on_engine_start(_engine: &Engine) -> EngineResult<()> {
    crate::not_ported!("identity_discord::on_engine_start (loopback + presence)")
}

/// Engine shutdown hook: discord.shutdown().
pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

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

impl Engine {
    /// GET /api/identity -> `{ service_url, accounts, error? }`.
    pub async fn identity_state(&self) -> EngineResult<IdentityState> {
        crate::not_ported!("Engine::identity_state")
    }

    /// POST /api/identity/:uuid/verify (400 BAD_UUID, 404 NOT_FOUND).
    pub async fn verify_identity(&self, uuid: &str) -> EngineResult<IdentityAccount> {
        let _ = uuid;
        crate::not_ported!("Engine::verify_identity")
    }

    /// GET /api/identity/:uuid/discord/start -> `{ url }` (409 NOT_VERIFIED).
    pub async fn identity_discord_start(&self, uuid: &str) -> EngineResult<UrlReply> {
        let _ = uuid;
        crate::not_ported!("Engine::identity_discord_start")
    }

    /// DELETE /api/identity/:uuid/discord.
    pub async fn identity_discord_unlink(&self, uuid: &str) -> EngineResult<IdentityAccount> {
        let _ = uuid;
        crate::not_ported!("Engine::identity_discord_unlink")
    }

    /// POST /api/identity/:uuid/refresh (also rewrites cosmetics for running games).
    pub async fn identity_refresh(&self, uuid: &str) -> EngineResult<IdentityAccount> {
        let _ = uuid;
        crate::not_ported!("Engine::identity_refresh")
    }

    /// GET /api/discord/auth/start.
    pub async fn discord_auth_start(&self) -> EngineResult<DiscordAuthStart> {
        crate::not_ported!("Engine::discord_auth_start")
    }

    /// GET /api/discord/claim?state= (one-time).
    pub async fn discord_claim(&self, state: &str) -> EngineResult<DiscordClaim> {
        let _ = state;
        crate::not_ported!("Engine::discord_claim")
    }

    /// GET /api/discord/callback (query: code, state, error, error_description, redirect).
    pub async fn discord_callback(&self, query: HashMap<String, String>) -> EngineResult<DiscordCallbackOutcome> {
        let _ = query;
        crate::not_ported!("Engine::discord_callback")
    }

    /// GET /api/discord/me — `session_id` is what the UI got from discord_claim
    /// (replaces the cookie / Authorization / x-discord-session headers).
    pub async fn discord_me(&self, session_id: Option<&str>, mock: bool) -> EngineResult<DiscordMeReply> {
        let _ = (session_id, mock);
        crate::not_ported!("Engine::discord_me")
    }

    /// POST /api/discord/logout -> `{ ok: true }`.
    pub async fn discord_logout(&self, session_id: Option<&str>) -> EngineResult<OkReply> {
        let _ = session_id;
        crate::not_ported!("Engine::discord_logout")
    }
}
