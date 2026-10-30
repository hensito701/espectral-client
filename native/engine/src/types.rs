//! Shared types — the cross-cluster contract.
//!
//! Two families live here:
//!
//! 1. **API shapes** — exact mirrors of `src/ui/src/lib/types.ts` (the JSON
//!    the JS engine returned). Field names are the wire names; serializing
//!    one of these gives the same JSON the JS engine answered with.
//! 2. **Domain shapes** that cross cluster boundaries (`InstanceMeta`,
//!    `Account`, `JavaRuntime`, `FabricProfile`, `ResolvedLaunch`, …) so
//!    game-launch can code against game-resolve / content before they land.
//!
//! On-disk documents (`instance.json`, `config.json` accounts, runtime
//! `info.json`) are deserialized LENIENTLY: every field has a default and
//! unknown keys are kept in a flattened `extra` map, so a document written by
//! the JS engine always round-trips.
//!
//! Changing a type here affects every cluster: do it in a dedicated commit
//! and mention it in the merge notes (see PORTING.md).

use std::path::PathBuf;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Lenient deserializers for documents written by older engines.
pub mod de {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    /// Any JSON -> `Some(u32)` only for a non-negative integer that fits (JS `Number.isInteger`).
    pub fn opt_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
        let v = Option::<Value>::deserialize(d)?;
        Ok(crate::config::json_integer(v.as_ref()).and_then(|i| u32::try_from(i).ok()))
    }

    /// Any JSON -> `Some(i64)` only for an integer.
    pub fn opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        let v = Option::<Value>::deserialize(d)?;
        Ok(crate::config::json_integer(v.as_ref()))
    }

    /// Any JSON -> `Some(f64)` for any number.
    pub fn opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
        let v = Option::<Value>::deserialize(d)?;
        Ok(v.and_then(|v| v.as_f64()))
    }

    /// Any JSON -> `Some(String)` only for a string.
    pub fn opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        let v = Option::<Value>::deserialize(d)?;
        Ok(match v {
            Some(Value::String(s)) => Some(s),
            _ => None,
        })
    }

    /// Any JSON -> `Some(bool)` only for a boolean.
    pub fn opt_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
        let v = Option::<Value>::deserialize(d)?;
        Ok(match v {
            Some(Value::Bool(b)) => Some(b),
            _ => None,
        })
    }

    /// Any JSON -> String (non-strings -> "").
    pub fn string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        Ok(opt_string(d)?.unwrap_or_default())
    }

    /// Distinguish "absent" (`None`) from explicit `null` (`Some(None)`) in patches.
    pub fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(d).map(Some)
    }
}

// =========================================================================
// Enums
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    #[default]
    Vanilla,
    Fabric,
    Neoforge,
}

impl Loader {
    pub fn as_str(self) -> &'static str {
        match self {
            Loader::Vanilla => "vanilla",
            Loader::Fabric => "fabric",
            Loader::Neoforge => "neoforge",
        }
    }
    pub fn parse(s: &str) -> Option<Loader> {
        match s {
            "vanilla" => Some(Loader::Vanilla),
            "fabric" => Some(Loader::Fabric),
            "neoforge" => Some(Loader::Neoforge),
            _ => None,
        }
    }
}

impl std::fmt::Display for Loader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn de_loader_lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Loader, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(v.as_ref().and_then(Value::as_str).and_then(Loader::parse).unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverwritePolicy {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "if-older")]
    IfOlder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportSourceKind {
    Vanilla,
    Fastclient,
    Lunar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JvmSource {
    Bundled,
    Fastclient,
    #[default]
    Path,
    Downloaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModPreset {
    Performance,
    Branding,
    Qol,
}

impl ModPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            ModPreset::Performance => "performance",
            ModPreset::Branding => "branding",
            ModPreset::Qol => "qol",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    #[default]
    Offline,
    Msa,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinVariant {
    Classic,
    Slim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchMode {
    #[default]
    Normal,
    /// AOT training run (-XX:AOTCacheOutput).
    Train,
}

// =========================================================================
// Versions / JVM (game-resolve)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub release_time: String,
    pub sha1: String,
    pub url: String,
}

/// GET /api/versions (cached 6h in `<data>/versions/manifest.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest_release: String,
    pub latest_snapshot: String,
    pub versions: Vec<VersionEntry>,
}

/// GET /api/versions/:id/resolve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionInfo {
    pub id: String,
    pub java_major: u32,
    pub supported: bool,
    pub reason: String,
}

/// A usable Java runtime (JS JvmInfo + `major`). Also the shape of
/// `config.jvm` and `<data>/runtimes/jdk-<major>/info.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct JavaRuntime {
    #[serde(default)]
    pub path: String,
    #[serde(default, deserialize_with = "de::string")]
    pub version: String,
    #[serde(default, deserialize_with = "de::string")]
    pub build: String,
    #[serde(default, deserialize_with = "de::string")]
    pub vendor: String,
    #[serde(default)]
    pub source: JvmSource,
    #[serde(default, deserialize_with = "de::opt_u32", skip_serializing_if = "Option::is_none")]
    pub major: Option<u32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// GET /api/jvm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JvmInfo {
    #[serde(flatten)]
    pub runtime: JavaRuntime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_majors: Option<Vec<u32>>,
}

/// Result of probing a `java` binary (jvm.mjs probeJava).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct JavaProbe {
    pub ok: bool,
    pub major: u32,
    pub version: Option<String>,
    pub build: Option<String>,
    pub vendor: Option<String>,
    pub error: Option<String>,
}

/// resolver.libraryDownload(lib): where one library jar comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryDownload {
    pub name: String,
    /// maven-relative path, forward slashes (`org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar`).
    #[serde(rename = "relPath")]
    pub rel_path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

/// resolver.resolveFabric() result; persisted (subset) as
/// `<instance>/fabric-profile.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FabricProfile {
    pub mc_version: String,
    pub loader_version: String,
    #[serde(default)]
    pub loader_maven: Option<String>,
    #[serde(default)]
    pub intermediary: Option<Value>,
    pub main_class: String,
    /// Strings in practice; `Value` because Mojang-style arguments may be rule objects.
    pub jvm_args: Vec<Value>,
    pub libraries: Vec<LibraryDownload>,
    #[serde(default)]
    pub min_java_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeded: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded: Option<u64>,
}

/// A Mojang version JSON (or a merged NeoForge chain). Kept as raw JSON:
/// the schema is Mojang's and evolves; read fields with serde_json.
pub type VersionJson = Value;

/// resolver.installLibraries() result.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LibrariesInstall {
    pub library_count: u64,
    pub seeded: u64,
    pub downloaded: u64,
    pub natives_extracted: u64,
    pub client_jar: Option<PathBuf>,
    pub classpath: Vec<PathBuf>,
}

/// resolver.installFabricLibraries() result.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FabricLibrariesInstall {
    pub library_count: u64,
    pub seeded: u64,
    pub downloaded: u64,
}

/// resolver.installAssets() result.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AssetsInstall {
    pub index: Option<String>,
    pub objects: u64,
}

// =========================================================================
// Accounts (accounts-skins)
// =========================================================================

/// `account.microsoft` as stored in config.json (refresh token only — the
/// access token is never persisted).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MicrosoftAuth {
    #[serde(default, deserialize_with = "de::opt_string", skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// ms epoch.
    #[serde(default, deserialize_with = "de::opt_f64", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
    /// Lunar-imported refresh token (bound to login.live.com).
    #[serde(default, deserialize_with = "de::opt_bool", skip_serializing_if = "Option::is_none")]
    pub lunar: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One entry of `config.accounts` (the stored, secret-bearing shape).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Account {
    #[serde(default, deserialize_with = "de::string")]
    pub username: String,
    /// Dashed UUID (offline: Java-compatible md5 v3; msa: Mojang profile id).
    #[serde(default, deserialize_with = "de::string")]
    pub uuid: String,
    #[serde(default)]
    pub token_kind: TokenKind,
    /// Hue 0-359, or null.
    #[serde(default, deserialize_with = "de::opt_i64")]
    pub avatar_color: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microsoft: Option<MicrosoftAuth>,
    /// Engine-internal: avatar was auto-derived from the skin head.
    #[serde(default, deserialize_with = "de::opt_bool", skip_serializing_if = "Option::is_none")]
    pub avatar_auto: Option<bool>,
    #[serde(default, deserialize_with = "de::opt_string", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "de::opt_string", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, deserialize_with = "de::opt_string", skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicMicrosoft {
    pub lunar: bool,
}

/// accounts.publicAccount(a) — what the UI may see (no tokens).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicAccount {
    pub username: String,
    pub uuid: String,
    pub token_kind: TokenKind,
    pub avatar_color: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microsoft: Option<PublicMicrosoft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    pub has_avatar: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The identity a game is launched with (launch.resolveAccount result).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchAccount {
    pub username: String,
    pub uuid: String,
    /// '0' for offline.
    pub access_token: String,
    /// 'msa' | 'mojang'
    pub user_type: String,
}

/// POST /api/accounts/microsoft/device-code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MsDeviceFlow {
    pub flow_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

// =========================================================================
// Config
// =========================================================================

/// GET /api/config (JS appConfig(), same key order).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_memory_mb: u32,
    pub recommended_memory_mb: u32,
    pub default_memory_auto: bool,
    pub accent: String,
    pub reduced_motion: bool,
    pub close_on_game_start: bool,
    pub download_concurrency: u32,
    pub aot_auto_train: bool,
    pub fast_boot: bool,
    pub fullbright_on_launch: bool,
    pub jdk_path_override: Option<String>,
    pub discord_enabled: bool,
    pub data_dir: String,
}

// =========================================================================
// Instances (content)
// =========================================================================

/// `<data>/instances/<name>/instance.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InstanceMeta {
    #[serde(default, deserialize_with = "de::string")]
    pub name: String,
    #[serde(default, deserialize_with = "de::string")]
    pub version: String,
    #[serde(default, deserialize_with = "de_loader_lenient")]
    pub loader: Loader,
    #[serde(default, deserialize_with = "de::opt_string")]
    pub loader_version: Option<String>,
    #[serde(default, deserialize_with = "de::opt_string")]
    pub modpack: Option<String>,
    #[serde(default, deserialize_with = "de::opt_string")]
    pub modpack_version: Option<String>,
    /// null = automatic (config default / system RAM).
    #[serde(default, deserialize_with = "de::opt_u32")]
    pub memory_mb: Option<u32>,
    #[serde(default, deserialize_with = "de::opt_bool", skip_serializing_if = "Option::is_none")]
    pub memory_auto_migrated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods: Option<Vec<Value>>,
    #[serde(default, deserialize_with = "de::opt_string")]
    pub imported_from: Option<String>,
    #[serde(default, deserialize_with = "de::opt_string")]
    pub jdk_path_override: Option<String>,
    #[serde(default, deserialize_with = "de::opt_bool")]
    pub aot_auto_train: Option<bool>,
    #[serde(default, deserialize_with = "de::opt_i64")]
    pub hue: Option<i64>,
    /// Absolute custom game folder; null = `<data>/instances/<name>`.
    #[serde(default, deserialize_with = "de::opt_string")]
    pub game_dir: Option<String>,
    #[serde(default, deserialize_with = "de::opt_string", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "de::opt_string", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, deserialize_with = "de::opt_bool", skip_serializing_if = "Option::is_none")]
    pub merge_optionslc: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// InstanceSummary (GET /api/instances rows; `running` added by the route).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceSummary {
    pub name: String,
    pub version: String,
    pub loader: Loader,
    pub loader_version: Option<String>,
    pub modpack: Option<String>,
    pub modpack_version: Option<String>,
    pub memory_mb: u32,
    pub memory_auto: bool,
    pub hue: Option<i64>,
    pub game_dir: Option<String>,
    pub has_icon: bool,
    pub mod_count: u32,
    pub enabled_mod_count: u32,
    pub aot_key: Option<String>,
    pub aot_cache_exists: bool,
    pub imported_from: Option<String>,
    pub created_at: Option<String>,
    /// Set by the list/detail facade methods (activeInstances).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModEntry {
    pub filename: String,
    /// null for untracked jars (JS sends explicit nulls).
    pub project_slug: Option<String>,
    pub version_number: Option<String>,
    pub version_id: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub enabled: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerEntry {
    pub name: String,
    pub ip: String,
    pub hidden: bool,
    pub accept_textures: bool,
    pub has_icon: bool,
}

/// options.txt `key:value` pair.
pub type OptionsPair = (String, String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AotProof {
    pub log_path: Option<String>,
    pub using_aot_linked_classes: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

/// GET /api/instances/:name/aot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AotStatus {
    pub key: Option<String>,
    pub cache_path: Option<String>,
    pub cache_exists: bool,
    pub cache_size_bytes: u64,
    /// Classpath drift vs the cache stamp; null = unverifiable.
    pub stale: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_to_train: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trained_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<AotProof>,
}

/// GET /api/instances/:name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceDetail {
    pub summary: InstanceSummary,
    pub mods: Vec<ModEntry>,
    pub servers: Vec<ServerEntry>,
    pub options: Vec<OptionsPair>,
    pub aot: AotStatus,
}

/// POST /api/instances body (instances.createInstance opts).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CreateInstanceRequest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub loader: Option<Loader>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub modpack: Option<String>,
    #[serde(default)]
    pub modpack_version: Option<String>,
    /// Omit for automatic memory.
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default)]
    pub jdk_path_override: Option<String>,
    #[serde(default)]
    pub import_from: Option<String>,
    /// Merge Lunar optionsLC.json into options.txt.
    #[serde(default)]
    pub merge_optionslc: bool,
    /// Onboarding: queue AOT training itself once presets are installed.
    #[serde(default)]
    pub defer_auto_train: bool,
    #[serde(default)]
    pub hue: Option<i64>,
    #[serde(default)]
    pub game_dir: Option<String>,
}

/// PATCH /api/instances/:name body. `Some(None)` = explicit null.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InstancePatch {
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<Option<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_mods: Option<Vec<String>>,
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub jdk_path_override: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aot_auto_train: Option<bool>,
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub hue: Option<Option<i64>>,
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub game_dir: Option<Option<String>>,
    /// Rename target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Alias of `name` (wins when both are set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
}

impl InstancePatch {
    /// JS `patch.new_name !== undefined ? patch.new_name : patch.name`.
    pub fn rename_target(&self) -> Option<&str> {
        self.new_name.as_deref().or(self.name.as_deref())
    }
}

// =========================================================================
// Version change (game-launch: version-change.mjs)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VersionTarget {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub loader: Option<Loader>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionLoaderRef {
    pub version: String,
    pub loader: Loader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionChangeModTarget {
    pub filename: String,
    pub version_number: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionChangeMod {
    pub filename: String,
    pub title: String,
    pub enabled: bool,
    /// 'keep' | 'update' | 'disable' | 'idle' | 'add'
    pub action: String,
    /// 'unknown' | 'no_build'
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<VersionChangeModTarget>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionChangePlan {
    pub from: VersionLoaderRef,
    pub to: VersionLoaderRef,
    pub downgrade: bool,
    pub backup_worlds: bool,
    pub espectral_menu: bool,
    pub mods: Vec<VersionChangeMod>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModRename {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModError {
    pub filename: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionChangeModsResult {
    pub updated: Vec<ModRename>,
    pub added: Vec<String>,
    pub disabled: Vec<String>,
    pub errors: Vec<ModError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionChangeResult {
    pub summary: InstanceSummary,
    pub backup: Option<String>,
    pub mods: VersionChangeModsResult,
}

// =========================================================================
// Mods (content)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrandingPresetInfo {
    pub supported: bool,
    pub note: Option<String>,
}

/// GET /api/instances/:name/mods/preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModPresetInfo {
    pub supported: bool,
    pub loader: Loader,
    pub note: Option<String>,
    pub branding: BrandingPresetInfo,
}

/// mods.installPreset() result.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PresetInstallResult {
    pub installed: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthProject {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub game_versions: Vec<String>,
    pub latest_version_number: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,
}

/// GET /api/modrinth/search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthSearchResult {
    pub results: Vec<ModrinthProject>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModRef {
    pub project_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModDependencyRef {
    pub project_id: String,
    pub title: String,
    pub filename: String,
}

/// POST /api/instances/:name/mods/install-modrinth reply (`queued` + summarizePlan).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModInstallPlan {
    pub queued: bool,
    pub project: ModRef,
    pub dependencies: Vec<ModDependencyRef>,
    pub missing: Vec<ModRef>,
    pub optional: Vec<ModRef>,
    pub conflicts: Vec<ModRef>,
}

// =========================================================================
// Import (content)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LunarInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocated_memory: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fov_degrees: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods_json_toggles: Option<Vec<String>>,
}

/// GET /api/import/sources rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportSource {
    pub id: String,
    pub kind: ImportSourceKind,
    pub label: String,
    pub path: String,
    pub options_exists: bool,
    pub servers_exists: bool,
    pub options_key_count: u32,
    pub servers_entry_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lunar: Option<LunarInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportSkipped {
    pub file: String,
    pub reason: String,
}

/// POST /api/instances/:name/import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportResult {
    pub copied: Vec<String>,
    pub skipped: Vec<ImportSkipped>,
    pub servers_parsed: Vec<ServerEntry>,
    pub options_keys: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lunar: Option<LunarInfo>,
}

// =========================================================================
// Servers status (content: servers.mjs)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub host: String,
    pub online: bool,
    pub hostname: String,
    /// null when offline / unknown (JS sends explicit nulls).
    pub ip: Option<String>,
    pub port: u16,
    pub version: Option<String>,
    pub software: Option<String>,
    pub motd_raw: Vec<String>,
    pub motd_clean: Vec<String>,
    pub players_online: u32,
    pub players_max: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub fetched_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// =========================================================================
// Launch (game-launch)
// =========================================================================

/// resolveLaunch().version
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedVersionRef {
    /// Game version id, or `neoforge-<v>` for NeoForge.
    pub id: String,
    pub main_class: String,
    pub asset_index_id: Option<String>,
    /// The (merged) version JSON everything else was derived from.
    pub json: VersionJson,
}

/// resolveLaunch().loader — loader-specific main class + extra args.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoaderLaunch {
    pub main_class: String,
    pub jvm_args: Vec<Value>,
    pub game_args: Vec<Value>,
    /// 'neoforge' for NeoForge, None for Fabric.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Everything a launch (or AOT training run, or dry-run) needs — launch.mjs
/// resolveLaunch() result. Produced by game-launch from game-resolve's
/// resolver/runtimes output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedLaunch {
    pub java: JavaRuntime,
    pub version: ResolvedVersionRef,
    pub loader: Option<LoaderLaunch>,
    pub fabric_profile: Option<FabricProfile>,
    pub classpath: Vec<PathBuf>,
    pub natives_dir: PathBuf,
    pub game_dir: PathBuf,
    pub assets_dir: PathBuf,
    pub memory_mb: u32,
    pub account: LaunchAccount,
    /// Per-launch bridge token (real launches only).
    pub bridge_token: Option<String>,
    pub mode: LaunchMode,
    pub aot_key: String,
    pub aot_cache_path: PathBuf,
    pub aot_cache_exists: bool,
    pub aot_cache_stale: bool,
    /// JDK >= 25.
    pub aot_available: bool,
    pub warnings: Vec<String>,
}

/// POST /api/instances/:name/launch { dry_run: true }.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunResult {
    pub argv: Vec<String>,
    pub classpath: Vec<String>,
    pub classpath_missing: Vec<String>,
    pub natives_dir: String,
    pub assets_ok: bool,
    pub java_path: String,
    pub warnings: Vec<String>,
}

/// POST /api/instances/:name/launch (real launch; the game spawns in the background).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchReply {
    pub key: String,
    pub pid: Option<u32>,
    pub preparing: bool,
    pub instance: String,
    pub version: String,
    pub account: Option<String>,
}

/// A live/recent launch handle (GET /api/launches row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchHandle {
    pub key: String,
    pub instance: String,
    pub version: String,
    pub account: Option<String>,
    pub running: bool,
    pub started_at: i64,
    pub menu_at: Option<i64>,
    pub ended_at: Option<i64>,
}

/// GET /api/launch/:key/log?cursor=N.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogChunk {
    pub lines: Vec<String>,
    pub cursor: u64,
    pub running: bool,
}

/// One `<data>/launch-stats.jsonl` record (stats.rs).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LaunchStat {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub instance: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub menu_at: Option<i64>,
    #[serde(default)]
    pub menu_ms: Option<i64>,
    #[serde(default)]
    pub played_ms: Option<i64>,
    #[serde(default)]
    pub spawn_ms: Option<i64>,
    #[serde(default)]
    pub boot_ms: Option<i64>,
    /// phase name -> ms epoch; None for legacy (pre-C5) rows.
    #[serde(default)]
    pub phases: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One `<data>/running-games.json` entry (game-sessions.mjs), keyed by instance.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GameSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default)]
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_token: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// =========================================================================
// Espectral Client config (identity-discord: client.mjs)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientMacroAction {
    /// 'chat' | 'command'
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientMacro {
    pub id: String,
    pub name: String,
    pub keybind: String,
    pub actions: Vec<ClientMacroAction>,
}

/// `<gameDir>/config/espectral-client.json` (schema 2). Features carry
/// arbitrary per-feature keys, so they stay JSON objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientConfig {
    pub schema: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personalization: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<Value>,
    pub features: Map<String, Value>,
    pub macros: Vec<ClientMacro>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientRegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 'owned' | 'managed'
    pub kind: String,
    #[serde(rename = "defaultEnabled")]
    pub default_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keybind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientReconcileError {
    pub feature: String,
    pub message: String,
}

/// GET/PATCH /api/instances/:name/client, PATCH .../personalization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub config: ClientConfig,
    #[serde(default)]
    pub registry: Vec<ClientRegistryEntry>,
    #[serde(default)]
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ClientReconcileError>>,
}

/// PATCH /api/instances/:name/client body (merge semantics). Kept as JSON
/// pieces because client.mjs validates structure itself and answers specific
/// 400 codes (BAD_PATCH, …) for malformed parts.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ClientPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macros: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personalization: Option<Value>,
}

/// PATCH /api/instances/:name/personalization body. `Some(None)` = explicit null.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PersonalizationPatch {
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub theme: Option<Option<String>>,
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub hud_theme: Option<Option<String>>,
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub background: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduced_motion: Option<bool>,
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub tagline: Option<Option<String>>,
}

// =========================================================================
// Skins (accounts-skins)
// =========================================================================

/// GET /api/accounts/:username/skin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinInfo {
    pub username: String,
    pub variant: SkinVariant,
    pub has_skin: bool,
    pub cape: bool,
}

/// Skin gallery entry (public shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinLibraryEntry {
    pub id: String,
    pub name: String,
    pub variant: SkinVariant,
    pub created_at: String,
    /// 'upload' | 'vanilla'
    pub source: String,
}

// =========================================================================
// Identity & cosmetics (identity-discord / accounts-skins)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityDiscord {
    pub id: String,
    pub username: String,
    pub avatar: Option<String>,
}

/// One account's identity overlay (identity.accountIdentityEntry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityAccount {
    /// 32 lowercase hex, no dashes.
    pub uuid: String,
    pub username: String,
    pub verified: bool,
    pub discord: Option<IdentityDiscord>,
    pub flags: Vec<String>,
    /// Unix seconds.
    pub grant_exp: Option<i64>,
}

/// GET /api/identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityState {
    pub service_url: String,
    pub accounts: Vec<IdentityAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosmeticItemName {
    pub es: String,
    pub en: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosmeticItem {
    pub id: String,
    pub slot: String,
    pub requires: String,
    pub name: CosmeticItemName,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosmeticEmote {
    pub id: String,
    pub requires: String,
    pub name: CosmeticItemName,
}

/// `src/engine/cosmetics-catalog.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosmeticsCatalog {
    pub schema: u32,
    pub slots: Vec<String>,
    /// `[slot, [itemIds…]]` exclusivity rules.
    pub exclusive: Vec<(String, Vec<String>)>,
    pub items: Vec<CosmeticItem>,
    pub emotes: Vec<CosmeticEmote>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosmeticsAccountState {
    pub share: bool,
    /// slot -> item id | null
    pub loadout: IndexMap<String, Option<String>>,
    pub hidden_players: Vec<String>,
    pub emotes: bool,
    pub flags: Vec<String>,
}

/// GET /api/cosmetics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosmeticsState {
    pub catalog: CosmeticsCatalog,
    /// flat uuid -> state
    pub accounts: IndexMap<String, CosmeticsAccountState>,
}

/// PUT /api/cosmetics/:uuid body.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CosmeticsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loadout: Option<IndexMap<String, Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_players: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotes: Option<bool>,
}

// =========================================================================
// Backgrounds (accounts-skins)
// =========================================================================

/// `<data>/backgrounds/<id>/pack.json` (public shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundPack {
    pub id: String,
    pub name: String,
    /// 'image' | 'frames'
    #[serde(rename = "type")]
    pub kind: String,
    pub fps: u32,
    pub frame_count: u32,
    pub width: u32,
    pub height: u32,
}

// =========================================================================
// Misc
// =========================================================================

/// GET /api/health.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthInfo {
    pub ok: bool,
    pub version: String,
}

/// `{ ok: true }` replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkReply {
    pub ok: bool,
}

impl OkReply {
    pub const TRUE: OkReply = OkReply { ok: true };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn instance_meta_round_trips_js_documents_with_unknown_keys() {
        let js = json!({
            "name": "Espectral default", "version": "26.3", "loader": "fabric", "loader_version": null,
            "modpack": null, "modpack_version": null, "memory_mb": null, "memory_auto_migrated": true,
            "mods": [], "imported_from": "lunar", "jdk_path_override": null, "aot_auto_train": true,
            "hue": 210, "game_dir": null, "created_at": "2026-01-01T00:00:00.000Z",
            "updated_at": "2026-01-01T00:00:00.000Z", "future_key": { "x": 1 }
        });
        let meta: InstanceMeta = serde_json::from_value(js.clone()).unwrap();
        assert_eq!(meta.loader, Loader::Fabric);
        assert_eq!(meta.hue, Some(210));
        assert_eq!(meta.extra["future_key"], json!({ "x": 1 }));
        let back = serde_json::to_value(&meta).unwrap();
        for (k, v) in js.as_object().unwrap() {
            assert_eq!(&back[k], v, "{k}");
        }
    }

    #[test]
    fn instance_meta_is_lenient() {
        let meta: InstanceMeta = serde_json::from_value(json!({
            "name": "x", "version": "1.21.11", "loader": "weird", "memory_mb": 3072.5, "hue": "red"
        }))
        .unwrap();
        assert_eq!(meta.loader, Loader::Vanilla);
        assert_eq!(meta.memory_mb, None);
        assert_eq!(meta.hue, None);
    }

    #[test]
    fn account_keeps_unknown_fields() {
        let a: Account = serde_json::from_value(json!({
            "username": "Steve", "uuid": "u", "token_kind": "msa", "avatar_color": 12,
            "microsoft": { "refresh_token": "R", "expires_at": 1.5, "lunar": true, "weird": 1 },
            "created_at": "c", "last_used": "l", "lunar_badge": true
        }))
        .unwrap();
        assert_eq!(a.token_kind, TokenKind::Msa);
        assert_eq!(a.microsoft.as_ref().unwrap().extra["weird"], 1);
        assert_eq!(a.extra["lunar_badge"], true);
    }

    #[test]
    fn patches_distinguish_null_from_absent() {
        let p: InstancePatch = serde_json::from_value(json!({ "memory_mb": null, "hue": 5 })).unwrap();
        assert_eq!(p.memory_mb, Some(None));
        assert_eq!(p.hue, Some(Some(5)));
        assert_eq!(p.game_dir, None);
        let p: InstancePatch = serde_json::from_value(json!({ "name": "a", "new_name": "b" })).unwrap();
        assert_eq!(p.rename_target(), Some("b"));
    }

    #[test]
    fn java_runtime_reads_runtime_info_json() {
        let j: JavaRuntime = serde_json::from_value(json!({
            "path": "/d/runtimes/jdk-21/bin/java", "version": "21.0.5", "build": "21.0.5+11-LTS",
            "vendor": "Eclipse Adoptium", "source": "downloaded", "major": 21
        }))
        .unwrap();
        assert_eq!(j.major, Some(21));
        assert_eq!(j.source, JvmSource::Downloaded);
    }
}
