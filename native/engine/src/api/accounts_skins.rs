//! Facade — cluster **accounts-skins** (routes/skins.mjs, cosmetics.mjs,
//! backgrounds.mjs, and the account / avatar / MSA / skin routes of misc.mjs).
//!
//! | JS route                                       | method |
//! |------------------------------------------------|--------|
//! | GET    /api/accounts                           | [`Engine::list_accounts`] |
//! | POST   /api/accounts                           | [`Engine::create_account`] |
//! | POST   /api/accounts/active                    | [`Engine::set_active_account`] |
//! | POST   /api/accounts/:username/avatar          | [`Engine::upload_account_avatar`] |
//! | DELETE /api/accounts/:username/avatar          | [`Engine::remove_account_avatar`] |
//! | GET    /api/accounts/:username/avatar          | [`Engine::account_avatar`] |
//! | POST   /api/accounts/:username/avatar-color    | [`Engine::set_account_avatar_color`] |
//! | GET    /api/accounts/microsoft/client-id       | [`Engine::msa_client_id`] |
//! | PUT    /api/accounts/microsoft/client-id       | [`Engine::set_msa_client_id`] |
//! | POST   /api/accounts/microsoft/device-code     | [`Engine::msa_device_code`] |
//! | POST   /api/accounts/microsoft/poll            | [`Engine::msa_poll`] |
//! | POST   /api/accounts/microsoft/logout          | [`Engine::msa_logout`] |
//! | GET    /api/accounts/:username/skin            | [`Engine::account_skin`] |
//! | GET    /api/accounts/:username/skin.png        | [`Engine::account_skin_png`] |
//! | POST   /api/accounts/:username/skin            | [`Engine::upload_account_skin`] |
//! | DELETE /api/accounts/:username/skin            | [`Engine::reset_account_skin`] |
//! | GET    /api/skins                              | [`Engine::list_library_skins`] |
//! | GET    /api/skins/:id.png                      | [`Engine::library_skin_png`] |
//! | POST   /api/skins                              | [`Engine::save_library_skin`] |
//! | PATCH  /api/skins/:id                          | [`Engine::patch_library_skin`] |
//! | DELETE /api/skins/:id                          | [`Engine::delete_library_skin`] |
//! | POST   /api/skins/:id/apply                    | [`Engine::apply_library_skin`] |
//! | POST   /api/skins/import-vanilla               | [`Engine::import_vanilla_skins`] |
//! | GET    /api/cosmetics                          | [`Engine::get_cosmetics`] |
//! | PUT    /api/cosmetics/:uuid                    | [`Engine::put_cosmetics`] |
//! | POST   /api/bridge/visible                     | [`Engine::bridge_visible`] (called by crate::loopback) |
//! | GET    /api/backgrounds                        | [`Engine::list_backgrounds`] |
//! | POST   /api/backgrounds                        | [`Engine::import_image_background`] |
//! | POST   /api/backgrounds/frames                 | [`Engine::import_frames_background`] |
//! | DELETE /api/backgrounds/:id                    | [`Engine::delete_background`] |
//! | GET    /api/backgrounds/:id/image              | [`Engine::background_image`] |
//! | PATCH  /api/instances/:name/personalization    | [`Engine::patch_personalization`] |
//!
//! State to port into [`AccountsSkinsState`]: MSA device flows
//! (flow_id -> device_code, interval, transient streak with 5/10/15 s
//! backoff), in-memory MC access tokens. Startup: prewarmMsaToken.
//! Opening the MSA verification URL in the browser: crate::platform::open_url.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{
    Account, BackgroundPack, ClientInfo, CosmeticsAccountState, CosmeticsPatch, CosmeticsState, MsDeviceFlow, OkReply,
    PersonalizationPatch, PublicAccount, SkinInfo, SkinLibraryEntry, SkinVariant,
};

use super::{DeletedReply, RemovedReply};

/// accounts-skins in-memory state. Add fields freely — only this cluster owns it.
#[derive(Debug, Default)]
pub struct AccountsSkinsState {}

impl Engine {
    /// This cluster's in-memory state.
    #[allow(dead_code)]
    pub(crate) fn accounts_skins_state(&self) -> &AccountsSkinsState {
        &self.inner.accounts_skins
    }
}

/// Engine start hook: prewarm the active MSA account's token (best-effort).
pub(crate) async fn on_engine_start(_engine: &Engine) -> EngineResult<()> {
    crate::not_ported!("accounts_skins::on_engine_start (prewarmMsaToken)")
}

pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

/// POST /api/accounts/active -> `{ active }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveReply {
    pub active: String,
}

/// Avatar upload/remove -> `{ ok, has_avatar }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvatarReply {
    pub ok: bool,
    pub has_avatar: bool,
}

/// `{ client_id }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientIdReply {
    pub client_id: String,
}

/// POST /api/accounts/microsoft/poll -> pending (keep polling) or the signed-in account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MsaPollOutcome {
    Pending {
        pending: bool,
        /// seconds; present for slow_down / transient back-off
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after: Option<u64>,
    },
    Account(PublicAccount),
}

/// Skin upload / apply -> `{ ok: true, variant }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinUploadReply {
    pub ok: bool,
    pub variant: SkinVariant,
}

/// DELETE skin -> `{ reset: true }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResetReply {
    pub reset: bool,
}

/// GET /api/skins -> `{ skins }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinsReply {
    pub skins: Vec<SkinLibraryEntry>,
}

/// POST /api/skins body.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SaveSkinRequest {
    pub image_base64: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub variant: Option<SkinVariant>,
}

/// PATCH /api/skins/:id body (400 BAD_SKIN_PATCH when both absent).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SkinPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<SkinVariant>,
}

/// POST /api/skins/import-vanilla -> `{ imported, skipped, total }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VanillaImportReply {
    pub imported: u32,
    pub skipped: u32,
    pub total: u32,
}

/// One visible player answered to the mod.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgePlayer {
    pub uuid: String,
    /// signed grant (compact JWS) or null
    pub grant: Option<String>,
    pub loadout: Option<IndexMap<String, Option<String>>>,
}

/// POST /api/bridge/visible -> `{ players }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeVisibleReply {
    pub players: Vec<BridgePlayer>,
}

/// GET /api/backgrounds -> `{ packs }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacksReply {
    pub packs: Vec<BackgroundPack>,
}

/// POST backgrounds -> `{ pack }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackReply {
    pub pack: BackgroundPack,
}

/// POST /api/backgrounds body: `{ name, image_base64? | source_path? }`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportImagePackRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// POST /api/backgrounds/frames body: `{ name, fps?, frames[]? | frame_paths[]? | source_path? }`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportFramesPackRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    /// base64 PNG frames
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

impl Engine {
    /// GET /api/accounts -> publicAccount[] (never tokens).
    pub async fn list_accounts(&self) -> EngineResult<Vec<PublicAccount>> {
        crate::not_ported!("Engine::list_accounts")
    }

    /// POST /api/accounts `{ username }` -> the new offline account (400 INVALID_USERNAME, 409 ALREADY_EXISTS).
    pub async fn create_account(&self, username: &str) -> EngineResult<Account> {
        let _ = username;
        crate::not_ported!("Engine::create_account")
    }

    /// POST /api/accounts/active `{ username }` -> `{ active }` (404 NOT_FOUND).
    pub async fn set_active_account(&self, username: &str) -> EngineResult<ActiveReply> {
        let _ = username;
        crate::not_ported!("Engine::set_active_account")
    }

    /// POST /api/accounts/:username/avatar `{ image_base64 }`.
    pub async fn upload_account_avatar(&self, username: &str, image_base64: &str) -> EngineResult<AvatarReply> {
        let _ = (username, image_base64);
        crate::not_ported!("Engine::upload_account_avatar")
    }

    /// DELETE /api/accounts/:username/avatar.
    pub async fn remove_account_avatar(&self, username: &str) -> EngineResult<AvatarReply> {
        let _ = username;
        crate::not_ported!("Engine::remove_account_avatar")
    }

    /// GET /api/accounts/:username/avatar -> PNG bytes (404 when absent).
    pub async fn account_avatar(&self, username: &str) -> EngineResult<Vec<u8>> {
        let _ = username;
        crate::not_ported!("Engine::account_avatar")
    }

    /// POST /api/accounts/:username/avatar-color `{ avatar_color: 0..359 | null }` -> publicAccount.
    pub async fn set_account_avatar_color(
        &self,
        username: &str,
        avatar_color: Option<i64>,
    ) -> EngineResult<PublicAccount> {
        let _ = (username, avatar_color);
        crate::not_ported!("Engine::set_account_avatar_color")
    }

    /// GET /api/accounts/microsoft/client-id -> `{ client_id }` (config msa_client_id or msauth default).
    pub async fn msa_client_id(&self) -> EngineResult<ClientIdReply> {
        crate::not_ported!("Engine::msa_client_id")
    }

    /// PUT /api/accounts/microsoft/client-id (400 BAD_CLIENT_ID unless `^[0-9a-fA-F-]{10,}$`).
    pub async fn set_msa_client_id(&self, client_id: &str) -> EngineResult<ClientIdReply> {
        let _ = client_id;
        crate::not_ported!("Engine::set_msa_client_id")
    }

    /// POST /api/accounts/microsoft/device-code — starts the flow and opens
    /// the verification URL in the system browser.
    pub async fn msa_device_code(&self) -> EngineResult<MsDeviceFlow> {
        crate::not_ported!("Engine::msa_device_code")
    }

    /// POST /api/accounts/microsoft/poll `{ flow_id }` (404 MSA_FLOW_NOT_FOUND).
    pub async fn msa_poll(&self, flow_id: &str) -> EngineResult<MsaPollOutcome> {
        let _ = flow_id;
        crate::not_ported!("Engine::msa_poll")
    }

    /// POST /api/accounts/microsoft/logout `{ username }` -> `{ deleted }`.
    pub async fn msa_logout(&self, username: &str) -> EngineResult<DeletedReply> {
        let _ = username;
        crate::not_ported!("Engine::msa_logout")
    }

    /// GET /api/accounts/:username/skin -> `{ username, variant, has_skin, cape }`.
    pub async fn account_skin(&self, username: &str) -> EngineResult<SkinInfo> {
        let _ = username;
        crate::not_ported!("Engine::account_skin")
    }

    /// GET /api/accounts/:username/skin.png -> PNG (1h cache; 404 NO_SKIN, 502 SKIN_FETCH_FAILED).
    pub async fn account_skin_png(&self, username: &str) -> EngineResult<Vec<u8>> {
        let _ = username;
        crate::not_ported!("Engine::account_skin_png")
    }

    /// POST /api/accounts/:username/skin `{ image_base64, variant }` (400 BAD_SKIN_VARIANT / BAD_SKIN_DIMS).
    pub async fn upload_account_skin(
        &self,
        username: &str,
        image_base64: &str,
        variant: SkinVariant,
    ) -> EngineResult<SkinUploadReply> {
        let _ = (username, image_base64, variant);
        crate::not_ported!("Engine::upload_account_skin")
    }

    /// DELETE /api/accounts/:username/skin -> `{ reset: true }`.
    pub async fn reset_account_skin(&self, username: &str) -> EngineResult<ResetReply> {
        let _ = username;
        crate::not_ported!("Engine::reset_account_skin")
    }

    /// GET /api/skins.
    pub async fn list_library_skins(&self) -> EngineResult<SkinsReply> {
        crate::not_ported!("Engine::list_library_skins")
    }

    /// GET /api/skins/:id.png -> PNG bytes.
    pub async fn library_skin_png(&self, id: &str) -> EngineResult<Vec<u8>> {
        let _ = id;
        crate::not_ported!("Engine::library_skin_png")
    }

    /// POST /api/skins -> the saved entry.
    pub async fn save_library_skin(&self, req: SaveSkinRequest) -> EngineResult<SkinLibraryEntry> {
        let _ = req;
        crate::not_ported!("Engine::save_library_skin")
    }

    /// PATCH /api/skins/:id -> the updated entry.
    pub async fn patch_library_skin(&self, id: &str, patch: SkinPatch) -> EngineResult<SkinLibraryEntry> {
        let _ = (id, patch);
        crate::not_ported!("Engine::patch_library_skin")
    }

    /// DELETE /api/skins/:id -> `{ removed: true }`.
    pub async fn delete_library_skin(&self, id: &str) -> EngineResult<RemovedReply> {
        let _ = id;
        crate::not_ported!("Engine::delete_library_skin")
    }

    /// POST /api/skins/:id/apply `{ username }` -> `{ ok, variant }` (MSA only).
    pub async fn apply_library_skin(&self, id: &str, username: &str) -> EngineResult<SkinUploadReply> {
        let _ = (id, username);
        crate::not_ported!("Engine::apply_library_skin")
    }

    /// POST /api/skins/import-vanilla.
    pub async fn import_vanilla_skins(&self) -> EngineResult<VanillaImportReply> {
        crate::not_ported!("Engine::import_vanilla_skins")
    }

    /// GET /api/cosmetics -> `{ catalog, accounts: { <flat uuid>: state } }`.
    pub async fn get_cosmetics(&self) -> EngineResult<CosmeticsState> {
        crate::not_ported!("Engine::get_cosmetics")
    }

    /// PUT /api/cosmetics/:uuid (400 BAD_UUID, 404 NOT_FOUND, validation codes from cosmetics.mjs).
    pub async fn put_cosmetics(&self, uuid: &str, patch: CosmeticsPatch) -> EngineResult<CosmeticsAccountState> {
        let _ = (uuid, patch);
        crate::not_ported!("Engine::put_cosmetics")
    }

    /// POST /api/bridge/visible — `bridge_token` from the x-espectral-bridge
    /// header (403 BRIDGE_AUTH when unknown); answers from cache, queues the rest.
    pub async fn bridge_visible(&self, bridge_token: &str, uuids: Vec<String>) -> EngineResult<BridgeVisibleReply> {
        let _ = (bridge_token, uuids);
        crate::not_ported!("Engine::bridge_visible")
    }

    /// GET /api/backgrounds.
    pub async fn list_backgrounds(&self) -> EngineResult<PacksReply> {
        crate::not_ported!("Engine::list_backgrounds")
    }

    /// POST /api/backgrounds (image pack).
    pub async fn import_image_background(&self, req: ImportImagePackRequest) -> EngineResult<PackReply> {
        let _ = req;
        crate::not_ported!("Engine::import_image_background")
    }

    /// POST /api/backgrounds/frames (frame pack).
    pub async fn import_frames_background(&self, req: ImportFramesPackRequest) -> EngineResult<PackReply> {
        let _ = req;
        crate::not_ported!("Engine::import_frames_background")
    }

    /// DELETE /api/backgrounds/:id -> `{ ok }`.
    pub async fn delete_background(&self, id: &str) -> EngineResult<OkReply> {
        let _ = id;
        crate::not_ported!("Engine::delete_background")
    }

    /// GET /api/backgrounds/:id/image -> preview PNG bytes.
    pub async fn background_image(&self, id: &str) -> EngineResult<Vec<u8>> {
        let _ = id;
        crate::not_ported!("Engine::background_image")
    }

    /// PATCH /api/instances/:name/personalization -> ClientInfo (400 BAD_PERSONALIZATION;
    /// `background` must name an existing pack).
    pub async fn patch_personalization(&self, name: &str, patch: PersonalizationPatch) -> EngineResult<ClientInfo> {
        let _ = (name, patch);
        crate::not_ported!("Engine::patch_personalization")
    }
}
