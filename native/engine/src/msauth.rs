//! Port of `src/engine/msauth.mjs` — Microsoft device-code login, XBL/XSTS
//! -> Minecraft token exchange, Lunar refresh-token path, token refresh +
//! `<data>/mc-token-cache.json`, profile skin get/upload/reset.
//!
//! **Owner: accounts-skins.** Status: STUB.
//!
//! JS exports to port: clientId (ESPECTRAL_MSA_CLIENT_ID / config
//! msa_client_id / default) timers startDeviceLogin checkDeviceToken
//! pollDeviceToken exchangeForMinecraft refreshLunarMsaToken
//! exchangeLunarForMinecraft refreshMinecraftToken ensureMinecraftToken
//! prewarmMsaToken getProfileSkin uploadProfileSkin resetProfileSkin.
//! Tests: tests/msauth.test.mjs, skin.test.mjs. Error codes the device-flow
//! poll route keys on: MSA_PENDING MSA_SLOW_DOWN MSA_TRANSIENT MSA_NETWORK.
//! Access tokens live in memory (AccountsSkinsState) + the token cache file,
//! NEVER in config.json (ConfigStore strips them anyway).
//!
//! CONTRACT (called by game-launch resolveAccountFor and identity verify):
//! ensure_minecraft_token, prewarm_msa_token.

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::Account;

/// A usable Minecraft access token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McToken {
    pub access_token: String,
    /// ms epoch.
    pub expires_at: i64,
}

/// Fresh-or-cached MC token for an MSA account (None for offline accounts).
pub async fn ensure_minecraft_token(engine: &Engine, account: &Account) -> EngineResult<Option<McToken>> {
    let _ = (engine, account);
    crate::not_ported!("msauth::ensure_minecraft_token")
}

/// Startup prewarm of the active MSA account's token (best-effort).
pub async fn prewarm_msa_token(engine: &Engine) -> EngineResult<()> {
    let _ = engine;
    crate::not_ported!("msauth::prewarm_msa_token")
}
