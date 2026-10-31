//! Port of `src/engine/resolver.mjs` — version manifest, version JSON,
//! Fabric profiles, library / natives / asset installation, classpath.
//!
//! **Owner: game-resolve.**
//!
//! On-disk layout (identical to the JS engine):
//! - `<data>/versions/manifest.json` — `{ fetched_at, latest_release, latest_snapshot, versions[] }`, 6h TTL,
//!   stale-while-revalidate, stale fallback when offline;
//! - `<data>/versions/<id>.json` — Mojang version JSON (flat; sha1-reverified against the manifest);
//! - `<data>/versions/<id>/<id>.jar` — client jar;
//! - `<data>/versions/<id>/<id>.json` — locally-persisted profiles (NeoForge), read by
//!   [`read_local_version_json`];
//! - `<data>/versions/fabric-loaders.json`, `<data>/versions/fabric-loader-<mc>-<loader>.json`
//!   (the combo JSON also memoizes maven `.sha1` sidecars under `espectral_maven_sha1`);
//! - `<data>/libraries/<maven path>`, `<data>/assets/{indexes,objects}`, `<instance>/natives`.
//!
//! ALREADY IN THE FOUNDATION — used here: `crate::rules` (rulesAllow /
//! mavenPath / nativesClassifier / libraryDownload), `crate::paths`,
//! `crate::download` (fetch / verified downloads / Fabric mirrors).
//!
//! Base URLs come from `GameResolveState::endpoints()` (the JS constants by
//! default) so tests can point them at a loopback server.
//!
//! NOT ported: the `node src/engine/resolver.mjs install …` CLI argument
//! parser (the native app has no Node CLI); its body is [`install_instance`].
//!
//! CONTRACT: the `pub` functions below are called by other clusters
//! (game-launch, content). Keep their signatures; add more freely.

mod natives;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::download::{sha1_file, sha1_hex, DownloadItem, FetchOptions};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::events::{EngineEvent, ProgressChannel, ProgressEvent};
use crate::types::{
    AssetsInstall, FabricLibrariesInstall, FabricProfile, InstanceMeta, LibrariesInstall, LibraryDownload, Loader,
    VersionEntry, VersionInfo, VersionJson, VersionManifest,
};

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const FABRIC_META_BASE: &str = "https://meta.fabricmc.net/v2";
pub const FABRIC_MAVEN: &str = "https://maven.fabricmc.net/";
pub const ASSET_BASE: &str = "https://resources.download.minecraft.net";
pub const MAVEN_CENTRAL: &str = crate::rules::MAVEN_CENTRAL;
/// Manifest / Fabric loader list cache TTL (6h).
pub const MANIFEST_TTL_MS: i64 = 6 * 60 * 60 * 1000;
/// Java tiers the launcher can provision.
pub const SUPPORTED_MAJORS: [u32; 5] = [8, 16, 17, 21, 25];

const DEFAULT_FABRIC_MAIN: &str = "net.fabricmc.loader.impl.launch.knot.KnotClient";
const MAVEN_SHA1_MEMO_KEY: &str = "espectral_maven_sha1";

/// installLibraries() options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallLibrariesOptions {
    pub seed_from_fast_client: bool,
    pub force_extract: bool,
}

impl Default for InstallLibrariesOptions {
    fn default() -> Self {
        InstallLibrariesOptions { seed_from_fast_client: true, force_extract: false }
    }
}

/// seedInstance() result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedResult {
    pub version_json: VersionJson,
    pub libraries: LibrariesInstall,
    pub fabric: Option<FabricProfile>,
    pub neoforge: Option<Value>,
    pub assets: AssetsInstall,
}

/// One row of the cached manifest (`versions/manifest.json`), read leniently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    #[serde(default, deserialize_with = "crate::types::de::string")]
    pub id: String,
    #[serde(rename = "type", default, deserialize_with = "crate::types::de::string")]
    pub kind: String,
    #[serde(default, deserialize_with = "crate::types::de::string")]
    pub release_time: String,
    #[serde(default, deserialize_with = "crate::types::de::opt_string")]
    pub sha1: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de::opt_string")]
    pub url: Option<String>,
}

/// The cached manifest document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedManifest {
    pub fetched_at: i64,
    pub latest_release: String,
    pub latest_snapshot: String,
    pub versions: Vec<ManifestEntry>,
}

impl CachedManifest {
    fn into_api(self) -> VersionManifest {
        VersionManifest {
            latest_release: self.latest_release,
            latest_snapshot: self.latest_snapshot,
            versions: self
                .versions
                .into_iter()
                .map(|v| VersionEntry {
                    id: v.id,
                    kind: v.kind,
                    release_time: v.release_time,
                    sha1: v.sha1.unwrap_or_default(),
                    url: v.url.unwrap_or_default(),
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// `path.join(base, rel)` for a forward-slash relative path: each segment is
/// pushed separately so Windows paths use `\` throughout (byte-identical to
/// what Node produced — the AOT cache stamps classpath strings).
pub fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    for seg in rel.split(seps) {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            p.pop();
            continue;
        }
        p.push(seg);
    }
    p
}

/// JS `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn assert_valid_instance_name(name: &str) -> EngineResult<()> {
    if !crate::util::is_valid_instance_name(name) {
        return Err(EngineError::api(409, "invalid_instance_name", format!("invalid instance name: {name}")));
    }
    Ok(())
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn str_at<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(Value::as_str)
}

async fn write_pretty(path: &Path, value: &Value) -> EngineResult<()> {
    let (p, v) = (path.to_path_buf(), value.clone());
    tokio::task::spawn_blocking(move || crate::paths::write_json_atomic(&p, &v))
        .await
        .map_err(|e| EngineError::other(e.to_string()))??;
    Ok(())
}

fn manifest_memo_key(path: &Path) -> Option<String> {
    let st = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", crate::jvm::node_mtime_ms(&st), st.len()))
}

// ---------------------------------------------------------------------------
// Instance meta / classpath (foundation helpers)
// ---------------------------------------------------------------------------

/// loadInstanceMeta(name): instance.json, 409 `invalid_instance_name` /
/// 404 `instance_not_found` (lowercase codes, as in the JS).
pub fn load_instance_meta(engine: &Engine, name: &str) -> EngineResult<InstanceMeta> {
    assert_valid_instance_name(name)?;
    std::fs::read_to_string(engine.paths().instance_json(name))
        .ok()
        .and_then(|s| serde_json::from_str::<InstanceMeta>(&s).ok())
        .ok_or_else(|| EngineError::api(404, "instance_not_found", format!("instance not found: {name}")))
}

/// Classpath: loader libs first, then allowed non-natives vanilla libs, then
/// the client jar (resolver.resolveClasspath).
pub fn resolve_classpath(
    engine: &Engine,
    instance: &InstanceMeta,
    version_json: &VersionJson,
    fabric: Option<&FabricProfile>,
) -> Vec<PathBuf> {
    let libs_dir = engine.paths().libraries_dir();
    let mut out: Vec<PathBuf> =
        fabric.map(|f| f.libraries.iter().map(|l| join_rel(&libs_dir, &l.rel_path)).collect()).unwrap_or_default();
    if let Some(Value::Array(libs)) = version_json.get("libraries") {
        for lib in libs {
            let name = lib.get("name").and_then(Value::as_str).unwrap_or("");
            if crate::rules::rules_allow(lib) && crate::rules::natives_classifier(name).is_none() {
                out.push(join_rel(&libs_dir, &crate::rules::maven_path(name)));
            }
        }
    }
    out.push(engine.paths().client_jar(&instance.version));
    out
}

// ---------------------------------------------------------------------------
// Version manifest (cached 6h in data/versions/manifest.json)
// ---------------------------------------------------------------------------

fn parse_cached_manifest(raw: &str) -> Option<(Value, CachedManifest)> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let versions = v.get("versions")?.as_array()?;
    let entries = versions.iter().filter_map(|e| serde_json::from_value::<ManifestEntry>(e.clone()).ok()).collect();
    let m = CachedManifest {
        fetched_at: v.get("fetched_at").and_then(Value::as_f64).map(|f| f as i64).unwrap_or(0),
        latest_release: v.get("latest_release").and_then(Value::as_str).unwrap_or("").to_string(),
        latest_snapshot: v.get("latest_snapshot").and_then(Value::as_str).unwrap_or("").to_string(),
        versions: entries,
    };
    Some((v, m))
}

async fn fetch_manifest(engine: &Engine) -> EngineResult<CachedManifest> {
    let url = engine.game_resolve_state().endpoints().manifest_url;
    let raw: Value = engine.http().fetch_json(&url, &FetchOptions::default()).await?;
    let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let versions = raw
        .get("versions")
        .and_then(Value::as_array)
        .map(|vs| {
            vs.iter()
                .map(|v| ManifestEntry {
                    id: s(v.get("id")).unwrap_or_default(),
                    kind: s(v.get("type")).unwrap_or_else(|| "release".into()),
                    release_time: s(v.get("releaseTime")).or_else(|| s(v.get("time"))).unwrap_or_default(),
                    sha1: s(v.get("sha1")),
                    url: s(v.get("url")),
                })
                .collect()
        })
        .unwrap_or_default();
    let manifest = CachedManifest {
        fetched_at: crate::platform::now_ms(),
        latest_release: str_at(&raw, "/latest/release").unwrap_or("").to_string(),
        latest_snapshot: str_at(&raw, "/latest/snapshot").unwrap_or("").to_string(),
        versions,
    };
    write_pretty(&engine.paths().version_manifest_cache(), &serde_json::to_value(&manifest)?).await?;
    Ok(manifest)
}

/// Full cached manifest document (resolver.getVersionManifest).
pub async fn load_manifest(engine: &Engine, force: bool) -> EngineResult<CachedManifest> {
    let file = engine.paths().version_manifest_cache();
    if !force {
        if let Some((raw, cached)) = tokio::fs::read_to_string(&file).await.ok().and_then(|s| parse_cached_manifest(&s))
        {
            let fetched_truthy = crate::config::json_truthy(raw.get("fetched_at"));
            if fetched_truthy && crate::platform::now_ms() - cached.fetched_at < MANIFEST_TTL_MS {
                return Ok(cached);
            }
            // Stale-while-revalidate: return stale, refresh in the background.
            let e = engine.clone();
            engine.spawn_background("resolver", async move {
                let _ = fetch_manifest(&e).await;
                Ok(())
            });
            return Ok(cached);
        }
    }
    match fetch_manifest(engine).await {
        Ok(m) => Ok(m),
        Err(err) => {
            // Offline resilience: whatever is cached, however stale.
            if let Some((_, stale)) =
                tokio::fs::read_to_string(&file).await.ok().and_then(|s| parse_cached_manifest(&s))
            {
                return Ok(stale);
            }
            Err(err)
        }
    }
}

/// GET /api/versions source: cached manifest (6h TTL, stale-while-revalidate).
pub async fn get_version_manifest(engine: &Engine, force: bool) -> EngineResult<VersionManifest> {
    Ok(load_manifest(engine, force).await?.into_api())
}

// ---------------------------------------------------------------------------
// Version JSON (cached + sha1-reverified against the manifest entry)
// ---------------------------------------------------------------------------

async fn manifest_entry(engine: &Engine, id: &str) -> EngineResult<Option<ManifestEntry>> {
    let state = engine.game_resolve_state();
    let file = engine.paths().version_manifest_cache();
    if let Some(key) = manifest_memo_key(&file) {
        let memo = state.manifest_memo.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((k, map)) = memo.as_ref() {
            if *k == key {
                return Ok(map.get(id).cloned());
            }
        }
    }
    let manifest = load_manifest(engine, false).await?;
    let mut map = HashMap::new();
    for v in manifest.versions {
        if !v.id.is_empty() {
            map.entry(v.id.clone()).or_insert(v);
        }
    }
    let entry = map.get(id).cloned();
    let key = manifest_memo_key(&file).unwrap_or_else(|| format!("{}:{}", crate::platform::now_ms(), map.len()));
    *state.manifest_memo.lock().unwrap_or_else(|p| p.into_inner()) = Some((key, Arc::new(map)));
    Ok(entry)
}

fn version_not_found(id: &str) -> EngineError {
    EngineError::api(404, "version_not_found", format!("version not found in manifest: {id}"))
}

async fn fetch_version_json(engine: &Engine, id: &str, entry: &ManifestEntry) -> EngineResult<VersionJson> {
    let Some(url) = entry.url.as_deref().filter(|u| !u.is_empty()) else { return Err(version_not_found(id)) };
    let sha1 = entry.sha1.as_deref().filter(|s| !s.is_empty());
    let file = engine.paths().versions_dir().join(format!("{id}.json"));
    if let Ok(buf) = tokio::fs::read(&file).await {
        if sha1.is_none() || sha1 == Some(sha1_hex(&buf).as_str()) {
            if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                return Ok(v);
            }
        }
    }
    let bytes = engine.http().fetch_bytes(url, &FetchOptions::default()).await?;
    if let Some(s) = sha1 {
        if sha1_hex(&bytes) != s {
            return Err(EngineError::api(502, "sha1_mismatch", format!("sha1 mismatch for version {id} ({url})")));
        }
    }
    tokio::fs::create_dir_all(engine.paths().versions_dir()).await?;
    tokio::fs::write(&file, &bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Mojang version JSON for `id` (cached flat as `versions/<id>.json`,
/// sha1-checked against the manifest entry). 404 `version_not_found`.
pub async fn get_version_json(engine: &Engine, id: &str) -> EngineResult<VersionJson> {
    match manifest_entry(engine, id).await? {
        Some(entry) => fetch_version_json(engine, id, &entry).await,
        None => Err(version_not_found(id)),
    }
}

/// `Number(json.javaVersion?.majorVersion ?? 8)`.
fn java_major_of(json: &Value) -> u32 {
    match json.pointer("/javaVersion/majorVersion") {
        None | Some(Value::Null) => 8,
        Some(Value::Number(n)) => n.as_f64().map(|f| f as u32).unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<f64>().map(|f| f as u32).unwrap_or(0),
        Some(_) => 0,
    }
}

/// resolveVersion(id): 404 `version_not_found` when absent from the manifest.
/// `supported` = the launcher provisions the version's Java tier.
pub async fn resolve_version(engine: &Engine, id: &str) -> EngineResult<VersionInfo> {
    let manifest = load_manifest(engine, false).await?;
    let entry = manifest.versions.into_iter().find(|v| v.id == id).ok_or_else(|| version_not_found(id))?;
    let json = fetch_version_json(engine, id, &entry).await?;
    let java_major = java_major_of(&json);
    let supported = SUPPORTED_MAJORS.contains(&java_major);
    let reason = if supported {
        String::new()
    } else {
        let list: Vec<String> = SUPPORTED_MAJORS.iter().map(u32::to_string).collect();
        format!("requires JDK {java_major} — the launcher provisions Temurin {}", list.join("/"))
    };
    Ok(VersionInfo { id: id.to_string(), java_major, supported, reason })
}

// ---------------------------------------------------------------------------
// Local version JSON + inheritance chain merge
// ---------------------------------------------------------------------------

/// Locally persisted version JSON (`versions/<id>/<id>.json`, no network),
/// None when absent/corrupt.
pub fn read_local_version_json(engine: &Engine, id: &str) -> Option<VersionJson> {
    let s = std::fs::read_to_string(engine.paths().version_json(id)).ok()?;
    serde_json::from_str(&s).ok()
}

/// Dedupe library entries by `name`; EARLIER lists win.
pub fn dedupe_by_name(lists: &[&[Value]]) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for list in lists {
        for lib in list.iter() {
            if let Some(name) = lib.get("name").and_then(Value::as_str) {
                if seen.insert(name.to_string()) {
                    out.push(lib.clone());
                }
            }
        }
    }
    out
}

/// Merge two Mojang argument templates (child first): non-flag strings
/// deduped by value, rule objects deduped by JSON; flag strings (`-…`) are
/// NEVER deduped (NeoForge repeats `--add-opens` once per export).
pub fn merge_args(child: Option<&[Value]>, parent: Option<&[Value]>) -> Vec<Value> {
    let mut out = vec![];
    let mut strings = HashSet::new();
    let mut objects = HashSet::new();
    for list in [child, parent].into_iter().flatten() {
        for entry in list {
            match entry {
                Value::String(s) => {
                    if s.starts_with('-') || !strings.contains(s) {
                        strings.insert(s.clone());
                        out.push(entry.clone());
                    }
                }
                other => {
                    if objects.insert(other.to_string()) {
                        out.push(other.clone());
                    }
                }
            }
        }
    }
    out
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn args_of<'a>(v: &'a Value, kind: &str) -> Option<&'a [Value]> {
    v.get("arguments").and_then(|a| a.get(kind)).and_then(Value::as_array).map(Vec::as_slice)
}

fn inherits_break(msg: String) -> EngineError {
    EngineError::api(502, "INHERITS_BREAK", msg)
}

/// [`resolve_version_chain`] with an injectable parent getter (JS `{ getParent }`).
pub async fn resolve_version_chain_with<F, Fut>(json: VersionJson, mut get_parent: F) -> EngineResult<VersionJson>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = EngineResult<Option<VersionJson>>>,
{
    let mut merged = json;
    let mut seen = HashSet::new();
    loop {
        let parent_id = match merged.get("inheritsFrom") {
            None | Some(Value::Null) => break,
            Some(Value::String(s)) if s.is_empty() => break,
            Some(Value::Bool(false)) => break,
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        };
        if !seen.insert(parent_id.clone()) {
            return Err(inherits_break(format!("inheritance cycle at {parent_id}")));
        }
        let Some(parent) = get_parent(parent_id.clone()).await? else {
            let child_id = merged.get("id").and_then(Value::as_str).unwrap_or("undefined");
            return Err(inherits_break(format!("parent version {parent_id} not found for {child_id}")));
        };
        let mut out: Map<String, Value> = parent.as_object().cloned().unwrap_or_default();
        let set = |out: &mut Map<String, Value>, key: &str, v: Option<Value>| match v {
            Some(v) => {
                out.insert(key.to_string(), v);
            }
            None => {
                out.remove(key);
            }
        };
        let pick = |key: &str| -> Option<Value> {
            match merged.get(key) {
                Some(v) if !v.is_null() => Some(v.clone()),
                _ => parent.get(key).cloned(),
            }
        };
        set(&mut out, "id", merged.get("id").cloned());
        for key in ["mainClass", "assetIndex", "javaVersion", "downloads"] {
            let v = pick(key);
            set(&mut out, key, v);
        }
        out.insert(
            "libraries".into(),
            Value::Array(dedupe_by_name(&[arr(&merged, "libraries"), arr(&parent, "libraries")])),
        );
        out.insert(
            "arguments".into(),
            json!({
                "game": merge_args(args_of(&merged, "game"), args_of(&parent, "game")),
                "jvm": merge_args(args_of(&merged, "jvm"), args_of(&parent, "jvm")),
            }),
        );
        merged = Value::Object(out);
    }
    Ok(merged)
}

/// Merge an `inheritsFrom` chain (NeoForge profile over vanilla); parents
/// come from [`get_version_json`]. 502 `INHERITS_BREAK` on a cycle / missing parent.
pub async fn resolve_version_chain(engine: &Engine, json: VersionJson) -> EngineResult<VersionJson> {
    resolve_version_chain_with(json, |id| async move { get_version_json(engine, &id).await.map(Some) }).await
}

// ---------------------------------------------------------------------------
// FastClient seed fast-path
// ---------------------------------------------------------------------------

/// `%APPDATA%/.fastclient` (or the test hook).
fn fast_client_root(engine: &Engine) -> Option<PathBuf> {
    match engine.game_resolve_state().hooks().fast_client_root {
        Some(o) => o,
        None => crate::paths::fast_client_root(),
    }
}

/// Identical files under the FastClient game dir are hardlinked (or copied
/// across volumes) instead of downloaded. True when `dest` is in place.
async fn seed_from_fast_client(engine: &Engine, rel_path: &str, sha1: Option<&str>, dest: &Path) -> bool {
    let Some(root) = fast_client_root(engine) else { return false };
    let src = join_rel(&root, rel_path);
    if !src.exists() {
        return false;
    }
    if let Some(s) = sha1 {
        match sha1_file(&src).await {
            Ok(h) if h == s => {}
            _ => return false,
        }
    }
    if let Some(p) = dest.parent() {
        if tokio::fs::create_dir_all(p).await.is_err() {
            return false;
        }
    }
    if dest.exists() {
        match sha1 {
            None => return true,
            Some(s) => {
                if sha1_file(dest).await.ok().as_deref() == Some(s) {
                    return true;
                }
            }
        }
        let _ = tokio::fs::remove_file(dest).await;
    }
    if tokio::fs::hard_link(&src, dest).await.is_ok() {
        return true;
    }
    tokio::fs::copy(&src, dest).await.is_ok()
}

// ---------------------------------------------------------------------------
// Library installation (classpath jars + natives extraction + client jar)
// ---------------------------------------------------------------------------

struct NativeJob {
    dest: PathBuf,
    classifier: String,
    exclude: Vec<String>,
}

fn extract_exclude(lib: &Value) -> Vec<String> {
    lib.pointer("/extract/exclude")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn lib_event(instance: &str, filename: String, kind: &str) -> ProgressEvent {
    ProgressEvent {
        instance: Some(instance.to_string()),
        filename: Some(filename),
        kind: Some(kind.to_string()),
        ..Default::default()
    }
}

fn is_plain_natives(classifier: &str) -> bool {
    classifier
        .strip_prefix("natives-")
        .map(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
        .unwrap_or(false)
}

/// Download (sha1-verified) libraries + natives + client jar; extract natives
/// into `<instance>/natives` (arch-suffixed classifiers first, the plain
/// default-arch natives last so they win filename collisions).
pub async fn install_libraries(
    engine: &Engine,
    instance: &InstanceMeta,
    version_json: &VersionJson,
    opts: InstallLibrariesOptions,
) -> EngineResult<LibrariesInstall> {
    assert_valid_instance_name(&instance.name)?;
    let seed = opts.seed_from_fast_client;
    let libs_dir = engine.paths().libraries_dir();
    let os = crate::platform::os_name();

    let mut classpath: Vec<PathBuf> = vec![];
    let mut natives: Vec<NativeJob> = vec![];
    let mut jobs: Vec<DownloadItem> = vec![];
    let mut seeded = 0u64;

    for lib in arr(version_json, "libraries") {
        if !crate::rules::rules_allow(lib) {
            continue;
        }
        let name = lib.get("name").and_then(Value::as_str).unwrap_or("");
        let classifier = crate::rules::natives_classifier(name);
        let info = crate::rules::library_download(lib);
        let dest = join_rel(&libs_dir, &info.rel_path);
        let item = DownloadItem::new(info.url.clone(), dest.clone()).sha1(info.sha1.clone()).size(info.size).event(
            ProgressChannel::Mod,
            lib_event(&instance.name, file_name_of(&dest), if classifier.is_some() { "natives" } else { "library" }),
        );
        if let Some(c) = classifier {
            natives.push(NativeJob { dest: dest.clone(), classifier: c, exclude: extract_exclude(lib) });
        } else {
            classpath.push(dest.clone());
            // Legacy schema (<= 1.16.x): `natives: { linux: "natives-linux" }`
            // + `downloads.classifiers[<os-classifier>]`.
            let os_class = lib.get("natives").and_then(|n| n.get(os)).and_then(Value::as_str);
            let cls = os_class.and_then(|c| lib.pointer("/downloads/classifiers").and_then(|m| m.get(c)));
            if let (Some(os_class), Some(cls)) = (os_class, cls.filter(|c| c.is_object())) {
                let cpath = cls.get("path").and_then(Value::as_str).unwrap_or("");
                let ndest = join_rel(&libs_dir, cpath);
                let csha1 = cls.get("sha1").and_then(Value::as_str).map(str::to_string);
                natives.push(NativeJob {
                    dest: ndest.clone(),
                    classifier: os_class.to_string(),
                    exclude: extract_exclude(lib),
                });
                let nitem = DownloadItem::new(cls.get("url").and_then(Value::as_str).unwrap_or(""), ndest.clone())
                    .sha1(csha1.clone())
                    .size(cls.get("size").and_then(Value::as_u64))
                    .event(ProgressChannel::Mod, lib_event(&instance.name, file_name_of(&ndest), "natives"));
                if seed && seed_from_fast_client(engine, &format!("libraries/{cpath}"), csha1.as_deref(), &ndest).await
                {
                    seeded += 1;
                } else {
                    jobs.push(nitem);
                }
            }
        }
        if seed
            && seed_from_fast_client(engine, &format!("libraries/{}", info.rel_path), info.sha1.as_deref(), &dest).await
        {
            seeded += 1;
        } else {
            jobs.push(item);
        }
    }

    // Client jar (downloads.client) -> versions/<id>/<id>.jar.
    let mut client_jar = None;
    if let Some(client) = version_json.pointer("/downloads/client") {
        if let Some(url) = client.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) {
            let version = &instance.version;
            let jar = engine.paths().client_jar(version);
            let csha1 = client.get("sha1").and_then(Value::as_str).map(str::to_string);
            let item = DownloadItem::new(url, jar.clone())
                .sha1(csha1.clone())
                .size(client.get("size").and_then(Value::as_u64))
                .event(ProgressChannel::Mod, lib_event(&instance.name, file_name_of(&jar), "client"));
            if seed
                && seed_from_fast_client(engine, &format!("versions/{version}/{version}.jar"), csha1.as_deref(), &jar)
                    .await
            {
                seeded += 1;
            } else {
                jobs.push(item);
            }
            client_jar = Some(jar);
        }
    }

    let downloaded = jobs.len() as u64;
    if !jobs.is_empty() {
        engine.http().download_all(jobs, Default::default()).await?;
    }

    let natives_dir = engine.paths().instance_natives_dir(&instance.name);
    tokio::fs::create_dir_all(&natives_dir).await?;
    if opts.force_extract {
        if let Ok(mut rd) = tokio::fs::read_dir(&natives_dir).await {
            while let Ok(Some(ent)) = rd.next_entry().await {
                if ent.file_name().to_string_lossy().starts_with(".extract-") {
                    let _ = tokio::fs::remove_file(ent.path()).await;
                }
            }
        }
    }
    natives.sort_by(|a, b| {
        let (ap, bp) = (is_plain_natives(&a.classifier), is_plain_natives(&b.classifier));
        if ap != bp {
            return if ap { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
        }
        a.classifier.cmp(&b.classifier)
    });
    for n in &natives {
        natives::extract_natives(&n.dest, &natives_dir, &n.exclude).await?;
    }

    Ok(LibrariesInstall {
        library_count: classpath.len() as u64,
        seeded,
        downloaded,
        natives_extracted: natives.len() as u64,
        client_jar,
        classpath,
    })
}

// ---------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------

/// Asset index + every object (deduped by hash; asset-progress events).
pub async fn install_assets(
    engine: &Engine,
    instance: &InstanceMeta,
    version_json: &VersionJson,
) -> EngineResult<AssetsInstall> {
    let Some(index) = version_json.get("assetIndex").filter(|i| i.is_object()) else {
        return Ok(AssetsInstall { index: None, objects: 0 });
    };
    let Some(url) = index.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) else {
        return Ok(AssetsInstall { index: None, objects: 0 });
    };
    let index_id = match index.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "undefined".to_string(),
        Some(other) => other.to_string(),
    };
    let index_path = engine.paths().asset_index(&index_id);
    engine
        .http()
        .download_file(
            url,
            &index_path,
            &crate::download::DownloadOptions {
                sha1: index.get("sha1").and_then(Value::as_str).map(str::to_string),
                size: index.get("size").and_then(Value::as_u64),
                ..Default::default()
            },
        )
        .await?;
    let parsed: Value = serde_json::from_slice(&tokio::fs::read(&index_path).await?)?;
    let empty = Map::new();
    let objects = parsed.get("objects").and_then(Value::as_object).unwrap_or(&empty);
    let asset_base = engine.game_resolve_state().endpoints().asset_base;
    let asset_base = asset_base.trim_end_matches('/');
    let objects_dir = engine.paths().assets_dir().join("objects");
    let mut seen = HashSet::new();
    let mut items = vec![];
    for (rel, o) in objects {
        let hash = o.get("hash").and_then(Value::as_str).unwrap_or("");
        if !seen.insert(hash.to_string()) {
            continue;
        }
        let prefix: String = hash.chars().take(2).collect();
        items.push(
            DownloadItem::new(format!("{asset_base}/{prefix}/{hash}"), objects_dir.join(&prefix).join(hash))
                .sha1(Some(hash.to_string()))
                .size(o.get("size").and_then(Value::as_u64))
                .event(
                    ProgressChannel::Asset,
                    ProgressEvent {
                        instance: Some(instance.name.clone()),
                        asset: Some(rel.clone()),
                        kind: Some("object".into()),
                        ..Default::default()
                    },
                ),
        );
    }
    engine.http().download_all(items, Default::default()).await?;
    let total = objects.len() as u64;
    engine.emit(EngineEvent::AssetProgress(ProgressEvent {
        instance: Some(instance.name.clone()),
        phase: Some("done".into()),
        kind: Some("index".into()),
        index: Some(Value::String(index_id.clone())),
        total: Some(total),
        message: Some(format!("asset index {index_id}: {total} objects")),
        ..Default::default()
    }));
    Ok(AssetsInstall { index: Some(index_id), objects: total })
}

// ---------------------------------------------------------------------------
// Fabric loader profile
// ---------------------------------------------------------------------------

/// `versions/fabric-loaders.json`: `{ fetched_at, loaders[], latest_stable }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FabricLoaders {
    pub fetched_at: i64,
    pub loaders: Vec<Value>,
    pub latest_stable: Option<String>,
}

fn parse_fabric_loaders(raw: &str) -> Option<(Value, FabricLoaders)> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let loaders = v.get("loaders")?.as_array()?.clone();
    let out = FabricLoaders {
        fetched_at: v.get("fetched_at").and_then(Value::as_f64).map(|f| f as i64).unwrap_or(0),
        loaders,
        latest_stable: v.get("latest_stable").and_then(Value::as_str).map(str::to_string),
    };
    Some((v, out))
}

fn latest_stable_of(loaders: &[Value]) -> Option<String> {
    let build = |l: &Value| l.get("build").and_then(Value::as_f64).unwrap_or(0.0);
    let mut best: Option<&Value> = None;
    for l in loaders.iter().filter(|l| crate::config::json_truthy(l.get("stable"))) {
        if best.map(|b| build(l) > build(b)).unwrap_or(true) {
            best = Some(l);
        }
    }
    best.or(loaders.first()).and_then(|l| l.get("version")).and_then(Value::as_str).map(str::to_string)
}

async fn fetch_fabric_loaders(engine: &Engine) -> EngineResult<FabricLoaders> {
    let base = engine.game_resolve_state().endpoints().fabric_meta_base;
    let url = format!("{}/versions/loader", base.trim_end_matches('/'));
    let v: Value = engine.http().fetch_json(&url, &FetchOptions::default()).await?;
    let Value::Array(loaders) = v else {
        return Err(EngineError::other(format!("unexpected fabric loader list from {url}")));
    };
    let out =
        FabricLoaders { fetched_at: crate::platform::now_ms(), latest_stable: latest_stable_of(&loaders), loaders };
    write_pretty(&engine.paths().versions_dir().join("fabric-loaders.json"), &serde_json::to_value(&out)?).await?;
    Ok(out)
}

/// Fabric loader list (6h cache, stale-while-revalidate, stale fallback on an
/// outage; `fabric_loader_unreachable` when there is no cache at all).
pub async fn get_fabric_loaders(engine: &Engine, force: bool) -> EngineResult<FabricLoaders> {
    let file = engine.paths().versions_dir().join("fabric-loaders.json");
    if !force {
        if let Some((raw, cached)) = tokio::fs::read_to_string(&file).await.ok().and_then(|s| parse_fabric_loaders(&s))
        {
            let fetched_truthy = crate::config::json_truthy(raw.get("fetched_at"));
            if fetched_truthy && crate::platform::now_ms() - cached.fetched_at < MANIFEST_TTL_MS {
                return Ok(cached);
            }
            if !cached.loaders.is_empty() {
                let e = engine.clone();
                engine.spawn_background("resolver", async move {
                    let _ = fetch_fabric_loaders(&e).await;
                    Ok(())
                });
                return Ok(cached);
            }
        }
    }
    match fetch_fabric_loaders(engine).await {
        Ok(v) => Ok(v),
        Err(err) => {
            if let Some((_, stale)) = tokio::fs::read_to_string(&file).await.ok().and_then(|s| parse_fabric_loaders(&s))
            {
                if !stale.loaders.is_empty() {
                    return Ok(stale);
                }
            }
            if err.raw_code().is_some() {
                return Err(err);
            }
            Err(EngineError::api(err.status(), "fabric_loader_unreachable", err.to_string()))
        }
    }
}

/// True when Fabric meta lists at least one loader for `mc_version`. HTTP
/// 4xx = definitive "no fabric"; network failure / 5xx -> optimistic true.
pub async fn fabric_supported_for_version(engine: &Engine, mc_version: &str) -> EngineResult<bool> {
    let base = engine.game_resolve_state().endpoints().fabric_meta_base;
    let url = format!("{}/versions/loader/{}", base.trim_end_matches('/'), encode_uri_component(mc_version));
    match engine.http().fetch_json::<Value>(&url, &FetchOptions::default()).await {
        Ok(Value::Array(a)) => Ok(!a.is_empty()),
        Ok(_) => Ok(false),
        Err(err) => {
            let status = match &err {
                EngineError::Http(h) => h.status().unwrap_or(0),
                EngineError::Api { status, .. } => *status,
                _ => 0,
            };
            Ok(!(400..500).contains(&status))
        }
    }
}

fn is_sha1(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// maven `<artifact>.sha1` sidecar -> 40-hex digest, None on any failure.
async fn maven_sha1(engine: &Engine, url: &str) -> Option<String> {
    let opts = FetchOptions { timeout: Duration::from_secs(15), ..Default::default() };
    let body = engine.http().fetch_text(&format!("{url}.sha1"), &opts).await.ok()?;
    let hash = body.split_whitespace().next().unwrap_or("").to_lowercase();
    is_sha1(&hash).then_some(hash)
}

/// Fill in sha1 for the intermediary / fabric-loader jars Fabric publishes
/// without one (AOT mtime-churn fix); memoized inside the combo JSON.
async fn hydrate_maven_sha1(engine: &Engine, libraries: &mut [LibraryDownload], combo: &mut Value, cache_file: &Path) {
    if libraries.iter().all(|l| l.sha1.is_some()) {
        return;
    }
    let Some(obj) = combo.as_object_mut() else { return };
    if obj.get(MAVEN_SHA1_MEMO_KEY).map(Value::is_null).unwrap_or(true) {
        obj.insert(MAVEN_SHA1_MEMO_KEY.into(), Value::Object(Map::new()));
    }
    let known = |obj: &Map<String, Value>, rel: &str| -> Option<String> {
        obj.get(MAVEN_SHA1_MEMO_KEY)
            .and_then(|m| m.get(rel))
            .and_then(Value::as_str)
            .filter(|h| is_sha1(h))
            .map(str::to_string)
    };
    let unknown: Vec<(String, String)> = libraries
        .iter()
        .filter(|l| l.sha1.is_none() && known(obj, &l.rel_path).is_none())
        .map(|l| (l.rel_path.clone(), l.url.clone()))
        .collect();
    if !unknown.is_empty() {
        let fetched = futures_util::future::join_all(unknown.iter().map(|(_, u)| maven_sha1(engine, u))).await;
        let mut learned = false;
        if let Some(Value::Object(memo)) = obj.get_mut(MAVEN_SHA1_MEMO_KEY) {
            for ((rel, _), hash) in unknown.iter().zip(fetched) {
                if let Some(h) = hash {
                    memo.insert(rel.clone(), Value::String(h));
                    learned = true;
                }
            }
        }
        if learned {
            // Cache rewrite is an optimization; hashes still apply to this run.
            let _ = write_pretty(cache_file, &Value::Object(obj.clone())).await;
        }
    }
    for lib in libraries.iter_mut().filter(|l| l.sha1.is_none()) {
        if let Some(h) = known(obj, &lib.rel_path) {
            lib.sha1 = Some(h);
        }
    }
}

/// Fabric loader profile (404 `fabric_loader_not_found`). Libraries =
/// launcherMeta.libraries.common + intermediary + fabric-loader.
pub async fn resolve_fabric(
    engine: &Engine,
    mc_version: &str,
    loader_version: Option<&str>,
) -> EngineResult<FabricProfile> {
    let loaders = get_fabric_loaders(engine, false).await?;
    let wanted = loader_version.map(str::to_string).or_else(|| loaders.latest_stable.clone());
    let chosen = wanted
        .as_deref()
        .and_then(|w| loaders.loaders.iter().find(|l| l.get("version").and_then(Value::as_str) == Some(w)))
        .cloned();
    let Some(chosen) = chosen else {
        return Err(EngineError::api(
            404,
            "fabric_loader_not_found",
            format!("fabric loader not found: {}", wanted.as_deref().unwrap_or("null")),
        ));
    };
    let chosen_version = chosen.get("version").and_then(Value::as_str).unwrap_or("").to_string();
    let cache_file = engine.paths().versions_dir().join(format!("fabric-loader-{mc_version}-{chosen_version}.json"));
    let mut combo: Value =
        match tokio::fs::read_to_string(&cache_file).await.ok().and_then(|s| serde_json::from_str(&s).ok()) {
            Some(v) => v,
            None => {
                let base = engine.game_resolve_state().endpoints().fabric_meta_base;
                let url = format!(
                    "{}/versions/loader/{}/{}",
                    base.trim_end_matches('/'),
                    encode_uri_component(mc_version),
                    encode_uri_component(&chosen_version)
                );
                let v: Value = engine.http().fetch_json(&url, &FetchOptions::default()).await?;
                write_pretty(&cache_file, &v).await?;
                v
            }
        };
    let fabric_maven = engine.game_resolve_state().endpoints().fabric_maven;
    let launcher_meta = combo.get("launcherMeta").cloned().unwrap_or(Value::Null);
    let mut libraries: Vec<LibraryDownload> = launcher_meta
        .pointer("/libraries/common")
        .and_then(Value::as_array)
        .map(|c| c.iter().map(crate::rules::library_download).collect())
        .unwrap_or_default();
    if let Some(m) = str_at(&combo, "/intermediary/maven").filter(|s| !s.is_empty()) {
        libraries.push(crate::rules::library_download(&json!({ "name": m, "url": fabric_maven })));
    }
    let loader_maven = chosen.get("maven").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(m) = &loader_maven {
        libraries.push(crate::rules::library_download(&json!({ "name": m, "url": fabric_maven })));
    }
    hydrate_maven_sha1(engine, &mut libraries, &mut combo, &cache_file).await;
    let main_class = str_at(&launcher_meta, "/mainClass/client").unwrap_or(DEFAULT_FABRIC_MAIN).to_string();
    let jvm_args = match combo.pointer("/arguments/jvm") {
        Some(Value::Array(a)) if !a.is_empty() => a.clone(),
        _ => vec![Value::String("-DFabricMcEmu= net.minecraft.client.main.Main ".into())],
    };
    let min_java = match launcher_meta.get("min_java_version") {
        None | Some(Value::Null) => Some(8),
        Some(v) => v.as_f64().map(|f| f as u32),
    };
    Ok(FabricProfile {
        mc_version: mc_version.to_string(),
        loader_version: chosen_version,
        loader_maven,
        intermediary: combo.get("intermediary").cloned().filter(|v| !v.is_null()),
        main_class,
        jvm_args,
        libraries,
        min_java_version: min_java,
        library_count: None,
        seeded: None,
        downloaded: None,
    })
}

/// Download the Fabric loader libraries.
pub async fn install_fabric_libraries(
    engine: &Engine,
    instance: &InstanceMeta,
    fabric: &FabricProfile,
    seed_from_fast_client: bool,
) -> EngineResult<FabricLibrariesInstall> {
    assert_valid_instance_name(&instance.name)?;
    let libs_dir = engine.paths().libraries_dir();
    let mut jobs = vec![];
    let mut seeded = 0u64;
    for lib in &fabric.libraries {
        let dest = join_rel(&libs_dir, &lib.rel_path);
        if seed_from_fast_client
            && self::seed_from_fast_client(engine, &format!("libraries/{}", lib.rel_path), lib.sha1.as_deref(), &dest)
                .await
        {
            seeded += 1;
            continue;
        }
        jobs.push(
            DownloadItem::new(lib.url.clone(), dest.clone())
                .sha1(lib.sha1.clone())
                .size(lib.size)
                .event(ProgressChannel::Mod, lib_event(&instance.name, file_name_of(&dest), "fabric-library")),
        );
    }
    let downloaded = jobs.len() as u64;
    if !jobs.is_empty() {
        engine.http().download_all(jobs, Default::default()).await?;
    }
    Ok(FabricLibrariesInstall { library_count: fabric.libraries.len() as u64, seeded, downloaded })
}

// ---------------------------------------------------------------------------
// Full background seed
// ---------------------------------------------------------------------------

/// Everything an instance needs to launch: libraries, natives, client jar,
/// Fabric profile (+ `<instance>/fabric-profile.json`), the NeoForge loader,
/// assets. Does NOT write instance.json.
pub async fn seed_instance(
    engine: &Engine,
    instance: &InstanceMeta,
    seed_from_fast_client: bool,
) -> EngineResult<SeedResult> {
    assert_valid_instance_name(&instance.name)?;
    if instance.version.is_empty() {
        return Err(EngineError::other("instance.version is required"));
    }
    let version_json = get_version_json(engine, &instance.version).await?;
    let dir = engine.paths().instance_dir(&instance.name);
    for sub in ["mods", "natives", "logs", "saves", "config", "cache"] {
        tokio::fs::create_dir_all(dir.join(sub)).await?;
    }
    let libraries = install_libraries(
        engine,
        instance,
        &version_json,
        InstallLibrariesOptions { seed_from_fast_client, force_extract: false },
    )
    .await?;
    let mut fabric = None;
    if instance.loader == Loader::Fabric {
        let mut f = resolve_fabric(engine, &instance.version, None).await?;
        let counts = install_fabric_libraries(engine, instance, &f, seed_from_fast_client).await?;
        f.library_count = Some(counts.library_count);
        f.seeded = Some(counts.seeded);
        f.downloaded = Some(counts.downloaded);
        let doc = json!({
            "mc_version": f.mc_version,
            "loader_version": f.loader_version,
            "main_class": f.main_class,
            "jvm_args": f.jvm_args,
            "libraries": f.libraries,
        });
        write_pretty(&engine.paths().instance_fabric_profile(&instance.name), &doc).await?;
        fabric = Some(f);
    }
    let mut neoforge = None;
    if instance.loader == Loader::Neoforge {
        let Some(lv) = instance.loader_version.as_deref().filter(|s| !s.is_empty()) else {
            return Err(EngineError::api(
                400,
                "LOADER_VERSION_REQUIRED",
                "loader_version es obligatorio para NeoForge",
            ));
        };
        let r = crate::neoforge::ensure_neoforge_loader(engine, lv, Some(instance)).await?;
        let mut out = Map::new();
        out.insert("loader_version".into(), Value::String(lv.to_string()));
        if let Value::Object(m) = r {
            for (k, v) in m {
                if k != "json" {
                    out.insert(k, v);
                }
            }
        }
        neoforge = Some(Value::Object(out));
    }
    let assets = install_assets(engine, instance, &version_json).await?;
    Ok(SeedResult { version_json, libraries, fabric, neoforge, assets })
}

/// installInstance() request (the body of the JS resolver CLI).
#[derive(Debug, Clone, Default)]
pub struct InstallInstanceRequest {
    pub name: String,
    pub version: String,
    /// 'vanilla' | 'fabric' | 'neoforge'
    pub loader: String,
    pub loader_version: Option<String>,
    pub memory_mb: Option<u32>,
    pub jdk_path_override: Option<String>,
    pub aot_auto_train: Option<bool>,
    pub imported_from: Option<String>,
}

/// installInstance() result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallInstanceResult {
    pub instance: Value,
    pub version_info: VersionInfo,
    #[serde(flatten)]
    pub seed: SeedResult,
}

/// Resolve + seed + write `instance.json` synchronously (JS installInstance,
/// used by `node resolver.mjs install`). 409 `jdk_unsupported` when the
/// version's Java tier cannot be provisioned.
pub async fn install_instance(engine: &Engine, req: InstallInstanceRequest) -> EngineResult<InstallInstanceResult> {
    assert_valid_instance_name(&req.name)?;
    if req.version.is_empty() {
        return Err(EngineError::other("version is required"));
    }
    let loader_s = if req.loader.is_empty() { "vanilla".to_string() } else { req.loader.clone() };
    let Some(loader) = Loader::parse(&loader_s) else {
        return Err(EngineError::other(format!("unknown loader: {loader_s}")));
    };
    if loader == Loader::Neoforge && req.loader_version.as_deref().unwrap_or("").is_empty() {
        return Err(EngineError::other("loader_version is required for neoforge"));
    }
    let version_info = resolve_version(engine, &req.version).await?;
    if !version_info.supported {
        let msg = if version_info.reason.is_empty() {
            format!("version {} is not supported", req.version)
        } else {
            version_info.reason.clone()
        };
        return Err(EngineError::api(409, "jdk_unsupported", msg));
    }
    let doc = json!({
        "name": req.name,
        "version": req.version,
        "loader": loader.as_str(),
        "loader_version": req.loader_version,
        "memory_mb": req.memory_mb.filter(|m| *m > 0),
        "memory_auto_migrated": true,
        "jdk_path_override": req.jdk_path_override,
        "aot_auto_train": req.aot_auto_train.unwrap_or(false),
        "imported_from": req.imported_from,
        "created_at": crate::platform::now_iso(),
    });
    let meta: InstanceMeta = serde_json::from_value(doc.clone())?;
    let seed = seed_instance(engine, &meta, true).await?;
    write_pretty(&engine.paths().instance_json(&req.name), &doc).await?;
    Ok(InstallInstanceResult { instance: doc, version_info, seed })
}

#[cfg(test)]
mod tests;
