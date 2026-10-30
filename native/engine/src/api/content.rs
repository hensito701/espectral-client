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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{
    ClientInfo, ClientPatch, CreateInstanceRequest, ImportResult, ImportSource, InstanceDetail, InstanceMeta,
    InstancePatch, InstanceSummary, Loader, ModEntry, ModInstallPlan, ModPreset, ModPresetInfo, ModrinthSearchResult,
    OptionsPair, OverwritePolicy, ServerEntry, ServerStatus,
};

use super::{DeletedReply, PathReply, QueuedReply};

/// Tunables / endpoints of the content cluster. Defaults are the JS
/// constants; tests point the endpoints at a loopback `MockServer` and
/// inject the import roots so nothing touches the real network or home dir.
#[derive(Debug, Clone)]
pub struct ContentSettings {
    /// mods.mjs MODRINTH_API.
    pub modrinth_api: String,
    /// servers.mjs status API base (`<base>/3/<host>`).
    pub mcsrvstat_api: String,
    /// mrpack.mjs PACK_FILE_HOSTS (`host` or `host:port`).
    pub pack_file_hosts: Vec<String>,
    /// Accept `http:` pack-file URLs (tests only; the JS requires https).
    pub pack_file_allow_http: bool,
    /// Bundled branding jars dir (None = resolve next to the exe / repo assets).
    pub branding_dir: Option<PathBuf>,
    /// Import detection roots (None = from the environment, like the JS).
    pub import_roots: Option<crate::import::ImportRoots>,
    /// createInstance background work: library seeding + Fabric performance preset.
    pub background_on_create: bool,
    /// mods.mjs fetchJsonWithRetry backoff base (JS 1000 ms * attempt).
    pub modrinth_retry_backoff: Duration,
    /// servers.mjs FETCH_TIMEOUT_MS.
    pub server_status_timeout: Duration,
}

impl Default for ContentSettings {
    fn default() -> Self {
        ContentSettings {
            modrinth_api: crate::mods::MODRINTH_API.to_string(),
            mcsrvstat_api: crate::servers::MCSRVSTAT_API.to_string(),
            pack_file_hosts: crate::mrpack::PACK_FILE_HOSTS.iter().map(|s| s.to_string()).collect(),
            pack_file_allow_http: false,
            branding_dir: None,
            import_roots: None,
            background_on_create: true,
            modrinth_retry_backoff: Duration::from_millis(1000),
            server_status_timeout: Duration::from_millis(10_000),
        }
    }
}

/// content in-memory state (caches that the JS kept at module level).
#[derive(Debug, Default)]
pub struct ContentState {
    pub(crate) settings: RwLock<ContentSettings>,
    /// instances.mjs modSetHashCache: game dir -> (stat key, hash).
    pub(crate) mod_set_hash: Mutex<HashMap<PathBuf, (String, String)>>,
    /// mods.mjs metaCache: Modrinth version id -> version JSON.
    pub(crate) version_meta: Mutex<HashMap<String, Value>>,
    /// mods.mjs bestVersionCache: `project\0version\0loader` -> best version (None = no listed build).
    pub(crate) best_version: Mutex<HashMap<String, Option<Value>>>,
    /// mods.mjs brandingManifestCache: dir -> branding.json (None = absent/broken).
    pub(crate) branding_manifest: Mutex<HashMap<PathBuf, Option<Value>>>,
    /// servers.mjs cache: host -> (fetched_at ms, status).
    pub(crate) server_status: Mutex<HashMap<String, (i64, ServerStatus)>>,
    /// reqwest client that never follows redirects (mrpack pack-file host checks).
    pub(crate) no_redirect_client: OnceLock<reqwest::Client>,
}

impl Engine {
    /// This cluster's in-memory state.
    pub(crate) fn content_state(&self) -> &ContentState {
        &self.inner.content
    }

    /// Current content settings (endpoints, import roots, background toggles).
    pub fn content_settings(&self) -> ContentSettings {
        self.inner.content.settings.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Change the content settings (tests point endpoints at a mock server).
    #[doc(hidden)]
    pub fn update_content_settings(&self, f: impl FnOnce(&mut ContentSettings)) {
        let mut g = self.inner.content.settings.write().unwrap_or_else(|p| p.into_inner());
        f(&mut g);
    }
}

pub(crate) async fn on_engine_start(_engine: &Engine) -> EngineResult<()> {
    Ok(())
}

pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

/// routes/mods.mjs `sendError(err, fallbackStatus, fallbackCode)`: the
/// error's own integer status and code win, else the fallbacks; the code is
/// sent verbatim (no UPPER_SNAKE normalization).
pub(crate) fn send_error(err: EngineError, fallback_status: u16, fallback_code: &str) -> EngineError {
    match err {
        EngineError::NotImplemented(_) => err,
        EngineError::Api { status, code, message, .. } => EngineError::api_verbatim(status, code, message),
        EngineError::Http(ref h) => {
            let status = h.status().unwrap_or(fallback_status);
            let code = err.raw_code().unwrap_or(fallback_code).to_string();
            EngineError::api_verbatim(status, code, err.to_string())
        }
        other => EngineError::api_verbatim(fallback_status, fallback_code, other.to_string()),
    }
}

/// routes/import.mjs NAME_RE (no dots, unlike the instances regex).
fn is_import_route_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=40).contains(&n) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
}

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
    fn with_running(&self, mut s: InstanceSummary) -> InstanceSummary {
        s.running = Some(self.active_instances().contains(&s.name));
        s
    }

    /// GET /api/instances -> summaries with `running`.
    pub async fn list_instances(&self) -> EngineResult<Vec<InstanceSummary>> {
        let list = crate::instances::list_instances(self).await?;
        Ok(list.into_iter().map(|s| self.with_running(s)).collect())
    }

    /// POST /api/instances -> InstanceSummary (409 INVALID_NAME / ALREADY_EXISTS /
    /// FABRIC_UNAVAILABLE, 400 VERSION_REQUIRED / BAD_LOADER / LOADER_VERSION_REQUIRED / BAD_HUE / BAD_GAME_DIR).
    pub async fn create_instance(&self, req: CreateInstanceRequest) -> EngineResult<InstanceSummary> {
        crate::instances::create_instance(self, req).await
    }

    /// GET /api/instances/:name -> `{ summary(+running), mods, servers, options, aot }`
    /// (also retro-seeds branding in the background).
    pub async fn get_instance_detail(&self, name: &str) -> EngineResult<InstanceDetail> {
        let mut detail = crate::instances::get_instance_detail(self, name).await?;
        // Retro-seed branding for old instances that predate the preset.
        let bg = self.clone();
        let inst = InstanceMeta {
            name: detail.summary.name.clone(),
            version: detail.summary.version.clone(),
            loader: detail.summary.loader,
            game_dir: detail.summary.game_dir.clone(),
            ..Default::default()
        };
        self.spawn_background("mods", async move { crate::mods::ensure_branding_seeded(&bg, &inst).await });
        detail.summary.running = Some(self.active_instances().contains(name));
        Ok(detail)
    }

    /// DELETE /api/instances/:name -> `{ deleted, trash }` (moved to `<name>.deleted.<ms>`).
    pub async fn delete_instance(&self, name: &str) -> EngineResult<DeletedReply> {
        let (deleted, trash) = crate::instances::delete_instance(self, name).await?;
        Ok(DeletedReply { deleted, trash: Some(trash) })
    }

    /// PATCH /api/instances/:name -> InstanceSummary (rename blocked while running: 409 ALREADY_RUNNING).
    pub async fn patch_instance(&self, name: &str, patch: InstancePatch) -> EngineResult<InstanceSummary> {
        let wants_rename = patch.rename_target().map(|r| r.trim()).map(|r| !r.is_empty() && r != name).unwrap_or(false);
        if wants_rename && self.active_instances().contains(name) {
            return Err(EngineError::conflict(
                "ALREADY_RUNNING",
                format!("instance '{name}' is running; stop it before renaming"),
            ));
        }
        crate::instances::patch_instance(self, name, &patch).await
    }

    /// GET /api/instances/:name/servers -> `{ servers }` (servers.dat via crate::nbt).
    pub async fn instance_servers(&self, name: &str) -> EngineResult<ServersReply> {
        Ok(ServersReply { servers: crate::instances::read_servers_dat(self, name).await? })
    }

    /// PUT /api/instances/:name/servers `{ servers }` -> `{ count }`.
    pub async fn put_instance_servers(&self, name: &str, servers: Vec<ServerEntry>) -> EngineResult<CountReply> {
        Ok(CountReply { count: crate::instances::write_servers(self, name, &servers).await? })
    }

    /// GET /api/instances/:name/options -> `{ options: [[key, value]…] }`.
    pub async fn instance_options(&self, name: &str) -> EngineResult<OptionsReply> {
        Ok(OptionsReply { options: crate::instances::read_options(self, name).await? })
    }

    /// POST /api/instances/:name/options/import `{ source }` (400 BAD_REQUEST when empty).
    pub async fn import_instance_options(&self, name: &str, source: &str) -> EngineResult<OptionsImportReply> {
        let source = source.trim();
        if source.is_empty() {
            return Err(EngineError::bad_request("BAD_REQUEST", "source is required"));
        }
        let (copied, count, options) = crate::instances::import_options_from_instance(self, name, source).await?;
        Ok(OptionsImportReply { copied, count, options })
    }

    /// POST /api/instances/:name/icon `{ image_base64 }` (util::decode_png_image).
    pub async fn upload_instance_icon(&self, name: &str, image_base64: &str) -> EngineResult<IconReply> {
        let png = crate::util::decode_png_image(Some(image_base64))?;
        crate::instances::write_instance_icon(self, name, &png).await?;
        Ok(IconReply { ok: true, has_icon: true })
    }

    /// DELETE /api/instances/:name/icon -> `{ ok, has_icon: false }`.
    pub async fn remove_instance_icon(&self, name: &str) -> EngineResult<IconReply> {
        crate::instances::remove_instance_icon(self, name).await?;
        Ok(IconReply { ok: true, has_icon: false })
    }

    /// GET /api/instances/:name/icon -> PNG bytes (404 NOT_FOUND when absent).
    pub async fn instance_icon(&self, name: &str) -> EngineResult<Vec<u8>> {
        crate::instances::read_instance_icon(self, name).await
    }

    /// GET /api/instances/:name/mods -> ModEntry[] (500 `mods_list_failed`).
    pub async fn list_mods(&self, name: &str) -> EngineResult<Vec<ModEntry>> {
        crate::mods::list_mods(self, name).await.map_err(|e| send_error(e, 500, "mods_list_failed"))
    }

    /// GET /api/instances/:name/mods-dir -> `{ path }` (effective dir, created).
    pub async fn mods_dir(&self, name: &str) -> EngineResult<PathReply> {
        let r: EngineResult<PathReply> = async {
            let inst = crate::instances::get_instance(self, name).await?;
            let dir = crate::instances::effective_mods_dir(self, &inst);
            std::fs::create_dir_all(&dir)?;
            Ok(PathReply { path: dir.to_string_lossy().into_owned() })
        }
        .await;
        r.map_err(|e| send_error(e, 500, "mods_dir_failed"))
    }

    /// POST .../mods/:filename/enable|disable -> `{ filename, enabled }`.
    pub async fn set_mod_enabled(&self, name: &str, filename: &str, enabled: bool) -> EngineResult<ModToggleReply> {
        let fallback = if enabled { "mod_enable_failed" } else { "mod_disable_failed" };
        let filename = crate::mods::set_mod_enabled(self, name, filename, enabled)
            .await
            .map_err(|e| send_error(e, 500, fallback))?;
        Ok(ModToggleReply { filename, enabled })
    }

    /// POST .../mods/install `{ preset }` -> `{ queued: true }` (background install).
    pub async fn install_mod_preset(&self, name: &str, preset: ModPreset) -> EngineResult<QueuedReply> {
        crate::resolver::load_instance_meta(self, name).map_err(|e| send_error(e, 500, "mod_install_failed"))?;
        let bg = self.clone();
        let n = name.to_string();
        self.spawn_background("mods", async move {
            if let Err(e) = crate::mods::install_preset(&bg, &n, preset).await {
                tracing::error!("[mods] install failed for {n}: {e}");
            }
            Ok(())
        });
        Ok(QueuedReply { queued: true })
    }

    /// GET .../mods/preset -> `{ supported, loader, note, branding }`.
    pub async fn mod_preset_info(&self, name: &str) -> EngineResult<ModPresetInfo> {
        let inst =
            crate::resolver::load_instance_meta(self, name).map_err(|e| send_error(e, 404, "instance_not_found"))?;
        let note = match inst.loader {
            Loader::Neoforge => Some("Conjunto de rendimiento solo disponible en Fabric".to_string()),
            Loader::Vanilla => {
                Some("vanilla instances use integrated no-fog/fullbright (Settings → Client)".to_string())
            }
            Loader::Fabric => crate::mods::pin_note_for_version(&inst.version),
        };
        let branding = crate::mods::supports_branding(self, &inst.version, inst.loader);
        let versions = crate::mods::branding_versions(self).join(", ");
        Ok(ModPresetInfo {
            supported: inst.loader == Loader::Fabric && crate::mods::supports_performance_bundle(&inst.version),
            loader: inst.loader,
            note,
            branding: crate::types::BrandingPresetInfo {
                supported: branding,
                note: if branding {
                    None
                } else {
                    Some(format!(
                        "no bundled brand mod for {} on {}; branding covers {}",
                        inst.version,
                        inst.loader,
                        if versions.is_empty() { "nothing" } else { versions.as_str() }
                    ))
                },
            },
        })
    }

    /// GET /api/modrinth/search -> `{ results, total, offset, limit }` (502 `modrinth_search_failed`).
    pub async fn search_modrinth(&self, query: ModrinthSearchQuery) -> EngineResult<ModrinthSearchResult> {
        let q = query.q.trim();
        let version = query.version.as_deref().filter(|v| !v.is_empty());
        let loader = query.loader.as_deref().filter(|v| !v.is_empty());
        let instance = query.instance.as_deref().filter(|v| !v.is_empty());
        crate::mods::search_modrinth(self, q, version, loader, instance, query.offset)
            .await
            .map_err(|e| send_error(e, 502, "modrinth_search_failed"))
    }

    /// POST .../mods/install-modrinth -> plan summary + `queued: true`
    /// (409 `mod_conflicts` unless force; 400 `modrinth_project_id_required`).
    pub async fn install_modrinth_mod(&self, name: &str, req: InstallModrinthRequest) -> EngineResult<ModInstallPlan> {
        if req.project_id.is_empty() {
            return Err(EngineError::api_verbatim(400, "modrinth_project_id_required", "missing project_id"));
        }
        let fail = |e| send_error(e, 500, "mod_install_modrinth_failed");
        let inst = crate::resolver::load_instance_meta(self, name).map_err(fail)?;
        let loader = if inst.loader == Loader::Neoforge { "neoforge" } else { "fabric" };
        let plan = crate::mods::plan_mod_install(self, &inst, &req.project_id, loader).await.map_err(fail)?;
        if !plan.conflicts.is_empty() && !req.force {
            let titles: Vec<&str> = plan.conflicts.iter().map(|c| c.title.as_str()).collect();
            return Err(EngineError::api_verbatim(409, "mod_conflicts", titles.join(", ")));
        }
        let reply = ModInstallPlan {
            queued: true,
            project: plan.project.clone(),
            dependencies: plan.dependencies.clone(),
            missing: plan.missing.clone(),
            optional: plan.optional.clone(),
            conflicts: plan.conflicts.clone(),
        };
        let bg = self.clone();
        let (n, pid) = (name.to_string(), req.project_id.clone());
        self.spawn_background("mods", async move {
            if let Err(e) = crate::mods::install_modrinth_mod(&bg, &n, &pid, Some(loader), Some(plan)).await {
                tracing::error!("[mods] install-modrinth failed for {n}: {e}");
            }
            Ok(())
        });
        Ok(reply)
    }

    /// POST /api/instances/import-mrpack -> `{ summary, already_exists? }` (files land in the background).
    pub async fn import_mrpack(&self, req: ImportMrpackRequest) -> EngineResult<ImportMrpackReply> {
        if req.path.is_empty() {
            return Err(EngineError::bad_request("BAD_PATH", "path is required and must be a non-empty string"));
        }
        if let Some(mb) = req.memory_mb {
            if mb < 512 {
                return Err(EngineError::bad_request("BAD_MEMORY", "memory_mb must be an integer >= 512"));
            }
        }
        let r = crate::mrpack::import_mrpack(self, &req.path, req.memory_mb, None).await?;
        Ok(ImportMrpackReply { summary: r.summary, already_exists: r.already_exists.then_some(true) })
    }

    /// GET /api/import/sources -> ImportSource[].
    pub async fn import_sources(&self) -> EngineResult<Vec<ImportSource>> {
        crate::import::detect_import_sources(self).await
    }

    /// POST /api/instances/:name/import (409 INVALID_NAME, 404 NOT_FOUND, 400 BAD_REQUEST).
    pub async fn import_profile(&self, name: &str, req: ImportProfileRequest) -> EngineResult<ImportResult> {
        if !is_import_route_name(name) {
            return Err(EngineError::conflict("INVALID_NAME", format!("invalid instance name: {name}")));
        }
        let raw = std::fs::read(self.paths().instance_json(name))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("instance \"{name}\" not found")))?;
        let mut obj = raw.as_object().cloned().unwrap_or_default();
        obj.insert("name".into(), Value::String(name.to_string()));
        let instance = crate::instances::meta_from_raw(&obj);
        if req.source_id.is_empty() {
            return Err(EngineError::bad_request("BAD_REQUEST", "source_id is required"));
        }
        let merge = instance.merge_optionslc == Some(true);
        crate::import::import_profile(self, &instance, &req.source_id, req.overwrite_policy, merge).await
    }

    /// GET /api/servers -> `{ servers }` (150 s cache).
    pub async fn server_statuses(&self) -> EngineResult<ServerStatusesReply> {
        Ok(ServerStatusesReply { servers: crate::servers::get_all_server_statuses(self).await })
    }

    /// GET /api/instances/:name/client -> `{ config, registry, supported }` (crate::client).
    pub async fn get_client_info(&self, name: &str) -> EngineResult<ClientInfo> {
        crate::client::get_client_info(self, name).await
    }

    /// PATCH /api/instances/:name/client -> `{ config, errors }` (crate::client::patch_client_config).
    pub async fn patch_client(&self, name: &str, patch: ClientPatch) -> EngineResult<ClientInfo> {
        crate::client::patch_client_config(self, name, patch).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    fn quiet() -> (tempfile::TempDir, Engine) {
        let (d, e) = temp_engine();
        e.update_content_settings(|s| {
            s.background_on_create = false;
            s.modrinth_api = "http://127.0.0.1:9/v2".into();
            s.modrinth_retry_backoff = Duration::from_millis(1);
        });
        (d, e)
    }

    fn req(name: &str) -> CreateInstanceRequest {
        CreateInstanceRequest { name: name.into(), version: "1.21.11".into(), ..Default::default() }
    }

    #[tokio::test]
    async fn running_flags_and_rename_guard() {
        let (_d, e) = quiet();
        e.create_instance(req("run")).await.unwrap();
        e.active_instances().insert("run");
        assert_eq!(e.list_instances().await.unwrap()[0].running, Some(true));
        let detail = e.get_instance_detail("run").await.unwrap();
        assert_eq!(detail.summary.running, Some(true));
        let p = InstancePatch { name: Some("other".into()), ..Default::default() };
        let err = e.patch_instance("run", p).await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (409, "ALREADY_RUNNING"));
        // a no-op rename (same name) is allowed while running
        let p = InstancePatch { name: Some(" run ".into()), hue: Some(Some(7)), ..Default::default() };
        assert_eq!(e.patch_instance("run", p).await.unwrap().hue, Some(7));
        e.active_instances().remove("run");
        let del = e.delete_instance("run").await.unwrap();
        let v = serde_json::to_value(&del).unwrap();
        assert_eq!(v["deleted"], "run");
        assert!(v["trash"].as_str().unwrap().starts_with("run.deleted."));
    }

    #[tokio::test]
    async fn icons_options_and_route_validation() {
        let (_d, e) = quiet();
        e.create_instance(req("a")).await.unwrap();
        let tiny = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        assert_eq!(e.upload_instance_icon("a", tiny).await.unwrap(), IconReply { ok: true, has_icon: true });
        assert_eq!(e.instance_icon("a").await.unwrap()[..4], [0x89, b'P', b'N', b'G']);
        assert_eq!(e.upload_instance_icon("a", "nope").await.unwrap_err().envelope().code, "BAD_IMAGE");
        assert_eq!(e.remove_instance_icon("a").await.unwrap(), IconReply { ok: true, has_icon: false });
        assert_eq!(e.import_instance_options("a", "  ").await.unwrap_err().envelope().code, "BAD_REQUEST");
        assert_eq!(e.instance_options("a").await.unwrap().options.len(), 0);
        let err = e.import_mrpack(ImportMrpackRequest { path: "x".into(), memory_mb: Some(100) }).await.unwrap_err();
        assert_eq!(err.envelope().code, "BAD_MEMORY");
        let err = e.import_mrpack(ImportMrpackRequest::default()).await.unwrap_err();
        assert_eq!(err.envelope().code, "BAD_PATH");
        let r = ImportProfileRequest { source_id: "vanilla".into(), overwrite_policy: OverwritePolicy::Never };
        assert_eq!(e.import_profile("a.b", r.clone()).await.unwrap_err().envelope().code, "INVALID_NAME");
        assert_eq!(e.import_profile("zz", r.clone()).await.unwrap_err().status(), 404);
        let empty = ImportProfileRequest { source_id: String::new(), ..r };
        assert_eq!(e.import_profile("a", empty).await.unwrap_err().envelope().code, "BAD_REQUEST");
        let mods_dir = e.mods_dir("a").await.unwrap();
        assert!(std::path::Path::new(&mods_dir.path).is_dir());
        let err = e.mods_dir("missing").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (404, "NOT_FOUND"));
    }

    #[tokio::test]
    async fn mods_routes_use_verbatim_codes() {
        let (_d, e) = quiet();
        let err = e.list_mods("missing").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (404, "instance_not_found"));
        let err = e.set_mod_enabled("missing", "x.jar", true).await.unwrap_err().envelope();
        assert_eq!(err.code, "instance_not_found");
        let err = e.install_mod_preset("missing", ModPreset::Performance).await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (404, "instance_not_found"));
        let err = e.mod_preset_info("bad/name").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (409, "invalid_instance_name"));
        let err = e.install_modrinth_mod("x", InstallModrinthRequest::default()).await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (400, "modrinth_project_id_required"));
        let err = e.search_modrinth(ModrinthSearchQuery::default()).await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (502, "MODRINTH_SEARCH_FAILED"));

        e.create_instance(CreateInstanceRequest { loader: Some(Loader::Vanilla), ..req("v") }).await.unwrap();
        let info = e.mod_preset_info("v").await.unwrap();
        assert!(!info.supported);
        assert!(info.note.unwrap().contains("vanilla"));
        assert!(!info.branding.supported);
        let v = serde_json::to_value(e.mod_preset_info("v").await.unwrap()).unwrap();
        assert_eq!(v["loader"], "vanilla");
        assert_eq!(e.install_mod_preset("v", ModPreset::Branding).await.unwrap(), QueuedReply { queued: true });
    }

    #[test]
    fn send_error_mapping() {
        let e = send_error(EngineError::other("boom"), 500, "mods_list_failed").envelope();
        assert_eq!((e.status, e.code.as_str(), e.message.as_str()), (500, "mods_list_failed", "boom"));
        let e = send_error(EngineError::api(404, "instance_not_found", "x"), 500, "f").envelope();
        assert_eq!((e.status, e.code.as_str()), (404, "instance_not_found"));
        assert!(is_import_route_name("a b_c-1") && !is_import_route_name("a.b") && !is_import_route_name(""));
    }
}
