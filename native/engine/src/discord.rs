//! Port of `src/engine/discord.mjs` — Discord Rich Presence over the local
//! IPC socket (Windows named pipe `\\?\pipe\discord-ipc-N`, unix
//! `$XDG_RUNTIME_DIR/discord-ipc-N` and friends).
//!
//! **Owner: identity-discord.** Status: STUB.
//!
//! JS exports to port: DISCORD_CLIENT_ID setPresence clearPresence shutdown.
//! Tests: tests/linux-compat.test.mjs (IPC path candidates).
//! IPC: tokio::net::windows::named_pipe::ClientOptions on Windows,
//! tokio::net::UnixStream elsewhere. Startup presence ("En el launcher",
//! image espectral_logo) is set from IdentityDiscordState's start hook when
//! `config.discord_enabled()`.
//!
//! CONTRACT (called by game-launch): set_presence, clear_presence.

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::EngineResult;

/// setPresence() argument (JS object keys).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub details: Option<String>,
    pub state: Option<String>,
    /// ms epoch
    pub start_timestamp: Option<i64>,
    pub end_timestamp: Option<i64>,
    pub image: Option<String>,
    pub small_image: Option<String>,
    /// default "Espectral Client"
    pub large_text: Option<String>,
}

pub async fn set_presence(engine: &Engine, presence: Presence) -> EngineResult<()> {
    let _ = (engine, presence);
    crate::not_ported!("discord::set_presence")
}

pub async fn clear_presence(engine: &Engine) -> EngineResult<()> {
    let _ = engine;
    crate::not_ported!("discord::clear_presence")
}
