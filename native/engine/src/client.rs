//! Port of `src/engine/client.mjs` — the Espectral Client mod config
//! `<gameDir>/config/espectral-client.json` (schema 2): suite registry
//! (`src/engine/suite-registry.json`), feature defaults, macros,
//! personalization (theme / hud_theme / background / reduced_motion /
//! tagline), seeding at launch, PATCH reconciliation.
//!
//! **Owner: identity-discord.** Status: STUB.
//!
//! JS exports to port: SUITE_DEFAULT_ENABLED CLIENT_CONFIG_SCHEMA REGISTRY
//! FEATURE_DEFAULTS clientConfigPath HUD_THEME_IDS PERSONALIZATION_DEFAULTS
//! TAGLINE_MAX loadClientConfig isSupported getClientInfo seedClientConfig
//! patchClientConfig. Tests: tests/client-config.test.mjs,
//! suite-registry.test.mjs. Registry JSON: include_str! the repo file
//! (`concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/engine/suite-registry.json")`).
//! THEME_IDS lives in crate::config::ACCENT_IDS (shared with PUT accent).
//!
//! CONTRACT (called by game-launch resolveLaunch, accounts-skins
//! backgrounds, content routes/client facade): load_client_config,
//! seed_client_config, get_client_info, patch_client_config.

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{ClientConfig, ClientInfo, ClientPatch};

pub use crate::config::ACCENT_IDS as THEME_IDS;
pub const HUD_THEME_IDS: [&str; 4] = ["cristal", "oro", "minimo", "pixel"];
pub const TAGLINE_MAX: usize = 40;

pub fn load_client_config(engine: &Engine, instance_name: &str) -> EngineResult<ClientConfig> {
    let _ = (engine, instance_name);
    crate::not_ported!("client::load_client_config")
}

/// Write/upgrade the canonical espectral-client.json before a launch.
pub fn seed_client_config(engine: &Engine, instance_name: &str) -> EngineResult<()> {
    let _ = (engine, instance_name);
    crate::not_ported!("client::seed_client_config")
}

pub async fn get_client_info(engine: &Engine, instance_name: &str) -> EngineResult<ClientInfo> {
    let _ = (engine, instance_name);
    crate::not_ported!("client::get_client_info")
}

pub async fn patch_client_config(engine: &Engine, instance_name: &str, patch: ClientPatch) -> EngineResult<ClientInfo> {
    let _ = (engine, instance_name, patch);
    crate::not_ported!("client::patch_client_config")
}
