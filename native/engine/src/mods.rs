//! Port of `src/engine/mods.mjs` — mod listing / enable-disable, pinned
//! presets (performance / QoL / branding), Modrinth search + dependency
//! planning + install, version-change mod planning.
//!
//! **Owner: content.** Status: PORTED.
//!
//! Modrinth version metadata stays raw JSON (`serde_json::Value`) like the JS:
//! only a handful of fields are read and the schema is Modrinth's. Every
//! download is SHA-1 verified by `HttpClient::download_all`. The Modrinth base
//! URL comes from `ContentSettings::modrinth_api` (tests: a mock server).
//!
//! Bundled branding jars: `<exe dir>/branding`, `<exe dir>/resources/branding`,
//! `<exe dir>/../branding` (installed layouts), then the repo's
//! `assets/branding` (dev) — or `ContentSettings::branding_dir`.
//!
//! Error codes from routes/mods.mjs are lowercase and VERBATIM (see
//! `api::content::send_error`).
//!
//! CONTRACT (called by onboarding, game-launch launch/version-change):
//! install_preset, ensure_branding_seeded, plan_mods_for_target, apply_mods_plan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::download::{DownloadAllOptions, DownloadItem, FetchOptions};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult, HttpError, HttpErrorKind};
use crate::events::{EngineEvent, ProgressChannel, ProgressEvent};
use crate::instances::effective_mods_dir;
use crate::types::{
    InstanceMeta, Loader, ModDependencyRef, ModEntry, ModPreset, ModRef, ModrinthProject, ModrinthSearchResult,
    PresetInstallResult, VersionTarget,
};

pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";
pub const PERFORMANCE_PRESET: &str = "performance";
pub const BRANDING_PRESET: &str = "branding";
pub const QOL_PRESET: &str = "qol";

pub const PINNED_VERSIONS: [&str; 3] = ["1.21.11", "26.2", "26.3"];
pub const PINNED_VERSION: &str = "1.21.11";

/// One researched Modrinth pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub slug: String,
    pub project_id: String,
    pub version_id: String,
    pub version_number: String,
}

type PinRow = (&'static str, &'static str, &'static str, &'static str);

const PINS_1_21_11: [PinRow; 7] = [
    ("sodium", "AANobbMI", "UddlN6L4", "0.8.7"),
    ("iris", "YL57xq9U", "fDpuVzVr", "1.10.7"),
    ("lithium", "gvQqBUqZ", "Ow7wA0kG", "0.21.4"),
    ("ferrite-core", "uXXizFIs", "Ii0gP3D8", "8.2.0"),
    ("krypton", "fQEb0iXm", "O9LmWYR7", "0.2.10"),
    ("modmenu", "mOgUt4GM", "Tyk71iSw", "17.0.0"),
    ("fabric-api", "P7dR8mSH", "6qAuTtLR", "0.141.6"),
];
const PINS_26_2: [PinRow; 7] = [
    ("sodium", "AANobbMI", "2Yom1N68", "mc26.2-0.9.1-fabric"),
    ("iris", "YL57xq9U", "oaD6KQls", "1.11.2+26.2-fabric"),
    ("lithium", "gvQqBUqZ", "f7vZ0VWU", "mc26.2-0.25.3-fabric"),
    ("ferrite-core", "uXXizFIs", "d5ddUdiB", "9.0.0-fabric"),
    ("krypton", "fQEb0iXm", "5WeL0Nkz", "0.3.1"),
    ("modmenu", "mOgUt4GM", "njXb639R", "20.0.1"),
    ("fabric-api", "P7dR8mSH", "3gT0I5vt", "0.156.0+26.2"),
];
// Iris 1.11.7 requires exactly Sodium 0.9.2 (bAZQdGpg); Sodium 0.9.3 is still beta.
const PINS_26_3: [PinRow; 7] = [
    ("sodium", "AANobbMI", "bAZQdGpg", "mc26.3-0.9.2-fabric"),
    ("iris", "YL57xq9U", "vTN4NRGW", "1.11.7+26.3-fabric"),
    ("lithium", "gvQqBUqZ", "fWv5bnZk", "mc26.3-0.26.3-fabric"),
    ("ferrite-core", "uXXizFIs", "d5ddUdiB", "9.0.0-fabric"),
    ("krypton", "fQEb0iXm", "UugdIYJw", "0.3.2"),
    ("modmenu", "mOgUt4GM", "kyy7dbrZ", "21.0.0"),
    ("fabric-api", "P7dR8mSH", "EQKgpeKu", "0.165.0+26.3"),
];
/// Legacy-optional QoL pins (gamma-utils / clear-fog).
const QOL_1_21_11: [PinRow; 2] =
    [("gamma-utils", "wdLuzzEP", "aVJkWMQl", "2.5.10"), ("clear-fog", "46n24c6r", "CjWMitqz", "1.21.11_Fabric")];
const QOL_26_2: [PinRow; 2] =
    [("gamma-utils", "wdLuzzEP", "bR4tEPFq", "3.1.1+Fabric"), ("clear-fog", "46n24c6r", "GZ2Uiwgv", "26.2_Fabric")];
const QOL_26_3: [PinRow; 2] =
    [("gamma-utils", "wdLuzzEP", "k6ywOirE", "3.2.2+Fabric"), ("clear-fog", "46n24c6r", "C8eOUsdr", "26.3_Fabric")];

/// The seven canonical performance projects.
const PERFORMANCE_PROJECTS: [(&str, &str); 7] = [
    ("sodium", "AANobbMI"),
    ("iris", "YL57xq9U"),
    ("lithium", "gvQqBUqZ"),
    ("ferrite-core", "uXXizFIs"),
    ("krypton", "fQEb0iXm"),
    ("modmenu", "mOgUt4GM"),
    ("fabric-api", "P7dR8mSH"),
];
const QOL_PROJECTS: [(&str, &str); 2] = [("gamma-utils", "wdLuzzEP"), ("clear-fog", "46n24c6r")];

/// Jar-in-jar bundled dependencies (placeholder-api inside modmenu).
const JAR_IN_JAR: [&str; 1] = ["eXts2L7r"];

fn to_pins(rows: &[PinRow]) -> Vec<Pin> {
    rows.iter()
        .map(|(s, p, v, n)| Pin {
            slug: s.to_string(),
            project_id: p.to_string(),
            version_id: v.to_string(),
            version_number: n.to_string(),
        })
        .collect()
}

/// PINS_BY_VERSION[version] (exact researched performance pins).
pub fn pins_by_version(version: &str) -> Option<Vec<Pin>> {
    match version {
        "1.21.11" => Some(to_pins(&PINS_1_21_11)),
        "26.2" => Some(to_pins(&PINS_26_2)),
        "26.3" => Some(to_pins(&PINS_26_3)),
        _ => None,
    }
}

/// PINS_QOL_BY_VERSION[version].
pub fn qol_pins_by_version(version: &str) -> Option<Vec<Pin>> {
    match version {
        "1.21.11" => Some(to_pins(&QOL_1_21_11)),
        "26.2" => Some(to_pins(&QOL_26_2)),
        "26.3" => Some(to_pins(&QOL_26_3)),
        _ => None,
    }
}

fn mod_progress(engine: &Engine, p: ProgressEvent) {
    engine.emit(EngineEvent::ModProgress(p));
}

/// JS `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn api(engine: &Engine) -> String {
    engine.content_settings().modrinth_api.trim_end_matches('/').to_string()
}

fn code_err(status: u16, code: &str, message: impl Into<String>) -> EngineError {
    EngineError::api(status, code, message)
}

// ---------------------------------------------------------------------------
// Version support (pure)
// ---------------------------------------------------------------------------

/// Release ids in [1.21, 26.x]: `1.2X[.patch]` (minor 21..=26) or `NN[.minor]` (21..=26).
pub fn supports_performance_bundle(version: &str) -> bool {
    if version.is_empty() {
        return false;
    }
    let b = version.as_bytes();
    if b.len() >= 4 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b'w' && b[3].is_ascii_digit() {
        return false; // snapshots (25w11a)
    }
    let parts: Vec<&str> = version.split('.').collect();
    if parts.iter().any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit())) {
        return false;
    }
    let num = |s: &str| s.parse::<u64>().unwrap_or(u64::MAX);
    if parts[0] == "1" && parts.len() >= 2 {
        let minor = num(parts[1]);
        return (21..=26).contains(&minor);
    }
    (21..=26).contains(&num(parts[0]))
}

pub fn pin_note_for_version(version: &str) -> Option<String> {
    if supports_performance_bundle(version) {
        None
    } else {
        Some(format!("no performance mod bundle for {version}; the bundle covers release versions 1.21 through 26.x"))
    }
}

/// Pin set for a version: exact pins, or dynamically resolved (None when
/// unsupported or any project lacks a build; never fails).
pub async fn pins_for_version(engine: &Engine, version: &str) -> Option<Vec<Pin>> {
    if let Some(p) = pins_by_version(version) {
        return Some(p);
    }
    if !supports_performance_bundle(version) {
        return None;
    }
    resolve_dynamic_pins(engine, version, &PERFORMANCE_PROJECTS).await
}

/// QoL bundle pins (exact or dynamic).
pub async fn qol_pins_for_version(engine: &Engine, version: &str) -> Option<Vec<Pin>> {
    if let Some(p) = qol_pins_by_version(version) {
        return Some(p);
    }
    if !supports_performance_bundle(version) {
        return None;
    }
    resolve_dynamic_pins(engine, version, &QOL_PROJECTS).await
}

async fn resolve_dynamic_pins(engine: &Engine, version: &str, projects: &[(&str, &str)]) -> Option<Vec<Pin>> {
    let resolved = futures_util::future::join_all(projects.iter().map(|(slug, pid)| async move {
        let meta = best_version_for_cached(engine, pid, version, "fabric").await?;
        Some(Pin {
            slug: slug.to_string(),
            project_id: pid.to_string(),
            version_id: s(&meta, "id")?,
            version_number: s(&meta, "version_number").unwrap_or_default(),
        })
    }))
    .await;
    resolved.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Branding preset (bundled jars)
// ---------------------------------------------------------------------------

/// Pin-shaped branding entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandingPin {
    pub slug: String,
    pub filename: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub version_number: Option<String>,
}

/// The bundled branding jars dir (first existing candidate, else the last).
pub fn branding_dir(engine: &Engine) -> PathBuf {
    if let Some(d) = engine.content_settings().branding_dir {
        return d;
    }
    let mut candidates = vec![];
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        candidates.push(exe_dir.join("branding"));
        candidates.push(exe_dir.join("resources").join("branding"));
        if let Some(up) = exe_dir.parent() {
            candidates.push(up.join("branding"));
        }
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    candidates.push(repo.join("resources").join("branding"));
    candidates.push(repo.join("assets").join("branding"));
    candidates.iter().find(|d| d.join("branding.json").exists()).cloned().unwrap_or_else(|| candidates.pop().unwrap())
}

fn branding_manifest(engine: &Engine) -> Option<Value> {
    let dir = branding_dir(engine);
    let state = engine.content_state();
    if let Some(m) = state.branding_manifest.lock().unwrap_or_else(|p| p.into_inner()).get(&dir) {
        return m.clone();
    }
    let m = std::fs::read(dir.join("branding.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    state.branding_manifest.lock().unwrap_or_else(|p| p.into_inner()).insert(dir, m.clone());
    m
}

/// Minecraft versions covered by the bundled brand jars (any loader).
pub fn branding_versions(engine: &Engine) -> Vec<String> {
    branding_manifest(engine)
        .and_then(|m| m.get("mods").and_then(Value::as_object).map(|o| o.keys().cloned().collect()))
        .unwrap_or_default()
}

/// Branding pin for version + loader (None when unsupported).
pub fn branding_pin_for_version(engine: &Engine, version: &str, loader: Loader) -> Option<BrandingPin> {
    let manifest = branding_manifest(engine)?;
    let entry = manifest.get("mods")?.get(version)?;
    let loaders: Vec<String> = match entry.get("loaders").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        None => vec!["fabric".into()],
    };
    if !loaders.iter().any(|l| l == loader.as_str()) {
        return None;
    }
    Some(BrandingPin {
        slug: s(entry, "mod_id").or_else(|| s(&manifest, "mod_id")).unwrap_or_default(),
        filename: s(entry, "filename")?,
        sha1: s(entry, "sha1"),
        size: entry.get("size").and_then(Value::as_u64),
        version_number: s(&manifest, "mod_version"),
    })
}

/// True when a bundled brand jar exists for this version + loader.
pub fn supports_branding(engine: &Engine, version: &str, loader: Loader) -> bool {
    branding_pin_for_version(engine, version, loader).is_some()
}

fn branding_note(engine: &Engine, version: &str, loader: Loader) -> String {
    let versions = branding_versions(engine).join(", ");
    let covers = if versions.is_empty() { "nothing".to_string() } else { versions };
    format!("no bundled brand mod for {version} on {loader}; branding covers {covers}")
}

fn is_in_use(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    match e.raw_os_error() {
        #[cfg(windows)]
        Some(5 | 32 | 33) => true,
        #[cfg(unix)]
        Some(1 | 13 | 16) => true,
        _ => false,
    }
}

async fn install_branding_preset(
    engine: &Engine,
    instance: &InstanceMeta,
    mods_dir: &Path,
) -> EngineResult<PresetInstallResult> {
    let ev = |phase: &str| ProgressEvent {
        preset: Some(BRANDING_PRESET.into()),
        ..ProgressEvent::for_instance(&instance.name).phase(phase).total(1)
    };
    let Some(pin) = branding_pin_for_version(engine, &instance.version, instance.loader) else {
        let note = branding_note(engine, &instance.version, instance.loader);
        tracing::warn!("[mods] {note}");
        mod_progress(
            engine,
            ProgressEvent { note: Some(note.clone()), done: Some(1), ..ev("error").index(1).message(&note) },
        );
        return Ok(PresetInstallResult { installed: vec![], notes: vec![note] });
    };
    let src = branding_dir(engine).join(&pin.filename);
    let dest = mods_dir.join(&pin.filename);
    mod_progress(
        engine,
        ProgressEvent {
            done: Some(0),
            ..ev("start").filename(&pin.filename).index(0).message(format!("installing {}", pin.filename))
        },
    );
    let _ = std::fs::remove_file(format!("{}.disabled", dest.to_string_lossy()));
    let data = std::fs::read(&src)?;
    let src_sha1 = crate::download::sha1_hex(&data);
    if let Some(expected) = &pin.sha1 {
        if &src_sha1 != expected {
            return Err(code_err(
                500,
                "branding_sha1_mismatch",
                format!("branding source sha1 mismatch for {}: {src_sha1} != {expected}", pin.filename),
            ));
        }
    }
    // Write beside the jar and rename over it — never in place: a running
    // game holds the jar open and an in-place write swaps bytes under the JVM.
    let tmp = PathBuf::from(format!("{}.{}.tmp", dest.to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, &data)?;
    let tmp_sha1 = crate::download::sha1_file(&tmp).await?;
    if tmp_sha1 != src_sha1 {
        let _ = std::fs::remove_file(&tmp);
        return Err(code_err(
            500,
            "branding_sha1_mismatch",
            format!("branding install failed sha1 verification: {tmp_sha1}"),
        ));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        if is_in_use(&e) {
            return Err(code_err(
                500,
                "branding_in_use",
                format!("{} is in use by a running game; it will be updated on the next launch", pin.filename),
            ));
        }
        return Err(e.into());
    }
    mod_progress(
        engine,
        ProgressEvent {
            done: Some(1),
            ..ev("done").filename(&pin.filename).index(1).message(format!("installed {}", pin.filename))
        },
    );
    Ok(PresetInstallResult { installed: vec![pin.filename], notes: vec![] })
}

/// Retro-seed the branding jar for instances that predate the preset (best-effort, never fails).
pub async fn ensure_branding_seeded(engine: &Engine, instance: &InstanceMeta) -> EngineResult<()> {
    if let Err(e) = ensure_branding_seeded_inner(engine, instance).await {
        tracing::warn!("[mods] ensureBrandingSeeded failed for {}: {e}", instance.name);
    }
    Ok(())
}

fn is_brand_jar(f: &str) -> bool {
    let l = f.to_ascii_lowercase();
    (l.starts_with("espectral-menu-") || l.starts_with("espectral-brand-"))
        && (l.ends_with(".jar") || l.ends_with(".jar.disabled"))
}

async fn ensure_branding_seeded_inner(engine: &Engine, instance: &InstanceMeta) -> EngineResult<()> {
    if instance.loader != Loader::Fabric && instance.loader != Loader::Neoforge {
        return Ok(());
    }
    let Some(pin) = branding_pin_for_version(engine, &instance.version, instance.loader) else { return Ok(()) };
    let mods_dir = effective_mods_dir(engine, instance);
    let dest = mods_dir.join(&pin.filename);
    let disabled_twin = format!("{}.disabled", pin.filename);
    let stale: Vec<String> = read_names(&mods_dir)
        .into_iter()
        .filter(|f| *f != pin.filename && *f != disabled_twin && is_brand_jar(f))
        .collect();
    let fresh = if dest.exists() {
        match crate::download::sha1_file(&dest).await {
            Ok(h) => pin.sha1.as_ref().map(|p| *p == h).unwrap_or(true),
            Err(_) => false,
        }
    } else {
        false
    };
    if !fresh {
        install_branding_preset(engine, instance, &mods_dir).await?;
    }
    for f in stale {
        let _ = std::fs::remove_file(mods_dir.join(&f));
        tracing::info!("[mods] pruned stale branding jar {f} from {}", instance.name);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Modrinth fetch helpers
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// fetchJsonWithRetry: 2 retries with linear backoff on top of the engine's
/// own retrying fetch; the final error carries the last status (else 502).
async fn fetch_json_with_retry(engine: &Engine, url: &str, retries: u32) -> EngineResult<Value> {
    let backoff = engine.content_settings().modrinth_retry_backoff;
    let opts = FetchOptions { timeout: std::time::Duration::from_secs(30), ..Default::default() };
    let mut last: Option<EngineError> = None;
    for attempt in 0..=retries {
        match engine.http().fetch_json::<Value>(url, &opts).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                if attempt < retries {
                    tokio::time::sleep(backoff * (attempt + 1)).await;
                }
            }
        }
    }
    let last = last.expect("at least one attempt");
    let status = match &last {
        EngineError::Http(h) => h.status(),
        EngineError::Api { status, .. } => Some(*status),
        _ => None,
    }
    .unwrap_or(502);
    Err(EngineError::Http(HttpError {
        kind: HttpErrorKind::Status(status),
        url: url.to_string(),
        message: format!("fetch failed for {url}: {last}"),
    }))
}

/// Modrinth version metadata (cached per process).
async fn version_meta(engine: &Engine, version_id: &str) -> EngineResult<Value> {
    if let Some(v) = engine.content_state().version_meta.lock().unwrap_or_else(|p| p.into_inner()).get(version_id) {
        return Ok(v.clone());
    }
    let url = format!("{}/version/{}", api(engine), encode_uri_component(version_id));
    let meta = fetch_json_with_retry(engine, &url, 2).await?;
    engine
        .content_state()
        .version_meta
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(version_id.into(), meta.clone());
    Ok(meta)
}

/// Newest listed/approved version of a project for game version + loader.
async fn best_version_for(
    engine: &Engine,
    project_id: &str,
    game_version: &str,
    loader: &str,
) -> EngineResult<Option<Value>> {
    let q = format!(
        "game_versions={}&loaders={}",
        encode_uri_component(&json!([game_version]).to_string()),
        encode_uri_component(&json!([loader]).to_string())
    );
    let url = format!("{}/project/{}/version?{q}", api(engine), encode_uri_component(project_id));
    let list = fetch_json_with_retry(engine, &url, 2).await?;
    Ok(list.as_array().and_then(|a| {
        a.iter().find(|v| matches!(v.get("status").and_then(Value::as_str), Some("listed") | Some("approved"))).cloned()
    }))
}

/// Cached best_version_for; fetch failures are NOT cached (returns None).
async fn best_version_for_cached(engine: &Engine, project_id: &str, game_version: &str, loader: &str) -> Option<Value> {
    let key = format!("{project_id}\u{0}{game_version}\u{0}{loader}");
    if let Some(v) = engine.content_state().best_version.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return v.clone();
    }
    let meta = best_version_for(engine, project_id, game_version, loader).await.ok()?;
    engine.content_state().best_version.lock().unwrap_or_else(|p| p.into_inner()).insert(key, meta.clone());
    meta
}

/// The primary file of a version (else the first).
fn primary_file(meta: &Value) -> Option<&Value> {
    let files = meta.get("files")?.as_array()?;
    files.iter().find(|f| f.get("primary") == Some(&Value::Bool(true))).or_else(|| files.first())
}

fn file_name(f: &Value) -> String {
    s(f, "filename").unwrap_or_default()
}

fn file_sha1(f: &Value) -> Option<String> {
    f.get("hashes").and_then(|h| s(h, "sha1"))
}

fn no_file_error(meta: &Value) -> EngineError {
    EngineError::other(format!("version {} has no downloadable file", s(meta, "id").unwrap_or_default()))
}

fn read_names(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![],
    }
}

fn is_mod_file(f: &str) -> bool {
    f.ends_with(".jar") || f.ends_with(".jar.disabled")
}

// ---------------------------------------------------------------------------
// List / enable-disable
// ---------------------------------------------------------------------------

/// ModEntry[] for an instance: on-disk jars joined with pin metadata, then
/// not-installed pins, branding jar tagged. Uninstalled-pin rows need Modrinth.
pub async fn list_mods(engine: &Engine, instance_name: &str) -> EngineResult<Vec<ModEntry>> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;

    let pins = pins_for_version(engine, &instance.version).await.unwrap_or_default();
    if pins.is_empty() {
        if let Some(note) = pin_note_for_version(&instance.version) {
            tracing::warn!("[mods] {note}");
        }
    }
    let metas: Vec<Option<Value>> =
        futures_util::future::join_all(pins.iter().map(|p| async { version_meta(engine, &p.version_id).await.ok() }))
            .await;
    let mut file_by_version: IndexMap<String, Value> = IndexMap::new();
    let mut pin_filename_to_version: IndexMap<String, String> = IndexMap::new();
    for (p, meta) in pins.iter().zip(&metas) {
        if let Some(file) = meta.as_ref().and_then(primary_file) {
            file_by_version.insert(p.version_id.clone(), file.clone());
            pin_filename_to_version.insert(file_name(file), p.version_id.clone());
        }
    }
    let meta_of = |vid: &str| pins.iter().position(|p| p.version_id == vid).and_then(|i| metas[i].clone());

    let mut on_disk: Vec<String> = read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f)).collect();
    on_disk.sort();
    let mut entries = vec![];
    for file in on_disk {
        let disabled = file.ends_with(".jar.disabled");
        let base = if disabled { file[..file.len() - ".disabled".len()].to_string() } else { file.clone() };
        let abs = mods_dir.join(&file);
        let (mut sha1, mut size) = (None, None);
        if let Ok(st) = std::fs::metadata(&abs) {
            size = Some(st.len());
            sha1 = crate::download::sha1_file(&abs).await.ok();
        }
        let version_id = pin_filename_to_version.get(&base).cloned();
        let pin = version_id.as_ref().and_then(|v| pins.iter().find(|p| &p.version_id == v));
        entries.push(ModEntry {
            filename: base,
            project_slug: pin.map(|p| p.slug.clone()),
            version_number: pin.map(|p| {
                meta_of(&p.version_id).and_then(|m| s(&m, "version_number")).unwrap_or_else(|| p.version_number.clone())
            }),
            version_id,
            sha1,
            size,
            enabled: !disabled,
            installed: true,
        });
    }
    let seen: HashSet<String> = entries.iter().filter_map(|e| e.version_id.clone()).collect();
    for p in &pins {
        if seen.contains(&p.version_id) {
            continue;
        }
        let meta = meta_of(&p.version_id);
        let file = file_by_version.get(&p.version_id);
        entries.push(ModEntry {
            filename: file.map(file_name).unwrap_or_else(|| format!("{}.jar", p.slug)),
            project_slug: Some(p.slug.clone()),
            version_number: Some(
                meta.and_then(|m| s(&m, "version_number")).unwrap_or_else(|| p.version_number.clone()),
            ),
            version_id: Some(p.version_id.clone()),
            sha1: file.and_then(file_sha1),
            size: file.and_then(|f| f.get("size").and_then(Value::as_u64)),
            enabled: false,
            installed: false,
        });
    }
    if let Some(bp) = branding_pin_for_version(engine, &instance.version, instance.loader) {
        if let Some(e) = entries.iter_mut().find(|e| e.filename == bp.filename) {
            e.project_slug = Some(bp.slug.clone());
            e.version_number = bp.version_number.clone();
        }
    }
    Ok(entries)
}

/// `<base>` of a safe mod filename (400 `invalid_filename`).
fn assert_safe_filename(filename: &str) -> EngineResult<String> {
    let base = filename.strip_suffix(".jar.disabled").or_else(|| filename.strip_suffix(".jar"));
    let ok = base.map(|b| {
        !b.is_empty()
            && b.encode_utf16().count() <= 120
            && !b.contains("..")
            && b.chars().all(|c| c.is_ascii_alphanumeric() || " _.+@-".contains(c))
    });
    match (base, ok) {
        (Some(b), Some(true)) => Ok(b.to_string()),
        _ => Err(code_err(400, "invalid_filename", format!("invalid mod filename: {filename}"))),
    }
}

/// Rename `<base>.jar` <-> `<base>.jar.disabled` -> `(filename, enabled)`.
pub async fn set_mod_enabled(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
    enabled: bool,
) -> EngineResult<String> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let base = assert_safe_filename(filename)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let jar = mods_dir.join(format!("{base}.jar"));
    let disabled = mods_dir.join(format!("{base}.jar.disabled"));
    let (src, dst) = if enabled { (&disabled, &jar) } else { (&jar, &disabled) };
    let (src_exists, dst_exists) = (src.exists(), dst.exists());
    if !src_exists && !dst_exists {
        return Err(code_err(404, "mod_not_found", format!("mod not found: {filename}")));
    }
    if src_exists {
        if dst_exists {
            let _ = std::fs::remove_file(dst);
        }
        std::fs::rename(src, dst)?;
    }
    Ok(format!("{base}.jar"))
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// Install a preset into the instance's effective mods dir (mod-progress events).
pub async fn install_preset(
    engine: &Engine,
    instance_name: &str,
    preset: ModPreset,
) -> EngineResult<PresetInstallResult> {
    match install_preset_inner(engine, instance_name, preset).await {
        Ok(r) => Ok(r),
        Err(e) => {
            mod_progress(
                engine,
                ProgressEvent {
                    preset: Some(preset.as_str().into()),
                    error: Some(e.to_string()),
                    ..ProgressEvent::for_instance(instance_name).phase("error").message(e.to_string())
                },
            );
            Err(e)
        }
    }
}

async fn install_preset_inner(
    engine: &Engine,
    instance_name: &str,
    preset: ModPreset,
) -> EngineResult<PresetInstallResult> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let pname = preset.as_str();
    let note_event = |note: &str| ProgressEvent {
        preset: Some(pname.into()),
        note: Some(note.into()),
        ..ProgressEvent::for_instance(instance_name).phase("error").message(note)
    };
    if preset != ModPreset::Branding && instance.loader != Loader::Fabric {
        let note = format!(
            "preset '{pname}' needs a Fabric instance (this one is '{}'); integrated fullbright/no-fog are available under Settings → Client",
            instance.loader
        );
        mod_progress(engine, note_event(&note));
        return Ok(PresetInstallResult { installed: vec![], notes: vec![note] });
    }
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    if preset == ModPreset::Branding {
        return install_branding_preset(engine, &instance, &mods_dir).await;
    }

    let pins = if preset == ModPreset::Qol {
        qol_pins_for_version(engine, &instance.version).await
    } else {
        pins_for_version(engine, &instance.version).await
    };
    let Some(pins) = pins else {
        // Deliberate difference: the JS sent a null note for a supported
        // version whose dynamic pins could not be resolved.
        let note = pin_note_for_version(&instance.version).unwrap_or_else(|| {
            format!("could not resolve the {pname} mods for {} (Modrinth unreachable?)", instance.version)
        });
        tracing::warn!("[mods] {note}");
        mod_progress(engine, note_event(&note));
        return Ok(PresetInstallResult { installed: vec![], notes: vec![note] });
    };

    mod_progress(
        engine,
        ProgressEvent {
            preset: Some(pname.into()),
            ..ProgressEvent::for_instance(instance_name)
                .phase("start")
                .total(pins.len() as u64)
                .index(0)
                .message(format!("installing {} pinned mods", pins.len()))
        },
    );

    let metas = futures_util::future::try_join_all(pins.iter().map(|p| version_meta(engine, &p.version_id))).await?;
    let mut install_map: IndexMap<String, Value> = IndexMap::new();
    for (p, m) in pins.iter().zip(metas) {
        install_map.insert(p.version_id.clone(), m);
    }
    let mut notes = vec![];
    let mut qi = 0;
    while qi < install_map.len() {
        let meta = install_map[qi].clone();
        qi += 1;
        for dep in meta.get("dependencies").and_then(Value::as_array).cloned().unwrap_or_default() {
            if dep.get("dependency_type").and_then(Value::as_str) != Some("required") {
                continue;
            }
            if let Some(vid) = s(&dep, "version_id") {
                if !install_map.contains_key(&vid) {
                    let m = version_meta(engine, &vid).await?;
                    install_map.insert(vid, m);
                }
                continue;
            }
            let pid = s(&dep, "project_id").unwrap_or_default();
            if pins.iter().any(|p| p.project_id == pid) {
                continue;
            }
            if JAR_IN_JAR.contains(&pid.as_str()) {
                notes.push(format!("dependency {pid} is jar-in-jar bundled; skipped"));
                continue;
            }
            match best_version_for(engine, &pid, &instance.version, "fabric").await? {
                Some(best) => {
                    let id = s(&best, "id").unwrap_or_default();
                    install_map.entry(id).or_insert(best);
                }
                None => notes.push(format!("could not resolve dependency {pid} for {}", instance.version)),
            }
        }
    }

    let total = install_map.len();
    let mut items = vec![];
    let mut installed = vec![];
    for (i, meta) in install_map.values().enumerate() {
        let file = primary_file(meta).ok_or_else(|| no_file_error(meta))?;
        let fname = file_name(file);
        installed.push(fname.clone());
        items.push(
            DownloadItem::new(s(file, "url").unwrap_or_default(), mods_dir.join(&fname))
                .sha1(file_sha1(file))
                .size(file.get("size").and_then(Value::as_u64))
                .event(
                    ProgressChannel::Mod,
                    ProgressEvent {
                        preset: Some(pname.into()),
                        ..ProgressEvent::for_instance(instance_name)
                            .filename(&fname)
                            .index(i as u64)
                            .total(total as u64)
                    },
                ),
        );
    }
    engine.http().download_all(items, DownloadAllOptions::default()).await?;
    mod_progress(
        engine,
        ProgressEvent {
            preset: Some(pname.into()),
            ..ProgressEvent::for_instance(instance_name)
                .phase("done")
                .total(total as u64)
                .index(total as u64)
                .message(format!("installed {total} mods"))
        },
    );
    Ok(PresetInstallResult { installed, notes })
}

// ---------------------------------------------------------------------------
// Installed-mod detection (pure)
// ---------------------------------------------------------------------------

/// Filename plausibly belongs to the project slug (whole token / prefix, case-insensitive).
pub fn is_filename_for_slug(filename: &str, slug: &str) -> bool {
    let mut base = filename.to_lowercase();
    if let Some(b) = base.strip_suffix(".jar.disabled") {
        base = b.to_string();
    } else if let Some(b) = base.strip_suffix(".jar") {
        base = b.to_string();
    }
    let target = slug.to_lowercase();
    if base == target || base.starts_with(&format!("{target}-")) || base.starts_with(&format!("{target}_")) {
        return true;
    }
    base.split(['.', '_', '+', '-']).any(|t| t == target)
}

/// 'Ferrite Core' -> 'ferrite-core'.
pub fn slugify_title(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out
}

/// *.jar / *.jar.disabled basenames ('.disabled' stripped); no network.
pub fn installed_mod_filenames(engine: &Engine, instance_name: &str) -> EngineResult<Vec<String>> {
    if !crate::util::is_valid_instance_name(instance_name) {
        return Err(code_err(409, "invalid_instance_name", format!("invalid instance name: {instance_name}")));
    }
    let dir = crate::instances::effective_game_dir_by_name(engine, instance_name).join("mods");
    Ok(read_names(&dir)
        .into_iter()
        .filter_map(|e| {
            if e.ends_with(".jar") {
                Some(e)
            } else {
                e.strip_suffix(".disabled").filter(|b| b.ends_with(".jar")).map(str::to_string)
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Modrinth search
// ---------------------------------------------------------------------------

/// Browse Modrinth (12 per page, sorted by downloads). Empty query = explore.
pub async fn search_modrinth(
    engine: &Engine,
    query: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
    instance_name: Option<&str>,
    offset: u64,
) -> EngineResult<ModrinthSearchResult> {
    let mut facets = vec![json!(["project_type:mod"])];
    if let Some(l) = loader.filter(|l| !l.is_empty()) {
        facets.push(json!([format!("loaders:{l}")]));
    }
    if let Some(v) = game_version.filter(|v| !v.is_empty()) {
        facets.push(json!([format!("versions:{v}")]));
    }
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("query", query)
        .append_pair("facets", &Value::Array(facets).to_string())
        .append_pair("limit", "12")
        .append_pair("offset", &offset.to_string())
        .append_pair("index", "downloads")
        .finish();
    let url = format!("{}/search?{params}", api(engine));
    let data = match fetch_json_with_retry(engine, &url, 2).await {
        Ok(d) => d,
        Err(err) => {
            let status = if let EngineError::Http(h) = &err { h.status().unwrap_or(502) } else { 502 };
            return Err(EngineError::api(status, "MODRINTH_SEARCH_FAILED", format!("Modrinth search failed: {err}")));
        }
    };
    let mut results: Vec<ModrinthProject> = data
        .get("hits")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .map(|hit| ModrinthProject {
                    project_id: s(hit, "project_id").unwrap_or_default(),
                    slug: s(hit, "slug").unwrap_or_default(),
                    title: s(hit, "title").unwrap_or_default(),
                    description: s(hit, "description").unwrap_or_default(),
                    icon_url: s(hit, "icon_url"),
                    downloads: hit.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                    game_versions: hit
                        .get("versions")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                        .unwrap_or_default(),
                    latest_version_number: s(hit, "latest_version").unwrap_or_default(),
                    installed: None,
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(name) = instance_name.filter(|n| !n.is_empty()) {
        let installed = installed_mod_filenames(engine, name)?;
        for hit in &mut results {
            let slug_t = slugify_title(&hit.title);
            hit.installed =
                Some(installed.iter().any(|f| is_filename_for_slug(f, &hit.slug) || is_filename_for_slug(f, &slug_t)));
        }
    }
    let total = data.get("total_hits").and_then(Value::as_u64).unwrap_or(results.len() as u64);
    Ok(ModrinthSearchResult { results, total, offset, limit: 12 })
}

// ---------------------------------------------------------------------------
// Single-mod install with dependencies
// ---------------------------------------------------------------------------

async fn post_version_files(engine: &Engine, hashes: &[String]) -> Result<Map<String, Value>, Option<u16>> {
    let url = format!("{}/version_files", api(engine));
    let res = engine
        .http()
        .client()
        .post(&url)
        .header("Content-Type", "application/json")
        .body(json!({ "hashes": hashes, "algorithm": "sha1" }).to_string())
        .send()
        .await
        .map_err(|_| None)?;
    if !res.status().is_success() {
        return Err(Some(res.status().as_u16()));
    }
    match res.json::<Value>().await {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Ok(Map::new()),
        Err(_) => Err(None),
    }
}

/// Project ids of the jars already in the mods dir (by sha1); None when the lookup fails.
async fn installed_project_ids(engine: &Engine, mods_dir: &Path, files: &[String]) -> Option<HashSet<String>> {
    let mut hashes = vec![];
    for f in files {
        if let Ok(h) = crate::download::sha1_file(&mods_dir.join(f)).await {
            hashes.push(h);
        }
    }
    if hashes.is_empty() {
        return Some(HashSet::new());
    }
    let by_hash = post_version_files(engine, &hashes).await.ok()?;
    Some(by_hash.values().filter_map(|v| s(v, "project_id")).collect())
}

/// project id -> title (cosmetic; never fails).
async fn project_titles(engine: &Engine, ids: &[String]) -> IndexMap<String, String> {
    let mut unique: IndexSet<String> = IndexSet::new();
    for id in ids.iter().filter(|i| !i.is_empty()) {
        unique.insert(id.clone());
    }
    let mut out = IndexMap::new();
    if unique.is_empty() {
        return out;
    }
    let list: Vec<&String> = unique.iter().collect();
    let url = format!("{}/projects?ids={}", api(engine), encode_uri_component(&json!(list).to_string()));
    if let Ok(Value::Array(projects)) = fetch_json_with_retry(engine, &url, 1).await {
        for p in projects {
            if let (Some(id), Some(title)) = (s(&p, "id"), s(&p, "title")) {
                out.insert(id, title);
            }
        }
    }
    out
}

/// planModInstall() result.
#[derive(Debug, Clone)]
pub struct ModPlan {
    pub root: Value,
    /// (project id, version meta), root first.
    pub installs: Vec<(String, Value)>,
    pub project: ModRef,
    pub dependencies: Vec<ModDependencyRef>,
    pub missing: Vec<ModRef>,
    pub optional: Vec<ModRef>,
    pub conflicts: Vec<ModRef>,
}

/// Work out what installing `project_id` means (installs / missing / optional / conflicts).
pub async fn plan_mod_install(
    engine: &Engine,
    instance: &InstanceMeta,
    project_id: &str,
    loader: &str,
) -> EngineResult<ModPlan> {
    let not_found = |msg: String| code_err(404, "MOD_NOT_FOUND_FOR_VERSION", msg);
    let root = best_version_for_cached(engine, project_id, &instance.version, loader)
        .await
        .ok_or_else(|| not_found(format!("no {loader} build of project {project_id} for {}", instance.version)))?;
    if primary_file(&root).is_none() {
        return Err(not_found(format!("project {project_id} has no downloadable file")));
    }
    let mods_dir = effective_mods_dir(engine, instance);
    let files: Vec<String> = read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f)).collect();
    let installed_ids = installed_project_ids(engine, &mods_dir, &files).await;
    let on_disk: HashSet<String> = files.iter().map(|f| f.strip_suffix(".disabled").unwrap_or(f).to_string()).collect();
    let installed_has = |pid: &str| installed_ids.as_ref().map(|s| s.contains(pid)).unwrap_or(false);
    let already_installed = |pid: &str, meta: &Value| match &installed_ids {
        Some(ids) => ids.contains(pid),
        None => primary_file(meta).map(|f| on_disk.contains(&file_name(f))).unwrap_or(false),
    };

    let root_id = s(&root, "project_id").unwrap_or_else(|| project_id.to_string());
    let mut installs: IndexMap<String, Value> = IndexMap::new();
    installs.insert(root_id.clone(), root.clone());
    let mut missing: IndexSet<String> = IndexSet::new();
    let mut optional: IndexSet<String> = IndexSet::new();
    let mut conflicts: IndexSet<String> = IndexSet::new();
    let mut qi = 0;
    while qi < installs.len() {
        let meta = installs[qi].clone();
        let is_root = qi == 0;
        qi += 1;
        for dep in meta.get("dependencies").and_then(Value::as_array).cloned().unwrap_or_default() {
            let pid = s(&dep, "project_id");
            match dep.get("dependency_type").and_then(Value::as_str) {
                Some("incompatible") => {
                    if let Some(p) = pid.filter(|p| installed_has(p)) {
                        conflicts.insert(p);
                    }
                    continue;
                }
                Some("optional") => {
                    if let Some(p) = pid.filter(|p| is_root && !installs.contains_key(p) && !installed_has(p)) {
                        optional.insert(p);
                    }
                    continue;
                }
                Some("required") => {}
                _ => continue, // embedded ships inside the parent
            }
            if let Some(p) = &pid {
                if installs.contains_key(p) || JAR_IN_JAR.contains(&p.as_str()) || installed_has(p) {
                    continue;
                }
            }
            let mut dep_meta = None;
            if let Some(vid) = s(&dep, "version_id") {
                dep_meta = version_meta(engine, &vid).await.ok();
            }
            if dep_meta.is_none() {
                if let Some(p) = &pid {
                    dep_meta = best_version_for_cached(engine, p, &instance.version, loader).await;
                }
            }
            let Some(dep_meta) = dep_meta.filter(|m| primary_file(m).is_some()) else {
                if let Some(id) = pid.clone().or_else(|| s(&dep, "version_id")) {
                    missing.insert(id);
                }
                continue;
            };
            let dep_id = s(&dep_meta, "project_id").or(pid).unwrap_or_default();
            if installs.contains_key(&dep_id) || already_installed(&dep_id, &dep_meta) {
                continue;
            }
            installs.insert(dep_id, dep_meta);
        }
    }
    for m in &missing {
        optional.shift_remove(m);
    }

    let mut all_ids: Vec<String> = installs.keys().cloned().collect();
    all_ids.extend(missing.iter().cloned());
    all_ids.extend(optional.iter().cloned());
    all_ids.extend(conflicts.iter().cloned());
    let titles = project_titles(engine, &all_ids).await;
    let label = |pid: &str| ModRef {
        project_id: pid.to_string(),
        title: titles.get(pid).cloned().unwrap_or_else(|| pid.to_string()),
    };
    Ok(ModPlan {
        project: label(&root_id),
        dependencies: installs
            .iter()
            .filter(|(pid, _)| **pid != root_id)
            .map(|(pid, meta)| {
                let l = label(pid);
                ModDependencyRef {
                    project_id: l.project_id,
                    title: l.title,
                    filename: primary_file(meta).map(file_name).unwrap_or_default(),
                }
            })
            .collect(),
        missing: missing.iter().map(|p| label(p)).collect(),
        optional: optional.iter().map(|p| label(p)).collect(),
        conflicts: conflicts.iter().map(|p| label(p)).collect(),
        installs: installs.into_iter().collect(),
        root,
    })
}

/// installModrinthMod() result: installed filenames (+ the plan summary).
#[derive(Debug, Clone)]
pub struct ModrinthInstallResult {
    pub installed: Vec<String>,
    pub plan: ModPlan,
}

/// Install a Modrinth project plus its missing required dependencies.
pub async fn install_modrinth_mod(
    engine: &Engine,
    instance_name: &str,
    project_id: &str,
    loader: Option<&str>,
    plan: Option<ModPlan>,
) -> EngineResult<ModrinthInstallResult> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let default_loader = if instance.loader == Loader::Neoforge { "neoforge" } else { "fabric" };
    let loader = loader.unwrap_or(default_loader);
    let plan = match plan {
        Some(p) => p,
        None => plan_mod_install(engine, &instance, project_id, loader).await?,
    };
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    let root_file = primary_file(&plan.root).map(file_name).unwrap_or_default();
    let total = plan.installs.len();
    let deps = if total > 1 { format!(" + {} dependencies", total - 1) } else { String::new() };
    mod_progress(
        engine,
        ProgressEvent::for_instance(instance_name)
            .phase("start")
            .filename(&root_file)
            .index(0)
            .total(total as u64)
            .message(format!("installing {root_file}{deps}")),
    );
    let mut items = vec![];
    let mut installed = vec![];
    for (i, (_, meta)) in plan.installs.iter().enumerate() {
        let file = primary_file(meta).ok_or_else(|| no_file_error(meta))?;
        let fname = file_name(file);
        installed.push(fname.clone());
        items.push(
            DownloadItem::new(s(file, "url").unwrap_or_default(), mods_dir.join(&fname))
                .sha1(file_sha1(file))
                .size(file.get("size").and_then(Value::as_u64))
                .event(
                    ProgressChannel::Mod,
                    ProgressEvent::for_instance(instance_name).filename(&fname).index(i as u64).total(total as u64),
                ),
        );
    }
    if let Err(err) = engine.http().download_all(items, DownloadAllOptions::default()).await {
        let e: EngineError = err.into();
        mod_progress(
            engine,
            ProgressEvent {
                error: Some(e.to_string()),
                ..ProgressEvent::for_instance(instance_name).phase("error").filename(&root_file)
            },
        );
        return Err(e);
    }
    mod_progress(
        engine,
        ProgressEvent::for_instance(instance_name)
            .phase("done")
            .filename(&root_file)
            .index(total as u64)
            .total(total as u64)
            .message(format!("installed {root_file}")),
    );
    Ok(ModrinthInstallResult { installed, plan })
}

// ---------------------------------------------------------------------------
// Version / platform change
// ---------------------------------------------------------------------------

fn obj(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Per-jar plan for a version/loader change (JS planModsForTarget). Rows:
/// keep / update / disable / idle / add (see the JS doc); brand jars excluded.
pub async fn plan_mods_for_target(
    engine: &Engine,
    instance: &InstanceMeta,
    target: &VersionTarget,
) -> EngineResult<Value> {
    let target_version = target.version.clone().unwrap_or_else(|| instance.version.clone());
    let target_loader = target.loader.unwrap_or(instance.loader);
    let from_loader = instance.loader;
    let mods_dir = effective_mods_dir(engine, instance);
    let mut files: Vec<String> =
        read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f) && !is_brand_jar(f)).collect();
    files.sort();
    struct Row {
        filename: String,
        enabled: bool,
        sha1: String,
    }
    let mut rows = vec![];
    for file in files {
        let enabled = file.ends_with(".jar");
        let filename = if enabled { file.clone() } else { file[..file.len() - ".disabled".len()].to_string() };
        let Ok(sha1) = crate::download::sha1_file(&mods_dir.join(&file)).await else { continue };
        rows.push(Row { filename, enabled, sha1 });
    }
    if target_loader == Loader::Vanilla {
        return Ok(Value::Array(
            rows.into_iter()
                .map(|r| {
                    Value::Object(obj(vec![
                        ("filename", r.filename.clone().into()),
                        ("enabled", r.enabled.into()),
                        ("title", r.filename.into()),
                        ("action", "idle".into()),
                    ]))
                })
                .collect(),
        ));
    }
    let hashes: Vec<String> = rows.iter().map(|r| r.sha1.clone()).collect();
    let by_hash = if hashes.is_empty() {
        Map::new()
    } else {
        post_version_files(engine, &hashes).await.map_err(|status| match status {
            Some(st) => code_err(502, "MODRINTH_UNREACHABLE", format!("Modrinth version lookup failed (HTTP {st})")),
            None => {
                code_err(502, "MODRINTH_UNREACHABLE", "Modrinth is unreachable; cannot check mods for the new version")
            }
        })?
    };
    let pins = if target_loader == Loader::Fabric {
        pins_for_version(engine, &target_version).await.unwrap_or_default()
    } else {
        vec![]
    };
    let mut present = HashSet::new();
    let mut out: Vec<Map<String, Value>> = vec![];
    for row in &rows {
        let base = |extra: Vec<(&str, Value)>| {
            let mut m = obj(vec![
                ("filename", row.filename.clone().into()),
                ("enabled", row.enabled.into()),
                ("title", row.filename.clone().into()),
            ]);
            for (k, v) in extra {
                m.insert(k.into(), v);
            }
            m
        };
        let Some(pid) = by_hash.get(&row.sha1).and_then(|c| s(c, "project_id")) else {
            out.push(base(vec![("action", "disable".into()), ("reason", "unknown".into())]));
            continue;
        };
        present.insert(pid.clone());
        let pin = pins.iter().find(|p| p.project_id == pid);
        let mut next = None;
        if let Some(p) = pin {
            next = version_meta(engine, &p.version_id).await.ok();
        }
        if next.is_none() {
            next = best_version_for_cached(engine, &pid, &target_version, target_loader.as_str()).await;
        }
        let file = next.as_ref().and_then(primary_file);
        match (file, &next) {
            (Some(f), Some(n)) if file_sha1(f).as_deref() == Some(row.sha1.as_str()) => {
                let _ = n;
                out.push(base(vec![("project_id", pid.clone().into()), ("action", "keep".into())]));
            }
            (Some(f), Some(n)) => out.push(base(vec![
                ("project_id", pid.clone().into()),
                ("action", "update".into()),
                ("bundle", pin.is_some().into()),
                ("version_id", s(n, "id").map(Value::String).unwrap_or(Value::Null)),
                ("to", json!({ "filename": file_name(f), "version_number": n.get("version_number").cloned().unwrap_or(Value::Null) })),
            ])),
            _ => out.push(base(vec![
                ("project_id", pid.clone().into()),
                ("action", "disable".into()),
                ("reason", "no_build".into()),
            ])),
        }
    }
    if target_loader == Loader::Fabric && from_loader != Loader::Fabric {
        for pin in &pins {
            if present.contains(&pin.project_id) {
                continue;
            }
            let Ok(meta) = version_meta(engine, &pin.version_id).await else { continue };
            let Some(f) = primary_file(&meta) else { continue };
            out.push(obj(vec![
                ("filename", file_name(f).into()),
                ("enabled", true.into()),
                ("title", pin.slug.clone().into()),
                ("project_id", pin.project_id.clone().into()),
                ("action", "add".into()),
                ("bundle", true.into()),
                ("version_id", pin.version_id.clone().into()),
                ("to", json!({ "filename": file_name(f), "version_number": meta.get("version_number").cloned().unwrap_or(Value::Null) })),
            ]));
        }
    }
    let ids: Vec<String> =
        out.iter().filter_map(|e| e.get("project_id").and_then(Value::as_str).map(str::to_string)).collect();
    let titles = project_titles(engine, &ids).await;
    for e in &mut out {
        if let Some(t) = e.get("project_id").and_then(Value::as_str).and_then(|p| titles.get(p)).cloned() {
            e.insert("title".into(), Value::String(t));
        }
    }
    Ok(Value::Array(out.into_iter().map(Value::Object).collect()))
}

/// Apply a plan from plan_mods_for_target -> `{ updated, added, disabled, errors }`.
pub async fn apply_mods_plan(engine: &Engine, instance_name: &str, plan: &Value) -> EngineResult<Value> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    let mut updated = vec![];
    let mut added = vec![];
    let disabled = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut errors = vec![];
    let disable = |filename: String| {
        let disabled = disabled.clone();
        let mods_dir = mods_dir.clone();
        async move {
            if mods_dir.join(&filename).exists() {
                set_mod_enabled(engine, instance_name, &filename, false).await?;
            }
            disabled.lock().unwrap_or_else(|p| p.into_inner()).push(filename);
            Ok::<_, EngineError>(())
        }
    };
    for entry in plan.as_array().cloned().unwrap_or_default() {
        let filename = s(&entry, "filename").unwrap_or_default();
        let action = s(&entry, "action").unwrap_or_default();
        let result: EngineResult<()> = async {
            match action.as_str() {
                "disable" => disable(filename.clone()).await?,
                "update" | "add" => {
                    let new_file = if entry.get("bundle") == Some(&Value::Bool(true)) {
                        let vid = s(&entry, "version_id").unwrap_or_default();
                        let meta = version_meta(engine, &vid).await?;
                        let file = primary_file(&meta).ok_or_else(|| no_file_error(&meta))?;
                        let fname = file_name(file);
                        engine
                            .http()
                            .download_all(
                                vec![DownloadItem::new(s(file, "url").unwrap_or_default(), mods_dir.join(&fname))
                                    .sha1(file_sha1(file))
                                    .size(file.get("size").and_then(Value::as_u64))],
                                DownloadAllOptions { concurrency: Some(1), on_item: None },
                            )
                            .await?;
                        fname
                    } else {
                        let pid = s(&entry, "project_id").unwrap_or_default();
                        let res =
                            install_modrinth_mod(engine, instance_name, &pid, Some(instance.loader.as_str()), None)
                                .await?;
                        res.installed.first().cloned().unwrap_or_default()
                    };
                    if action == "update" {
                        if filename != new_file {
                            let _ = std::fs::remove_file(mods_dir.join(&filename));
                            let _ = std::fs::remove_file(mods_dir.join(format!("{filename}.disabled")));
                        }
                        if entry.get("enabled") == Some(&Value::Bool(false)) {
                            set_mod_enabled(engine, instance_name, &new_file, false).await?;
                        }
                        updated.push(json!({ "from": filename, "to": new_file }));
                    } else {
                        added.push(Value::String(new_file));
                    }
                }
                _ => {}
            }
            Ok(())
        }
        .await;
        if let Err(err) = result {
            errors.push(json!({ "filename": filename, "error": err.to_string() }));
            if action == "update" {
                let _ = disable(filename.clone()).await;
            }
        }
    }
    let disabled: Vec<Value> =
        disabled.lock().unwrap_or_else(|p| p.into_inner()).drain(..).map(Value::String).collect();
    Ok(json!({ "updated": updated, "added": added, "disabled": disabled, "errors": errors }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockRequest, MockResponse, MockServer};

    fn branding_assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("branding")
    }

    fn engine_with(server: Option<&MockServer>) -> (tempfile::TempDir, Engine) {
        let (d, e) = temp_engine();
        let api = server.map(|s| s.url("/v2")).unwrap_or_else(|| "http://127.0.0.1:9/v2".into());
        e.update_content_settings(|s| {
            s.modrinth_api = api;
            s.modrinth_retry_backoff = std::time::Duration::from_millis(1);
            s.branding_dir = Some(branding_assets());
            s.background_on_create = false;
        });
        (d, e)
    }

    fn write_instance(e: &Engine, name: &str, version: &str, loader: &str) -> PathBuf {
        let dir = e.paths().instance_dir(name);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(
            dir.join("instance.json"),
            json!({ "name": name, "version": version, "loader": loader }).to_string(),
        )
        .unwrap();
        dir.join("mods")
    }

    #[test]
    fn performance_bundle_support() {
        for v in ["1.21.11", "26.2", "1.22.4", "21.0", "1.21", "1.26", "21", "26"] {
            assert!(supports_performance_bundle(v), "{v}");
        }
        for v in ["1.20.4", "25w11a", "1.15.2", "27.0", "1.27.0", "27", "1.21.11-pre1", "", "not-a-version"] {
            assert!(!supports_performance_bundle(v), "{v}");
        }
        assert!(pin_note_for_version("1.20.4").unwrap().contains("1.21 through 26.x"));
    }

    #[tokio::test]
    async fn pins_exact_and_unsupported_offline() {
        let (_d, e) = engine_with(None);
        for v in PINNED_VERSIONS {
            let pins = pins_for_version(&e, v).await.unwrap();
            assert_eq!(pins.len(), 7, "{v}");
            assert_eq!(pins, pins_by_version(v).unwrap());
        }
        for v in ["1.20.4", "1.15.2", "25w11a", "27.0"] {
            assert!(pins_for_version(&e, v).await.is_none(), "{v}");
        }
        assert_eq!(pins_by_version("26.3").unwrap()[0].version_id, "bAZQdGpg");
    }

    #[test]
    fn branding_support_and_pins_match_bundled_jars() {
        let (_d, e) = engine_with(None);
        assert!(supports_branding(&e, "1.21.11", Loader::Fabric));
        assert!(supports_branding(&e, "26.2", Loader::Fabric));
        assert!(supports_branding(&e, "1.21.1", Loader::Neoforge));
        assert!(!supports_branding(&e, "1.21.1", Loader::Fabric));
        assert!(!supports_branding(&e, "1.21.11", Loader::Neoforge));
        assert!(!supports_branding(&e, "1.21.11", Loader::Vanilla));
        assert!(!supports_branding(&e, "1.22.4", Loader::Fabric));
        assert!(!supports_branding(&e, "", Loader::Fabric));
        let versions = branding_versions(&e);
        for v in ["1.21.11", "26.2", "1.21.1"] {
            assert!(versions.iter().any(|x| x == v), "{v}");
        }
        for (v, loader, prefix) in [
            ("1.21.11", Loader::Fabric, "espectral-menu"),
            ("26.2", Loader::Fabric, "espectral-menu"),
            ("1.21.1", Loader::Neoforge, "espectral-brand"),
        ] {
            let pin = branding_pin_for_version(&e, v, loader).unwrap();
            assert_eq!(pin.slug, prefix);
            assert_eq!(pin.filename, format!("{prefix}-{v}-{}.jar", pin.version_number.clone().unwrap()));
            let data = std::fs::read(branding_assets().join(&pin.filename)).unwrap();
            assert_eq!(pin.sha1.as_deref(), Some(crate::download::sha1_hex(&data).as_str()));
            assert_eq!(pin.size, Some(data.len() as u64));
        }
        assert!(branding_pin_for_version(&e, "1.20.4", Loader::Fabric).is_none());
    }

    #[tokio::test]
    async fn list_mods_tags_branding_even_offline() {
        let server = MockServer::start(|_| MockResponse::status(404)).await;
        let (_d, e) = engine_with(Some(&server));
        for (name, version, loader, slug) in [
            ("brand-fabric-test", "1.21.11", Loader::Fabric, "espectral-menu"),
            ("brand-neoforge-test", "1.21.1", Loader::Neoforge, "espectral-brand"),
        ] {
            let mods = write_instance(&e, name, version, loader.as_str());
            let pin = branding_pin_for_version(&e, version, loader).unwrap();
            std::fs::copy(branding_assets().join(&pin.filename), mods.join(&pin.filename)).unwrap();
            std::fs::write(mods.join("other.jar.disabled"), "x").unwrap();
            let rows = list_mods(&e, name).await.unwrap();
            let row = rows.iter().find(|r| r.filename == pin.filename).expect("branding jar listed");
            assert_eq!(row.project_slug.as_deref(), Some(slug));
            assert_eq!(row.version_number, pin.version_number);
            assert!(row.installed && row.enabled);
            assert_eq!(row.sha1, pin.sha1);
            let other = rows.iter().find(|r| r.filename == "other.jar").unwrap();
            assert!(!other.enabled && other.project_slug.is_none());
            if version == "1.21.11" {
                // the 7 pins are listed as not-installed rows even when Modrinth is down
                assert_eq!(rows.iter().filter(|r| !r.installed).count(), 7);
                assert!(rows
                    .iter()
                    .any(|r| r.filename == "sodium.jar" && r.version_number.as_deref() == Some("0.8.7")));
            }
        }
        let err = list_mods(&e, "missing").await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("instance_not_found")));
    }

    #[tokio::test]
    async fn ensure_branding_seeded_never_rewrites_under_open_handle() {
        let (_d, e) = engine_with(None);
        let pin = branding_pin_for_version(&e, "26.2", Loader::Fabric).unwrap();
        let mods = e.paths().instance_dir("reseed-test").join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let jar = mods.join(&pin.filename);
        let stale = b"stale brand jar bytes from the previous launcher".to_vec();
        std::fs::write(&jar, &stale).unwrap();
        std::fs::write(mods.join("espectral-menu-26.2-0.0.1.jar"), "old").unwrap();
        let inst = InstanceMeta {
            name: "reseed-test".into(),
            version: "26.2".into(),
            loader: Loader::Fabric,
            ..Default::default()
        };
        {
            use std::io::Read;
            let mut fh = std::fs::File::open(&jar).unwrap();
            ensure_branding_seeded(&e, &inst).await.unwrap();
            if cfg!(unix) {
                let mut seen = vec![];
                fh.read_to_end(&mut seen).unwrap();
                assert_eq!(seen, stale, "the open handle still reads the bytes it opened");
            }
        }
        let names = read_names(&mods);
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "no temp file left: {names:?}");
        assert!(!mods.join("espectral-menu-26.2-0.0.1.jar").exists(), "stale brand jar pruned");
        ensure_branding_seeded(&e, &inst).await.unwrap();
        assert_eq!(crate::download::sha1_file(&jar).await.unwrap(), pin.sha1.unwrap());
    }

    #[tokio::test]
    async fn install_preset_codes_and_notes() {
        let (_d, e) = engine_with(None);
        assert_eq!(install_preset(&e, "Invalid!", ModPreset::Branding).await.unwrap_err().status(), 409);
        assert_eq!(install_preset(&e, "no-such-instance", ModPreset::Branding).await.unwrap_err().status(), 404);
        write_instance(&e, "vanilla", "1.21.11", "vanilla");
        let r = install_preset(&e, "vanilla", ModPreset::Performance).await.unwrap();
        assert!(r.installed.is_empty() && r.notes[0].contains("needs a Fabric instance"));
        write_instance(&e, "old", "1.20.4", "fabric");
        let r = install_preset(&e, "old", ModPreset::Performance).await.unwrap();
        assert!(r.notes[0].contains("no performance mod bundle"));
        let mods = write_instance(&e, "brand", "26.3", "fabric");
        let r = install_preset(&e, "brand", ModPreset::Branding).await.unwrap();
        assert_eq!(r.installed, vec!["espectral-menu-26.3-1.6.2.jar"]);
        assert!(mods.join("espectral-menu-26.3-1.6.2.jar").exists());
    }

    #[test]
    fn filename_slug_helpers() {
        assert!(is_filename_for_slug("sodium-fabric-0.8.7.jar", "sodium"));
        assert!(is_filename_for_slug("ferrite-core-8.2.0.jar", "ferrite-core"));
        assert!(is_filename_for_slug("iris-mc1.21.1.jar", "iris"));
        assert!(is_filename_for_slug("modmenu_1.0.jar", "modmenu"));
        assert!(is_filename_for_slug("lithium.jar", "lithium"));
        assert!(is_filename_for_slug("a.jar", "a"));
        assert!(!is_filename_for_slug("lithium.jar", "sodium"));
        assert!(!is_filename_for_slug("ab.jar", "a"));
        assert!(is_filename_for_slug("SODIUM-FABRIC-0.8.7.JAR", "sodium"));
        assert!(is_filename_for_slug("sodium-fabric-0.8.7.jar.disabled", "sodium"));
        assert!(is_filename_for_slug("ferrite-core-8.2.0.jar", "Ferrite-Core"));
        assert_eq!(slugify_title("Ferrite Core"), "ferrite-core");
        assert_eq!(slugify_title("Sodium Extra"), "sodium-extra");
        assert_eq!(slugify_title("  X Y Z  "), "x-y-z");
        assert_eq!(slugify_title(""), "");
        assert_eq!(encode_uri_component("[\"26.2\"]"), "%5B%2226.2%22%5D");
        assert!(assert_safe_filename("a b+c@1.jar").is_ok());
        for bad in ["../x.jar", "x.zip", ".jar", "a/b.jar"] {
            assert_eq!(assert_safe_filename(bad).unwrap_err().raw_code(), Some("invalid_filename"), "{bad}");
        }
    }

    #[tokio::test]
    async fn set_mod_enabled_renames() {
        let (_d, e) = engine_with(None);
        let mods = write_instance(&e, "toggle", "1.21.11", "fabric");
        std::fs::write(mods.join("x.jar"), "x").unwrap();
        assert_eq!(set_mod_enabled(&e, "toggle", "x.jar", false).await.unwrap(), "x.jar");
        assert!(mods.join("x.jar.disabled").exists() && !mods.join("x.jar").exists());
        set_mod_enabled(&e, "toggle", "x.jar.disabled", true).await.unwrap();
        assert!(mods.join("x.jar").exists());
        let err = set_mod_enabled(&e, "toggle", "nope.jar", true).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("mod_not_found")));
    }

    fn modrinth_version(project: &str, version: &str, deps: Value, file_url: &str) -> Value {
        json!({
            "id": version, "project_id": project, "version_number": "1.0.0", "status": "listed",
            "dependencies": deps,
            "files": [{ "primary": true, "filename": format!("{project}-1.0.0.jar"), "url": file_url, "hashes": {}, "size": null }]
        })
    }

    /// Fake Modrinth: versions per project, pinned versions, installed-by-hash, titles, jar bytes.
    fn fake_modrinth(
        versions: Value,
        pinned: Value,
        installed_by_hash: Value,
    ) -> impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static {
        move |req: &MockRequest| {
            let path = req.path.clone();
            if let Some(rest) = path.strip_prefix("/cdn/") {
                return MockResponse::ok(&format!("jar:{rest}"));
            }
            if path.ends_with("/version_files") && req.method == "POST" {
                let body = req.body_json().unwrap();
                let mut out = Map::new();
                for h in body["hashes"].as_array().unwrap() {
                    if let Some(p) = installed_by_hash.get(h.as_str().unwrap()) {
                        out.insert(h.as_str().unwrap().into(), json!({ "project_id": p }));
                    }
                }
                return MockResponse::json(&Value::Object(out));
            }
            if let Some(rest) = path.strip_prefix("/v2/project/") {
                let id = rest.split('/').next().unwrap();
                return MockResponse::json(&versions.get(id).cloned().unwrap_or(json!([])));
            }
            if let Some(id) = path.strip_prefix("/v2/version/") {
                if let Some(v) = pinned.get(id) {
                    return MockResponse::json(v);
                }
            }
            if let Some(q) = path.strip_prefix("/v2/projects?ids=") {
                let decoded = url::form_urlencoded::parse(format!("x={q}").as_bytes()).next().unwrap().1.into_owned();
                let ids: Vec<String> = serde_json::from_str(&decoded).unwrap();
                return MockResponse::json(&Value::Array(
                    ids.iter()
                        .map(|id| json!({ "id": id, "title": format!("Title {id}"), "slug": id.to_lowercase() }))
                        .collect(),
                ));
            }
            MockResponse::status(404)
        }
    }

    #[tokio::test]
    async fn plan_mod_install_classifies_dependencies() {
        let sha = crate::download::sha1_hex;
        let server = MockServer::start(fake_modrinth(
            json!({
                "PROOT1": [modrinth_version("PROOT1", "root-v", json!([
                    { "project_id": "PDEPB1", "dependency_type": "required" },
                    { "project_id": "PAPI01", "dependency_type": "required" },
                    { "project_id": "PEMB01", "dependency_type": "embedded" },
                    { "project_id": "PMISS1", "dependency_type": "required" },
                    { "project_id": "POPT01", "dependency_type": "optional" },
                    { "project_id": "PENEMY", "dependency_type": "incompatible" }
                ]), "http://x/cdn/r")],
                "PDEPB1": [modrinth_version("PDEPB1", "b-v", json!([{ "project_id": "PDEPC1", "version_id": "c-pinned", "dependency_type": "required" }]), "http://x/cdn/b")],
                "PMISS1": []
            }),
            json!({ "c-pinned": modrinth_version("PDEPC1", "c-pinned", json!([]), "http://x/cdn/c") }),
            json!({ sha(b"installed-api"): "PAPI01", sha(b"installed-enemy"): "PENEMY" }),
        ))
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "deps-plan", "26.2", "fabric");
        std::fs::write(mods.join("whatever-api.jar"), "installed-api").unwrap();
        std::fs::write(mods.join("old.jar"), "installed-enemy").unwrap();
        let inst = crate::resolver::load_instance_meta(&e, "deps-plan").unwrap();
        let plan = plan_mod_install(&e, &inst, "PROOT1", "fabric").await.unwrap();
        let ids = |v: &[ModRef]| v.iter().map(|r| r.project_id.clone()).collect::<Vec<_>>();
        assert_eq!(
            plan.installs.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            vec!["PROOT1", "PDEPB1", "PDEPC1"]
        );
        assert_eq!(
            plan.dependencies.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            vec!["Title PDEPB1", "Title PDEPC1"]
        );
        assert_eq!(ids(&plan.missing), vec!["PMISS1"]);
        assert_eq!(ids(&plan.optional), vec!["POPT01"]);
        assert_eq!(ids(&plan.conflicts), vec!["PENEMY"]);
        assert_eq!(plan.project.title, "Title PROOT1");
        let err = plan_mod_install(&e, &inst, "NOPE", "fabric").await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("MOD_NOT_FOUND_FOR_VERSION")));
    }

    #[tokio::test]
    async fn install_modrinth_mod_downloads_mod_and_missing_deps() {
        let srv = MockServer::start(fake_modrinth(json!({}), json!({}), json!({}))).await;
        let cdn = srv.url("/cdn");
        let server = MockServer::start(fake_modrinth(
            json!({
                "PROOT2": [modrinth_version("PROOT2", "root2-v", json!([{ "project_id": "PDEPB2", "dependency_type": "required" }]), &format!("{cdn}/PROOT2.jar"))],
                "PDEPB2": [modrinth_version("PDEPB2", "b2-v", json!([]), &format!("{cdn}/PDEPB2.jar"))]
            }),
            json!({}),
            json!({}),
        ))
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "deps-install", "26.2", "fabric");
        let mut rx = e.subscribe();
        let res = install_modrinth_mod(&e, "deps-install", "PROOT2", Some("fabric"), None).await.unwrap();
        let mut installed = res.installed.clone();
        installed.sort();
        assert_eq!(installed, vec!["PDEPB2-1.0.0.jar", "PROOT2-1.0.0.jar"]);
        let mut on_disk = read_names(&mods);
        on_disk.sort();
        assert_eq!(on_disk, installed);
        assert_eq!(std::fs::read_to_string(mods.join("PROOT2-1.0.0.jar")).unwrap(), "jar:PROOT2.jar");
        let mut phases = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let EngineEvent::ModProgress(p) = ev {
                phases.push(p.phase.unwrap_or_default());
            }
        }
        assert_eq!(phases.first().map(String::as_str), Some("start"));
        assert_eq!(phases.last().map(String::as_str), Some("done"));
        assert_eq!(phases.iter().filter(|p| *p == "done").count(), 3, "{phases:?}");
    }

    #[tokio::test]
    async fn search_maps_hits_and_installed_flags() {
        let server = MockServer::start(|req| {
            assert!(req.path.starts_with("/v2/search?query="), "{}", req.path);
            MockResponse::json(&json!({
                "hits": [
                    { "project_id": "uXXizFIs", "slug": "ferrite-core", "title": "FerriteCore", "description": "d",
                      "icon_url": null, "downloads": 5, "versions": ["26.2"], "latest_version": "9.0.0" },
                    { "project_id": "x", "slug": "other", "title": "Ferrite Core", "description": "", "downloads": 1, "versions": [] }
                ],
                "total_hits": 40
            }))
        })
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "s", "26.2", "fabric");
        std::fs::write(mods.join("ferrite-core-9.0.0.jar.disabled"), "x").unwrap();
        let r = search_modrinth(&e, "fer core", Some("26.2"), Some("fabric"), Some("s"), 12).await.unwrap();
        assert_eq!((r.total, r.offset, r.limit), (40, 12, 12));
        assert_eq!(r.results[0].installed, Some(true));
        assert_eq!(r.results[1].installed, Some(true), "slugified title matches");
        let facets = server.requests()[0].path.clone();
        assert!(facets.starts_with("/v2/search?query=fer+core&facets="), "{facets}");
        assert!(facets.contains("limit=12&offset=12&index=downloads"));
        let q: Vec<(String, String)> =
            url::form_urlencoded::parse(facets.split_once('?').unwrap().1.as_bytes()).into_owned().collect();
        assert_eq!(q[1].1, r#"[["project_type:mod"],["loaders:fabric"],["versions:26.2"]]"#);
        let err = search_modrinth(&e, "", None, None, Some("bad/name"), 0).await.unwrap_err();
        assert_eq!(err.status(), 409);
    }

    #[tokio::test]
    async fn search_failure_is_modrinth_search_failed() {
        let server = MockServer::start(|_| MockResponse::status(404)).await;
        let (_d, e) = engine_with(Some(&server));
        let err = search_modrinth(&e, "", None, None, None, 0).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("MODRINTH_SEARCH_FAILED")));
        assert_eq!(server.hits(), 3, "2 retries");
    }

    #[tokio::test]
    async fn version_change_plan_and_apply() {
        let sha = crate::download::sha1_hex;
        let srv = MockServer::start(|_| MockResponse::status(404)).await;
        let cdn = srv.url("/cdn");
        drop(srv);
        let server = MockServer::start(fake_modrinth(
            json!({
                "PKEEP": [{ "id": "k2", "project_id": "PKEEP", "version_number": "2", "status": "listed",
                            "files": [{ "primary": true, "filename": "keep.jar", "url": "u", "hashes": { "sha1": sha(b"keep") } }] }],
                "PUPD": [{ "id": "u2", "project_id": "PUPD", "version_number": "2.0", "status": "listed",
                           "files": [{ "primary": true, "filename": "upd-2.jar", "url": format!("{cdn}/upd"), "hashes": {} }] }]
            }),
            json!({}),
            json!({ sha(b"keep"): "PKEEP", sha(b"upd"): "PUPD", sha(b"gone"): "PGONE" }),
        ))
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "vc", "26.2", "fabric");
        std::fs::write(mods.join("keep.jar"), "keep").unwrap();
        std::fs::write(mods.join("upd-1.jar.disabled"), "upd").unwrap();
        std::fs::write(mods.join("gone.jar"), "gone").unwrap();
        std::fs::write(mods.join("random.jar"), "random").unwrap();
        std::fs::write(mods.join("espectral-menu-26.2-1.6.2.jar"), "brand").unwrap();
        let inst = crate::resolver::load_instance_meta(&e, "vc").unwrap();

        let idle = plan_mods_for_target(&e, &inst, &VersionTarget { version: None, loader: Some(Loader::Vanilla) })
            .await
            .unwrap();
        assert_eq!(idle.as_array().unwrap().len(), 4);
        assert!(idle.as_array().unwrap().iter().all(|r| r["action"] == "idle"));

        let target = VersionTarget { version: Some("1.21.5".into()), loader: Some(Loader::Neoforge) };
        let plan = plan_mods_for_target(&e, &inst, &target).await.unwrap();
        let by: std::collections::HashMap<String, Value> =
            plan.as_array().unwrap().iter().map(|r| (r["filename"].as_str().unwrap().to_string(), r.clone())).collect();
        assert_eq!(by["keep.jar"]["action"], "keep");
        assert_eq!(by["keep.jar"]["title"], "Title PKEEP");
        assert_eq!(by["upd-1.jar"]["action"], "update");
        assert_eq!(by["upd-1.jar"]["to"]["filename"], "upd-2.jar");
        assert_eq!(by["upd-1.jar"]["enabled"], false);
        assert_eq!(by["gone.jar"]["reason"], "no_build");
        assert_eq!(by["random.jar"]["reason"], "unknown");
        let keys: Vec<&String> = by["random.jar"].as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["filename", "enabled", "title", "action", "reason"]);

        // apply: the cdn is gone (dropped server), so the update fails and the old jar is disabled
        let res = apply_mods_plan(&e, "vc", &plan).await.unwrap();
        assert_eq!(res["errors"].as_array().unwrap().len(), 1);
        assert_eq!(res["errors"][0]["filename"], "upd-1.jar");
        let disabled: Vec<&str> = res["disabled"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(disabled.contains(&"gone.jar") && disabled.contains(&"random.jar") && disabled.contains(&"upd-1.jar"));
        assert!(mods.join("gone.jar.disabled").exists() && mods.join("random.jar.disabled").exists());
        assert!(mods.join("keep.jar").exists());
    }
}
