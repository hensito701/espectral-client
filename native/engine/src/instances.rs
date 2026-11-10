//! Port of `src/engine/instances.mjs` — instance CRUD (instance.json),
//! summaries, mod-set hash, icons, servers.dat / options.txt access, AOT
//! status read side, background seeding at creation.
//!
//! **Owner: content.** Status: PORTED.
//!
//! instance.json is read and written as a raw JSON object (like the JS,
//! which mutated the parsed object): every key — unknown or oddly typed —
//! survives a PATCH or the one-shot memory migration. [`InstanceMeta`] is the
//! lenient typed view handed to other clusters.
//!
//! Tests: tests/multi-instance.test.mjs, personalization.test.mjs (instances
//! parts), backgrounds.test.mjs (instance parts) — see `mod tests` below.
//!
//! CONTRACT (called by game-launch, onboarding, accounts-skins, identity-discord):
//! get_instance, list_instances, create_instance, effective_game_dir,
//! effective_mods_dir, aot_status, compute_mod_set_hash.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use rand::Rng;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::config::LEGACY_DEFAULT_MEMORY_MB;
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::events::{EngineEvent, ProgressEvent};
use crate::types::{
    AotProof, AotStatus, CreateInstanceRequest, InstanceDetail, InstanceMeta, InstancePatch, InstanceSummary, Loader,
    ModEntry, ModPreset, OptionsPair, OverwritePolicy, ServerEntry,
};
use crate::util::{is_trash_dir_name, is_valid_instance_name};

/// Game subdirs created at instance creation (JS GAME_SUBDIRS).
const GAME_SUBDIRS: [&str; 6] = ["mods", "natives", "logs", "saves", "config", "cache"];

/// 409 `FORGE_UNAVAILABLE`: Forge is not offered for this Minecraft version.
fn forge_unavailable(version: &str) -> EngineError {
    EngineError::conflict(
        "FORGE_UNAVAILABLE",
        format!("Forge is not available for Minecraft {version} (supported: {})", crate::forge::SUPPORTED_MC_SUMMARY),
    )
}

/// `1.X[.Y]` release id -> (X, Y); None for 26.x-style ids and snapshots.
fn classic_release(mc: &str) -> Option<(u32, u32)> {
    let rest = mc.trim().strip_prefix("1.")?;
    let mut parts = rest.split('.');
    let minor = parts.next()?.parse().ok()?;
    let patch = match parts.next() {
        Some(p) => p.parse().ok()?,
        None => 0,
    };
    parts.next().is_none().then_some((minor, patch))
}

/// Whether the create/version-change UIs offer `loader` for Minecraft `mc`
/// (static rule, no network): Fabric from 1.14, NeoForge from 1.20.2,
/// Forge on [`crate::forge::forge_supported`], vanilla always. Snapshots and
/// 26.x ids count as modern (NeoForge needs a release id).
pub fn loader_offered(loader: Loader, mc: &str) -> bool {
    let classic = classic_release(mc);
    match loader {
        Loader::Vanilla => true,
        Loader::Forge => crate::forge::forge_supported(mc),
        Loader::Fabric => classic.map_or(true, |(minor, _)| minor >= 14),
        Loader::Neoforge => match classic {
            Some((minor, patch)) => minor > 20 || (minor == 20 && patch >= 2),
            None => crate::neoforge::neoforge_prefix_for(mc).is_some(),
        },
    }
}

/// Loader builds become file names and URL path segments
/// (`versions/<id>/<id>.json`, maven coordinates), and a pack can supply them:
/// only plain ids pass (`11.15.1.2318`, `21.1.242`, `0.16.14`).
pub fn is_safe_loader_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
        && !v.contains("..")
}

/// 400 BAD_LOADER_VERSION unless [`is_safe_loader_version`].
pub fn validate_loader_version(v: &str) -> EngineResult<()> {
    if is_safe_loader_version(v) {
        Ok(())
    } else {
        Err(EngineError::bad_request("BAD_LOADER_VERSION", format!("invalid loader version '{v}'")))
    }
}

/// Minecraft version ids also become path components (`versions/<id>/…`).
/// Mojang ids are free-form (`1.21.5-rc1`, `25w14a`), so only what could
/// leave the versions folder is refused: separators, drive colons, control
/// characters and dot-only names. 400 BAD_VERSION.
pub fn validate_mc_version(v: &str) -> EngineResult<()> {
    let ok = !v.is_empty()
        && v.len() <= 64
        && v != "."
        && !v.contains("..")
        && !v.chars().any(|c| matches!(c, '/' | '\\' | ':') || c.is_control());
    if ok {
        Ok(())
    } else {
        Err(EngineError::bad_request("BAD_VERSION", format!("invalid Minecraft version '{v}'")))
    }
}

fn invalid_name() -> EngineError {
    EngineError::conflict("INVALID_NAME", "instance name must match ^[A-Za-z0-9 ._-]{1,40}$")
}

// ---------------------------------------------------------------------------
// Paths / personalization helpers
// ---------------------------------------------------------------------------

/// Effective game dir: the custom absolute `game_dir` when set, else
/// `<data>/instances/<name>` (instances.effectiveGameDir).
pub fn effective_game_dir(engine: &Engine, instance: &InstanceMeta) -> PathBuf {
    match instance.game_dir.as_deref() {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => engine.paths().instance_dir(&instance.name),
    }
}

/// effectiveGameDir(name): reads instance.json best-effort.
pub fn effective_game_dir_by_name(engine: &Engine, name: &str) -> PathBuf {
    std::fs::read_to_string(engine.paths().instance_json(name))
        .ok()
        .and_then(|s| serde_json::from_str::<InstanceMeta>(&s).ok())
        .and_then(|m| m.game_dir.filter(|d| !d.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| engine.paths().instance_dir(name))
}

/// `<effective game dir>/mods` — the ONE mods dir every reader/writer uses.
pub fn effective_mods_dir(engine: &Engine, instance: &InstanceMeta) -> PathBuf {
    effective_game_dir(engine, instance).join("mods")
}

/// Hue is an integer on the 0-359 wheel.
pub fn is_valid_hue(h: i64) -> bool {
    (0..=359).contains(&h)
}

/// Circular distance between two hues on the 0-359 wheel.
pub fn hue_distance(a: i64, b: i64) -> i64 {
    let d = (a - b).abs() % 360;
    d.min(360 - d)
}

/// Random hue >= 30 degrees (circular distance) from every hue in `existing`;
/// 50 tries, then anything.
pub fn pick_distinct_hue(existing: &[i64]) -> i64 {
    let taken: Vec<i64> = existing.iter().copied().filter(|h| is_valid_hue(*h)).collect();
    let mut rng = rand::thread_rng();
    for _ in 0..50 {
        let h = rng.gen_range(0..360);
        if taken.iter().all(|t| hue_distance(h, *t) >= 30) {
            return h;
        }
    }
    rng.gen_range(0..360)
}

/// Node `path.isAbsolute` for the host platform (win32 also accepts a
/// leading slash / backslash).
pub(crate) fn is_absolute_like_node(p: &str) -> bool {
    if cfg!(windows) && (p.starts_with('/') || p.starts_with('\\')) {
        return true;
    }
    Path::new(p).is_absolute()
}

/// Validate a game_dir value: None stays None; a string must be absolute
/// (400 BAD_GAME_DIR). mkdir -p happens in create/patch.
pub fn normalize_game_dir(value: Option<&str>) -> EngineResult<Option<String>> {
    match value {
        None => Ok(None),
        Some(v) if is_absolute_like_node(v) => Ok(Some(v.to_string())),
        Some(_) => Err(EngineError::bad_request("BAD_GAME_DIR", "game_dir must be an absolute path or null")),
    }
}

/// JS `String.prototype.localeCompare` approximation for instance-name
/// sorting (ICU root collation over the `[A-Za-z0-9 ._-]` alphabet):
/// whitespace < `_` < `-` < `.` < digits < letters, case-insensitive first,
/// lowercase before uppercase on ties.
pub(crate) fn locale_compare(a: &str, b: &str) -> Ordering {
    fn primary(c: char) -> (u32, u32) {
        match c {
            ' ' => (0, 0),
            '_' => (1, 0),
            '-' => (2, 0),
            ',' => (3, 0),
            ';' => (4, 0),
            ':' => (5, 0),
            '!' => (6, 0),
            '?' => (7, 0),
            '.' => (8, 0),
            '0'..='9' => (100, c as u32),
            'a'..='z' => (200, c as u32),
            'A'..='Z' => (200, c.to_ascii_lowercase() as u32),
            _ => (50, c as u32),
        }
    }
    let pa: Vec<(u32, u32)> = a.chars().map(primary).collect();
    let pb: Vec<(u32, u32)> = b.chars().map(primary).collect();
    match pa.cmp(&pb) {
        Ordering::Equal => {}
        o => return o,
    }
    // Tertiary: lowercase sorts before uppercase.
    let ta: Vec<bool> = a.chars().map(|c| c.is_ascii_uppercase()).collect();
    let tb: Vec<bool> = b.chars().map(|c| c.is_ascii_uppercase()).collect();
    ta.cmp(&tb)
}

fn read_dir_names(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![],
    }
}

fn is_enabled_jar(f: &str) -> bool {
    f.ends_with(".jar")
}

fn is_disabled_jar(f: &str) -> bool {
    f.ends_with(".jar.disabled")
}

/// Hues of all existing instances (auto-assign spacing). Never fails.
fn existing_instance_hues(engine: &Engine) -> Vec<i64> {
    let mut hues = vec![];
    for n in read_dir_names(&engine.paths().instances_dir()) {
        if is_trash_dir_name(&n) {
            continue;
        }
        if let Some(raw) = read_raw_file(&engine.paths().instance_json(&n)) {
            if let Some(h) = crate::config::json_integer(raw.get("hue")).filter(|h| is_valid_hue(*h)) {
                hues.push(h);
            }
        }
    }
    hues
}

/// `config.jvm.build` (config cache only — a listing must never probe or
/// download a JVM). Missing/non-string = None (`unknown` in the key).
fn config_java_build(engine: &Engine) -> Option<String> {
    engine.config().get("jvm").and_then(|j| j.get("build").and_then(Value::as_str).map(str::to_string))
}

// ---------------------------------------------------------------------------
// Raw instance.json access
// ---------------------------------------------------------------------------

fn read_raw_file(path: &Path) -> Option<Map<String, Value>> {
    let s = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&s).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

fn write_raw(engine: &Engine, name: &str, raw: &Map<String, Value>) -> std::io::Result<()> {
    crate::paths::write_json_atomic(&engine.paths().instance_json(name), raw)
}

/// Typed lenient view of a raw instance.json object.
pub(crate) fn meta_from_raw(raw: &Map<String, Value>) -> InstanceMeta {
    if let Ok(m) = serde_json::from_value::<InstanceMeta>(Value::Object(raw.clone())) {
        return m;
    }
    // `mods` is the only strictly typed field; drop it when it is not an array.
    let mut r = raw.clone();
    r.remove("mods");
    serde_json::from_value::<InstanceMeta>(Value::Object(r)).unwrap_or_default()
}

/// getInstance(): raw object, 409 INVALID_NAME, 404 NOT_FOUND, one-shot
/// 3072/4096 -> automatic memory migration (persisted).
pub(crate) fn get_instance_raw(engine: &Engine, name: &str) -> EngineResult<Map<String, Value>> {
    if !is_valid_instance_name(name) {
        return Err(invalid_name());
    }
    let mut raw = read_raw_file(&engine.paths().instance_json(name))
        .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("instance '{name}' does not exist")))?;
    if raw.get("memory_auto_migrated") != Some(&Value::Bool(true)) {
        let legacy = raw
            .get("memory_mb")
            .and_then(Value::as_f64)
            .map(|mb| LEGACY_DEFAULT_MEMORY_MB.iter().any(|l| *l as f64 == mb))
            .unwrap_or(false);
        if legacy {
            raw.insert("memory_mb".into(), Value::Null);
        }
        raw.insert("memory_auto_migrated".into(), Value::Bool(true));
        if let Err(e) = write_raw(engine, name, &raw) {
            tracing::warn!("[instances] could not persist memory migration for '{name}': {e}");
        }
    }
    Ok(raw)
}

/// Load an instance (409 INVALID_NAME, 404 NOT_FOUND; applies the memory migration).
pub async fn get_instance(engine: &Engine, name: &str) -> EngineResult<InstanceMeta> {
    get_instance_raw(engine, name).map(|r| meta_from_raw(&r))
}

// ---------------------------------------------------------------------------
// Summaries / listing / mod-set hash
// ---------------------------------------------------------------------------

/// InstanceSummary for one instance (contract shape; `running` unset).
pub async fn get_summary(engine: &Engine, name: &str) -> EngineResult<InstanceSummary> {
    let raw = get_instance_raw(engine, name)?;
    let inst = meta_from_raw(&raw);
    let files = read_dir_names(&effective_mods_dir(engine, &inst));
    let enabled = files.iter().filter(|f| is_enabled_jar(f) && !is_disabled_jar(f)).count() as u32;
    let disabled = files.iter().filter(|f| is_disabled_jar(f)).count() as u32;

    let key = crate::aot::cache_key(&inst.version, config_java_build(engine).as_deref(), None);
    let cache_exists = engine.paths().aot_cache_file(&key).exists();
    let memory_int = crate::config::json_integer(raw.get("memory_mb"));

    Ok(InstanceSummary {
        name: inst.name.clone(),
        version: inst.version.clone(),
        loader: inst.loader,
        loader_version: inst.loader_version.clone(),
        modpack: inst.modpack.clone(),
        modpack_version: inst.modpack_version.clone(),
        memory_mb: match memory_int {
            Some(mb) => u32::try_from(mb).unwrap_or(0),
            None => engine.config().default_memory_mb(),
        },
        memory_auto: memory_int.is_none(),
        hue: inst.hue,
        game_dir: inst.game_dir.clone(),
        has_icon: engine.paths().instance_icon(name).exists(),
        mod_count: enabled + disabled,
        enabled_mod_count: enabled,
        aot_key: Some(key),
        aot_cache_exists: cache_exists,
        imported_from: inst.imported_from.clone(),
        capture_tools: inst.capture_tools,
        created_at: inst.created_at.clone(),
        running: None,
    })
}

/// Every instance summary, sorted by name (trash dirs skipped). Never fails on a bad entry.
pub async fn list_instances(engine: &Engine) -> EngineResult<Vec<InstanceSummary>> {
    let mut out = vec![];
    for n in read_dir_names(&engine.paths().instances_dir()) {
        if is_trash_dir_name(&n) {
            continue;
        }
        if !engine.paths().instance_json(&n).is_file() {
            continue;
        }
        if let Ok(s) = get_summary(engine, &n).await {
            out.push(s);
        }
    }
    out.sort_by(|a, b| locale_compare(&a.name, &b.name));
    Ok(out)
}

/// Canonical sha256 over the enabled jars' `name:sha1` lines (memoized by stat).
pub async fn compute_mod_set_hash(engine: &Engine, instance: &InstanceMeta) -> EngineResult<String> {
    let dir = effective_game_dir(engine, instance);
    let mods_dir = dir.join("mods");
    let enabled: Vec<String> =
        read_dir_names(&mods_dir).into_iter().filter(|f| is_enabled_jar(f) && !is_disabled_jar(f)).collect();
    let mut stat_rows = vec![];
    let mut stat_error = false;
    for f in &enabled {
        match std::fs::metadata(mods_dir.join(f)) {
            Ok(st) => stat_rows.push(format!("{f}:{}:{}", st.len(), crate::platform::mtime_ms(&st))),
            Err(_) => stat_error = true,
        }
    }
    stat_rows.sort();
    let stat_key = if stat_error { None } else { Some(stat_rows.join("\n")) };

    let state = engine.content_state();
    if let Some(k) = &stat_key {
        let cache = state.mod_set_hash.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((ck, hash)) = cache.get(&dir) {
            if ck == k {
                return Ok(hash.clone());
            }
        }
    }

    let mut lines = vec![];
    for f in &enabled {
        match crate::download::sha1_file(&mods_dir.join(f)).await {
            Ok(h) => lines.push(format!("{f}:{h}")),
            Err(_) => lines.push(format!("{f}:error")),
        }
    }
    lines.sort();
    let hash = hex::encode(Sha256::digest(lines.join("\n").as_bytes()));
    if let Some(k) = stat_key {
        state.mod_set_hash.lock().unwrap_or_else(|p| p.into_inner()).insert(dir, (k, hash.clone()));
    }
    Ok(hash)
}

/// GET /api/instances/:name detail: `{ summary, mods, servers, options, aot }`.
pub async fn get_instance_detail(engine: &Engine, name: &str) -> EngineResult<InstanceDetail> {
    let inst = get_instance(engine, name).await?;
    let summary = get_summary(engine, name).await?;
    let mods = list_mods_scan(engine, &inst).await;
    let servers = read_servers(engine, &inst);
    let options = read_options_for(engine, &inst);
    let aot = aot_status(engine, &inst).await?;
    Ok(InstanceDetail { summary, mods, servers, options, aot })
}

// ---------------------------------------------------------------------------
// Create / delete / patch
// ---------------------------------------------------------------------------

/// Loader build recorded on the instance: the requested one, else the
/// loader's default for this Minecraft version (NeoForge: newest stable,
/// Forge: promotions recommended/latest). Probe failures (offline, mocked
/// resolver) stay optimistic — seeding and the launch resolve it again — but a
/// definitive "no build exists" answer is a 409 before anything is written.
async fn auto_loader_version(
    engine: &Engine,
    loader: Loader,
    version: &str,
    requested: Option<String>,
) -> EngineResult<Option<String>> {
    if let Some(v) = requested.clone().filter(|s| !s.trim().is_empty()) {
        validate_loader_version(&v)?;
        return Ok(Some(v));
    }
    match loader {
        Loader::Forge => match crate::forge::resolve_forge_version(engine, version, None).await {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.raw_code() == Some("FORGE_UNAVAILABLE") => Err(e),
            Err(e) => {
                tracing::debug!("[instances] forge default build probe failed ({e}); resolving at launch");
                Ok(None)
            }
        },
        Loader::Neoforge => match crate::neoforge::latest_neoforge_for(engine, version).await {
            Ok(Some(v)) => Ok(Some(v)),
            Ok(None) => {
                Err(EngineError::conflict("NEOFORGE_UNAVAILABLE", format!("no NeoForge build for Minecraft {version}")))
            }
            Err(e) => {
                tracing::debug!("[instances] neoforge default build probe failed ({e}); resolving at launch");
                Ok(None)
            }
        },
        _ => Ok(None),
    }
}

/// Create an instance (validation, hue, game dirs, optional two-hop import,
/// background seeding, Fabric performance preset).
pub async fn create_instance(engine: &Engine, req: CreateInstanceRequest) -> EngineResult<InstanceSummary> {
    let name = req.name.clone();
    let loader = req.loader.unwrap_or(Loader::Vanilla);
    if !is_valid_instance_name(&name) {
        return Err(invalid_name());
    }
    if req.version.is_empty() {
        return Err(EngineError::bad_request("VERSION_REQUIRED", "version is required"));
    }
    validate_mc_version(&req.version)?;
    // Loader builds are auto-picked when the caller sends none (the wizard
    // never does): NeoForge -> newest stable for this Minecraft version,
    // Forge -> the promotions default build.
    if loader == Loader::Forge && !crate::forge::forge_supported(&req.version) {
        return Err(forge_unavailable(&req.version));
    }
    if loader == Loader::Fabric {
        // Best-effort probe: a failure (network, or resolver not available)
        // continues optimistically — resolveFabric at launch is the authority.
        match crate::resolver::fabric_supported_for_version(engine, &req.version).await {
            Ok(false) => {
                return Err(EngineError::conflict(
                    "FABRIC_UNAVAILABLE",
                    format!("no Fabric loader for Minecraft {}", req.version),
                ))
            }
            Ok(true) => {}
            Err(e) => tracing::debug!("[instances] fabric support probe failed ({e}); continuing"),
        }
    }
    if engine.paths().instance_dir(&name).exists() {
        return Err(EngineError::conflict("ALREADY_EXISTS", format!("instance '{name}' already exists")));
    }
    let hue = match req.hue {
        None => pick_distinct_hue(&existing_instance_hues(engine)),
        Some(h) if is_valid_hue(h) => h,
        Some(_) => {
            return Err(EngineError::bad_request("BAD_HUE", "hue must be an integer between 0 and 359 or null"));
        }
    };
    let game_dir = normalize_game_dir(req.game_dir.as_deref())?;
    if let Some(d) = &game_dir {
        std::fs::create_dir_all(d)?;
    }

    let now = crate::platform::now_iso();
    let aot_auto_train = match engine.config().get("aot_auto_train") {
        None | Some(Value::Null) => Value::Bool(true),
        Some(v) => v,
    };
    let import_from = req.import_from.clone().filter(|s| !s.is_empty());
    let loader_version = auto_loader_version(engine, loader, &req.version, req.loader_version.clone()).await?;
    let mut raw = match json!({
        "name": name,
        "version": req.version,
        "loader": loader.as_str(),
        "loader_version": loader_version,
        "modpack": req.modpack,
        "modpack_version": req.modpack_version,
        "memory_mb": req.memory_mb.filter(|mb| *mb >= 512),
        "memory_auto_migrated": true,
        "mods": [],
        "imported_from": import_from,
        "jdk_path_override": req.jdk_path_override,
        "aot_auto_train": aot_auto_train,
        "hue": hue,
        "game_dir": game_dir,
        "created_at": now,
        "updated_at": now,
    }) {
        Value::Object(m) => m,
        _ => unreachable!(),
    };
    if req.merge_optionslc {
        raw.insert("merge_optionslc".into(), Value::Bool(true));
    }

    std::fs::create_dir_all(engine.paths().instance_dir(&name))?;
    let game_base = game_dir.as_ref().map(PathBuf::from).unwrap_or_else(|| engine.paths().instance_dir(&name));
    for sub in GAME_SUBDIRS {
        std::fs::create_dir_all(game_base.join(sub))?;
    }
    write_raw(engine, &name, &raw)?;
    let inst = meta_from_raw(&raw);

    if let Some(source_id) = &import_from {
        if let Err(e) =
            crate::import::import_profile(engine, &inst, source_id, OverwritePolicy::Never, req.merge_optionslc).await
        {
            tracing::warn!("[instances] two-hop import from '{source_id}' failed for '{name}': {e}");
            engine.emit(EngineEvent::ModProgress(ProgressEvent {
                status: Some("error".into()),
                ..ProgressEvent::for_instance(&name).phase("import").message(e.to_string())
            }));
        }
    }

    if engine.content_settings().background_on_create {
        seed_libraries(engine, inst.clone());
        // Contract decision 6: a Fabric instance auto-queues the performance preset.
        if inst.loader == Loader::Fabric && crate::mods::supports_performance_bundle(&inst.version) {
            let bg = engine.clone();
            let n = name.clone();
            engine.spawn_background("instances", async move {
                if let Err(e) = crate::mods::install_preset(&bg, &n, ModPreset::Performance).await {
                    tracing::warn!("[instances] auto-install performance preset for '{n}' failed: {e}");
                }
                Ok(())
            });
        }
    }
    get_summary(engine, &name).await
}

/// Background library seeding (resolver.seedInstance) — best effort, reported
/// over mod-progress `{ phase: 'seeding', status }`.
fn seed_libraries(engine: &Engine, instance: InstanceMeta) {
    let seeding = |status: &str| ProgressEvent {
        status: Some(status.into()),
        ..ProgressEvent::for_instance(&instance.name).phase("seeding")
    };
    engine.emit(EngineEvent::ModProgress(seeding("start")));
    let bg = engine.clone();
    engine.spawn_background("instances", async move {
        match crate::resolver::seed_instance(&bg, &instance, true).await {
            Ok(_) => bg.emit(EngineEvent::ModProgress(ProgressEvent {
                status: Some("done".into()),
                ..ProgressEvent::for_instance(&instance.name).phase("seeding")
            })),
            Err(e) => {
                tracing::warn!("[instances] background library seeding skipped for '{}': {e}", instance.name);
                bg.emit(EngineEvent::ModProgress(ProgressEvent {
                    status: Some("error".into()),
                    ..ProgressEvent::for_instance(&instance.name).phase("seeding").message(e.to_string())
                }));
            }
        }
        Ok(())
    });
}

/// Unique trash dir name `<name>.deleted.<ms>[-<n>]`.
fn trash_dir_name(engine: &Engine, name: &str) -> String {
    let stamp = crate::platform::now_ms();
    let root = engine.paths().instances_dir();
    let mut candidate = format!("{name}.deleted.{stamp}");
    let mut n = 2;
    while root.join(&candidate).exists() {
        candidate = format!("{name}.deleted.{stamp}-{n}");
        n += 1;
    }
    candidate
}

/// DELETE: move the instance dir to a recoverable `<name>.deleted.<ms>` sibling.
pub async fn delete_instance(engine: &Engine, name: &str) -> EngineResult<(String, String)> {
    get_instance_raw(engine, name)?;
    let trash = trash_dir_name(engine, name);
    std::fs::rename(engine.paths().instance_dir(name), engine.paths().instances_dir().join(&trash))?;
    Ok((name.to_string(), trash))
}

/// PATCH an instance (rename, memory, jdk override, aot flag, hue, game_dir,
/// enabled mods). The running guard lives in the facade.
pub async fn patch_instance(engine: &Engine, name: &str, patch: &InstancePatch) -> EngineResult<InstanceSummary> {
    let mut raw = get_instance_raw(engine, name)?;
    let mut current = name.to_string();
    if let Some(raw_rename) = patch.rename_target() {
        let next = raw_rename.trim();
        if !next.is_empty() && next != current {
            if !is_valid_instance_name(next) {
                return Err(invalid_name());
            }
            if engine.paths().instance_dir(next).exists() {
                return Err(EngineError::conflict("ALREADY_EXISTS", format!("instance '{next}' already exists")));
            }
            let from = engine.paths().instance_dir(&current);
            std::fs::rename(&from, engine.paths().instance_dir(next))?;
            engine.content_state().mod_set_hash.lock().unwrap_or_else(|p| p.into_inner()).remove(&from);
            raw.insert("name".into(), Value::String(next.to_string()));
            current = next.to_string();
        }
    }
    if let Some(mem) = patch.memory_mb {
        match mem {
            None => {
                raw.insert("memory_mb".into(), Value::Null);
            }
            Some(mb) => {
                if !(512..=65536).contains(&mb) {
                    return Err(EngineError::bad_request(
                        "BAD_MEMORY",
                        "memory_mb must be an integer between 512 and 65536",
                    ));
                }
                raw.insert("memory_mb".into(), Value::from(mb));
            }
        }
    }
    if let Some(jdk) = &patch.jdk_path_override {
        raw.insert("jdk_path_override".into(), jdk.clone().map(Value::String).unwrap_or(Value::Null));
    }
    if let Some(b) = patch.aot_auto_train {
        raw.insert("aot_auto_train".into(), Value::Bool(b));
    }
    if let Some(hue) = patch.hue {
        if let Some(h) = hue {
            if !is_valid_hue(h) {
                return Err(EngineError::bad_request("BAD_HUE", "hue must be an integer between 0 and 359 or null"));
            }
        }
        raw.insert("hue".into(), hue.map(Value::from).unwrap_or(Value::Null));
    }
    if let Some(gd) = &patch.game_dir {
        let next = normalize_game_dir(gd.as_deref())?;
        if let Some(d) = &next {
            std::fs::create_dir_all(d)?;
        }
        raw.insert("game_dir".into(), next.map(Value::String).unwrap_or(Value::Null));
    }
    if let Some(enabled) = &patch.enabled_mods {
        let inst = meta_from_raw(&raw);
        apply_enabled_mods(engine, &inst, enabled)?;
    }
    if let Some(want) = patch.capture_tools {
        // Flipping the flag OWNS the Replay Mod jar: on installs it, off
        // removes it (shader packs + Iris stay). The install runs before the
        // flag is persisted so a failed download never leaves
        // `capture_tools: true` on an instance that has no Replay Mod.
        let want = want == Some(true);
        let inst = meta_from_raw(&raw);
        if want != inst.capture_tools {
            if want {
                crate::mods::install_capture_tools(engine, &inst).await?;
            } else {
                crate::mods::remove_capture_tools(engine, &inst).await?;
            }
        }
        raw.insert("capture_tools".into(), Value::Bool(want));
    }
    raw.insert("updated_at".into(), Value::String(crate::platform::now_iso()));
    write_raw(engine, &current, &raw)?;
    get_summary(engine, &current).await
}

/// Rename mod jars to match the desired enabled set.
fn apply_enabled_mods(engine: &Engine, inst: &InstanceMeta, enabled_mods: &[String]) -> EngineResult<()> {
    let mods_dir = effective_mods_dir(engine, inst);
    let wanted: std::collections::HashSet<String> =
        enabled_mods.iter().map(|f| f.strip_suffix(".disabled").unwrap_or(f).to_string()).collect();
    for f in read_dir_names(&mods_dir) {
        let is_jar = is_enabled_jar(&f);
        let is_disabled = is_disabled_jar(&f);
        if !is_jar && !is_disabled {
            continue;
        }
        let base = if is_disabled { f[..f.len() - ".disabled".len()].to_string() } else { f.clone() };
        let enable = wanted.contains(&base);
        if enable && is_disabled {
            std::fs::rename(mods_dir.join(&f), mods_dir.join(&base))?;
        } else if !enable && is_jar {
            std::fs::rename(mods_dir.join(&f), mods_dir.join(format!("{base}.disabled")))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// Read an instance icon (PNG bytes). 404 when the instance or icon is missing.
pub async fn read_instance_icon(engine: &Engine, name: &str) -> EngineResult<Vec<u8>> {
    get_instance_raw(engine, name)?;
    std::fs::read(engine.paths().instance_icon(name))
        .map_err(|_| EngineError::not_found("NOT_FOUND", format!("instance '{name}' has no icon")))
}

/// Store an instance icon (validated PNG bytes).
pub async fn write_instance_icon(engine: &Engine, name: &str, png: &[u8]) -> EngineResult<()> {
    get_instance_raw(engine, name)?;
    std::fs::write(engine.paths().instance_icon(name), png)?;
    Ok(())
}

/// Remove an instance icon (missing file is a no-op).
pub async fn remove_instance_icon(engine: &Engine, name: &str) -> EngineResult<()> {
    get_instance_raw(engine, name)?;
    let _ = std::fs::remove_file(engine.paths().instance_icon(name));
    Ok(())
}

// ---------------------------------------------------------------------------
// Mods scan / servers.dat / options.txt
// ---------------------------------------------------------------------------

/// The detail view's mod rows: a plain directory scan (no network, sha1 null).
/// The JS called `mods.listMods(inst)` with the object, which always threw
/// and fell back to this scan — kept as-is for parity.
async fn list_mods_scan(engine: &Engine, inst: &InstanceMeta) -> Vec<ModEntry> {
    let mods_dir = effective_mods_dir(engine, inst);
    let mut out = vec![];
    for f in read_dir_names(&mods_dir) {
        if !is_enabled_jar(&f) && !is_disabled_jar(&f) {
            continue;
        }
        let base = f.strip_suffix(".disabled").unwrap_or(&f).to_string();
        let size = std::fs::metadata(mods_dir.join(&f)).map(|m| m.len()).unwrap_or(0);
        out.push(ModEntry {
            filename: base,
            project_slug: None,
            version_number: None,
            version_id: None,
            sha1: None,
            size: Some(size),
            enabled: is_enabled_jar(&f),
            installed: true,
        });
    }
    out.sort_by(|a, b| locale_compare(&a.filename, &b.filename));
    out
}

fn read_servers(engine: &Engine, inst: &InstanceMeta) -> Vec<ServerEntry> {
    std::fs::read(effective_game_dir(engine, inst).join("servers.dat"))
        .ok()
        .and_then(|b| crate::nbt::parse_servers_dat(&b).ok())
        .unwrap_or_default()
}

/// GET /api/instances/:name/servers
pub async fn read_servers_dat(engine: &Engine, name: &str) -> EngineResult<Vec<ServerEntry>> {
    let inst = get_instance(engine, name).await?;
    Ok(read_servers(engine, &inst))
}

/// PUT /api/instances/:name/servers — write servers.dat (modern format).
pub async fn write_servers(engine: &Engine, name: &str, servers: &[ServerEntry]) -> EngineResult<usize> {
    let inst = get_instance(engine, name).await?;
    let buf = crate::nbt::write_servers_dat(servers)?;
    std::fs::write(effective_game_dir(engine, &inst).join("servers.dat"), buf)?;
    Ok(servers.len())
}

/// instances.parseOptionsTxt (raw pairs, header included) — the fallback
/// parser; readers use [`crate::import::read_options_txt`].
pub fn parse_options_txt(content: &str) -> Vec<OptionsPair> {
    let mut pairs = vec![];
    for line in content.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if line.is_empty() {
            continue;
        }
        match line.find(':') {
            None => pairs.push((line.to_string(), String::new())),
            Some(i) => pairs.push((line[..i].to_string(), line[i + 1..].to_string())),
        }
    }
    pairs
}

fn read_options_for(engine: &Engine, inst: &InstanceMeta) -> Vec<OptionsPair> {
    match std::fs::read(effective_game_dir(engine, inst).join("options.txt")) {
        Ok(b) => crate::import::read_options_txt(&String::from_utf8_lossy(&b)),
        Err(_) => vec![],
    }
}

/// GET /api/instances/:name/options -> `[[k, v], …]`
pub async fn read_options(engine: &Engine, name: &str) -> EngineResult<Vec<OptionsPair>> {
    let inst = get_instance(engine, name).await?;
    Ok(read_options_for(engine, &inst))
}

/// importOptionsFromInstance result `(copied, count, options)`.
pub type OptionsImport = (bool, usize, Vec<OptionsPair>);

/// Copy `<source>/options.txt` into `<target>/options.txt` (missing source = no-op).
pub async fn import_options_from_instance(engine: &Engine, target: &str, source: &str) -> EngineResult<OptionsImport> {
    if !is_valid_instance_name(target) || !is_valid_instance_name(source) {
        return Err(EngineError::conflict("INVALID_NAME", "invalid instance name"));
    }
    let target_inst = get_instance(engine, target).await?;
    let source_inst = get_instance(engine, source).await?;
    if target == source {
        return Err(EngineError::conflict("SAME_INSTANCE", "no puedes importar opciones de la misma instancia"));
    }
    let source_path = effective_game_dir(engine, &source_inst).join("options.txt");
    if !source_path.exists() {
        return Ok((false, 0, read_options_for(engine, &target_inst)));
    }
    std::fs::copy(&source_path, effective_game_dir(engine, &target_inst).join("options.txt"))?;
    let options = read_options_for(engine, &target_inst);
    Ok((true, options.len(), options))
}

// ---------------------------------------------------------------------------
// AOT status (read side)
// ---------------------------------------------------------------------------

/// AotStatus snapshot for an instance (read side).
pub async fn aot_status(engine: &Engine, instance: &InstanceMeta) -> EngineResult<AotStatus> {
    let summary = get_summary(engine, &instance.name).await?;
    let ready = Some(crate::aot::loader_supports_aot(instance.loader));
    let Some(key) = summary.aot_key else {
        return Ok(AotStatus {
            key: None,
            cache_path: None,
            cache_exists: false,
            cache_size_bytes: 0,
            stale: None,
            ready_to_train: ready,
            trained_at: None,
            proof: None,
        });
    };
    let cache_path = engine.paths().aot_cache_file(&key);
    let (cache_exists, cache_size) = match std::fs::metadata(&cache_path) {
        Ok(st) => (st.is_file(), st.len()),
        Err(_) => (false, 0),
    };
    let trained_at = std::fs::read_to_string(engine.paths().aot_meta(&key))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|m| m.get("trained_at").and_then(Value::as_str).map(str::to_string));

    let mut stale = None;
    if cache_exists && crate::aot::loader_supports_aot(instance.loader) {
        // Exactly the identity the next launch will use (the JVM gets a
        // different classpath on Fabric/JDK 25+, and the flags decide validity
        // too), so the chip converges once a retrain runs.
        if let Ok(identity) = crate::launch::aot_identity(engine, instance).await {
            stale = Some(crate::aot::is_cache_stale_with_flags(engine, &key, &identity.classpath, &identity.vm_flags));
        }
    }

    let proof = aot_proof_sweep(engine, instance);
    Ok(AotStatus {
        key: Some(key),
        cache_path: Some(cache_path.to_string_lossy().into_owned()),
        cache_exists,
        cache_size_bytes: cache_size,
        stale,
        ready_to_train: ready,
        trained_at,
        proof,
    })
}

/// Newest recent aot-<pid>.log proof; keeps at most 8 logs (L19b).
fn aot_proof_sweep(engine: &Engine, instance: &InstanceMeta) -> Option<AotProof> {
    let base = effective_game_dir(engine, instance);
    let re = regex::Regex::new(r"^aot-\d+\.log$").expect("regex");
    let mtime = |f: &str| std::fs::metadata(base.join(f)).map(|m| crate::platform::mtime_ms(&m)).unwrap_or(0.0);
    let mut logs: Vec<(String, f64)> =
        read_dir_names(&base).into_iter().filter(|f| re.is_match(f)).map(|f| (f.clone(), mtime(&f))).collect();
    logs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
    const MAX_AOT_LOGS: usize = 8;
    if logs.len() > MAX_AOT_LOGS {
        for (f, _) in &logs[..logs.len() - MAX_AOT_LOGS] {
            let _ = std::fs::remove_file(base.join(f));
        }
    }
    let now = crate::platform::now_ms() as f64;
    let recent: Vec<&String> = logs.iter().map(|(f, _)| f).filter(|f| now - mtime(f) < 24.0 * 3_600_000.0).collect();
    let last = recent.last()?;
    let log_path = base.join(last);
    let content = String::from_utf8_lossy(&std::fs::read(&log_path).ok()?).into_owned();
    let using = content.contains("Using AOT-linked classes: true");
    Some(AotProof {
        log_path: Some(log_path.to_string_lossy().into_owned()),
        using_aot_linked_classes: using,
        refusal: if using { None } else { crate::aot::aot_refusal_line(&content) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    fn quiet(e: &Engine) {
        e.update_content_settings(|s| s.background_on_create = false);
    }

    fn req(name: &str) -> CreateInstanceRequest {
        CreateInstanceRequest { name: name.into(), version: "1.21.11".into(), ..Default::default() }
    }

    fn code(r: EngineResult<impl std::fmt::Debug>) -> (u16, String) {
        let e = r.unwrap_err().envelope();
        (e.status, e.code)
    }

    #[test]
    fn loader_offered_follows_each_loaders_range() {
        use Loader::*;
        let offered = |l: Loader, v: &str| loader_offered(l, v);
        // 1.8.9: only vanilla + Forge (no Fabric/NeoForge build exists).
        assert!(offered(Vanilla, "1.8.9") && offered(Forge, "1.8.9"));
        assert!(!offered(Fabric, "1.8.9") && !offered(Neoforge, "1.8.9"));
        assert!(!offered(Fabric, "1.13.2") && offered(Fabric, "1.14") && offered(Fabric, "1.21.11"));
        assert!(!offered(Neoforge, "1.20.1") && offered(Neoforge, "1.20.2") && offered(Neoforge, "1.21.1"));
        assert!(offered(Neoforge, "26.3") && offered(Fabric, "26.3") && offered(Forge, "26.3"));
        assert!(!offered(Forge, "1.20.1") && !offered(Forge, "1.16.5"));
        // Snapshots: Fabric yes, NeoForge needs a release id.
        assert!(offered(Fabric, "26.4-snapshot-6") && !offered(Neoforge, "26.4-snapshot-6"));
        assert!(offered(Fabric, "24w14a") && !offered(Neoforge, "24w14a"));
    }

    #[test]
    fn hue_helpers() {
        assert_eq!(hue_distance(10, 350), 20);
        assert_eq!(hue_distance(0, 180), 180);
        assert!(is_valid_hue(0) && is_valid_hue(359) && !is_valid_hue(360) && !is_valid_hue(-1));
        for _ in 0..20 {
            let h = pick_distinct_hue(&[0, 100, 200]);
            assert!(is_valid_hue(h));
        }
    }

    #[test]
    fn locale_compare_orders_like_js() {
        let mut v = vec!["b", "B", "a", "A b", "a-b", "a_b", "a.b", "a1", "Zeta", "alpha"];
        v.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(v, vec!["a", "A b", "a_b", "a-b", "a.b", "a1", "alpha", "b", "B", "Zeta"]);
    }

    #[tokio::test]
    async fn create_auto_assigns_spaced_hues_and_stores_explicit_hue() {
        let (_d, e) = temp_engine();
        quiet(&e);
        let a = create_instance(&e, req("hue-a")).await.unwrap();
        let b = create_instance(&e, req("hue-b")).await.unwrap();
        let (ha, hb) = (a.hue.unwrap(), b.hue.unwrap());
        assert!(is_valid_hue(ha) && is_valid_hue(hb));
        assert!(hue_distance(ha, hb) >= 30, "{ha}/{hb}");
        let c = create_instance(&e, CreateInstanceRequest { hue: Some(123), ..req("hue-c") }).await.unwrap();
        assert_eq!(c.hue, Some(123));
        // instance.json shape (key order of the JS engine)
        let raw = read_raw_file(&e.paths().instance_json("hue-c")).unwrap();
        let keys: Vec<&str> = raw.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "name",
                "version",
                "loader",
                "loader_version",
                "modpack",
                "modpack_version",
                "memory_mb",
                "memory_auto_migrated",
                "mods",
                "imported_from",
                "jdk_path_override",
                "aot_auto_train",
                "hue",
                "game_dir",
                "created_at",
                "updated_at"
            ]
        );
        for sub in GAME_SUBDIRS {
            assert!(e.paths().instance_dir("hue-c").join(sub).is_dir(), "{sub}");
        }
    }

    #[tokio::test]
    async fn create_validation_codes() {
        let (_d, e) = temp_engine();
        quiet(&e);
        for hue in [-1, 360] {
            assert_eq!(
                code(create_instance(&e, CreateInstanceRequest { hue: Some(hue), ..req("hue-bad") }).await),
                (400, "BAD_HUE".into())
            );
        }
        let rel = CreateInstanceRequest { game_dir: Some("relative/path".into()), ..req("dir-bad") };
        assert_eq!(code(create_instance(&e, rel).await), (400, "BAD_GAME_DIR".into()));
        let empty = CreateInstanceRequest { game_dir: Some(String::new()), ..req("dir-bad2") };
        assert_eq!(code(create_instance(&e, empty).await), (400, "BAD_GAME_DIR".into()));
        assert_eq!(code(create_instance(&e, req("bad/name")).await), (409, "INVALID_NAME".into()));
        let nover = CreateInstanceRequest { version: String::new(), ..req("nover") };
        assert_eq!(code(create_instance(&e, nover).await), (400, "VERSION_REQUIRED".into()));
        // Loader builds are auto-picked: the wizard never sends one.
        let nf = CreateInstanceRequest { loader: Some(Loader::Neoforge), ..req("nf") };
        let created = create_instance(&e, nf).await.unwrap();
        assert!(
            created.loader_version.as_deref().is_some_and(|v| !v.is_empty()),
            "NeoForge build auto-picked: {:?}",
            created.loader_version
        );
        let forge = CreateInstanceRequest { loader: Some(Loader::Forge), version: "1.20.1".into(), ..req("forge-bad") };
        assert_eq!(code(create_instance(&e, forge).await), (409, "FORGE_UNAVAILABLE".into()));
        create_instance(&e, req("dup")).await.unwrap();
        assert_eq!(code(create_instance(&e, req("dup")).await), (409, "ALREADY_EXISTS".into()));
    }

    #[tokio::test]
    async fn create_refuses_path_like_versions_and_loader_builds() {
        let (_d, e) = temp_engine();
        quiet(&e);
        let nf = CreateInstanceRequest {
            loader: Some(Loader::Neoforge),
            loader_version: Some("21.1.242/neoforge-21.1.242-installer.jar#/../../../config".into()),
            ..req("nf-escape")
        };
        assert_eq!(code(create_instance(&e, nf).await), (400, "BAD_LOADER_VERSION".into()));
        let mc = CreateInstanceRequest { version: "../../config".into(), ..req("mc-escape") };
        assert_eq!(code(create_instance(&e, mc).await), (400, "BAD_VERSION".into()));
        assert!(!e.paths().instance_dir("nf-escape").exists() && !e.paths().instance_dir("mc-escape").exists());
    }

    #[test]
    fn loader_and_minecraft_version_rules() {
        for ok in ["11.15.1.2318", "21.1.242", "0.16.14", "14.23.5.2859", "47.4.28-beta", "1.0+build.5"] {
            assert!(is_safe_loader_version(ok), "{ok}");
        }
        for bad in ["", ".5", "a/b", "a\\b", "1..2", "1#x", "1:2", "-1", &"9".repeat(65)] {
            assert!(!is_safe_loader_version(bad), "{bad}");
        }
        for ok in ["1.21.11", "26.3", "25w14a", "1.21.5-rc1", "1.8.9"] {
            validate_mc_version(ok).unwrap();
        }
        for bad in ["", ".", "..", "1.8/../x", "C:x", "a\\b", "x\ny"] {
            assert!(validate_mc_version(bad).is_err(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn game_dir_lifecycle() {
        let (d, e) = temp_engine();
        quiet(&e);
        let custom = d.path().join("custom-game-dir").to_string_lossy().into_owned();
        let created = create_instance(&e, CreateInstanceRequest { game_dir: Some(custom.clone()), ..req("dir-inst") })
            .await
            .unwrap();
        assert_eq!(created.game_dir.as_deref(), Some(custom.as_str()));
        assert!(Path::new(&custom).join("mods").is_dir());
        // instance.json stays in the instance dir
        assert!(e.paths().instance_json("dir-inst").is_file());

        let p = InstancePatch { game_dir: Some(None), ..Default::default() };
        assert_eq!(patch_instance(&e, "dir-inst", &p).await.unwrap().game_dir, None);
        let custom2 = d.path().join("custom-game-dir-2").to_string_lossy().into_owned();
        let p = InstancePatch { game_dir: Some(Some(custom2.clone())), ..Default::default() };
        assert_eq!(patch_instance(&e, "dir-inst", &p).await.unwrap().game_dir.as_deref(), Some(custom2.as_str()));
        assert!(Path::new(&custom2).exists());
        let p = InstancePatch { game_dir: Some(Some("relative/again".into())), ..Default::default() };
        assert_eq!(code(patch_instance(&e, "dir-inst", &p).await), (400, "BAD_GAME_DIR".into()));
    }

    #[tokio::test]
    async fn patch_hue_and_summary_fields() {
        let (_d, e) = temp_engine();
        quiet(&e);
        create_instance(&e, req("sum-inst")).await.unwrap();
        let p = patch_instance(&e, "sum-inst", &InstancePatch { hue: Some(Some(200)), ..Default::default() })
            .await
            .unwrap();
        assert_eq!((p.hue, p.game_dir.clone(), p.has_icon), (Some(200), None, false));
        let bad = InstancePatch { hue: Some(Some(999)), ..Default::default() };
        assert_eq!(code(patch_instance(&e, "sum-inst", &bad).await), (400, "BAD_HUE".into()));
        let cleared =
            patch_instance(&e, "sum-inst", &InstancePatch { hue: Some(None), ..Default::default() }).await.unwrap();
        assert_eq!(cleared.hue, None);
    }

    #[tokio::test]
    async fn icon_roundtrip() {
        let (_d, e) = temp_engine();
        quiet(&e);
        create_instance(&e, req("icon-inst")).await.unwrap();
        let png = crate::util::lenient_base64_decode(TINY_PNG_B64);
        assert!(!get_summary(&e, "icon-inst").await.unwrap().has_icon);
        assert_eq!(read_instance_icon(&e, "icon-inst").await.unwrap_err().status(), 404);
        write_instance_icon(&e, "icon-inst", &png).await.unwrap();
        assert!(get_summary(&e, "icon-inst").await.unwrap().has_icon);
        assert_eq!(read_instance_icon(&e, "icon-inst").await.unwrap(), png);
        remove_instance_icon(&e, "icon-inst").await.unwrap();
        assert!(!get_summary(&e, "icon-inst").await.unwrap().has_icon);
        remove_instance_icon(&e, "icon-inst").await.unwrap();
        assert_eq!(write_instance_icon(&e, "no-such-instance", &png).await.unwrap_err().status(), 404);
    }

    #[tokio::test]
    async fn legacy_memory_defaults_migrate_once() {
        let (_d, e) = temp_engine();
        let write = |name: &str, mb: u32| {
            crate::paths::write_json_atomic(
                &e.paths().instance_json(name),
                &json!({ "name": name, "version": "26.2", "loader": "fabric", "memory_mb": mb, "odd": [1] }),
            )
            .unwrap();
        };
        write("mem-legacy-3g", 3072);
        write("mem-legacy-4g", 4096);
        write("mem-chosen", 6144);
        for n in ["mem-legacy-3g", "mem-legacy-4g"] {
            let s = get_summary(&e, n).await.unwrap();
            assert!(s.memory_auto, "{n}");
            assert_eq!(s.memory_mb, e.config().default_memory_mb());
        }
        let chosen = get_summary(&e, "mem-chosen").await.unwrap();
        assert_eq!((chosen.memory_auto, chosen.memory_mb), (false, 6144));
        // unknown keys survive the migration write
        assert_eq!(read_raw_file(&e.paths().instance_json("mem-chosen")).unwrap()["odd"], json!([1]));

        let p = InstancePatch { memory_mb: Some(Some(4096)), ..Default::default() };
        patch_instance(&e, "mem-legacy-4g", &p).await.unwrap();
        let pinned = get_summary(&e, "mem-legacy-4g").await.unwrap();
        assert_eq!((pinned.memory_auto, pinned.memory_mb), (false, 4096));
        let p = InstancePatch { memory_mb: Some(None), ..Default::default() };
        patch_instance(&e, "mem-legacy-4g", &p).await.unwrap();
        assert!(get_summary(&e, "mem-legacy-4g").await.unwrap().memory_auto);
        let p = InstancePatch { memory_mb: Some(Some(100)), ..Default::default() };
        assert_eq!(code(patch_instance(&e, "mem-legacy-4g", &p).await), (400, "BAD_MEMORY".into()));
    }

    #[tokio::test]
    async fn list_skips_trash_and_sorts_delete_moves_to_trash() {
        let (_d, e) = temp_engine();
        quiet(&e);
        for n in ["beta", "Alpha", "gamma"] {
            create_instance(&e, req(n)).await.unwrap();
        }
        std::fs::create_dir_all(e.paths().instance_dir("no-json")).unwrap();
        let names: Vec<String> = list_instances(&e).await.unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["Alpha", "beta", "gamma"]);
        let (deleted, trash) = delete_instance(&e, "beta").await.unwrap();
        assert_eq!(deleted, "beta");
        assert!(is_trash_dir_name(&trash));
        assert!(e.paths().instances_dir().join(&trash).join("instance.json").exists());
        let names: Vec<String> = list_instances(&e).await.unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["Alpha", "gamma"]);
        assert_eq!(delete_instance(&e, "beta").await.unwrap_err().status(), 404);
    }

    #[tokio::test]
    async fn rename_moves_dir_and_enabled_mods_toggle() {
        let (_d, e) = temp_engine();
        quiet(&e);
        create_instance(&e, req("old")).await.unwrap();
        create_instance(&e, req("taken")).await.unwrap();
        let mods = e.paths().instance_dir("old").join("mods");
        std::fs::write(mods.join("a.jar"), "a").unwrap();
        std::fs::write(mods.join("b.jar.disabled"), "b").unwrap();
        let p = InstancePatch { name: Some("taken".into()), ..Default::default() };
        assert_eq!(code(patch_instance(&e, "old", &p).await), (409, "ALREADY_EXISTS".into()));
        let p = InstancePatch {
            new_name: Some(" new ".into()),
            enabled_mods: Some(vec!["b.jar".into()]),
            ..Default::default()
        };
        let s = patch_instance(&e, "old", &p).await.unwrap();
        assert_eq!((s.name.as_str(), s.mod_count, s.enabled_mod_count), ("new", 2, 1));
        let mods = e.paths().instance_dir("new").join("mods");
        assert!(mods.join("a.jar.disabled").exists() && mods.join("b.jar").exists());
        assert!(!e.paths().instance_dir("old").exists());
        assert_eq!(get_instance(&e, "new").await.unwrap().name, "new");
    }

    #[tokio::test]
    async fn mod_set_hash_is_canonical_and_memoized() {
        let (_d, e) = temp_engine();
        quiet(&e);
        create_instance(&e, req("hash")).await.unwrap();
        let inst = get_instance(&e, "hash").await.unwrap();
        let mods = effective_mods_dir(&e, &inst);
        std::fs::write(mods.join("b.jar"), "bbb").unwrap();
        std::fs::write(mods.join("a.jar"), "aaa").unwrap();
        std::fs::write(mods.join("c.jar.disabled"), "ccc").unwrap();
        let expected = {
            let lines = [
                format!("a.jar:{}", crate::download::sha1_hex(b"aaa")),
                format!("b.jar:{}", crate::download::sha1_hex(b"bbb")),
            ];
            hex::encode(Sha256::digest(lines.join("\n").as_bytes()))
        };
        assert_eq!(compute_mod_set_hash(&e, &inst).await.unwrap(), expected);
        assert_eq!(compute_mod_set_hash(&e, &inst).await.unwrap(), expected);
        std::fs::remove_file(mods.join("b.jar")).unwrap();
        assert_ne!(compute_mod_set_hash(&e, &inst).await.unwrap(), expected);
    }

    #[tokio::test]
    async fn servers_and_options_access() {
        let (_d, e) = temp_engine();
        quiet(&e);
        create_instance(&e, req("src")).await.unwrap();
        create_instance(&e, req("dst")).await.unwrap();
        let servers = vec![ServerEntry {
            name: "Main".into(),
            ip: "main.uhce.top".into(),
            hidden: false,
            accept_textures: true,
            has_icon: false,
        }];
        assert_eq!(write_servers(&e, "src", &servers).await.unwrap(), 1);
        assert_eq!(read_servers_dat(&e, "src").await.unwrap(), servers);
        assert!(read_servers_dat(&e, "dst").await.unwrap().is_empty());

        // missing source options -> no-op
        let (copied, count, _) = import_options_from_instance(&e, "dst", "src").await.unwrap();
        assert_eq!((copied, count), (false, 0));
        std::fs::write(e.paths().instance_dir("src").join("options.txt"), "version:3700\nfov:0.5\nkey_x:key.a\n")
            .unwrap();
        let (copied, count, options) = import_options_from_instance(&e, "dst", "src").await.unwrap();
        assert_eq!((copied, count), (true, 2));
        assert_eq!(options[0], ("fov".to_string(), "0.5".to_string()));
        assert_eq!(read_options(&e, "dst").await.unwrap(), options);
        assert_eq!(code(import_options_from_instance(&e, "dst", "dst").await), (409, "SAME_INSTANCE".into()));
        assert_eq!(code(import_options_from_instance(&e, "dst", "x/y").await), (409, "INVALID_NAME".into()));
        assert_eq!(
            parse_options_txt("version:1\r\nk:v:w\nlonely\n"),
            vec![("version".into(), "1".into()), ("k".into(), "v:w".into()), ("lonely".into(), String::new())]
        );
    }

    #[tokio::test]
    async fn aot_status_snapshot_and_proof_sweep() {
        let (_d, e) = temp_engine();
        quiet(&e);
        create_instance(&e, req("aot")).await.unwrap();
        let inst = get_instance(&e, "aot").await.unwrap();
        let s = aot_status(&e, &inst).await.unwrap();
        assert_eq!(s.key.as_deref(), Some(crate::aot::cache_key("1.21.11", None, None).as_str()));
        assert!(!s.cache_exists && s.stale.is_none() && s.proof.is_none());
        assert_eq!(s.ready_to_train, Some(true));

        let base = effective_game_dir(&e, &inst);
        for i in 0..10 {
            std::fs::write(base.join(format!("aot-{i}.log")), "x\n[0.1s][warning][aot] Unable to map shared spaces\n")
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let key = s.key.unwrap();
        std::fs::create_dir_all(e.paths().aot_cache_dir(&key)).unwrap();
        std::fs::write(e.paths().aot_cache_file(&key), b"aot").unwrap();
        std::fs::write(e.paths().aot_meta(&key), r#"{"trained_at":"2026-01-01T00:00:00.000Z"}"#).unwrap();
        // NeoForge skips the classpath-drift check (which would resolve the
        // version JSON — network once game-resolve lands): stale stays null.
        let nf = InstanceMeta { loader: Loader::Neoforge, ..inst.clone() };
        let s = aot_status(&e, &nf).await.unwrap();
        assert!(s.cache_exists && s.stale.is_none());
        assert_eq!(s.ready_to_train, Some(false));
        assert_eq!(s.cache_size_bytes, 3);
        assert_eq!(s.trained_at.as_deref(), Some("2026-01-01T00:00:00.000Z"));
        let proof = s.proof.unwrap();
        assert!(proof.log_path.unwrap().ends_with("aot-9.log"));
        assert!(!proof.using_aot_linked_classes);
        assert_eq!(proof.refusal.as_deref(), Some("[0.1s][warning][aot] Unable to map shared spaces"));
        let left = read_dir_names(&base).into_iter().filter(|f| f.starts_with("aot-")).count();
        assert_eq!(left, 8);
        assert!(get_summary(&e, "aot").await.unwrap().aot_cache_exists);
    }

    #[test]
    fn stamp_matching() {
        let (d, e) = temp_engine();
        let jar = d.path().join("x.jar");
        std::fs::write(&jar, b"abc").unwrap();
        let st = std::fs::metadata(&jar).unwrap();
        let key = "k";
        std::fs::create_dir_all(e.paths().aot_cache_dir(key)).unwrap();
        assert!(crate::aot::is_cache_stale(&e, key, std::slice::from_ref(&jar)), "no stamp = stale");
        let stamp = json!({ "classpath_stamp": [{ "path": jar.to_string_lossy(), "size": 3, "mtime_ms": crate::platform::mtime_ms(&st) }] });
        std::fs::write(e.paths().aot_meta(key), stamp.to_string()).unwrap();
        assert!(!crate::aot::is_cache_stale(&e, key, std::slice::from_ref(&jar)));
        std::fs::write(&jar, b"abcd").unwrap();
        assert!(crate::aot::is_cache_stale(&e, key, std::slice::from_ref(&jar)));
    }

    // --- aot_status agrees with the launch decision ------------------------

    /// A flat version JSON + manifest entry in the resolver's cache (no network).
    fn seed_version(e: &Engine, id: &str, json: serde_json::Value) {
        let p = e.paths().versions_dir().join(format!("{id}.json"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, json.to_string()).unwrap();
        let manifest = json!({
            "fetched_at": crate::platform::now_ms(),
            "latest_release": id,
            "latest_snapshot": id,
            "versions": [{ "id": id, "type": "release", "release_time": "2026-01-01T00:00:00+00:00",
                           "url": format!("http://127.0.0.1:9/{id}.json") }]
        });
        std::fs::write(e.paths().version_manifest_cache(), manifest.to_string()).unwrap();
        e.game_resolve_state().clear_manifest_memo();
    }

    #[tokio::test]
    async fn aot_status_tracks_the_launch_classpath_and_flags() {
        let (d, e) = temp_engine();
        quiet(&e);
        e.game_resolve_state().isolate();
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on("http://127.0.0.1:9"));
        seed_version(
            &e,
            "26.3",
            json!({
                "id": "26.3",
                "mainClass": "net.minecraft.client.main.Main",
                "javaVersion": { "majorVersion": 25 },
                "libraries": [{ "name": "com.mojang:brigadier:1.3.10" }],
                "arguments": { "jvm": ["-cp", "${classpath}"] }
            }),
        );
        let java = d.path().join("jdk25").join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });
        std::fs::create_dir_all(java.parent().unwrap()).unwrap();
        std::fs::write(&java, "x").unwrap();
        e.game_resolve_state().update_hooks(move |h| h.path_java = Some(Some(java.clone())));

        std::fs::create_dir_all(e.paths().instance_dir("converge")).unwrap();
        crate::paths::write_json_atomic(
            &e.paths().instance_json("converge"),
            &json!({ "name": "converge", "version": "26.3", "loader": "fabric", "aot_auto_train": false }),
        )
        .unwrap();
        let inst = get_instance(&e, "converge").await.unwrap();
        // Fabric meta is unreachable: the identity cannot be resolved, so the
        // chip stays undecided instead of guessing.
        let key = crate::aot::cache_key("26.3", None, None);
        std::fs::create_dir_all(e.paths().aot_cache_dir(&key)).unwrap();
        std::fs::write(e.paths().aot_cache_file(&key), b"aot").unwrap();
        let s = aot_status(&e, &inst).await.unwrap();
        assert!(s.cache_exists && s.stale.is_none(), "no identity, no verdict: {:?}", s.stale);

        // Once the launch identity resolves, a legacy meta.json (no vm_flags)
        // reads stale — the same verdict the launch itself makes.
        let client = e.paths().client_jar("26.3");
        std::fs::create_dir_all(client.parent().unwrap()).unwrap();
        std::fs::write(&client, b"jar").unwrap();
        std::fs::write(
            e.paths().versions_dir().join("fabric-loaders.json"),
            json!({ "fetched_at": crate::platform::now_ms(), "latest_stable": "0.16.0",
                    "loaders": [{ "version": "0.16.0", "stable": true, "build": 1 }] })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            e.paths().versions_dir().join("fabric-loader-26.3-0.16.0.json"),
            json!({ "launcherMeta": { "mainClass": { "client": "net.fabricmc.loader.impl.launch.knot.KnotClient" } },
                    "intermediary": { "maven": "net.fabricmc:intermediary:26.3" } })
            .to_string(),
        )
        .unwrap();
        let identity = crate::launch::aot_identity(&e, &inst).await.unwrap();
        assert!(!identity.classpath.contains(&client), "client jar off the launch classpath");
        // The stamp records size -1 for a path that does not exist and a stamped
        // -1 can never prove validity, so the jars must be on disk first; only
        // then does the verdict depend on vm_flags alone.
        for p in &identity.classpath {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"jar").unwrap();
        }
        let stamp = crate::aot::classpath_stamp(&identity.classpath);
        let write_meta = |flags: Option<Vec<String>>| {
            let mut m = json!({ "trained_at": "2026-01-01T00:00:00.000Z", "classpath_stamp": stamp.clone() });
            if let Some(f) = flags {
                m["vm_flags"] = serde_json::to_value(f).unwrap();
            }
            std::fs::write(e.paths().aot_meta(&key), m.to_string()).unwrap();
        };
        // No vm_flags (every cache trained before 1.8.2) => stale.
        write_meta(None);
        let s = aot_status(&e, &inst).await.unwrap();
        assert_eq!(s.stale, Some(true), "a meta.json without vm_flags is stale");
        // The trainer's own meta.json (stamp + flags) reads fresh, so the chip
        // converges after one retrain.
        let flags = identity.vm_flags.clone();
        write_meta(Some(flags.clone()));
        let s = aot_status(&e, &inst).await.unwrap();
        assert_eq!(s.stale, Some(false), "same stamp and flags => fresh");
        // One extra flag changes what the launch passes => stale again.
        let mut changed = flags;
        changed.push("-XX:+UseCompactObjectHeaders".into());
        write_meta(Some(changed));
        let s = aot_status(&e, &inst).await.unwrap();
        assert_eq!(s.stale, Some(true), "a different vm_flags set is stale");
    }
}
