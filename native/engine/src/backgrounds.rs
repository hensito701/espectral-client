//! Port of `src/engine/backgrounds.mjs` — menu background packs
//! (`<data>/backgrounds/<id>/pack.json` + frames), personalization write, and
//! staging the selected pack into `<gameDir>/config/espectral-menu-bg/<id>/`.
//!
//! **Owner: accounts-skins.** Status: STUB.
//!
//! JS exports to port: MAX_FRAMES MAX_FRAME_W MAX_FRAME_H MAX_FRAME_PACK_BYTES
//! MAX_IMAGE_DIM MAX_IMAGE_FILE_BYTES MAX_NAME backgroundsRoot packDir
//! isValidPackId readPackMeta listPacks packPreviewPath importImagePack
//! importFramesPack deletePack writePersonalization selectedBackground
//! migrateLegacyBackground stageBackgroundPack.
//! Tests: tests/backgrounds.test.mjs, personalization-routes.test.mjs.
//! writePersonalization goes through identity-discord's
//! crate::client::patch_client_config (the personalization whitelist owner).
//!
//! CONTRACT (called by game-launch resolveLaunch): stage_background_pack,
//! migrate_legacy_background.

use std::path::{Path, PathBuf};

use crate::engine::Engine;
use crate::error::EngineResult;

/// Stage `pack_id` (or clear the staging dir for None) under `<gameDir>/config/espectral-menu-bg/`.
pub fn stage_background_pack(engine: &Engine, pack_id: Option<&str>, game_dir: &Path) -> EngineResult<()> {
    let _ = (engine, pack_id, game_dir);
    crate::not_ported!("backgrounds::stage_background_pack")
}

/// Move a pre-store legacy background into the pack store (idempotent).
pub async fn migrate_legacy_background(engine: &Engine, instance_name: &str, dirs: &[PathBuf]) -> EngineResult<()> {
    let _ = (engine, instance_name, dirs);
    crate::not_ported!("backgrounds::migrate_legacy_background")
}
