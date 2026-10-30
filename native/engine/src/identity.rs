//! Port of `src/engine/identity.mjs` — espectral-id client: uuid ownership
//! proof (Mojang join + service verify), signed grants (crate::grant),
//! flags, Discord link/unlink, `<data>/identity.json`.
//!
//! **Owner: identity-discord.** Status: STUB (normalize_uuid implemented).
//!
//! JS exports to port: DEFAULT_IDENTITY_URL identityUrl (precedence:
//! ESPECTRAL_ID_URL env, then config identity_url, then the default; trailing
//! slash trimmed — implemented below as `identity_url`) identityPath
//! loadIdentityState getEntry updateEntry tokenFor flagsFor serviceFetch
//! normalizeDiscordIdentity refreshGrant refreshGrantSoft verifyAccount
//! discordIdentify discordLink accountIdentityEntry discordUnlink.
//! Tests: tests/identity.test.mjs, config-identity-migration.test.mjs
//! (identityUrl part; the config migration itself is ported in crate::config).
//! ESPECTRAL_SESSION_SERVER overrides the Mojang session server (tests).
//!
//! CONTRACT (called by game-launch, accounts-skins cosmetics, bridge):
//! normalize_uuid, identity_url, flags_for, refresh_grant_soft.

use crate::engine::Engine;
use crate::error::EngineResult;

pub use crate::config::DEFAULT_IDENTITY_URL;

/// Any uuid form (dashes ok) -> 32 lowercase hex, None when malformed.
pub fn normalize_uuid(uuid: &str) -> Option<String> {
    let flat: String = uuid.chars().filter(|c| *c != '-').collect::<String>().to_lowercase();
    (flat.len() == 32 && flat.chars().all(|c| c.is_ascii_hexdigit())).then_some(flat)
}

/// Effective service URL: ESPECTRAL_ID_URL env > config > default, no trailing slash.
pub fn identity_url(engine: &Engine) -> String {
    let raw = std::env::var("ESPECTRAL_ID_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| engine.config().identity_url().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| DEFAULT_IDENTITY_URL.to_string());
    raw.trim().trim_end_matches('/').to_string()
}

/// Verified flags for a uuid (`{ flags }`: grant-valid flags, else last-known while verified).
pub fn flags_for(engine: &Engine, flat_uuid: &str) -> EngineResult<Vec<String>> {
    let _ = (engine, flat_uuid);
    crate::not_ported!("identity::flags_for")
}

/// Refresh the grant within `budget_ms`; Ok(false) on timeout/failure (never blocks a launch).
pub async fn refresh_grant_soft(engine: &Engine, flat_uuid: &str, budget_ms: u64) -> EngineResult<bool> {
    let _ = (engine, flat_uuid, budget_ms);
    crate::not_ported!("identity::refresh_grant_soft")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_uuid_parity() {
        assert_eq!(
            normalize_uuid("B50AD385-829D-3141-A216-7E7D7539BA7F").as_deref(),
            Some("b50ad385829d3141a2167e7d7539ba7f")
        );
        assert_eq!(
            normalize_uuid("b50ad385829d3141a2167e7d7539ba7f").as_deref(),
            Some("b50ad385829d3141a2167e7d7539ba7f")
        );
        assert_eq!(normalize_uuid("xyz"), None);
        assert_eq!(normalize_uuid(""), None);
    }

    #[test]
    fn identity_url_defaults_and_trims() {
        let (_d, e) = crate::test_support::temp_engine();
        if std::env::var("ESPECTRAL_ID_URL").is_err() {
            assert_eq!(identity_url(&e), "https://id.uhce.top");
            e.config().set("identity_url", serde_json::json!("https://id.example.test/v1/"));
            assert_eq!(identity_url(&e), "https://id.example.test/v1");
        }
    }
}
