//! Port of `src/engine/cosmetics.mjs` — supporter cosmetics: catalog
//! (`src/engine/cosmetics-catalog.json`), per-uuid state
//! (`<data>/cosmetics.json`), loadout validation vs verified flags +
//! exclusivity, the per-gameDir `config/espectral-cosmetics.json` the mod reads.
//!
//! **Owner: accounts-skins.** Status: STUB.
//!
//! JS exports to port: catalog cosmeticsPath loadCosmeticsState
//! getCosmeticsState validateLoadout sanitizeLoadout rewriteForRunning
//! cosmeticsDoc writeCosmeticsFile patchCosmetics cosmeticsAccountView.
//! Tests: tests/cosmetics.test.mjs.
//! Load the catalog with
//! `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/engine/cosmetics-catalog.json"))`
//! so the JSON keeps a single source of truth.
//!
//! CONTRACT (called by game-launch resolveLaunch, identity-discord presence):
//! write_cosmetics_file, share_enabled.

use std::path::Path;

use crate::engine::Engine;
use crate::error::EngineResult;

/// Write `<gameDir>/config/espectral-cosmetics.json` (and the same bytes into
/// the canonical game dir when it differs).
pub fn write_cosmetics_file(
    engine: &Engine,
    flat_uuid: &str,
    game_dir: &Path,
    canonical_game_dir: Option<&Path>,
) -> EngineResult<()> {
    let _ = (engine, flat_uuid, game_dir, canonical_game_dir);
    crate::not_ported!("cosmetics::write_cosmetics_file")
}

/// `getCosmeticsState(uuid).share` — affirmative consent to publish presence.
pub fn share_enabled(engine: &Engine, flat_uuid: &str) -> EngineResult<bool> {
    let _ = (engine, flat_uuid);
    crate::not_ported!("cosmetics::share_enabled")
}
