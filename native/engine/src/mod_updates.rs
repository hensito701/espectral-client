//! Smart mod updates (plan Phase 2.1-2.4): batched "check for updates",
//! version pinning, "Update all" with companion alignment and rollback,
//! orphaned-library detection after a delete, and embedded jar icons.
//!
//! New code (no JS original). The network contract is Modrinth:
//! `POST /version_files/update` answers one latest matching version per
//! installed SHA-1 (regardless of `version_type`), so a release-channel check
//! falls back to the project version list only for the few projects it
//! answered with a beta/alpha. Every download is SHA-1 verified, and the old
//! jars are copied under `<instanceDir>/backups/` before any swap, so
//! `undo_last_mod_update` works offline.
//!
//! The facade lives in `api::content`; UI code codes against those `Engine`
//! methods and the report types in `types.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::download::{DownloadAllOptions, DownloadItem};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::events::{ProgressChannel, ProgressEvent};
use crate::instances::effective_mods_dir;
use crate::types::{
    ModBatchUpdateResult, ModPinReply, ModRename, ModUndoResult, ModUpdateEntry, ModUpdatesReport, OrphanLib,
    ReleaseChannel,
};

/// Decompressed ceiling for an embedded icon (plan cap).
const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
/// Descriptor ceiling (`fabric.mod.json` / `mods.toml`).
const MAX_ICON_META_BYTES: u64 = 1 << 20;
/// Modrinth `/projects?ids=` batch size.
const PROJECTS_BATCH: usize = 100;

// ---------------------------------------------------------------------------
// Project metadata cache (titles + icons)
// ---------------------------------------------------------------------------

/// Project JSON for `ids`, served from `ContentState::project_meta` and
/// filled by batched `GET /projects?ids=[...]` calls for the missing ids.
/// Network failures leave the ids uncached and out of the map (offline
/// listing still works; icons just stay empty).
pub(crate) async fn project_meta(engine: &Engine, ids: &[String]) -> IndexMap<String, Value> {
    let mut unique: IndexSet<String> = IndexSet::new();
    for id in ids.iter().filter(|i| !i.is_empty()) {
        unique.insert(id.clone());
    }
    let mut out: IndexMap<String, Value> = IndexMap::new();
    let mut missing: Vec<String> = Vec::new();
    {
        let cache = engine.content_state().project_meta.lock().unwrap_or_else(|p| p.into_inner());
        for id in unique {
            match cache.get(&id) {
                Some(v) => {
                    out.insert(id, v.clone());
                }
                None => missing.push(id),
            }
        }
    }
    for chunk in missing.chunks(PROJECTS_BATCH) {
        let list = json!(chunk);
        let url = format!(
            "{}/projects?ids={}",
            crate::mods::api(engine),
            crate::mods::encode_uri_component(&list.to_string())
        );
        let Ok(Value::Array(projects)) = crate::mods::fetch_json_with_retry(engine, &url, 1).await else {
            continue;
        };
        let mut cache = engine.content_state().project_meta.lock().unwrap_or_else(|p| p.into_inner());
        for p in projects {
            if let Some(id) = crate::mods::s(&p, "id") {
                cache.insert(id.clone(), p.clone());
                out.insert(id, p);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Version pinning
// ---------------------------------------------------------------------------

/// Strip `.disabled` / `.jar` from a filename or pin token.
fn normalize_pin_name(name: &str) -> String {
    let n = name.trim();
    n.strip_suffix(".jar.disabled").or_else(|| n.strip_suffix(".jar")).unwrap_or(n).to_string()
}

/// True when `pins` (Modrinth project ids or base filenames) covers this mod.
pub(crate) fn is_pinned(pins: &[String], project_id: Option<&str>, filename: &str) -> bool {
    let name = normalize_pin_name(filename);
    pins.iter().any(|p| p == &name || project_id.is_some_and(|id| p == id))
}

/// Modrinth project ids are 8 base62 characters.
fn is_project_id(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Pin or unpin a mod against "Update all". The canonical token is the
/// Modrinth project id when it can be resolved (offline: known mod ids, the
/// local pin table or the sha1 cache), else the base filename; both forms
/// match in [`is_pinned`], so a pin survives a version switch either way.
pub async fn set_mod_pinned(
    engine: &Engine,
    instance_name: &str,
    filename_or_pid: &str,
    pinned: bool,
) -> EngineResult<ModPinReply> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let name = normalize_pin_name(filename_or_pid);
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == ".." {
        return Err(crate::mods::code_err(400, "invalid_filename", format!("invalid mod pin: {filename_or_pid}")));
    }
    let mods_dir = effective_mods_dir(engine, &instance);
    let jar_name = format!("{name}.jar");
    let on_disk = mods_dir.join(&jar_name).is_file() || mods_dir.join(format!("{jar_name}.disabled")).is_file();
    let mut project_id = None;
    if on_disk {
        if let Ok(resolved) = crate::mods::resolve_installed_mod(engine, &instance, &mods_dir, &jar_name).await {
            project_id = resolved.project_id;
        }
    }
    if project_id.is_none() && is_project_id(&name) {
        project_id = Some(name.clone());
    }
    let canonical = project_id.clone().unwrap_or_else(|| name.clone());
    let mut pins = instance.pinned_mods.clone();
    pins.retain(|p| p != &canonical && p != &name && project_id.as_deref() != Some(p.as_str()));
    if pinned {
        pins.push(canonical);
    }
    crate::instances::write_pinned_mods(engine, instance_name, &pins)?;
    Ok(ModPinReply { pinned, pinned_mods: pins })
}

// ---------------------------------------------------------------------------
// Batched update check
// ---------------------------------------------------------------------------

struct Candidate {
    base: String,
    enabled: bool,
    hash: String,
    version: String,
    display: String,
    from_version_id: Option<String>,
}

/// `POST /version_files/update`: one latest matching version per SHA-1,
/// filtered by loader + game version. Returns the hash -> version map.
async fn post_version_files_update(
    engine: &Engine,
    hashes: &[String],
    loader: &str,
    game_version: &str,
) -> EngineResult<Map<String, Value>> {
    let url = format!("{}/version_files/update", crate::mods::api(engine));
    let body = json!({
        "hashes": hashes,
        "algorithm": "sha1",
        "loaders": [loader],
        "game_versions": [game_version],
    });
    let res = engine
        .http()
        .client()
        .post(&url)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| EngineError::other(format!("Modrinth update check failed for {url}: {e}")))?;
    if !res.status().is_success() {
        return Err(EngineError::other(format!("Modrinth update check answered {}", res.status().as_u16())));
    }
    match res.json::<Value>().await {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Ok(Map::new()),
        Err(e) => Err(EngineError::other(format!("Modrinth update check returned invalid JSON: {e}"))),
    }
}

/// Newest listed release of `project_id` for this game version + loader. The
/// batch endpoint ignores `version_type`, so this is the fallback used only
/// when `channel == Release` and the batch answered with a beta/alpha build.
async fn newest_release(engine: &Engine, project_id: &str, game_version: &str, loader: &str) -> Option<Value> {
    let list = crate::mods::list_project_versions(engine, project_id, game_version, loader).await.ok()?;
    list.into_iter()
        .filter(|v| crate::mods::s(v, "version_type").as_deref() == Some("release"))
        .max_by_key(|v| crate::mods::s(v, "date_published").unwrap_or_default())
}

/// One batched update check for every non-branding jar in the instance.
/// Pinned mods are still checked and reported (`pinned: true`) so the UI can
/// show "Update available (pinned)"; "Update all" skips them.
pub async fn check_mod_updates(
    engine: &Engine,
    instance_name: &str,
    channel: ReleaseChannel,
) -> EngineResult<ModUpdatesReport> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let mut candidates: Vec<Candidate> = Vec::new();
    for file in crate::mods::read_names(&mods_dir) {
        if !crate::mods::is_mod_file(&file) {
            continue;
        }
        let enabled = !file.ends_with(".jar.disabled");
        let base = if enabled { file.clone() } else { file[..file.len() - ".disabled".len()].to_string() };
        if crate::mods::is_brand_jar(&base) {
            continue;
        }
        let abs = mods_dir.join(&file);
        let Ok(hash) = crate::download::sha1_file(&abs).await else {
            continue;
        };
        let scan = crate::loader_meta::scan_jar(&abs, &base, enabled);
        let version = scan.as_ref().and_then(|j| j.primary_version()).unwrap_or_default().to_string();
        let display = scan.as_ref().and_then(|j| j.primary_name()).unwrap_or_default().to_string();
        // A normal check stays at one HTTP request: the current version id is
        // read from the sha1 cache when a previous listing filled it.
        let from_version_id = {
            let cache = engine.content_state().sha1_versions.lock().unwrap_or_else(|p| p.into_inner());
            cache.get(&hash).and_then(|o| o.as_ref()).and_then(|m| crate::mods::s(m, "id"))
        };
        candidates.push(Candidate { base, enabled, hash, version, display, from_version_id });
    }
    let checked = candidates.len() as u32;
    if candidates.is_empty() {
        return Ok(ModUpdatesReport { checked, updates: vec![] });
    }
    let hashes: Vec<String> = candidates.iter().map(|c| c.hash.clone()).collect();
    let loader = instance.loader.content_loader();
    let by_hash = post_version_files_update(engine, &hashes, loader, &instance.version).await?;

    let mut updates: Vec<ModUpdateEntry> = Vec::new();
    let mut title_ids: Vec<String> = Vec::new();
    for cand in &candidates {
        let Some(found) = by_hash.get(&cand.hash) else {
            continue;
        };
        let Some(pid) = crate::mods::s(found, "project_id") else {
            continue;
        };
        let mut target = found.clone();
        let mut version_type = crate::mods::s(&target, "version_type").unwrap_or_else(|| "release".into());
        if channel == ReleaseChannel::Release && version_type != "release" {
            if let Some(best) = newest_release(engine, &pid, &instance.version, loader).await {
                target = best;
                version_type = "release".into();
            }
        }
        let Some(file) = crate::mods::primary_file(&target) else {
            continue;
        };
        if crate::mods::file_sha1(file).as_deref() == Some(cand.hash.as_str()) {
            continue; // already the newest matching build
        }
        let Ok(to_filename) = crate::mods::safe_download_name(file) else {
            continue;
        };
        let Some(to_version_id) = crate::mods::s(&target, "id") else {
            continue;
        };
        let pinned = is_pinned(&instance.pinned_mods, Some(pid.as_str()), &cand.base);
        if !title_ids.iter().any(|id| id == &pid) {
            title_ids.push(pid.clone());
        }
        updates.push(ModUpdateEntry {
            filename: cand.base.clone(),
            project_id: pid,
            title: if cand.display.is_empty() { cand.base.clone() } else { cand.display.clone() },
            from_version: cand.version.clone(),
            from_version_id: cand.from_version_id.clone(),
            to_version_id,
            to_version_number: crate::mods::s(&target, "version_number").unwrap_or_default(),
            to_filename,
            version_type,
            pinned,
            enabled: cand.enabled,
        });
    }
    if !title_ids.is_empty() {
        let projects = project_meta(engine, &title_ids).await;
        for update in &mut updates {
            if let Some(title) = projects.get(&update.project_id).and_then(|p| crate::mods::s(p, "title")) {
                update.title = title;
            }
        }
    }
    updates.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(ModUpdatesReport { checked, updates })
}

// ---------------------------------------------------------------------------
// Apply / undo
// ---------------------------------------------------------------------------

/// One rollback manifest row (`backups/last-mod-update.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RollbackEntry {
    project_id: String,
    #[serde(default)]
    prev_version_id: String,
    prev_filename: String,
    new_filename: String,
}

/// True when another batch member's installed jar depends on `primary` (or
/// the Sodium/Iris pair), so switching `primary` may force a companion move.
fn has_batch_dependents(
    scanned: &[crate::fabric_compat::ScannedJar],
    entries: &[ModUpdateEntry],
    primary: &ModUpdateEntry,
) -> bool {
    if primary.project_id == crate::mods::SODIUM_PROJECT_ID
        && entries.iter().any(|e| e.project_id == crate::mods::IRIS_PROJECT_ID)
    {
        return true;
    }
    let Some(jar) = scanned.iter().find(|j| j.filename == primary.filename) else {
        return false;
    };
    let mut provided: HashSet<&str> = HashSet::new();
    provided.insert(jar.root.id.as_str());
    for p in &jar.root.provides {
        provided.insert(p.as_str());
    }
    for n in &jar.nested {
        provided.insert(n.id.as_str());
        for p in &n.provides {
            provided.insert(p.as_str());
        }
    }
    entries.iter().any(|e| {
        e.project_id != primary.project_id
            && scanned
                .iter()
                .find(|j| j.filename == e.filename)
                .is_some_and(|j| j.enabled && j.root.depends.keys().any(|d| provided.contains(d.as_str())))
    })
}

/// Point a batch entry at `version_id`; false when it already is that target
/// or the version cannot be fetched.
async fn apply_target(engine: &Engine, entry: &mut ModUpdateEntry, version_id: &str) -> bool {
    if entry.to_version_id == version_id {
        return false;
    }
    let Ok(meta) = crate::mods::version_meta(engine, version_id).await else {
        return false;
    };
    let Some(file) = crate::mods::primary_file(&meta) else {
        return false;
    };
    let Ok(filename) = crate::mods::safe_download_name(file) else {
        return false;
    };
    entry.to_version_id = version_id.to_string();
    entry.to_version_number = crate::mods::s(&meta, "version_number").unwrap_or_default();
    entry.to_filename = filename;
    entry.version_type = crate::mods::s(&meta, "version_type").unwrap_or_else(|| "release".into());
    true
}

/// Align versions inside one batch. Direction 2 reuses the existing
/// version-switch companion logic (Sodium -> Iris): switching a mod that
/// another batch member depends on may force that member onto a build pinning
/// the new target. Direction 1 then applies the hard rule: an exact
/// dependency pin in a target build wins over any version the batch picked
/// for that project (Iris pins Sodium, so the pair can never be mismatched).
async fn align_batch_companions(
    engine: &Engine,
    instance_name: &str,
    mods_dir: &Path,
    mut entries: Vec<ModUpdateEntry>,
) -> (Vec<ModUpdateEntry>, Vec<String>) {
    let mut warnings = Vec::new();
    if entries.len() < 2 {
        return (entries, warnings);
    }
    let scanned = crate::fabric_compat::scan_mods_dir(mods_dir);
    for _ in 0..2 {
        let mut changed = false;
        let snapshot = entries.clone();
        for primary in &snapshot {
            if !has_batch_dependents(&scanned, &snapshot, primary) {
                continue;
            }
            let Ok(plan) =
                crate::mods::plan_mod_version_switch(engine, instance_name, &primary.filename, &primary.to_version_id)
                    .await
            else {
                continue;
            };
            warnings.extend(plan.warnings);
            for companion in plan.companions {
                let Some(idx) = entries.iter().position(|e| e.project_id == companion.project_id) else {
                    continue;
                };
                if apply_target(engine, &mut entries[idx], &companion.to_version_id).await {
                    changed = true;
                }
            }
        }
        let snapshot = entries.clone();
        for primary in &snapshot {
            let Ok(meta) = crate::mods::version_meta(engine, &primary.to_version_id).await else {
                continue;
            };
            for dep in meta.get("dependencies").and_then(Value::as_array).cloned().unwrap_or_default() {
                if dep.get("dependency_type").and_then(Value::as_str) != Some("required") {
                    continue;
                }
                let (Some(pid), Some(vid)) = (crate::mods::s(&dep, "project_id"), crate::mods::s(&dep, "version_id"))
                else {
                    continue;
                };
                let Some(idx) = entries.iter().position(|e| e.project_id == pid && e.to_version_id != vid) else {
                    continue;
                };
                if apply_target(engine, &mut entries[idx], &vid).await {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    warnings.sort();
    warnings.dedup();
    (entries, warnings)
}

fn old_jar_path(mods_dir: &Path, filename: &str) -> Option<PathBuf> {
    let enabled = mods_dir.join(filename);
    if enabled.is_file() {
        return Some(enabled);
    }
    let disabled = mods_dir.join(format!("{filename}.disabled"));
    if disabled.is_file() {
        Some(disabled)
    } else {
        None
    }
}

/// Base filename of a rollback-manifest row (written as a base or with the
/// `.jar` / `.disabled` suffix). Re-validated because the manifest is local
/// data, never trusted as a path.
fn safe_manifest_base(name: &str) -> EngineResult<String> {
    let has_suffix = name.ends_with(".jar") || name.ends_with(".jar.disabled");
    let with_jar = if has_suffix { name.to_string() } else { format!("{name}.jar") };
    crate::mods::assert_safe_filename(&with_jar)
}

/// Move a verified staged jar into place. A cross-filesystem game dir (custom
/// `game_dir`) falls back to copy through a sibling `.tmp` then rename, so a
/// partially copied jar never lands in `mods/`.
fn move_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    if std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    let mut part = dest.as_os_str().to_owned();
    part.push(".tmp");
    let part = PathBuf::from(part);
    std::fs::copy(src, &part)?;
    std::fs::rename(&part, dest)?;
    let _ = std::fs::remove_file(src);
    Ok(())
}

/// Copy through a sibling `.tmp` then rename (rollback manifest and jar
/// restore).
fn copy_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    let mut part = dest.as_os_str().to_owned();
    part.push(".tmp");
    let part = PathBuf::from(part);
    std::fs::copy(src, &part)?;
    std::fs::rename(&part, dest)
}

/// "Update all" (or the given `filenames`). Pinned mods are skipped unless
/// explicitly named. Companion-aligned targets are downloaded and SHA-1
/// verified before any jar is swapped; `<instanceDir>/backups/` keeps the
/// previous jars plus `last-mod-update.json` for [`undo_last_mod_update`].
pub async fn apply_mod_updates(
    engine: &Engine,
    instance_name: &str,
    filenames: Option<Vec<String>>,
) -> EngineResult<ModBatchUpdateResult> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    let report = check_mod_updates(engine, instance_name, ReleaseChannel::Release).await?;
    let wanted: Option<HashSet<String>> = filenames.map(|list| list.iter().map(|f| normalize_pin_name(f)).collect());
    let mut entries: Vec<ModUpdateEntry> = Vec::new();
    let mut skipped_pinned: Vec<String> = Vec::new();
    for update in report.updates {
        match &wanted {
            Some(names) => {
                if names.contains(&normalize_pin_name(&update.filename)) {
                    entries.push(update);
                }
            }
            None => {
                if update.pinned {
                    skipped_pinned.push(update.filename.clone());
                } else {
                    entries.push(update);
                }
            }
        }
    }
    if entries.is_empty() {
        return Ok(ModBatchUpdateResult { updated: vec![], changed: vec![], skipped_pinned, warnings: vec![] });
    }
    let (mut entries, warnings) = align_batch_companions(engine, instance_name, &mods_dir, entries).await;

    // Rollback metadata: resolve the installed version ids in one batched
    // lookup (the check itself stays at one HTTP request).
    let mut hash_by_index: Vec<Option<String>> = vec![None; entries.len()];
    let mut old_hashes: Vec<String> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(path) = old_jar_path(&mods_dir, &entry.filename) else {
            continue;
        };
        if let Ok(hash) = crate::download::sha1_file(&path).await {
            hash_by_index[i] = Some(hash.clone());
            old_hashes.push(hash);
        }
    }
    if !old_hashes.is_empty() {
        let by_hash = crate::mods::cached_version_files(engine, &old_hashes).await;
        for (i, entry) in entries.iter_mut().enumerate() {
            if entry.from_version_id.is_none() {
                entry.from_version_id = hash_by_index[i]
                    .as_deref()
                    .and_then(|h| by_hash.get(h))
                    .and_then(|o| o.as_ref())
                    .and_then(|m| crate::mods::s(m, "id"));
            }
        }
    }

    struct Job {
        entry: ModUpdateEntry,
        was_enabled: bool,
        staged: PathBuf,
    }
    let staging_dir = engine.paths().data_dir().join("cache").join("mod-updates");
    std::fs::create_dir_all(&staging_dir)?;
    let total = entries.len() as u64;
    let mut jobs: Vec<Job> = Vec::new();
    let mut downloads = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        let meta = crate::mods::version_meta(engine, &entry.to_version_id).await?;
        let file = crate::mods::primary_file(&meta)
            .ok_or_else(|| EngineError::other(format!("version {} has no downloadable file", entry.to_version_id)))?;
        let sha1 = crate::mods::file_sha1(file).ok_or_else(|| {
            EngineError::other(format!(
                "{} {} publishes no SHA-1; refusing an unverified update",
                entry.title, entry.to_version_number
            ))
        })?;
        let fname = crate::mods::safe_download_name(file)?;
        let staged = staging_dir.join(format!("{sha1}-{i}.jar"));
        let was_enabled = mods_dir.join(&entry.filename).is_file();
        downloads.push(
            DownloadItem::new(crate::mods::s(file, "url").unwrap_or_default(), staged.clone())
                .sha1(Some(sha1))
                .size(file.get("size").and_then(Value::as_u64))
                .event(
                    ProgressChannel::Mod,
                    ProgressEvent::for_instance(instance_name)
                        .phase("update")
                        .filename(&fname)
                        .index(i as u64)
                        .total(total),
                ),
        );
        jobs.push(Job { entry, was_enabled, staged });
    }
    if !downloads.is_empty() {
        engine.http().download_all(downloads, DownloadAllOptions::default()).await?;
    }

    // Back up every previous jar before touching the mods dir. The manifest is
    // written first so a crash mid-swap is still recoverable.
    let backups_dir = engine.paths().instance_dir(instance_name).join("backups");
    let jar_dir = backups_dir.join("last-mod-update");
    let manifest_path = backups_dir.join("last-mod-update.json");
    if jar_dir.exists() {
        std::fs::remove_dir_all(&jar_dir)?;
    }
    std::fs::create_dir_all(&jar_dir)?;
    let mut manifest: Vec<RollbackEntry> = Vec::with_capacity(jobs.len());
    for job in &jobs {
        let Some(old_path) = old_jar_path(&mods_dir, &job.entry.filename) else {
            return Err(EngineError::other(format!("{} vanished before the update", job.entry.filename)));
        };
        let backup_base = crate::mods::assert_safe_filename(&job.entry.filename)?;
        let backup_name = if job.was_enabled { backup_base } else { format!("{backup_base}.disabled") };
        copy_atomic(&old_path, &jar_dir.join(backup_name))?;
        manifest.push(RollbackEntry {
            project_id: job.entry.project_id.clone(),
            prev_version_id: job.entry.from_version_id.clone().unwrap_or_default(),
            prev_filename: job.entry.filename.clone(),
            new_filename: job.entry.to_filename.clone(),
        });
    }
    crate::paths::write_json_atomic(&manifest_path, &manifest)?;

    // Remove every old twin first: a new filename may equal another job's old
    // filename in theory, and removing after moving would delete the new jar.
    for job in &jobs {
        let _ = std::fs::remove_file(mods_dir.join(&job.entry.filename));
        let _ = std::fs::remove_file(mods_dir.join(format!("{}.disabled", job.entry.filename)));
    }
    let mut changed: Vec<ModRename> = Vec::new();
    let mut updated: Vec<ModUpdateEntry> = Vec::new();
    for job in jobs {
        let dest = if job.was_enabled {
            mods_dir.join(&job.entry.to_filename)
        } else {
            mods_dir.join(format!("{}.disabled", job.entry.to_filename))
        };
        let twin = if job.was_enabled {
            mods_dir.join(format!("{}.disabled", job.entry.to_filename))
        } else {
            mods_dir.join(&job.entry.to_filename)
        };
        let _ = std::fs::remove_file(&twin);
        move_file(&job.staged, &dest)?;
        changed.push(ModRename { from: job.entry.filename.clone(), to: job.entry.to_filename.clone() });
        updated.push(job.entry);
    }
    Ok(ModBatchUpdateResult { updated, changed, skipped_pinned, warnings })
}

/// Restore the jars saved by the last batch update (offline: the backups are
/// local copies). The manifest is consumed so a second undo is a 404.
pub async fn undo_last_mod_update(engine: &Engine, instance_name: &str) -> EngineResult<ModUndoResult> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let backups_dir = engine.paths().instance_dir(instance_name).join("backups");
    let jar_dir = backups_dir.join("last-mod-update");
    let manifest_path = backups_dir.join("last-mod-update.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|_| crate::mods::code_err(404, "no_mod_update", "there is no mod update to undo"))?;
    let entries: Vec<RollbackEntry> = serde_json::from_str(&text)
        .map_err(|_| crate::mods::code_err(404, "no_mod_update", "last-mod-update.json is unreadable"))?;
    if entries.is_empty() {
        return Err(crate::mods::code_err(404, "no_mod_update", "there is no mod update to undo"));
    }
    // Validate every backup before changing anything. Manifest names are
    // re-checked as safe basenames: the file is local data, not a path input.
    let mut jobs: Vec<(String, String, PathBuf, bool)> = Vec::with_capacity(entries.len());
    for entry in entries {
        tracing::debug!("[mods] undo update of {} ({})", entry.project_id, entry.prev_version_id);
        let prev_base = safe_manifest_base(&entry.prev_filename)?;
        let new_base = safe_manifest_base(&entry.new_filename)?;
        // The backup's own suffix records the previous enabled state, so undo
        // is exact even if a batch failed halfway through its swaps.
        let enabled_backup = jar_dir.join(&prev_base);
        let disabled_backup = jar_dir.join(format!("{prev_base}.disabled"));
        let (backup, prev_enabled) = if enabled_backup.is_file() {
            (enabled_backup, true)
        } else if disabled_backup.is_file() {
            (disabled_backup, false)
        } else {
            return Err(EngineError::other(format!("rollback backup missing for {prev_base}")));
        };
        jobs.push((prev_base, new_base, backup, prev_enabled));
    }
    let mut restored = Vec::new();
    for (prev_base, new_base, backup, prev_enabled) in jobs {
        let _ = std::fs::remove_file(mods_dir.join(format!("{new_base}.jar")));
        let _ = std::fs::remove_file(mods_dir.join(format!("{new_base}.jar.disabled")));
        let enabled_path = mods_dir.join(format!("{prev_base}.jar"));
        let disabled_path = mods_dir.join(format!("{prev_base}.jar.disabled"));
        if prev_enabled {
            copy_atomic(&backup, &enabled_path)?;
            let _ = std::fs::remove_file(&disabled_path);
        } else {
            copy_atomic(&backup, &disabled_path)?;
            let _ = std::fs::remove_file(&enabled_path);
        }
        restored.push(ModRename { from: format!("{new_base}.jar"), to: format!("{prev_base}.jar") });
    }
    let _ = std::fs::remove_file(&manifest_path);
    let _ = std::fs::remove_dir_all(&jar_dir);
    Ok(ModUndoResult { restored })
}

// ---------------------------------------------------------------------------
// Disable / delete guards
// ---------------------------------------------------------------------------

/// Enabled mods that require this jar (`(filename, display name)`).
pub async fn dependents_of_mod(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
) -> EngineResult<Vec<(String, String)>> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let base = crate::mods::assert_safe_filename(filename)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let jars = crate::fabric_compat::scan_mods_dir(&mods_dir);
    Ok(crate::fabric_compat::dependents_of_jar(&jars, &format!("{base}.jar")))
}

/// True when `jar` (or a jar-in-jar) declares `id` as its mod id or a
/// provided id.
fn jar_provides_id(jar: &crate::fabric_compat::ScannedJar, id: &str) -> bool {
    jar.root.id == id
        || jar.root.provides.iter().any(|p| p == id)
        || jar.nested.iter().any(|n| n.id == id || n.provides.iter().any(|p| p == id))
}

/// Offline library heuristic: well-known API/config libraries, Fabric API
/// modules and ids that name an API/library. Modrinth's `is_library` tag is
/// network-only, so the delete prompt stays conservative.
fn is_library_id(id: &str) -> bool {
    if matches!(id, "minecraft" | "fabricloader" | "fabric-loader" | "java" | "forge" | "neoforge") {
        return false;
    }
    if crate::fabric_compat::is_fabric_api_module(id) {
        return true;
    }
    matches!(
        id,
        "fabric-api"
            | "fabric"
            | "cloth-config"
            | "cloth-config2"
            | "architectury"
            | "yet_another_config_lib_v3"
            | "placeholder-api"
            | "owo"
            | "owo-lib"
            | "cardinal-components"
            | "cardinal-components-base"
            | "collective"
            | "puzzleslib"
            | "forgeconfigapiport"
            | "fabric-language-kotlin"
            | "fabric-language-scala"
            | "quilted-fabric-api"
            | "midnightlib"
            | "yacl"
            | "badpackets"
            | "jamlib"
            | "kuma-api"
    ) || id.ends_with("-api")
        || id.ends_with("_api")
        || id.ends_with("-lib")
        || id.ends_with("_lib")
        || id.ends_with("-config")
        || id.ends_with("_config")
}

/// Libraries the deleted mod was the last dependent of: its `depends` that
/// are libraries (see [`is_library_id`]) and have zero remaining enabled
/// dependents once the mod is gone. The delete modal offers them as
/// "No longer needed by any mod: remove too?".
pub async fn orphaned_libraries_after_delete(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
) -> EngineResult<Vec<OrphanLib>> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let base = crate::mods::assert_safe_filename(filename)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let jars = crate::fabric_compat::scan_mods_dir(&mods_dir);
    let target_filename = format!("{base}.jar");
    let Some(target) = jars.iter().find(|j| j.filename == target_filename) else {
        return Ok(vec![]);
    };
    let mut out: Vec<OrphanLib> = Vec::new();
    for dep in target.root.depends.keys() {
        if !is_library_id(dep) {
            continue;
        }
        let Some(lib) = jars.iter().find(|j| j.filename != target_filename && jar_provides_id(j, dep)) else {
            continue;
        };
        if !lib.enabled {
            continue;
        }
        let remaining: Vec<(String, String)> = crate::fabric_compat::dependents_of_jar(&jars, &lib.filename)
            .into_iter()
            .filter(|(f, _)| f != &target_filename)
            .collect();
        if !remaining.is_empty() {
            continue;
        }
        let project_id = crate::fabric_compat::known_modrinth_for_id(dep)
            .or_else(|| crate::fabric_compat::known_modrinth_for_id(&lib.root.id))
            .map(|(pid, _)| pid.to_string());
        out.push(OrphanLib {
            project_id,
            mod_id: lib.root.id.clone(),
            filename: lib.filename.clone(),
            display_name: lib.root.name.clone(),
        });
    }
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    out.dedup_by(|a, b| a.filename == b.filename);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Embedded icons
// ---------------------------------------------------------------------------

fn read_entry_capped(path: &Path, entry: &str, max: u64) -> EngineResult<Option<Vec<u8>>> {
    Ok(crate::ziputil::read_zip_entry_capped(path, entry, max)?)
}

/// Zip entry paths must stay inside the jar.
fn safe_icon_entry(name: &str) -> Option<String> {
    let n = name.trim().trim_start_matches('/');
    if n.is_empty() || n.contains("..") || n.contains('\\') {
        return None;
    }
    Some(n.to_string())
}

/// The `icon` declared by `fabric.mod.json`: a string path, or a size map
/// (`{"64": "...", "128": "..."}`) where the largest entry <= 128 px wins.
fn fabric_icon_entry(path: &Path) -> EngineResult<Option<String>> {
    let Some(raw) = read_entry_capped(path, "fabric.mod.json", MAX_ICON_META_BYTES)? else {
        return Ok(None);
    };
    let Ok(meta) = serde_json::from_slice::<Value>(&raw) else {
        return Ok(None);
    };
    let Some(icon) = meta.get("icon") else {
        return Ok(None);
    };
    let name = match icon {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => map
            .iter()
            .filter_map(|(k, v)| {
                let size: u32 = k.parse().ok()?;
                if size > 128 {
                    return None;
                }
                Some((size, v.as_str()?.to_string()))
            })
            .max_by_key(|(size, _)| *size)
            .map(|(_, name)| name),
        _ => None,
    };
    Ok(name.as_deref().and_then(safe_icon_entry))
}

/// `logoFile` of `META-INF/neoforge.mods.toml` / `META-INF/mods.toml` (top
/// level or first `[[mods]]` entry).
fn forge_logo_entry(path: &Path) -> EngineResult<Option<String>> {
    for entry in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        let Some(raw) = read_entry_capped(path, entry, MAX_ICON_META_BYTES)? else {
            continue;
        };
        let Ok(text) = String::from_utf8(raw) else {
            continue;
        };
        let Ok(table) = toml::from_str::<toml::Table>(&text) else {
            continue;
        };
        let logo = table.get("logoFile").and_then(toml::Value::as_str).map(str::to_string).or_else(|| {
            table
                .get("mods")
                .and_then(toml::Value::as_array)
                .and_then(|mods| mods.iter().find_map(|m| m.get("logoFile").and_then(toml::Value::as_str)))
                .map(str::to_string)
        });
        if let Some(logo) = logo {
            if let Some(safe) = safe_icon_entry(&logo) {
                return Ok(Some(safe));
            }
        }
    }
    Ok(None)
}

/// Icon bytes declared by the jar itself (`fabric.mod.json` `icon`, else
/// `mods.toml` `logoFile`), capped at 2 MiB. Zero network, so local-only
/// mods without a Modrinth project still get a tile.
pub async fn mod_embedded_icon(engine: &Engine, instance_name: &str, filename: &str) -> EngineResult<Option<Vec<u8>>> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let base = crate::mods::assert_safe_filename(filename)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let enabled = mods_dir.join(format!("{base}.jar"));
    let disabled = mods_dir.join(format!("{base}.jar.disabled"));
    let path = if enabled.is_file() {
        enabled
    } else if disabled.is_file() {
        disabled
    } else {
        return Err(crate::mods::code_err(404, "mod_not_found", format!("mod not found: {filename}")));
    };
    if let Some(icon) = fabric_icon_entry(&path)? {
        if let Some(bytes) = read_entry_capped(&path, &icon, MAX_ICON_BYTES)? {
            return Ok(Some(bytes));
        }
    }
    if let Some(logo) = forge_logo_entry(&path)? {
        if let Some(bytes) = read_entry_capped(&path, &logo, MAX_ICON_BYTES)? {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod updates_tests {
    use super::*;
    use crate::test_support::{temp_engine, MockRequest, MockResponse, MockServer};
    use std::sync::{Arc, Mutex};

    fn configure(e: &Engine, server: &MockServer) {
        let api = server.url("/v2");
        e.update_content_settings(|s| {
            s.modrinth_api = api;
            s.modrinth_retry_backoff = std::time::Duration::from_millis(1);
            s.background_on_create = false;
        });
    }

    fn write_instance(e: &Engine, name: &str, pinned: &[&str]) -> PathBuf {
        let dir = e.paths().instance_dir(name);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(
            dir.join("instance.json"),
            json!({
                "name": name,
                "version": "26.3",
                "loader": "fabric",
                "pinned_mods": pinned,
            })
            .to_string(),
        )
        .unwrap();
        dir.join("mods")
    }

    fn write_jar(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn fabric_jar(path: &Path, id: &str, name: &str, version: &str, depends: Value) {
        let meta = json!({
            "schemaVersion": 1,
            "id": id,
            "name": name,
            "version": version,
            "depends": depends,
        });
        write_jar(path, &[("fabric.mod.json", meta.to_string().as_bytes())]);
    }

    fn version_json(project: &str, id: &str, number: &str, version_type: &str, deps: Value, file: Value) -> Value {
        json!({
            "id": id,
            "project_id": project,
            "version_number": number,
            "version_type": version_type,
            "status": "listed",
            "date_published": "2026-11-20T00:00:00Z",
            "dependencies": deps,
            "files": [file],
        })
    }

    fn file_json(name: &str, sha1: &str, url: &str, size: u64) -> Value {
        json!({
            "primary": true,
            "filename": name,
            "url": url,
            "size": size,
            "hashes": { "sha1": sha1, "sha512": "00" },
        })
    }

    #[tokio::test]
    async fn check_updates_batched_respects_pins_and_release_channel() {
        let (_d, e) = temp_engine();
        let mods_dir = write_instance(&e, "upd", &["PINNED01"]);
        fabric_jar(&mods_dir.join("sodium-fabric-0.9.2.jar"), "sodium", "Sodium", "0.9.2", json!({}));
        fabric_jar(&mods_dir.join("pinned-1.0.jar"), "pinnedmod", "Pinned Mod", "1.0", json!({}));
        fabric_jar(&mods_dir.join("local.jar"), "localmod", "Local Mod", "0.1", json!({}));
        let sodium_hash = crate::download::sha1_file(&mods_dir.join("sodium-fabric-0.9.2.jar")).await.unwrap();
        let pinned_hash = crate::download::sha1_file(&mods_dir.join("pinned-1.0.jar")).await.unwrap();

        let sodium_beta = version_json(
            crate::mods::SODIUM_PROJECT_ID,
            "SODBETA",
            "0.9.3-beta.1",
            "beta",
            json!([]),
            file_json("sodium-fabric-0.9.3-beta.1.jar", &"b1".repeat(20), "http://x/sb", 1),
        );
        let sodium_release = version_json(
            crate::mods::SODIUM_PROJECT_ID,
            "SODREL",
            "0.9.3",
            "release",
            json!([]),
            file_json("sodium-fabric-0.9.3.jar", &"r1".repeat(20), "http://x/sr", 1),
        );
        let pinned_beta = version_json(
            "PINNED01",
            "PINBETA",
            "1.1-beta.1",
            "beta",
            json!([]),
            file_json("pinned-1.1-beta.1.jar", &"p1".repeat(20), "http://x/pb", 1),
        );
        let pinned_release = version_json(
            "PINNED01",
            "PINREL",
            "1.1",
            "release",
            json!([]),
            file_json("pinned-1.1.jar", &"p2".repeat(20), "http://x/pr", 1),
        );
        let by_hash = json!({
            sodium_hash.clone(): sodium_beta.clone(),
            pinned_hash.clone(): pinned_beta.clone(),
        });
        let sodium_pid = crate::mods::SODIUM_PROJECT_ID;
        let versions = json!({
            sodium_pid: [sodium_beta.clone(), sodium_release.clone()],
            "PINNED01": [pinned_beta.clone(), pinned_release.clone()],
        });
        let by_id = json!({
            "SODBETA": sodium_beta,
            "SODREL": sodium_release,
            "PINBETA": pinned_beta,
            "PINREL": pinned_release,
        });
        let server = MockServer::start(move |req: &MockRequest| {
            if req.path == "/v2/version_files/update" && req.method == "POST" {
                return MockResponse::json(&by_hash);
            }
            if let Some(rest) = req.path.strip_prefix("/v2/project/") {
                let id = rest.split('/').next().unwrap();
                return MockResponse::json(&versions.get(id).cloned().unwrap_or(json!([])));
            }
            if let Some(id) = req.path.strip_prefix("/v2/version/") {
                if let Some(v) = by_id.get(id) {
                    return MockResponse::json(v);
                }
            }
            if let Some(q) = req.path.strip_prefix("/v2/projects?ids=") {
                let decoded = url::form_urlencoded::parse(format!("x={q}").as_bytes()).next().unwrap().1.into_owned();
                let ids: Vec<String> = serde_json::from_str(&decoded).unwrap();
                return MockResponse::json(&Value::Array(
                    ids.iter()
                        .map(|id| {
                            json!({
                                "id": id,
                                "title": format!("Title {id}"),
                                "icon_url": format!("http://x/{id}.png"),
                            })
                        })
                        .collect(),
                ));
            }
            MockResponse::status(404)
        })
        .await;
        configure(&e, &server);

        let report = e.check_mod_updates("upd", ReleaseChannel::Release).await.unwrap();
        assert_eq!(report.checked, 3);
        assert_eq!(report.updates.len(), 2, "{:?}", report.updates);
        let sodium = report.updates.iter().find(|u| u.filename == "sodium-fabric-0.9.2.jar").unwrap();
        assert_eq!(sodium.to_version_id, "SODREL");
        assert_eq!(sodium.version_type, "release");
        assert_eq!(sodium.from_version, "0.9.2");
        assert_eq!(sodium.title, format!("Title {}", crate::mods::SODIUM_PROJECT_ID));
        assert!(!sodium.pinned);
        let pinned = report.updates.iter().find(|u| u.filename == "pinned-1.0.jar").unwrap();
        assert!(pinned.pinned);
        assert_eq!(pinned.to_version_id, "PINREL");
        // One batched POST; the release fallback hit only the two projects it
        // answered with a beta (no per-mod version lookups).
        let requests = server.requests();
        assert_eq!(requests.iter().filter(|r| r.path == "/v2/version_files/update").count(), 1);
        assert_eq!(requests.iter().filter(|r| r.path.starts_with("/v2/project/")).count(), 2);

        let beta = e.check_mod_updates("upd", ReleaseChannel::Beta).await.unwrap();
        let sodium = beta.updates.iter().find(|u| u.filename == "sodium-fabric-0.9.2.jar").unwrap();
        assert_eq!(sodium.to_version_id, "SODBETA");
        assert_eq!(sodium.version_type, "beta");
        assert!(!beta.updates.iter().any(|u| u.filename == "local.jar"));
    }

    #[tokio::test]
    async fn apply_updates_keeps_iris_sodium_aligned_and_writes_rollback() {
        let (_d, e) = temp_engine();
        let mods_dir = write_instance(&e, "upd2", &[]);
        let sodium_old_path = mods_dir.join("sodium-fabric-0.10.0.jar.disabled");
        let iris_old_path = mods_dir.join("iris-fabric-1.11.7.jar");
        fabric_jar(&sodium_old_path, "sodium", "Sodium", "0.10.0", json!({}));
        fabric_jar(&iris_old_path, "iris", "Iris", "1.11.7", json!({ "sodium": [">=0.10.0 <0.10.1"] }));
        let sodium_old = std::fs::read(&sodium_old_path).unwrap();
        let iris_old = std::fs::read(&iris_old_path).unwrap();
        let sodium_hash = crate::download::sha1_hex(&sodium_old);
        let iris_hash = crate::download::sha1_hex(&iris_old);

        // New builds served over the mock CDN.
        let sodium_new = b"new sodium jar bytes".to_vec();
        let iris_match = b"iris build pinning the new sodium".to_vec();
        let iris_beta = b"iris beta build".to_vec();
        let iris_bad = b"iris newest release, not pinning the new sodium".to_vec();
        let sodium_sha = crate::download::sha1_hex(&sodium_new);
        let iris_match_sha = crate::download::sha1_hex(&iris_match);
        let iris_beta_sha = crate::download::sha1_hex(&iris_beta);
        let iris_bad_sha = crate::download::sha1_hex(&iris_bad);

        #[derive(Clone)]
        struct MockData {
            update_by_hash: Value,
            version_files: Value,
            by_id: Value,
            version_lists: Value,
            sodium_new: Vec<u8>,
            iris_match: Vec<u8>,
            iris_beta: Vec<u8>,
            sodium_old: Vec<u8>,
            iris_old: Vec<u8>,
        }
        let state: Arc<Mutex<Option<MockData>>> = Arc::new(Mutex::new(None));
        let server = MockServer::start({
            let state = state.clone();
            move |req: &MockRequest| {
                let Some(data) = state.lock().unwrap().clone() else {
                    return MockResponse::status(500);
                };
                if req.path == "/v2/version_files/update" && req.method == "POST" {
                    return MockResponse::json(&data.update_by_hash);
                }
                if req.path == "/v2/version_files" && req.method == "POST" {
                    return MockResponse::json(&data.version_files);
                }
                if let Some(rest) = req.path.strip_prefix("/v2/project/") {
                    let id = rest.split('/').next().unwrap();
                    return MockResponse::json(&data.version_lists.get(id).cloned().unwrap_or(json!([])));
                }
                if let Some(id) = req.path.strip_prefix("/v2/version/") {
                    if let Some(v) = data.by_id.get(id) {
                        return MockResponse::json(v);
                    }
                }
                if let Some(q) = req.path.strip_prefix("/v2/projects?ids=") {
                    let decoded =
                        url::form_urlencoded::parse(format!("x={q}").as_bytes()).next().unwrap().1.into_owned();
                    let ids: Vec<String> = serde_json::from_str(&decoded).unwrap();
                    return MockResponse::json(&Value::Array(
                        ids.iter().map(|id| json!({ "id": id, "title": format!("Title {id}") })).collect(),
                    ));
                }
                match req.path.as_str() {
                    "/cdn/snew" => return MockResponse::bytes(data.sodium_new),
                    "/cdn/imatch" => return MockResponse::bytes(data.iris_match),
                    "/cdn/ibeta" => return MockResponse::bytes(data.iris_beta),
                    "/cdn/sold" => return MockResponse::bytes(data.sodium_old),
                    "/cdn/iold" => return MockResponse::bytes(data.iris_old),
                    _ => {}
                }
                MockResponse::status(404)
            }
        })
        .await;
        let cdn = server.url("/cdn");
        let sod_old = version_json(
            crate::mods::SODIUM_PROJECT_ID,
            "SODOLD",
            "0.10.0",
            "release",
            json!([]),
            file_json("sodium-fabric-0.10.0.jar", &sodium_hash, &format!("{cdn}/sold"), sodium_old.len() as u64),
        );
        let iris_old_meta = version_json(
            crate::mods::IRIS_PROJECT_ID,
            "IROLD",
            "1.11.7",
            "release",
            json!([{
                "project_id": crate::mods::SODIUM_PROJECT_ID,
                "version_id": "SODOLD",
                "dependency_type": "required"
            }]),
            file_json("iris-fabric-1.11.7.jar", &iris_hash, &format!("{cdn}/iold"), iris_old.len() as u64),
        );
        let sod_new = version_json(
            crate::mods::SODIUM_PROJECT_ID,
            "SODNEW",
            "0.10.1",
            "release",
            json!([]),
            file_json("sodium-fabric-0.10.1.jar", &sodium_sha, &format!("{cdn}/snew"), sodium_new.len() as u64),
        );
        let iris_beta_meta = version_json(
            crate::mods::IRIS_PROJECT_ID,
            "IRBETA",
            "1.12.0-beta.1",
            "beta",
            json!([{ "project_id": crate::mods::SODIUM_PROJECT_ID, "dependency_type": "required" }]),
            file_json("iris-fabric-1.12.0-beta.1.jar", &iris_beta_sha, &format!("{cdn}/ibeta"), iris_beta.len() as u64),
        );
        let iris_match_meta = version_json(
            crate::mods::IRIS_PROJECT_ID,
            "IRMATCH",
            "1.12.0",
            "release",
            json!([{
                "project_id": crate::mods::SODIUM_PROJECT_ID,
                "version_id": "SODNEW",
                "dependency_type": "required"
            }]),
            file_json("iris-fabric-1.12.0.jar", &iris_match_sha, &format!("{cdn}/imatch"), iris_match.len() as u64),
        );
        // Newest release for the instance: it does not pin the new Sodium, so
        // the companion logic must move Iris to IRMATCH instead.
        let mut iris_bad_meta = version_json(
            crate::mods::IRIS_PROJECT_ID,
            "IRBAD",
            "1.12.1",
            "release",
            json!([{ "project_id": crate::mods::SODIUM_PROJECT_ID, "dependency_type": "required" }]),
            file_json("iris-fabric-1.12.1.jar", &iris_bad_sha, &format!("{cdn}/ibad"), iris_bad.len() as u64),
        );
        iris_bad_meta["date_published"] = json!("2026-11-21T00:00:00Z");
        let version_files = json!({
            sodium_hash.clone(): sod_old.clone(),
            iris_hash.clone(): iris_old_meta.clone(),
        });
        let update_by_hash = json!({
            sodium_hash.clone(): sod_new.clone(),
            iris_hash.clone(): iris_beta_meta.clone(),
        });
        let sodium_list = json!([sod_new]);
        let iris_list = json!([iris_beta_meta.clone(), iris_match_meta.clone(), iris_bad_meta.clone()]);
        let by_id = json!({
            "SODOLD": sod_old,
            "IROLD": iris_old_meta,
            "SODNEW": sodium_list[0].clone(),
            "IRBETA": iris_beta_meta,
            "IRMATCH": iris_match_meta,
            "IRBAD": iris_bad_meta,
        });
        let (sodium_pid, iris_pid) = (crate::mods::SODIUM_PROJECT_ID, crate::mods::IRIS_PROJECT_ID);
        let version_lists = json!({
            sodium_pid: sodium_list,
            iris_pid: iris_list,
        });
        *state.lock().unwrap() = Some(MockData {
            update_by_hash,
            version_files,
            by_id,
            version_lists,
            sodium_new: sodium_new.clone(),
            iris_match: iris_match.clone(),
            iris_beta: iris_beta.clone(),
            sodium_old: sodium_old.clone(),
            iris_old: iris_old.clone(),
        });
        configure(&e, &server);

        let result = e.apply_mod_updates("upd2", None).await.unwrap();
        assert_eq!(result.changed.len(), 2, "{:?}", result.changed);
        let sodium = result.updated.iter().find(|u| u.project_id == crate::mods::SODIUM_PROJECT_ID).unwrap();
        assert_eq!(sodium.to_version_id, "SODNEW");
        let iris = result.updated.iter().find(|u| u.project_id == crate::mods::IRIS_PROJECT_ID).unwrap();
        assert_eq!(iris.to_version_id, "IRMATCH", "Iris must move to the build pinning the new Sodium");

        // Applied files preserve the enabled/disabled state of the old jars.
        assert_eq!(std::fs::read(mods_dir.join("sodium-fabric-0.10.1.jar.disabled")).unwrap(), sodium_new);
        assert_eq!(std::fs::read(mods_dir.join("iris-fabric-1.12.0.jar")).unwrap(), iris_match);
        assert!(!mods_dir.join("sodium-fabric-0.10.0.jar").exists());
        assert!(!mods_dir.join("sodium-fabric-0.10.0.jar.disabled").exists());
        assert!(!mods_dir.join("iris-fabric-1.11.7.jar").exists());
        assert!(!mods_dir.join("iris-fabric-1.12.0-beta.1.jar").exists());

        let backups = e.paths().instance_dir("upd2").join("backups");
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(backups.join("last-mod-update.json")).unwrap()).unwrap();
        let rows = manifest.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let sodium_row = rows
            .iter()
            .find(|r| r.get("project_id").and_then(Value::as_str) == Some(crate::mods::SODIUM_PROJECT_ID))
            .unwrap();
        assert_eq!(sodium_row["prev_filename"], "sodium-fabric-0.10.0.jar");
        assert_eq!(sodium_row["prev_version_id"], "SODOLD");
        assert_eq!(sodium_row["new_filename"], "sodium-fabric-0.10.1.jar");
        assert_eq!(std::fs::read(backups.join("last-mod-update/sodium-fabric-0.10.0.disabled")).unwrap(), sodium_old);
        assert_eq!(std::fs::read(backups.join("last-mod-update/iris-fabric-1.11.7")).unwrap(), iris_old);

        let undo = e.undo_last_mod_update("upd2").await.unwrap();
        assert_eq!(undo.restored.len(), 2);
        assert_eq!(std::fs::read(mods_dir.join("sodium-fabric-0.10.0.jar.disabled")).unwrap(), sodium_old);
        assert_eq!(std::fs::read(mods_dir.join("iris-fabric-1.11.7.jar")).unwrap(), iris_old);
        assert!(!mods_dir.join("sodium-fabric-0.10.1.jar.disabled").exists());
        assert!(!mods_dir.join("iris-fabric-1.12.0.jar").exists());
        assert!(!backups.join("last-mod-update.json").exists());
    }

    #[tokio::test]
    async fn orphan_detector_finds_unused_libraries_after_delete() {
        let (_d, e) = temp_engine();
        let mods_dir = write_instance(&e, "orph", &[]);
        fabric_jar(
            &mods_dir.join("target.jar"),
            "target",
            "Target Mod",
            "1.0",
            json!({
                "cloth-config": [">=1"],
                "architectury": [">=1"],
                "fabric-api": [">=1"],
                "sodium": [">=1"],
            }),
        );
        fabric_jar(&mods_dir.join("helper.jar"), "helper", "Helper", "1.0", json!({ "cloth-config": [">=1"] }));
        fabric_jar(&mods_dir.join("cloth.jar"), "cloth-config", "Cloth Config", "1.0", json!({}));
        fabric_jar(&mods_dir.join("arch.jar"), "architectury", "Architectury API", "1.0", json!({}));
        fabric_jar(&mods_dir.join("api.jar"), "fabric-api", "Fabric API", "1.0", json!({}));
        fabric_jar(&mods_dir.join("sodium.jar"), "sodium", "Sodium", "1.0", json!({}));

        let orphans = e.orphaned_libraries_after_delete("orph", "target.jar").await.unwrap();
        let filenames: Vec<&str> = orphans.iter().map(|o| o.filename.as_str()).collect();
        assert_eq!(filenames, vec!["api.jar", "arch.jar"], "{orphans:?}");
        let arch = orphans.iter().find(|o| o.filename == "arch.jar").unwrap();
        assert_eq!(arch.display_name, "Architectury API");
        assert_eq!(arch.project_id.as_deref(), Some("lhGA9TYQ"));
        let api = orphans.iter().find(|o| o.filename == "api.jar").unwrap();
        assert_eq!(api.project_id.as_deref(), Some("P7dR8mSH"));

        let dependents = e.dependents_of_mod("orph", "cloth.jar").await.unwrap();
        let names: Vec<&str> = dependents.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(names, vec!["helper.jar", "target.jar"]);
    }

    #[tokio::test]
    async fn extracts_embedded_fabric_icon() {
        let (_d, e) = temp_engine();
        let mods_dir = write_instance(&e, "icons", &[]);
        let meta = json!({
            "schemaVersion": 1,
            "id": "iconmod",
            "version": "1.0",
            "icon": {
                "32": "assets/icon32.png",
                "64": "assets/icon64.png",
                "256": "assets/icon256.png"
            }
        });
        write_jar(
            &mods_dir.join("icon-mod.jar"),
            &[
                ("fabric.mod.json", meta.to_string().as_bytes()),
                ("assets/icon32.png", b"i32"),
                ("assets/icon64.png", b"i64"),
                ("assets/icon256.png", b"i256"),
            ],
        );
        let icon = e.mod_embedded_icon("icons", "icon-mod.jar").await.unwrap();
        assert_eq!(icon.as_deref(), Some(&b"i64"[..]), "largest entry <= 128 px");

        // Only a 256 px entry: nothing to pick <= 128.
        let huge = json!({ "schemaVersion": 1, "id": "huge", "version": "1", "icon": { "256": "big.png" } });
        write_jar(&mods_dir.join("huge.jar"), &[("fabric.mod.json", huge.to_string().as_bytes()), ("big.png", b"big")]);
        assert_eq!(e.mod_embedded_icon("icons", "huge.jar").await.unwrap(), None);

        // Forge fallback: mods.toml logoFile.
        let toml = concat!(
            "modLoader=\"javafml\"\n",
            "loaderVersion=\"[1,)\"\n",
            "logoFile=\"logo.png\"\n\n",
            "[[mods]]\nmodId=\"forgemod\"\nversion=\"1\"\n"
        );
        write_jar(&mods_dir.join("forge-mod.jar"), &[("META-INF/mods.toml", toml.as_bytes()), ("logo.png", b"logo")]);
        assert_eq!(e.mod_embedded_icon("icons", "forge-mod.jar").await.unwrap().as_deref(), Some(&b"logo"[..]));
    }
}
