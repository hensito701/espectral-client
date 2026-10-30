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
//! [`AccountsSkinsState`] holds the MSA device flows (flow_id -> device
//! code, interval, transient streak with 5/10/15 s backoff), the in-memory
//! Minecraft access tokens, the prewarm dedupe locks and the injectable
//! endpoints / test hooks. Startup: `prewarm_msa_token` in the background.
//!
//! `bridge_visible` stays a stub: its logic (token registry + presence
//! cache) lives in `bridge.rs`, owned by identity-discord, which has no
//! contract function for it yet — wire `bridge::authenticate` +
//! `bridge::visible` at integration.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::msauth::{DevicePoll, McToken};
use crate::types::{
    Account, BackgroundPack, ClientInfo, CosmeticsAccountState, CosmeticsPatch, CosmeticsState, MsDeviceFlow, OkReply,
    PersonalizationPatch, PublicAccount, SkinInfo, SkinLibraryEntry, SkinVariant,
};

use super::{DeletedReply, RemovedReply};

/// M8: transient poll failures tolerated per flow, each waiting a little longer.
const TRANSIENT_BACKOFF_SECONDS: [u64; 3] = [5, 10, 15];

/// One active device-login flow (memory only).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MsaFlow {
    pub device_code: String,
    pub interval: u64,
    pub transient_streak: usize,
}

/// accounts-skins in-memory state. Add fields freely — only this cluster owns it.
#[derive(Debug, Default)]
pub struct AccountsSkinsState {
    /// flow_id -> device flow.
    pub(crate) msa_flows: Mutex<HashMap<String, MsaFlow>>,
    /// username -> Minecraft access token (never persisted to config.json).
    pub(crate) mc_tokens: Mutex<HashMap<String, McToken>>,
    /// Per-username prewarm dedupe.
    pub(crate) prewarm_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Remote endpoints (tests point them at a loopback mock).
    pub(crate) endpoints: RwLock<crate::msauth::Endpoints>,
    /// Test hook: Some(log) records msauth back-off sleeps instead of sleeping.
    pub(crate) fake_sleeps: Mutex<Option<Vec<u64>>>,
    /// Test hook: launcher_custom_skins.json override.
    pub(crate) vanilla_skins_path: Mutex<Option<PathBuf>>,
    /// Test hook: never spawn the system browser.
    pub(crate) no_browser: AtomicBool,
    /// Serializes cosmetics.json read-modify-write.
    pub(crate) cosmetics_lock: Mutex<()>,
    /// Serializes skin-library.json read-modify-write.
    pub(crate) library_lock: tokio::sync::Mutex<()>,
}

impl Engine {
    /// This cluster's in-memory state.
    pub(crate) fn accounts_skins_state(&self) -> &AccountsSkinsState {
        &self.inner.accounts_skins
    }
}

/// Engine start hook: prewarm the active MSA account's token in the background (best-effort).
pub(crate) async fn on_engine_start(engine: &Engine) -> EngineResult<()> {
    let e = engine.clone();
    engine.spawn_background("msauth-prewarm", async move { crate::msauth::prewarm_msa_token(&e).await });
    Ok(())
}

pub(crate) async fn on_engine_shutdown(engine: &Engine) {
    engine.accounts_skins_state().msa_flows.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

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

fn skin_dims_ok(buf: &[u8]) -> EngineResult<()> {
    if crate::skins::is_valid_skin_png(buf) {
        Ok(())
    } else {
        Err(EngineError::bad_request("BAD_SKIN_DIMS", "skin must be a 64x64 PNG (64x32 legacy accepted)"))
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> EngineResult<T> + Send + 'static) -> EngineResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| EngineError::other(format!("background task failed: {e}")))?
}

impl Engine {
    /// GET /api/accounts -> publicAccount[] (never tokens).
    pub async fn list_accounts(&self) -> EngineResult<Vec<PublicAccount>> {
        Ok(crate::accounts::list_accounts(self).iter().map(|a| crate::accounts::public_account(self, a)).collect())
    }

    /// POST /api/accounts `{ username }` -> the new offline account (400 INVALID_USERNAME, 409 ALREADY_EXISTS).
    pub async fn create_account(&self, username: &str) -> EngineResult<Account> {
        crate::accounts::create_account(self, username)
    }

    /// POST /api/accounts/active `{ username }` -> `{ active }` (404 NOT_FOUND).
    pub async fn set_active_account(&self, username: &str) -> EngineResult<ActiveReply> {
        Ok(ActiveReply { active: crate::accounts::set_active_account(self, username)? })
    }

    /// POST /api/accounts/:username/avatar `{ image_base64 }` (400 BAD_IMAGE, 404 NOT_FOUND).
    pub async fn upload_account_avatar(&self, username: &str, image_base64: &str) -> EngineResult<AvatarReply> {
        let buf = crate::util::decode_png_image(Some(image_base64))?;
        let has_avatar = crate::accounts::write_account_avatar(self, username, &buf)?;
        Ok(AvatarReply { ok: true, has_avatar })
    }

    /// DELETE /api/accounts/:username/avatar.
    pub async fn remove_account_avatar(&self, username: &str) -> EngineResult<AvatarReply> {
        let has_avatar = crate::accounts::remove_account_avatar(self, username)?;
        Ok(AvatarReply { ok: true, has_avatar })
    }

    /// GET /api/accounts/:username/avatar -> PNG bytes (404 when absent).
    pub async fn account_avatar(&self, username: &str) -> EngineResult<Vec<u8>> {
        crate::accounts::read_account_avatar(self, username)
    }

    /// POST /api/accounts/:username/avatar-color `{ avatar_color: 0..359 | null }` -> publicAccount.
    pub async fn set_account_avatar_color(
        &self,
        username: &str,
        avatar_color: Option<i64>,
    ) -> EngineResult<PublicAccount> {
        crate::accounts::set_avatar_color(self, username, avatar_color)
    }

    /// GET /api/accounts/microsoft/client-id -> `{ client_id }` (config msa_client_id or msauth default).
    pub async fn msa_client_id(&self) -> EngineResult<ClientIdReply> {
        Ok(ClientIdReply { client_id: crate::msauth::client_id(self) })
    }

    /// PUT /api/accounts/microsoft/client-id (400 BAD_CLIENT_ID unless `^[0-9a-fA-F-]{10,}$`).
    pub async fn set_msa_client_id(&self, client_id: &str) -> EngineResult<ClientIdReply> {
        let id = client_id.trim();
        if id.len() < 10 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return Err(EngineError::bad_request("BAD_CLIENT_ID", "client_id must be a valid Azure application ID"));
        }
        self.config().set("msa_client_id", serde_json::Value::from(id));
        Ok(ClientIdReply { client_id: id.to_string() })
    }

    /// POST /api/accounts/microsoft/device-code — starts the flow and opens
    /// the verification URL in the system browser (Microsoft's own
    /// verification_uri_complete when it returns one, else verification_uri).
    pub async fn msa_device_code(&self) -> EngineResult<MsDeviceFlow> {
        let flow = crate::msauth::start_device_login(self).await?;
        lock(&self.accounts_skins_state().msa_flows).insert(
            flow.flow_id.clone(),
            MsaFlow { device_code: flow.flow_id.clone(), interval: flow.interval, transient_streak: 0 },
        );
        let url = flow.verification_uri_complete.clone().unwrap_or_else(|| flow.verification_uri.clone());
        let browser = !cfg!(test) && !self.accounts_skins_state().no_browser.load(Ordering::Relaxed);
        if browser && !url.is_empty() {
            if let Err(e) = crate::platform::open_url(&url) {
                // no opener — the UI still shows the code + plain link
                tracing::debug!("[msauth] could not open the browser: {e}");
            }
        }
        Ok(flow)
    }

    /// POST /api/accounts/microsoft/poll `{ flow_id }` (404 MSA_FLOW_NOT_FOUND).
    ///
    /// `{ pending: true[, retry_after] }` while the user has not signed in
    /// (slow_down carries Microsoft's wait; transient failures back off
    /// 5/10/15 s before surfacing), or the signed-in public account.
    pub async fn msa_poll(&self, flow_id: &str) -> EngineResult<MsaPollOutcome> {
        let state = self.accounts_skins_state();
        let Some(flow) = lock(&state.msa_flows).get(flow_id).cloned() else {
            return Err(EngineError::not_found("MSA_FLOW_NOT_FOUND", "device login not found — start again"));
        };
        let set_streak = |n: usize| {
            if let Some(f) = lock(&state.msa_flows).get_mut(flow_id) {
                f.transient_streak = n;
            }
        };
        let transient = |err: Option<EngineError>| -> EngineResult<MsaPollOutcome> {
            let streak = flow.transient_streak + 1;
            if streak <= TRANSIENT_BACKOFF_SECONDS.len() {
                set_streak(streak);
                return Ok(MsaPollOutcome::Pending {
                    pending: true,
                    retry_after: Some(TRANSIENT_BACKOFF_SECONDS[streak - 1]),
                });
            }
            set_streak(0); // budget spent — surface the failure
            Err(err.unwrap_or_else(|| {
                EngineError::api(503, "MSA_TRANSIENT", "Microsoft is temporarily unavailable — will retry")
            }))
        };
        let tokens = match crate::msauth::check_device_token(self, &flow.device_code).await {
            Ok(DevicePoll::Tokens(t)) => t,
            Ok(DevicePoll::Pending) => {
                set_streak(0);
                return Ok(MsaPollOutcome::Pending { pending: true, retry_after: None });
            }
            Ok(DevicePoll::SlowDown(s)) => {
                set_streak(0);
                return Ok(MsaPollOutcome::Pending { pending: true, retry_after: Some(s) });
            }
            Ok(DevicePoll::Transient) => return transient(None),
            Err(e) if e.has_code("MSA_NETWORK") => return transient(Some(e)),
            Err(e) => return Err(e),
        };
        set_streak(0); // a real outcome clears the transient budget
        let mc = crate::msauth::exchange_for_minecraft(self, &tokens.access_token).await?;
        let account = crate::accounts::upsert_msa_account(
            self,
            &crate::accounts::MsaLogin {
                username: mc.username.clone(),
                uuid: mc.uuid.clone(),
                refresh_token: tokens.refresh_token.clone(),
                expires_in: Some(mc.expires_in),
            },
        );
        // Keep the fresh Minecraft token in memory (never on disk in config.json)
        // so the first launch after sign-in does not refresh again.
        crate::msauth::remember_token(
            self,
            &account.username,
            &McToken {
                access_token: mc.access_token,
                expires_at: crate::platform::now_ms() + (mc.expires_in * 1000.0) as i64,
            },
        );
        lock(&state.msa_flows).remove(flow_id);
        Ok(MsaPollOutcome::Account(crate::accounts::public_account(self, &account)))
    }

    /// POST /api/accounts/microsoft/logout `{ username }` -> `{ deleted }`.
    pub async fn msa_logout(&self, username: &str) -> EngineResult<DeletedReply> {
        let r = crate::accounts::delete_msa_account(self, username)?;
        crate::msauth::forget_memory_token(self, username);
        Ok(r)
    }

    /// GET /api/accounts/:username/skin -> `{ username, variant, has_skin, cape }`.
    pub async fn account_skin(&self, username: &str) -> EngineResult<SkinInfo> {
        let (account, token) = crate::skins::resolve_skin_account(self, username).await?;
        let p = crate::msauth::get_profile_skin(self, &token).await?;
        Ok(SkinInfo { username: account.username, variant: p.variant, has_skin: p.has_skin, cape: p.cape })
    }

    /// GET /api/accounts/:username/skin.png -> PNG (1h cache; 404 NO_SKIN, 502 SKIN_FETCH_FAILED).
    pub async fn account_skin_png(&self, username: &str) -> EngineResult<Vec<u8>> {
        let (account, token) = crate::skins::resolve_skin_account(self, username).await?;
        let profile = crate::msauth::get_profile_skin(self, &token).await?;
        let Some(url) = profile.skin_url.filter(|_| profile.has_skin) else {
            return Err(EngineError::not_found(
                "NO_SKIN",
                format!("account '{}' has no custom skin", account.username),
            ));
        };
        if let Some(cached) = crate::skins::read_skin_cache(self, &account.uuid) {
            return Ok(cached);
        }
        let fail = |m: String| EngineError::api(502, "SKIN_FETCH_FAILED", m);
        let res = self
            .http()
            .client()
            .get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| fail(format!("could not fetch skin image: {e}")))?;
        if !res.status().is_success() {
            return Err(fail(format!("could not fetch skin image (HTTP {})", res.status().as_u16())));
        }
        let png = res.bytes().await.map_err(|e| fail(format!("could not fetch skin image: {e}")))?.to_vec();
        crate::skins::write_skin_cache(self, &account.uuid, &png);
        // MSA avatar auto = skin head, unless the user uploaded a custom one.
        crate::skins::sync_avatar_best_effort(self, &account.username, &png);
        Ok(png)
    }

    /// POST /api/accounts/:username/skin `{ image_base64, variant }` (400 BAD_IMAGE / BAD_SKIN_DIMS).
    /// Validates the PNG before touching Mojang, then refreshes the texture cache.
    pub async fn upload_account_skin(
        &self,
        username: &str,
        image_base64: &str,
        variant: SkinVariant,
    ) -> EngineResult<SkinUploadReply> {
        let buf = crate::util::decode_png_image(Some(image_base64))?;
        skin_dims_ok(&buf)?;
        let (account, token) = crate::skins::resolve_skin_account(self, username).await?;
        crate::msauth::upload_profile_skin(self, &token, &buf, variant).await?;
        crate::skins::write_skin_cache(self, &account.uuid, &buf);
        crate::skins::sync_avatar_best_effort(self, &account.username, &buf);
        Ok(SkinUploadReply { ok: true, variant })
    }

    /// DELETE /api/accounts/:username/skin -> `{ reset: true }` (clears the cache).
    pub async fn reset_account_skin(&self, username: &str) -> EngineResult<ResetReply> {
        let (account, token) = crate::skins::resolve_skin_account(self, username).await?;
        crate::msauth::reset_profile_skin(self, &token).await?;
        crate::skins::clear_skin_cache(self, &account.uuid);
        // Skin reset -> drop a skin-derived avatar, keep a user-uploaded one.
        let _ = crate::skins::clear_account_auto_avatar(self, &account.username);
        Ok(ResetReply { reset: true })
    }

    /// GET /api/skins.
    pub async fn list_library_skins(&self) -> EngineResult<SkinsReply> {
        let skins = crate::skins::load_library_index(self).iter().map(crate::skins::public_skin_entry).collect();
        Ok(SkinsReply { skins })
    }

    /// GET /api/skins/:id.png -> PNG bytes (a trailing `.png` on the id is tolerated).
    pub async fn library_skin_png(&self, id: &str) -> EngineResult<Vec<u8>> {
        let id = match id.len().checked_sub(4) {
            Some(cut) if id.is_char_boundary(cut) && id[cut..].eq_ignore_ascii_case(".png") => &id[..cut],
            _ => id,
        };
        crate::skins::read_library_png(self, id)
    }

    /// POST /api/skins -> the saved entry.
    pub async fn save_library_skin(&self, req: SaveSkinRequest) -> EngineResult<SkinLibraryEntry> {
        let png = crate::skins::decode_skin_png(Some(&req.image_base64))?;
        let _g = self.accounts_skins_state().library_lock.lock().await;
        crate::skins::save_library_skin(self, req.name.as_deref(), req.variant, &png, "upload", None)
    }

    /// PATCH /api/skins/:id -> the updated entry.
    pub async fn patch_library_skin(&self, id: &str, patch: SkinPatch) -> EngineResult<SkinLibraryEntry> {
        if patch.name.is_none() && patch.variant.is_none() {
            return Err(EngineError::bad_request("BAD_SKIN_PATCH", "nothing to update (name?, variant?)"));
        }
        let _g = self.accounts_skins_state().library_lock.lock().await;
        crate::skins::update_library_skin(self, id, patch.name.as_deref(), patch.variant)
    }

    /// DELETE /api/skins/:id -> `{ removed: true }`.
    pub async fn delete_library_skin(&self, id: &str) -> EngineResult<RemovedReply> {
        let _g = self.accounts_skins_state().library_lock.lock().await;
        crate::skins::delete_library_skin(self, id)?;
        Ok(RemovedReply { removed: true })
    }

    /// POST /api/skins/:id/apply `{ username }` -> `{ ok, variant }` (MSA only).
    pub async fn apply_library_skin(&self, id: &str, username: &str) -> EngineResult<SkinUploadReply> {
        let entry = crate::skins::get_library_entry(self, id)?;
        let png = crate::skins::read_library_png(self, id)?;
        let (account, token) = crate::skins::resolve_skin_account(self, username).await?;
        let variant = entry.variant();
        crate::msauth::upload_profile_skin(self, &token, &png, variant).await?;
        crate::skins::write_skin_cache(self, &account.uuid, &png);
        crate::skins::sync_avatar_best_effort(self, &account.username, &png);
        Ok(SkinUploadReply { ok: true, variant })
    }

    /// POST /api/skins/import-vanilla.
    pub async fn import_vanilla_skins(&self) -> EngineResult<VanillaImportReply> {
        let path = lock(&self.accounts_skins_state().vanilla_skins_path)
            .clone()
            .unwrap_or_else(crate::skins::default_vanilla_skins_path);
        let _g = self.accounts_skins_state().library_lock.lock().await;
        let r = crate::skins::import_vanilla_skins(self, &path)?;
        Ok(VanillaImportReply { imported: r.imported, skipped: r.skipped, total: r.total })
    }

    /// GET /api/cosmetics -> `{ catalog, accounts: { <flat uuid>: state } }`.
    pub async fn get_cosmetics(&self) -> EngineResult<CosmeticsState> {
        let mut accounts = IndexMap::new();
        for a in crate::accounts::list_accounts(self) {
            let Some(flat) = crate::identity::normalize_uuid(&a.uuid) else { continue };
            let flags = crate::cosmetics::verified_flags(self, &flat);
            accounts.insert(flat.clone(), crate::cosmetics::cosmetics_account_view(self, &flat, &flags));
        }
        Ok(CosmeticsState { catalog: crate::cosmetics::catalog().doc.clone(), accounts })
    }

    /// PUT /api/cosmetics/:uuid (400 BAD_UUID, 404 NOT_FOUND, validation codes from cosmetics.mjs).
    pub async fn put_cosmetics(&self, uuid: &str, patch: CosmeticsPatch) -> EngineResult<CosmeticsAccountState> {
        let flat = crate::identity::normalize_uuid(uuid)
            .ok_or_else(|| EngineError::bad_request("BAD_UUID", "uuid must be a 32-char hex string"))?;
        let known = crate::accounts::list_accounts(self)
            .iter()
            .any(|a| crate::identity::normalize_uuid(&a.uuid).as_deref() == Some(flat.as_str()));
        if !known {
            return Err(EngineError::not_found("NOT_FOUND", format!("no account with uuid {uuid}")));
        }
        let flags = crate::cosmetics::verified_flags(self, &flat);
        let body = serde_json::to_value(&patch)?;
        crate::cosmetics::patch_cosmetics(self, &flat, &body, &flags)?;
        let flags = crate::cosmetics::verified_flags(self, &flat);
        Ok(crate::cosmetics::cosmetics_account_view(self, &flat, &flags))
    }

    /// POST /api/bridge/visible — `bridge_token` from the x-espectral-bridge
    /// header (403 BRIDGE_AUTH when unknown); answers from cache, queues the rest.
    ///
    /// INTEGRATION: the logic (launch token registry, presence cache, batched
    /// queries) is identity-discord's `bridge.rs`; wire
    /// `bridge::authenticate(token)` + `bridge::visible(owner, uuids)` here.
    pub async fn bridge_visible(&self, bridge_token: &str, uuids: Vec<String>) -> EngineResult<BridgeVisibleReply> {
        let _ = (bridge_token, uuids);
        crate::not_ported!("Engine::bridge_visible (bridge.rs authenticate/visible — identity-discord)")
    }

    /// GET /api/backgrounds.
    pub async fn list_backgrounds(&self) -> EngineResult<PacksReply> {
        Ok(PacksReply { packs: crate::backgrounds::list_packs(self) })
    }

    /// POST /api/backgrounds (image pack).
    pub async fn import_image_background(&self, req: ImportImagePackRequest) -> EngineResult<PackReply> {
        let e = self.clone();
        let pack = blocking(move || crate::backgrounds::import_image_pack(&e, &req)).await?;
        Ok(PackReply { pack })
    }

    /// POST /api/backgrounds/frames (frame pack).
    pub async fn import_frames_background(&self, req: ImportFramesPackRequest) -> EngineResult<PackReply> {
        let e = self.clone();
        let pack: BackgroundPack = blocking(move || crate::backgrounds::import_frames_pack(&e, &req)).await?;
        Ok(PackReply { pack })
    }

    /// DELETE /api/backgrounds/:id -> `{ ok }`.
    pub async fn delete_background(&self, id: &str) -> EngineResult<OkReply> {
        crate::backgrounds::delete_pack(self, id)?;
        Ok(OkReply::TRUE)
    }

    /// GET /api/backgrounds/:id/image -> preview PNG bytes.
    pub async fn background_image(&self, id: &str) -> EngineResult<Vec<u8>> {
        let file = crate::backgrounds::pack_preview_path(self, id)?;
        Ok(std::fs::read(file)?)
    }

    /// PATCH /api/instances/:name/personalization -> ClientInfo (400 BAD_PERSONALIZATION;
    /// `background` must name an existing pack).
    pub async fn patch_personalization(&self, name: &str, patch: PersonalizationPatch) -> EngineResult<ClientInfo> {
        crate::backgrounds::write_personalization(self, name, &patch).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msauth::Endpoints;
    use crate::test_support::{temp_engine, MockRequest, MockResponse, MockServer};
    use serde_json::{json, Value};
    use std::sync::atomic::AtomicUsize;

    const TINY: &str =
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    fn env<T: std::fmt::Debug>(r: EngineResult<T>) -> (u16, String) {
        let e = r.unwrap_err().envelope();
        (e.status, e.code)
    }

    async fn mock_engine(
        handler: impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static,
    ) -> (tempfile::TempDir, Engine, MockServer) {
        let srv = MockServer::start(handler).await;
        let (d, e) = temp_engine();
        crate::msauth::set_endpoints(&e, Endpoints::on_base(&srv.url("")));
        *lock(&e.accounts_skins_state().fake_sleeps) = Some(vec![]);
        e.accounts_skins_state().no_browser.store(true, Ordering::Relaxed);
        (d, e, srv)
    }

    #[tokio::test]
    async fn account_avatar_and_color_routes() {
        let (_d, e) = temp_engine();
        let created = e.create_account("HttpAvatar").await.unwrap();
        assert!(created.avatar_color.is_some());
        let list = e.list_accounts().await.unwrap();
        let me = list.iter().find(|a| a.username == "HttpAvatar").unwrap();
        assert!(!me.has_avatar);
        assert_eq!(me.avatar_color, created.avatar_color);
        assert_eq!(env(e.account_avatar("HttpAvatar").await).0, 404);
        assert_eq!(env(e.upload_account_avatar("HttpAvatar", "aGVsbG8=").await), (400, "BAD_IMAGE".into()));
        let put = e.upload_account_avatar("HttpAvatar", TINY).await.unwrap();
        assert_eq!(serde_json::to_value(put).unwrap(), json!({ "ok": true, "has_avatar": true }));
        assert_eq!(e.account_avatar("HttpAvatar").await.unwrap(), crate::util::decode_png_image(Some(TINY)).unwrap());
        assert_eq!(e.set_account_avatar_color("HttpAvatar", Some(77)).await.unwrap().avatar_color, Some(77));
        assert_eq!(env(e.set_account_avatar_color("HttpAvatar", Some(500)).await), (400, "BAD_AVATAR_COLOR".into()));
        assert_eq!(env(e.set_account_avatar_color("NobodyHere", Some(5)).await).0, 404);
        assert_eq!(env(e.upload_account_avatar("NobodyHere", TINY).await).0, 404);
        let del = e.remove_account_avatar("HttpAvatar").await.unwrap();
        assert_eq!(serde_json::to_value(del).unwrap(), json!({ "ok": true, "has_avatar": false }));
        assert_eq!(env(e.account_avatar("HttpAvatar").await).0, 404);
        assert_eq!(e.set_active_account("HttpAvatar").await.unwrap().active, "HttpAvatar");
        assert_eq!(env(e.set_active_account("Ghost").await), (404, "NOT_FOUND".into()));
    }

    #[tokio::test]
    async fn avatar_upload_clears_the_auto_marker() {
        let (_d, e) = temp_engine();
        e.create_account("HookUser").await.unwrap();
        crate::skins::sync_account_avatar_from_skin(
            &e,
            "HookUser",
            &crate::skins::tests::skin_png([255, 0, 0, 255], [0; 4]),
        )
        .unwrap();
        assert_eq!(crate::accounts::get_account(&e, "HookUser").unwrap().avatar_auto, Some(true));
        e.upload_account_avatar("HookUser", TINY).await.unwrap();
        assert_eq!(crate::accounts::get_account(&e, "HookUser").unwrap().avatar_auto, None);
    }

    #[tokio::test]
    async fn client_id_routes() {
        let (_d, e) = temp_engine();
        assert_eq!(env(e.set_msa_client_id("nope").await), (400, "BAD_CLIENT_ID".into()));
        assert_eq!(env(e.set_msa_client_id("zzzzzzzzzzzz").await).1, "BAD_CLIENT_ID");
        let r = e.set_msa_client_id("  0123456789-abcdef ").await.unwrap();
        assert_eq!(r.client_id, "0123456789-abcdef");
        assert_eq!(e.msa_client_id().await.unwrap().client_id, "0123456789-abcdef");
        assert_eq!(e.config().get_str("msa_client_id").as_deref(), Some("0123456789-abcdef"));
    }

    fn device_flow_server(
        token_answers: Arc<Mutex<Vec<MockResponse>>>,
    ) -> impl Fn(&MockRequest) -> MockResponse + Send + Sync {
        move |r: &MockRequest| match r.path.as_str() {
            "/consumers/oauth2/v2.0/devicecode" => MockResponse::json(&json!({
                "device_code": "dev-1", "user_code": "ABCD", "verification_uri": "https://www.microsoft.com/link",
                "expires_in": 900, "interval": 5
            })),
            "/consumers/oauth2/v2.0/token" => {
                let mut q = token_answers.lock().unwrap();
                if q.is_empty() {
                    MockResponse::json(&json!({ "access_token": "msa-at", "refresh_token": "msa-rt" }))
                } else {
                    q.remove(0)
                }
            }
            "/user/authenticate" => MockResponse::json(&json!({ "Token": "xbl" })),
            "/xsts/authorize" => {
                MockResponse::json(&json!({ "Token": "xsts", "DisplayClaims": { "xui": [{ "uhs": "42" }] } }))
            }
            "/authentication/login_with_xbox" => {
                MockResponse::json(&json!({ "access_token": "MC-AT", "expires_in": 86400 }))
            }
            "/minecraft/profile" => {
                MockResponse::json(&json!({ "id": "0123456789abcdef0123456789abcdef", "name": "Online" }))
            }
            _ => MockResponse::status(404),
        }
    }

    #[tokio::test]
    async fn device_flow_polls_back_off_then_sign_in() {
        let answers = Arc::new(Mutex::new(vec![
            MockResponse::json(&json!({ "error": "authorization_pending" })).with_status(400),
            MockResponse::json(&json!({ "error": "slow_down", "retry_after": 9 })).with_status(400),
            MockResponse::json(&json!({ "error": "temporarily_unavailable" })).with_status(503),
            MockResponse::drop_connection(),
        ]));
        let (_d, e, _srv) = mock_engine(device_flow_server(answers)).await;
        assert_eq!(env(e.msa_poll("unknown").await), (404, "MSA_FLOW_NOT_FOUND".into()));
        let flow = e.msa_device_code().await.unwrap();
        assert_eq!(flow.flow_id, "dev-1");
        let poll = |v: MsaPollOutcome| serde_json::to_value(v).unwrap();
        assert_eq!(poll(e.msa_poll("dev-1").await.unwrap()), json!({ "pending": true }));
        assert_eq!(poll(e.msa_poll("dev-1").await.unwrap()), json!({ "pending": true, "retry_after": 9 }));
        assert_eq!(poll(e.msa_poll("dev-1").await.unwrap()), json!({ "pending": true, "retry_after": 5 }));
        assert_eq!(poll(e.msa_poll("dev-1").await.unwrap()), json!({ "pending": true, "retry_after": 10 }));
        let MsaPollOutcome::Account(acct) = e.msa_poll("dev-1").await.unwrap() else { panic!("expected account") };
        assert_eq!(acct.username, "Online");
        assert_eq!(acct.token_kind, crate::types::TokenKind::Msa);
        let text = serde_json::to_string(&acct).unwrap();
        assert!(!text.contains("msa-rt") && !text.contains("MC-AT"), "{text}");
        assert_eq!(env(e.msa_poll("dev-1").await).1, "MSA_FLOW_NOT_FOUND", "flow consumed");
        // refresh token persisted, MC token only in memory
        let raw = std::fs::read_to_string(e.paths().config_json()).unwrap();
        assert!(raw.contains("msa-rt") && !raw.contains("MC-AT"));
        let stored = crate::accounts::get_account(&e, "Online").unwrap();
        assert_eq!(crate::msauth::ensure_minecraft_token(&e, &stored).await.unwrap().unwrap().access_token, "MC-AT");
        assert_eq!(e.config().active_username().as_deref(), Some("Online"));
        // logout removes the account
        assert_eq!(e.msa_logout("Online").await.unwrap().deleted, "Online");
        assert_eq!(env(e.msa_logout("Online").await).0, 404);
    }

    #[tokio::test]
    async fn transient_budget_is_spent_then_surfaced() {
        let answers = Arc::new(Mutex::new(
            (0..4)
                .map(|_| MockResponse::json(&json!({ "error": "server_error" })).with_status(500))
                .collect::<Vec<_>>(),
        ));
        let (_d, e, _srv) = mock_engine(device_flow_server(answers)).await;
        e.msa_device_code().await.unwrap();
        for wait in [5, 10, 15] {
            assert_eq!(
                serde_json::to_value(e.msa_poll("dev-1").await.unwrap()).unwrap(),
                json!({ "pending": true, "retry_after": wait })
            );
        }
        assert_eq!(env(e.msa_poll("dev-1").await), (503, "MSA_TRANSIENT".into()));
        // budget reset: the flow survives
        assert!(matches!(e.msa_poll("dev-1").await.unwrap(), MsaPollOutcome::Account(_)));
    }

    #[tokio::test]
    async fn skin_routes_validate_before_any_network() {
        let (_d, e) = temp_engine();
        let png = |w: u32, h: u32| crate::util::base64_encode(&crate::skins::tests::png_header(w, h));
        assert_eq!(
            env(e.upload_account_skin("nobody", &png(16, 16), SkinVariant::Classic).await),
            (400, "BAD_SKIN_DIMS".into())
        );
        for (w, h) in [(64, 64), (64, 32)] {
            assert_eq!(
                env(e.upload_account_skin("nobody", &png(w, h), SkinVariant::Slim).await),
                (404, "UNKNOWN_ACCOUNT".into())
            );
        }
        assert_eq!(env(e.account_skin("ghost").await), (404, "UNKNOWN_ACCOUNT".into()));
        e.create_account("Off").await.unwrap();
        assert_eq!(env(e.account_skin("Off").await), (404, "OFFLINE_ACCOUNT".into()));
    }

    fn skin_server(hits: Arc<AtomicUsize>, skin: Vec<u8>) -> impl Fn(&MockRequest) -> MockResponse + Send + Sync {
        move |r: &MockRequest| match (r.method.as_str(), r.path.as_str()) {
            ("GET", "/minecraft/profile") => {
                let host = r.header("host").unwrap_or_default();
                MockResponse::json(&json!({
                    "id": "0123456789abcdef0123456789abcdef", "name": "Sk",
                    "skins": [{ "state": "ACTIVE", "url": format!("http://{host}/texture/abc"), "variant": "SLIM" }],
                    "capes": []
                }))
            }
            ("GET", "/texture/abc") => {
                hits.fetch_add(1, Ordering::SeqCst);
                MockResponse::bytes(skin.clone())
            }
            ("POST", "/minecraft/profile/skins") => MockResponse::json(&json!({ "id": "x" })),
            ("DELETE", "/minecraft/profile/skins/active") => MockResponse::bytes(vec![]).with_status(204),
            _ => MockResponse::status(404),
        }
    }

    fn msa_with_token(e: &Engine, username: &str) {
        e.config().set(
            "accounts",
            json!([{ "username": username, "uuid": "01234567-89ab-cdef-0123-456789abcdef", "token_kind": "msa",
                     "microsoft": { "refresh_token": "r" } }]),
        );
        crate::msauth::remember_token(
            e,
            username,
            &McToken { access_token: "tok".into(), expires_at: crate::platform::now_ms() + 3_600_000 },
        );
    }

    #[tokio::test]
    async fn skin_png_is_proxied_cached_and_syncs_the_avatar() {
        let skin = crate::skins::tests::skin_png([255, 0, 0, 255], [0; 4]);
        let hits = Arc::new(AtomicUsize::new(0));
        let (_d, e, _srv) = mock_engine(skin_server(hits.clone(), skin.clone())).await;
        msa_with_token(&e, "Sk");
        let info = e.account_skin("Sk").await.unwrap();
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({ "username": "Sk", "variant": "slim", "has_skin": true, "cape": false })
        );
        assert_eq!(e.account_skin_png("Sk").await.unwrap(), skin);
        assert_eq!(e.account_skin_png("Sk").await.unwrap(), skin);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "second read served from the 1h cache");
        let acct = crate::accounts::get_account(&e, "Sk").unwrap();
        assert_eq!(acct.avatar_auto, Some(true), "MSA avatar auto = skin head");
        // upload + reset
        let b64 = crate::util::base64_encode(&skin);
        let up = e.upload_account_skin("Sk", &b64, SkinVariant::Classic).await.unwrap();
        assert_eq!(serde_json::to_value(up).unwrap(), json!({ "ok": true, "variant": "classic" }));
        assert_eq!(serde_json::to_value(e.reset_account_skin("Sk").await.unwrap()).unwrap(), json!({ "reset": true }));
        let acct = crate::accounts::get_account(&e, "Sk").unwrap();
        assert!(!crate::accounts::has_avatar(&e, &acct), "auto avatar dropped on reset");
        assert!(crate::skins::read_skin_cache(&e, &acct.uuid).is_none());
    }

    #[tokio::test]
    async fn library_routes() {
        let skin = crate::skins::tests::skin_png([1, 2, 3, 255], [0; 4]);
        let (d, e, srv) = mock_engine(skin_server(Arc::default(), skin.clone())).await;
        let saved = e
            .save_library_skin(SaveSkinRequest {
                image_base64: format!("data:image/png;base64,{}", crate::util::base64_encode(&skin)),
                name: Some("mine".into()),
                variant: Some(SkinVariant::Slim),
            })
            .await
            .unwrap();
        assert_eq!(e.list_library_skins().await.unwrap().skins, vec![saved.clone()]);
        assert_eq!(e.library_skin_png(&format!("{}.png", saved.id)).await.unwrap(), skin);
        assert_eq!(e.library_skin_png(&saved.id).await.unwrap(), skin);
        assert_eq!(env(e.patch_library_skin(&saved.id, SkinPatch::default()).await), (400, "BAD_SKIN_PATCH".into()));
        let p =
            e.patch_library_skin(&saved.id, SkinPatch { name: Some("renamed".into()), variant: None }).await.unwrap();
        assert_eq!((p.name.as_str(), p.variant), ("renamed", SkinVariant::Slim));
        msa_with_token(&e, "Sk");
        let applied = e.apply_library_skin(&saved.id, "Sk").await.unwrap();
        assert_eq!(applied.variant, SkinVariant::Slim);
        let upload = srv.requests().into_iter().find(|r| r.path == "/minecraft/profile/skins").unwrap();
        assert!(String::from_utf8_lossy(&upload.body).contains("\r\n\r\nSLIM\r\n"));
        assert_eq!(env(e.apply_library_skin("nope", "Sk").await), (404, "UNKNOWN_SKIN".into()));
        assert_eq!(e.delete_library_skin(&saved.id).await.unwrap(), RemovedReply { removed: true });
        assert_eq!(env(e.delete_library_skin(&saved.id).await).1, "UNKNOWN_SKIN");
        // vanilla import through the injectable path
        *lock(&e.accounts_skins_state().vanilla_skins_path) = Some(d.path().join("missing.json"));
        assert_eq!(env(e.import_vanilla_skins().await), (404, "NO_VANILLA_SKINS".into()));
        let f = d.path().join("launcher_custom_skins.json");
        std::fs::write(
            &f,
            json!({ "customSkins": { "a": { "name": "v1", "skinImage": format!("data:image/png;base64,{}", crate::util::base64_encode(&skin)) } } })
                .to_string(),
        )
        .unwrap();
        *lock(&e.accounts_skins_state().vanilla_skins_path) = Some(f);
        let r = e.import_vanilla_skins().await.unwrap();
        assert_eq!(serde_json::to_value(r).unwrap(), json!({ "imported": 1, "skipped": 0, "total": 1 }));
    }

    #[tokio::test]
    async fn cosmetics_routes() {
        let (_d, e) = temp_engine();
        e.create_account("Cos").await.unwrap();
        let flat = crate::identity::normalize_uuid(&crate::accounts::offline_uuid("Cos")).unwrap();
        let all = e.get_cosmetics().await.unwrap();
        assert_eq!(all.catalog.slots.len(), 4);
        let mine = &all.accounts[&flat];
        assert!(mine.share && mine.emotes && mine.loadout.is_empty() && mine.flags.is_empty());
        assert_eq!(env(e.put_cosmetics("bad", CosmeticsPatch::default()).await), (400, "BAD_UUID".into()));
        assert_eq!(
            env(e.put_cosmetics("ffffffffffffffffffffffffffffffff", CosmeticsPatch::default()).await),
            (404, "NOT_FOUND".into())
        );
        let patch =
            CosmeticsPatch { share: Some(false), hidden_players: Some(vec!["Griefer_1".into()]), ..Default::default() };
        let out = e.put_cosmetics(&crate::accounts::offline_uuid("Cos"), patch).await.unwrap();
        assert!(!out.share);
        assert_eq!(out.hidden_players, ["Griefer_1"]);
        // without verified flags, mvp items are refused
        let mut lo = IndexMap::new();
        lo.insert("espalda".to_string(), Some("alas_eco".to_string()));
        let patch = CosmeticsPatch { loadout: Some(lo), ..Default::default() };
        assert_eq!(env(e.put_cosmetics(&flat, patch).await), (400, "FLAG_REQUIRED".into()));
        assert!(!crate::cosmetics::share_enabled(&e, &flat).unwrap());
        let wire = serde_json::to_value(e.get_cosmetics().await.unwrap()).unwrap();
        assert_eq!(wire["accounts"][&flat]["hidden_players"], json!(["Griefer_1"]));
        assert!(wire["catalog"]["items"].as_array().unwrap().len() >= 10);
    }

    #[tokio::test]
    async fn background_routes() {
        let (_d, e) = temp_engine();
        let mut png = crate::util::PNG_MAGIC.to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&20u32.to_be_bytes());
        png.extend_from_slice(&10u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let b64 = format!("data:image/png;base64,{}", crate::util::base64_encode(&png));
        let PackReply { pack } = e
            .import_image_background(ImportImagePackRequest {
                name: "http pack".into(),
                image_base64: Some(b64.clone()),
                source_path: None,
            })
            .await
            .unwrap();
        assert_eq!((pack.kind.as_str(), pack.width), ("image", 20));
        assert!(e.list_backgrounds().await.unwrap().packs.iter().any(|p| p.id == pack.id));
        assert_eq!(e.background_image(&pack.id).await.unwrap(), png);
        let frames = e
            .import_frames_background(ImportFramesPackRequest {
                name: "anim".into(),
                frames: Some(vec![b64.clone(), b64]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(frames.pack.frame_count, 2);
        let bad = PersonalizationPatch { background: Some(Some("missing-pack".into())), ..Default::default() };
        assert_eq!(env(e.patch_personalization("bg-http", bad).await), (404, "PACK_NOT_FOUND".into()));
        assert_eq!(e.delete_background(&pack.id).await.unwrap(), OkReply::TRUE);
        assert!(!e.list_backgrounds().await.unwrap().packs.iter().any(|p| p.id == pack.id));
        assert_eq!(env(e.background_image(&pack.id).await), (404, "PACK_NOT_FOUND".into()));
        let v: Value = serde_json::to_value(e.list_backgrounds().await.unwrap()).unwrap();
        assert_eq!(v["packs"][0]["type"], "frames");
    }

    #[tokio::test]
    async fn start_hook_prewarms_without_failing() {
        let (_d, e) = temp_engine();
        on_engine_start(&e).await.unwrap();
        on_engine_shutdown(&e).await;
    }
}
