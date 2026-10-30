//! Facade — cluster **content** (routes/instances.mjs, mods.mjs, mrpack.mjs,
//! import.mjs, servers.mjs, client.mjs, and the instance-icon routes of
//! misc.mjs).
//!
//! | JS route                                               | method |
//! |--------------------------------------------------------|--------|
//! | GET    /api/instances                                  | [`Engine::list_instances`] |
//! | POST   /api/instances                                  | [`Engine::create_instance`] |
//! | GET    /api/instances/:name                            | [`Engine::get_instance_detail`] |
//! | DELETE /api/instances/:name                            | [`Engine::delete_instance`] |
//! | PATCH  /api/instances/:name                            | [`Engine::patch_instance`] |
//! | GET    /api/instances/:name/servers                    | [`Engine::instance_servers`] |
//! | PUT    /api/instances/:name/servers                    | [`Engine::put_instance_servers`] |
//! | GET    /api/instances/:name/options                    | [`Engine::instance_options`] |
//! | POST   /api/instances/:name/options/import             | [`Engine::import_instance_options`] |
//! | POST   /api/instances/:name/icon                       | [`Engine::upload_instance_icon`] |
//! | DELETE /api/instances/:name/icon                       | [`Engine::remove_instance_icon`] |
//! | GET    /api/instances/:name/icon                       | [`Engine::instance_icon`] |
//! | GET    /api/instances/:name/mods                       | [`Engine::list_mods`] |
//! | GET    /api/instances/:name/mods-dir                   | [`Engine::mods_dir`] |
//! | POST   /api/instances/:name/mods/:filename/enable      | [`Engine::set_mod_enabled`] (true) |
//! | POST   /api/instances/:name/mods/:filename/disable     | [`Engine::set_mod_enabled`] (false) |
//! | POST   /api/instances/:name/mods/install               | [`Engine::install_mod_preset`] |
//! | GET    /api/instances/:name/mods/preset                | [`Engine::mod_preset_info`] |
//! | GET    /api/modrinth/search                            | [`Engine::search_modrinth`] |
//! | POST   /api/instances/:name/mods/install-modrinth      | [`Engine::install_modrinth_mod`] |
//! | POST   /api/instances/import-mrpack                    | [`Engine::import_mrpack`] |
//! | GET    /api/import/sources                             | [`Engine::import_sources`] |
//! | POST   /api/instances/:name/import                     | [`Engine::import_profile`] |
//! | GET    /api/servers                                    | [`Engine::server_statuses`] |
//! | GET    /api/instances/:name/client                     | [`Engine::get_client_info`] |
//! | PATCH  /api/instances/:name/client                     | [`Engine::patch_client`] |
//!
//! Guards that read game-launch state use `Engine::active_instances()`
//! (rename / delete-while-running → 409 ALREADY_RUNNING; `running` flags).
//! routes/mods.mjs codes are lowercase and verbatim (EngineError::api_verbatim).

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::types::{
    ClientInfo, ClientPatch, CreateInstanceRequest, ImportResult, ImportSource, InstanceDetail, InstancePatch,
    InstanceSummary, ModEntry, ModInstallPlan, ModPreset, ModPresetInfo, ModrinthSearchResult, OptionsPair,
    OverwritePolicy, ServerEntry, ServerStatus,
};

use super::{DeletedReply, PathReply, QueuedReply};

/// content in-memory state (server-status cache, mod-set hash memo, …).
#[derive(Debug, Default)]
pub struct ContentState {}

impl Engine {
    /// This cluster's in-memory state.
    #[allow(dead_code)]
    pub(crate) fn content_state(&self) -> &ContentState {
        &self.inner.content
    }
}

pub(crate) async fn on_engine_start(_engine: &Engine) -> EngineResult<()> {
    Ok(())
}

pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

/// `{ servers }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServersReply {
    pub servers: Vec<ServerEntry>,
}

/// PUT servers -> `{ count }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CountReply {
    pub count: usize,
}

/// `{ options }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionsReply {
    pub options: Vec<OptionsPair>,
}

/// POST options/import -> `{ copied, count, options }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionsImportReply {
    pub copied: bool,
    pub count: usize,
    pub options: Vec<OptionsPair>,
}

/// Icon upload/remove -> `{ ok, has_icon }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconReply {
    pub ok: bool,
    pub has_icon: bool,
}

/// Mod toggle -> `{ filename, enabled }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModToggleReply {
    pub filename: String,
    pub enabled: bool,
}

/// GET /api/modrinth/search query (`q`, `version`, `loader`, `instance`, `offset`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModrinthSearchQuery {
    /// Empty = explore mode (most downloaded).
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub offset: u64,
}

/// POST install-modrinth body.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InstallModrinthRequest {
    pub project_id: String,
    /// Install despite declared conflicts (else 409 `mod_conflicts`).
    #[serde(default)]
    pub force: bool,
}

/// POST /api/instances/import-mrpack body.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportMrpackRequest {
    /// .mrpack file or unpacked pack dir (400 BAD_PATH when empty).
    pub path: String,
    /// >= 512 (400 BAD_MEMORY).
    #[serde(default)]
    pub memory_mb: Option<u32>,
}

/// import-mrpack -> `{ summary, already_exists? }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportMrpackReply {
    pub summary: InstanceSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub already_exists: Option<bool>,
}

/// POST /api/instances/:name/import body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportProfileRequest {
    pub source_id: String,
    pub overwrite_policy: OverwritePolicy,
}

/// GET /api/servers -> `{ servers }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerStatusesReply {
    pub servers: Vec<ServerStatus>,
}

impl Engine {
    /// GET /api/instances -> summaries with `running`.
    pub async fn list_instances(&self) -> EngineResult<Vec<InstanceSummary>> {
        crate::not_ported!("Engine::list_instances")
    }

    /// POST /api/instances -> InstanceSummary (409 INVALID_NAME / ALREADY_EXISTS /
    /// FABRIC_UNAVAILABLE, 400 VERSION_REQUIRED / BAD_LOADER / LOADER_VERSION_REQUIRED / BAD_HUE / BAD_GAME_DIR).
    pub async fn create_instance(&self, req: CreateInstanceRequest) -> EngineResult<InstanceSummary> {
        let _ = req;
        crate::not_ported!("Engine::create_instance")
    }

    /// GET /api/instances/:name -> `{ summary(+running), mods, servers, options, aot }`
    /// (also retro-seeds branding in the background).
    pub async fn get_instance_detail(&self, name: &str) -> EngineResult<InstanceDetail> {
        let _ = name;
        crate::not_ported!("Engine::get_instance_detail")
    }

    /// DELETE /api/instances/:name -> `{ deleted, trash }` (moved to `<name>.deleted.<ms>`).
    pub async fn delete_instance(&self, name: &str) -> EngineResult<DeletedReply> {
        let _ = name;
        crate::not_ported!("Engine::delete_instance")
    }

    /// PATCH /api/instances/:name -> InstanceSummary (rename blocked while running: 409 ALREADY_RUNNING).
    pub async fn patch_instance(&self, name: &str, patch: InstancePatch) -> EngineResult<InstanceSummary> {
        let _ = (name, patch);
        crate::not_ported!("Engine::patch_instance")
    }

    /// GET /api/instances/:name/servers -> `{ servers }` (servers.dat via crate::nbt).
    pub async fn instance_servers(&self, name: &str) -> EngineResult<ServersReply> {
        let _ = name;
        crate::not_ported!("Engine::instance_servers")
    }

    /// PUT /api/instances/:name/servers `{ servers }` -> `{ count }`.
    pub async fn put_instance_servers(&self, name: &str, servers: Vec<ServerEntry>) -> EngineResult<CountReply> {
        let _ = (name, servers);
        crate::not_ported!("Engine::put_instance_servers")
    }

    /// GET /api/instances/:name/options -> `{ options: [[key, value]…] }`.
    pub async fn instance_options(&self, name: &str) -> EngineResult<OptionsReply> {
        let _ = name;
        crate::not_ported!("Engine::instance_options")
    }

    /// POST /api/instances/:name/options/import `{ source }` (400 BAD_REQUEST when empty).
    pub async fn import_instance_options(&self, name: &str, source: &str) -> EngineResult<OptionsImportReply> {
        let _ = (name, source);
        crate::not_ported!("Engine::import_instance_options")
    }

    /// POST /api/instances/:name/icon `{ image_base64 }` (util::decode_png_image).
    pub async fn upload_instance_icon(&self, name: &str, image_base64: &str) -> EngineResult<IconReply> {
        let _ = (name, image_base64);
        crate::not_ported!("Engine::upload_instance_icon")
    }

    /// DELETE /api/instances/:name/icon -> `{ ok, has_icon: false }`.
    pub async fn remove_instance_icon(&self, name: &str) -> EngineResult<IconReply> {
        let _ = name;
        crate::not_ported!("Engine::remove_instance_icon")
    }

    /// GET /api/instances/:name/icon -> PNG bytes (404 NOT_FOUND when absent).
    pub async fn instance_icon(&self, name: &str) -> EngineResult<Vec<u8>> {
        let _ = name;
        crate::not_ported!("Engine::instance_icon")
    }

    /// GET /api/instances/:name/mods -> ModEntry[] (500 `mods_list_failed`).
    pub async fn list_mods(&self, name: &str) -> EngineResult<Vec<ModEntry>> {
        let _ = name;
        crate::not_ported!("Engine::list_mods")
    }

    /// GET /api/instances/:name/mods-dir -> `{ path }` (effective dir, created).
    pub async fn mods_dir(&self, name: &str) -> EngineResult<PathReply> {
        let _ = name;
        crate::not_ported!("Engine::mods_dir")
    }

    /// POST .../mods/:filename/enable|disable -> `{ filename, enabled }`.
    pub async fn set_mod_enabled(&self, name: &str, filename: &str, enabled: bool) -> EngineResult<ModToggleReply> {
        let _ = (name, filename, enabled);
        crate::not_ported!("Engine::set_mod_enabled")
    }

    /// POST .../mods/install `{ preset }` -> `{ queued: true }` (background install).
    pub async fn install_mod_preset(&self, name: &str, preset: ModPreset) -> EngineResult<QueuedReply> {
        let _ = (name, preset);
        crate::not_ported!("Engine::install_mod_preset")
    }

    /// GET .../mods/preset -> `{ supported, loader, note, branding }`.
    pub async fn mod_preset_info(&self, name: &str) -> EngineResult<ModPresetInfo> {
        let _ = name;
        crate::not_ported!("Engine::mod_preset_info")
    }

    /// GET /api/modrinth/search -> `{ results, total, offset, limit }` (502 `modrinth_search_failed`).
    pub async fn search_modrinth(&self, query: ModrinthSearchQuery) -> EngineResult<ModrinthSearchResult> {
        let _ = query;
        crate::not_ported!("Engine::search_modrinth")
    }

    /// POST .../mods/install-modrinth -> plan summary + `queued: true`
    /// (409 `mod_conflicts` unless force; 400 `modrinth_project_id_required`).
    pub async fn install_modrinth_mod(&self, name: &str, req: InstallModrinthRequest) -> EngineResult<ModInstallPlan> {
        let _ = (name, req);
        crate::not_ported!("Engine::install_modrinth_mod")
    }

    /// POST /api/instances/import-mrpack -> `{ summary, already_exists? }` (files land in the background).
    pub async fn import_mrpack(&self, req: ImportMrpackRequest) -> EngineResult<ImportMrpackReply> {
        let _ = req;
        crate::not_ported!("Engine::import_mrpack")
    }

    /// GET /api/import/sources -> ImportSource[].
    pub async fn import_sources(&self) -> EngineResult<Vec<ImportSource>> {
        crate::not_ported!("Engine::import_sources")
    }

    /// POST /api/instances/:name/import (409 INVALID_NAME, 404 NOT_FOUND, 400 BAD_REQUEST).
    pub async fn import_profile(&self, name: &str, req: ImportProfileRequest) -> EngineResult<ImportResult> {
        let _ = (name, req);
        crate::not_ported!("Engine::import_profile")
    }

    /// GET /api/servers -> `{ servers }` (150 s cache).
    pub async fn server_statuses(&self) -> EngineResult<ServerStatusesReply> {
        crate::not_ported!("Engine::server_statuses")
    }

    /// GET /api/instances/:name/client -> `{ config, registry, supported }` (crate::client).
    pub async fn get_client_info(&self, name: &str) -> EngineResult<ClientInfo> {
        let _ = name;
        crate::not_ported!("Engine::get_client_info")
    }

    /// PATCH /api/instances/:name/client -> `{ config, errors }` (crate::client::patch_client_config).
    pub async fn patch_client(&self, name: &str, patch: ClientPatch) -> EngineResult<ClientInfo> {
        let _ = (name, patch);
        crate::not_ported!("Engine::patch_client")
    }
}
