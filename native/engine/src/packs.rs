//! Resource-pack and shader-pack manager (smart-mods plan §4.1).
//!
//! Packs live in `<game dir>/resourcepacks/` and `<game dir>/shaderpacks/`, each
//! entry a `.zip` archive or a folder. The resource-pack stack is the
//! `resourcePacks` array of `options.txt` (Minecraft lists the lowest priority
//! first); the active shader is `shaderPack` + `enableShaders` in
//! `config/iris.properties`.
//!
//! Every mutating call refuses a running instance (`409 ALREADY_RUNNING`), takes
//! plain basenames only, caps archive reads and writes config files through a
//! sibling `.tmp` renamed into place. Archive contents are read from the zip
//! central directory; nothing is decompressed except `pack.mcmeta`, `pack.png`
//! and the explicit unwrap/copy operations.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::download::{DownloadOptions, FetchOptions};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::import::{parse_options_txt, set_option_key, write_options_txt, ParsedOptions};
use crate::instances::{effective_game_dir, get_instance};
use crate::mods::{cached_version_files, encode_uri_component, set_property};
use crate::types::InstanceMeta;

/// Largest `pack.mcmeta` read (real files are a few hundred bytes).
const MAX_MCMETA_BYTES: u64 = 256 * 1024;
/// Largest `pack.png` returned to the UI.
const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
/// Total decompressed size an unwrap may write (zip-bomb cap).
const MAX_UNWRAP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Entries a pack may hold before it is refused (zip-bomb cap).
const MAX_PACK_ENTRIES: usize = 50_000;
/// Memo size before a per-engine pack cache is dropped wholesale.
const MEMO_CAP: usize = 4096;
const RESOURCE_PACK_LOADER: &str = "minecraft";
const SHADER_LOADER: &str = "iris";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Which pack folder an operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    ResourcePack,
    Shader,
}

/// How a resource pack fares on the instance's Minecraft version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackCompatStatus {
    Compatible,
    /// The pack targets an older format than the game (game > pack max).
    TooOld,
    /// The pack targets a newer format than the game (game < pack min).
    TooNew,
    /// Missing or corrupt `pack.mcmeta`.
    InvalidMeta,
    /// `pack.mcmeta` only exists one folder deep inside the zip.
    NestedArchive,
    /// The instance's Minecraft version has no known resource-pack format.
    Unknown,
}

/// Inclusive `(major, minor)` range of resource-pack formats a pack supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFormatRange {
    pub min: (u32, u32),
    pub max: (u32, u32),
}

/// One resource pack in `resourcepacks/`, joined with options.txt and Modrinth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePackEntry {
    /// Basename on disk (`MyPack.zip` or the folder name `MyPack`).
    pub filename: String,
    /// Entry id in options.txt (`file/MyPack.zip`).
    pub option_id: String,
    pub is_dir: bool,
    pub size: u64,
    pub sha1: Option<String>,
    /// Filename without `.zip`.
    pub display_name: String,
    /// Plain text description (`§` codes and JSON text components flattened).
    pub description: String,
    pub has_icon: bool,
    pub enabled: bool,
    /// 0 = highest priority (top of the in-game list); None when disabled.
    pub priority: Option<usize>,
    pub compat: PackCompatStatus,
    pub format_range: Option<PackFormatRange>,
    /// Resource-pack format of the instance's Minecraft version.
    pub target_format: Option<(u32, u32)>,
    /// Folder holding `pack.mcmeta` when `compat == NestedArchive` (e.g. `"MyPack/"`).
    pub nested_prefix: Option<String>,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub version_number: Option<String>,
    /// Textures this pack defines that an enabled lower-priority pack also defines.
    pub overrides_count: usize,
    /// Textures this pack defines that an enabled higher-priority pack also defines.
    pub overridden_by_count: usize,
    /// Higher-priority packs that override this one, top first.
    pub overridden_by_names: Vec<String>,
    /// Coarse content tags: `items`, `blocks`, `gui`, `particles`, `entity`, `sky`, `font`.
    pub texture_tags: Vec<String>,
}

/// One shader pack in `shaderpacks/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderPackEntry {
    pub filename: String,
    pub is_dir: bool,
    pub size: u64,
    pub sha1: Option<String>,
    /// Matches `shaderPack` in iris.properties while `enableShaders=true`.
    pub active: bool,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub version_number: Option<String>,
}

/// Iris state plus the shader packs of one instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadersState {
    pub iris_installed: bool,
    pub shaders_enabled: bool,
    /// `shaderPack` from iris.properties (None when unset or `off`).
    pub active_pack: Option<String>,
    pub packs: Vec<ShaderPackEntry>,
}

/// What a `.zip` (or a Modrinth file) turned out to be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZipPackKind {
    /// `pack.mcmeta` at the root (`nested_prefix: None`) or one folder deep.
    ResourcePack { nested_prefix: Option<String> },
    /// `shaders/gbuffers_*` or `shaders/shaders.properties` (root or one folder deep).
    ShaderPack,
    /// Contains `modrinth.index.json`: a modpack, not a pack.
    Mrpack,
    Unknown,
}

/// Result of installing a pack file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackInstallResult {
    /// Name the pack was stored under (may carry a numeric suffix).
    pub filename: String,
    pub kind: PackKind,
    pub activated: bool,
    /// Shader packs only: Iris is not in `mods/`, so the shader cannot run yet.
    pub needs_iris: bool,
    /// Local zips only: the pack is nested one folder deep and can be unwrapped.
    pub nested: bool,
    /// Machine-readable reason an auto-activation was skipped (`"running"`, `"nested_zip"`).
    pub note: Option<String>,
}

/// A pack the target Minecraft version cannot load, for the version-change preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionChangePack {
    pub filename: String,
    pub display_name: String,
    pub compat: PackCompatStatus,
    /// Newer Modrinth build for the target version, when one exists.
    pub update_version_id: Option<String>,
    pub update_version_number: Option<String>,
}

/// A newer Modrinth build of an installed pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackUpdate {
    pub filename: String,
    pub project_id: Option<String>,
    /// Installed build (None when Modrinth does not know the file).
    pub version_id: Option<String>,
    pub to_version_id: String,
    pub to_version_number: Option<String>,
    pub to_filename: Option<String>,
}

/// Result of unwrapping a nested resource pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackUnwrapResult {
    pub filename: String,
    /// Where the original archive was kept.
    pub backup: String,
    /// Files written into the new archive.
    pub entries: usize,
}

/// Size of an instance's `downloads/` (server resource-pack cache).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerPackCacheInfo {
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Result of copying packs between instances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackCopyResult {
    /// Names the packs were stored under in the target.
    pub copied: Vec<String>,
    /// Source packs already present in the target with identical bytes.
    pub skipped: Vec<String>,
}

/// Identity of a pack file for the memo caches: a changed file never reuses a stale result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackStamp {
    pub path: PathBuf,
    pub mtime_ns: u128,
    pub len: u64,
}

/// Texture paths and tags a pack defines, relative to its pack root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackScan {
    pub textures: BTreeSet<String>,
    pub tags: Vec<String>,
}

/// Parsed `pack.mcmeta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackMeta {
    pub range: PackFormatRange,
    pub description: String,
}

// ---------------------------------------------------------------------------
// Paths, versions and metadata
// ---------------------------------------------------------------------------

/// `<effective game dir>/resourcepacks`.
pub fn effective_resourcepacks_dir(engine: &Engine, inst: &InstanceMeta) -> PathBuf {
    effective_game_dir(engine, inst).join("resourcepacks")
}

/// `<effective game dir>/shaderpacks`.
pub fn effective_shaderpacks_dir(engine: &Engine, inst: &InstanceMeta) -> PathBuf {
    effective_game_dir(engine, inst).join("shaderpacks")
}

fn pack_dir(game: &Path, kind: PackKind) -> PathBuf {
    game.join(match kind {
        PackKind::ResourcePack => "resourcepacks",
        PackKind::Shader => "shaderpacks",
    })
}

/// Resource-pack format of a Minecraft version: a downloaded client jar's `version.json`
/// first (exact, covers releases the table lacks), then the static table. None when neither
/// knows the version (`26.3` until its jar is downloaded).
pub fn target_resource_pack_format(engine: &Engine, mc_version: &str) -> Option<(u32, u32)> {
    if let Some(found) = engine.content_state().pack_format.lock().unwrap_or_else(|p| p.into_inner()).get(mc_version) {
        return Some(*found);
    }
    if let Some(found) = jar_pack_format(&engine.paths().client_jar(mc_version)) {
        engine
            .content_state()
            .pack_format
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(mc_version.to_string(), found);
        return Some(found);
    }
    fallback_pack_format(mc_version)
}

/// `pack_version` from a client jar's `version.json` (`resource` or `resource_major/minor`).
fn jar_pack_format(jar: &Path) -> Option<(u32, u32)> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(jar).ok()?).ok()?;
    let entry = archive.by_name("version.json").ok()?;
    let text = String::from_utf8(read_capped(entry, 1024 * 1024).ok()?).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let pv = root.get("pack_version")?;
    if let Some(n) = pv.get("resource").and_then(int_u32) {
        return Some((n, 0));
    }
    let major = pv.get("resource_major").and_then(int_u32)?;
    let minor = pv.get("resource_minor").and_then(int_u32).unwrap_or(0);
    Some((major, minor))
}

/// Static fallback: the releases the smart-mods plan lists, plus the 1.20–1.21.7 formats
/// verified against minecraft.wiki. Versions outside this table need their client jar.
fn fallback_pack_format(mc: &str) -> Option<(u32, u32)> {
    const TABLE: &[(&str, u32)] = &[
        ("1.8.9", 1),
        ("1.20", 15),
        ("1.20.1", 15),
        ("1.20.2", 18),
        ("1.20.3", 22),
        ("1.20.4", 22),
        ("1.20.5", 32),
        ("1.20.6", 32),
        ("1.21", 34),
        ("1.21.1", 34),
        ("1.21.2", 42),
        ("1.21.3", 42),
        ("1.21.4", 46),
        ("1.21.5", 55),
        ("1.21.6", 63),
        ("1.21.7", 64),
        ("1.21.8", 64),
        ("1.21.9", 69),
        ("1.21.10", 69),
        ("1.21.11", 75),
        ("26.1", 84),
        ("26.2", 88),
    ];
    TABLE.iter().find(|(v, _)| *v == mc).map(|(_, major)| (*major, 0))
}

fn int_u32(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// `N` → `(N, 0)`, `[major, minor]` → `(major, minor)`.
fn format_version(v: &Value) -> Option<(u32, u32)> {
    match v {
        Value::Array(a) if a.len() == 2 => Some((int_u32(&a[0])?, int_u32(&a[1])?)),
        other => Some((int_u32(other)?, 0)),
    }
}

/// Every format from `lo` through `hi` (the minor component is open-ended, as in the game).
fn span(lo: u32, hi: u32) -> PackFormatRange {
    PackFormatRange { min: (lo, 0), max: (hi, u32::MAX) }
}

/// Format range from `min_format`/`max_format` (1.21.9+), `supported_formats` (1.20.2–1.21.8)
/// or `pack_format` (legacy), in that precedence.
fn parse_range(pack: &Value) -> Option<PackFormatRange> {
    if pack.get("min_format").is_some() || pack.get("max_format").is_some() {
        let min = pack.get("min_format").or_else(|| pack.get("max_format")).and_then(format_version)?;
        let max = pack.get("max_format").or_else(|| pack.get("min_format")).and_then(format_version)?;
        return Some(PackFormatRange { min, max });
    }
    if let Some(sf) = pack.get("supported_formats") {
        return match sf {
            Value::Number(_) => int_u32(sf).map(|n| span(n, n)),
            Value::Array(a) if a.len() == 2 => Some(span(int_u32(&a[0])?, int_u32(&a[1])?)),
            Value::Object(o) => Some(span(int_u32(o.get("min_inclusive")?)?, int_u32(o.get("max_inclusive")?)?)),
            _ => None,
        };
    }
    int_u32(pack.get("pack_format")?).map(|n| span(n, n))
}

/// Parse a `pack.mcmeta` document. None when it is not JSON, has no `pack` object or
/// declares no format range. A UTF-8 BOM is tolerated.
pub fn parse_pack_mcmeta(raw: &str) -> Option<PackMeta> {
    let root: Value = serde_json::from_str(raw.strip_prefix('\u{feff}').unwrap_or(raw)).ok()?;
    let pack = root.get("pack")?;
    let range = parse_range(pack)?;
    let description = pack.get("description").map(flatten_text).unwrap_or_default();
    Some(PackMeta { range, description })
}

/// Plain text of a `description` value (string, text component or array).
fn flatten_text(v: &Value) -> String {
    let mut out = String::new();
    flatten_into(v, &mut out);
    out
}

fn flatten_into(v: &Value, out: &mut String) {
    match v {
        Value::String(s) => out.push_str(&strip_codes(s)),
        Value::Array(items) => {
            for item in items {
                flatten_into(item, out);
            }
        }
        Value::Object(o) => {
            match (o.get("text").and_then(Value::as_str), o.get("translate").and_then(Value::as_str)) {
                (Some(text), _) => out.push_str(&strip_codes(text)),
                (None, Some(key)) => out.push_str(key),
                (None, None) => {}
            }
            if let Some(extra) = o.get("extra") {
                flatten_into(extra, out);
            }
        }
        _ => {}
    }
}

/// Remove Minecraft `§x` formatting codes (`§` plus one code character).
fn strip_codes(s: &str) -> String {
    const CODES: &str = "0123456789abcdefklmnorxABCDEFKLMNORX";
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '§' && chars.peek().is_some_and(|n| CODES.contains(*n)) {
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Compare a game format with a pack's range.
fn compare(target: (u32, u32), range: &PackFormatRange) -> PackCompatStatus {
    if target < range.min {
        PackCompatStatus::TooNew
    } else if target > range.max {
        PackCompatStatus::TooOld
    } else {
        PackCompatStatus::Compatible
    }
}

fn compat_of(meta: Option<&PackMeta>, nested: bool, target: Option<(u32, u32)>) -> PackCompatStatus {
    if nested {
        return PackCompatStatus::NestedArchive;
    }
    let Some(meta) = meta else { return PackCompatStatus::InvalidMeta };
    match target {
        Some(t) => compare(t, &meta.range),
        None => PackCompatStatus::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Archive and folder access (central directory only unless content is asked for)
// ---------------------------------------------------------------------------

/// One entry of a pack. Folders carry a trailing `/` in `name`, as zip entries do.
#[derive(Debug, Clone)]
struct Listed {
    name: String,
    size: u64,
    is_dir: bool,
}

/// A pack on disk: a zip archive or a folder.
enum Source {
    Zip(PathBuf),
    Dir(PathBuf),
}

impl Source {
    fn list(&self) -> EngineResult<Vec<Listed>> {
        match self {
            Source::Zip(path) => list_zip(path),
            Source::Dir(root) => {
                let mut out = Vec::new();
                walk_dir(root, "", 0, &mut out)?;
                Ok(out)
            }
        }
    }

    /// One entry's bytes, at most `cap`; None when absent (or unreadable as a zip).
    fn read(&self, name: &str, cap: u64) -> EngineResult<Option<Vec<u8>>> {
        match self {
            Source::Zip(path) => {
                let mut archive = open_zip(path)?;
                let Ok(entry) = archive.by_name(name) else { return Ok(None) };
                read_capped(entry, cap).map(Some)
            }
            Source::Dir(root) => {
                let path = root.join(name);
                let Ok(md) = std::fs::metadata(&path) else { return Ok(None) };
                if !md.is_file() || md.len() > cap {
                    return Ok(None);
                }
                Ok(Some(std::fs::read(path)?))
            }
        }
    }
}

fn zip_err(e: impl std::fmt::Display) -> EngineError {
    EngineError::other(format!("zip: {e}"))
}

fn too_many_entries() -> EngineError {
    EngineError::bad_request("PACK_TOO_MANY_ENTRIES", format!("a pack may hold at most {MAX_PACK_ENTRIES} entries"))
}

fn open_zip(path: &Path) -> EngineResult<zip::ZipArchive<std::fs::File>> {
    zip::ZipArchive::new(std::fs::File::open(path)?).map_err(zip_err)
}

/// Read an entry through a byte cap: a declared or actual size above `cap` is an error.
fn read_capped(reader: impl Read, cap: u64) -> EngineResult<Vec<u8>> {
    let mut buf = Vec::new();
    let mut reader = reader;
    (&mut reader).take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(EngineError::other("pack entry is larger than its size cap"));
    }
    Ok(buf)
}

/// Entries of a zip from its central directory (nothing is decompressed).
fn list_zip(path: &Path) -> EngineResult<Vec<Listed>> {
    let mut archive = open_zip(path)?;
    if archive.len() > MAX_PACK_ENTRIES {
        return Err(too_many_entries());
    }
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(zip_err)?;
        out.push(Listed { name: entry.name().map_err(zip_err)?.into_owned(), size: entry.size(), is_dir: entry.is_dir() });
    }
    Ok(out)
}

fn walk_dir(root: &Path, rel: &str, depth: usize, out: &mut Vec<Listed>) -> EngineResult<()> {
    if depth > 16 {
        return Err(EngineError::bad_request("PACK_TOO_DEEP", "pack folder nests too deeply"));
    }
    for entry in std::fs::read_dir(root.join(rel))? {
        let entry = entry?;
        let leaf = entry.file_name().to_string_lossy().into_owned();
        let name = if rel.is_empty() { leaf } else { format!("{rel}/{leaf}") };
        let ft = entry.file_type()?;
        if ft.is_dir() {
            out.push(Listed { name: format!("{name}/"), size: 0, is_dir: true });
            walk_dir(root, &name, depth + 1, out)?;
        } else if ft.is_file() {
            out.push(Listed { name, size: entry.metadata()?.len(), is_dir: false });
        }
        if out.len() > MAX_PACK_ENTRIES {
            return Err(too_many_entries());
        }
    }
    Ok(())
}

/// Where a pack's `pack.mcmeta`, `pack.png` and content root are.
#[derive(Debug, Default)]
struct Layout {
    mcmeta: Option<String>,
    icon: Option<String>,
    nested_prefix: Option<String>,
}

/// Root `pack.mcmeta` first; otherwise exactly one folder holding one (`nested_prefix`).
fn layout(entries: &[Listed]) -> Layout {
    if let Some(mcmeta) = find_leaf(entries, "", "pack.mcmeta") {
        return Layout { mcmeta: Some(mcmeta), icon: find_leaf(entries, "", "pack.png"), nested_prefix: None };
    }
    let folders: BTreeSet<String> = entries
        .iter()
        .filter(|e| !e.is_dir)
        .filter_map(|e| {
            let (top, rest) = e.name.split_once('/')?;
            (!rest.contains('/') && rest.eq_ignore_ascii_case("pack.mcmeta")).then(|| format!("{top}/"))
        })
        .collect();
    if folders.len() != 1 {
        return Layout::default();
    }
    let prefix = folders.into_iter().next().unwrap_or_default();
    Layout {
        mcmeta: find_leaf(entries, &prefix, "pack.mcmeta"),
        icon: find_leaf(entries, &prefix, "pack.png"),
        nested_prefix: Some(prefix),
    }
}

/// The file `prefix + leaf` (exact case preferred, ASCII case-insensitive otherwise).
fn find_leaf(entries: &[Listed], prefix: &str, leaf: &str) -> Option<String> {
    let mut fallback: Option<&Listed> = None;
    for e in entries.iter().filter(|e| !e.is_dir) {
        let Some(rest) = e.name.strip_prefix(prefix) else { continue };
        if rest.contains('/') || !rest.eq_ignore_ascii_case(leaf) {
            continue;
        }
        if rest == leaf {
            return Some(e.name.clone());
        }
        if fallback.is_none() {
            fallback = Some(e);
        }
    }
    fallback.map(|e| e.name.clone())
}

fn is_shader_entry(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    match parts.as_slice() {
        ["shaders", leaf] | [_, "shaders", leaf] => leaf.starts_with("gbuffers_") || *leaf == "shaders.properties",
        _ => false,
    }
}

fn classify_entries(entries: &[Listed]) -> ZipPackKind {
    let files = || entries.iter().filter(|e| !e.is_dir).map(|e| e.name.as_str());
    if files().any(|n| n == "modrinth.index.json") {
        return ZipPackKind::Mrpack;
    }
    if files().any(is_shader_entry) {
        return ZipPackKind::ShaderPack;
    }
    let lay = layout(entries);
    if lay.mcmeta.is_some() {
        return ZipPackKind::ResourcePack { nested_prefix: lay.nested_prefix };
    }
    ZipPackKind::Unknown
}

/// Classify a local `.zip` from its central directory (no decompression).
pub fn classify_zip_pack(path: &Path) -> EngineResult<ZipPackKind> {
    Ok(classify_entries(&list_zip(path)?))
}

// ---------------------------------------------------------------------------
// Texture scan and overlap
// ---------------------------------------------------------------------------

fn texture_tag(rel: &str) -> Option<&'static str> {
    let sub = rel.strip_prefix("assets/minecraft/textures/")?;
    match sub.split('/').next()? {
        "item" | "items" => Some("items"),
        "block" | "blocks" => Some("blocks"),
        "gui" => Some("gui"),
        "particle" | "particles" => Some("particles"),
        "entity" => Some("entity"),
        "environment" => Some("sky"),
        _ => None,
    }
}

/// Textures a pack defines (PNG under `assets/minecraft/textures/`, plus the OptiFine sky and
/// the font folder), relative to the pack root.
fn scan(entries: &[Listed], prefix: &str) -> PackScan {
    let mut out = PackScan::default();
    let mut tags: BTreeSet<&'static str> = BTreeSet::new();
    for e in entries.iter().filter(|e| !e.is_dir) {
        let Some(rel) = e.name.strip_prefix(prefix) else { continue };
        if rel.starts_with("assets/minecraft/textures/") && rel.ends_with(".png") {
            if let Some(tag) = texture_tag(rel) {
                tags.insert(tag);
            }
        } else if rel.starts_with("assets/minecraft/optifine/sky/") {
            tags.insert("sky");
        } else if rel.starts_with("assets/minecraft/font/") {
            tags.insert("font");
        } else {
            continue;
        }
        out.textures.insert(rel.to_string());
    }
    out.tags = tags.into_iter().map(str::to_string).collect();
    out
}

fn stamp_of(path: &Path, md: &std::fs::Metadata, len: u64) -> PackStamp {
    let mtime_ns = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    PackStamp { path: path.to_path_buf(), mtime_ns, len }
}

fn scan_cached(engine: &Engine, stamp: PackStamp, entries: &[Listed], prefix: &str) -> Arc<PackScan> {
    let cache = &engine.content_state().pack_scans;
    if let Some(found) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&stamp) {
        return found.clone();
    }
    let fresh = Arc::new(scan(entries, prefix));
    let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
    if guard.len() >= MEMO_CAP {
        guard.clear();
    }
    guard.insert(stamp, fresh.clone());
    fresh
}

/// Overlap counts for one pack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Overlap {
    overrides: usize,
    overridden_by: usize,
    names: Vec<String>,
}

/// Overlaps for enabled packs. `packs` is ordered lowest priority first; the position is the
/// priority rank, so a higher rank overrides a lower one. Returns `(filename, overlap)` in input order.
fn compute_overlaps(packs: &[(String, Arc<PackScan>)]) -> Vec<(String, Overlap)> {
    let mut owners: HashMap<&str, Vec<usize>> = HashMap::new();
    for (rank, (_, scan)) in packs.iter().enumerate() {
        for path in &scan.textures {
            owners.entry(path.as_str()).or_default().push(rank);
        }
    }
    let mut counts = vec![Overlap::default(); packs.len()];
    let mut higher: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); packs.len()];
    for ranks in owners.values().filter(|r| r.len() > 1) {
        for (i, &rank) in ranks.iter().enumerate() {
            if i > 0 {
                counts[rank].overrides += 1;
            }
            if i + 1 < ranks.len() {
                counts[rank].overridden_by += 1;
                higher[rank].extend(ranks[i + 1..].iter().copied());
            }
        }
    }
    packs
        .iter()
        .enumerate()
        .map(|(rank, (name, _))| {
            let mut overlap = std::mem::take(&mut counts[rank]);
            // Highest priority first.
            overlap.names = higher[rank].iter().rev().map(|r| display_name(&packs[*r].0)).collect();
            (name.clone(), overlap)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pack inspection
// ---------------------------------------------------------------------------

/// A pack read from disk, before it is joined with options.txt and Modrinth.
struct Inspected {
    filename: String,
    is_dir: bool,
    size: u64,
    sha1: Option<String>,
    meta: Option<PackMeta>,
    nested_prefix: Option<String>,
    has_icon: bool,
    scan: Arc<PackScan>,
}

/// Read one pack. `with_hash` also computes its SHA-1 (zip files only, memoized).
async fn inspect(engine: &Engine, dir: &Path, filename: &str, with_hash: bool) -> EngineResult<Inspected> {
    let path = dir.join(filename);
    let md = std::fs::metadata(&path)?;
    let is_dir = md.is_dir();
    let src = if is_dir { Source::Dir(path.clone()) } else { Source::Zip(path.clone()) };
    let entries = src.list()?;
    let size: u64 = if is_dir { entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum() } else { md.len() };
    let lay = layout(&entries);
    let meta = lay
        .mcmeta
        .as_deref()
        .and_then(|name| src.read(name, MAX_MCMETA_BYTES).ok().flatten())
        .and_then(|bytes| parse_pack_mcmeta(&String::from_utf8_lossy(&bytes)));
    let prefix = lay.nested_prefix.clone().unwrap_or_default();
    let scan = scan_cached(engine, stamp_of(&path, &md, size), &entries, &prefix);
    let sha1 = if with_hash && !is_dir { sha1_of(engine, &path).await } else { None };
    Ok(Inspected {
        filename: filename.to_string(),
        is_dir,
        size,
        sha1,
        meta,
        nested_prefix: lay.nested_prefix,
        has_icon: lay.icon.is_some(),
        scan,
    })
}

/// Placeholder row for a pack that could not be read at all.
fn unreadable_entry(dir: &Path, filename: &str) -> Option<ResourcePackEntry> {
    let md = std::fs::metadata(dir.join(filename)).ok()?;
    Some(base_entry(filename, md.is_dir(), if md.is_dir() { 0 } else { md.len() }, PackCompatStatus::InvalidMeta))
}

fn base_entry(filename: &str, is_dir: bool, size: u64, compat: PackCompatStatus) -> ResourcePackEntry {
    ResourcePackEntry {
        filename: filename.to_string(),
        option_id: format!("file/{filename}"),
        is_dir,
        size,
        sha1: None,
        display_name: display_name(filename),
        description: String::new(),
        has_icon: false,
        enabled: false,
        priority: None,
        compat,
        format_range: None,
        target_format: None,
        nested_prefix: None,
        project_id: None,
        version_id: None,
        version_number: None,
        overrides_count: 0,
        overridden_by_count: 0,
        overridden_by_names: Vec::new(),
        texture_tags: Vec::new(),
    }
}

fn display_name(filename: &str) -> String {
    if filename.to_ascii_lowercase().ends_with(".zip") {
        filename[..filename.len() - 4].to_string()
    } else {
        filename.to_string()
    }
}

fn has_zip_ext(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}

/// Entries of a pack folder that may be listed: folders and `.zip` files, never hidden or temp files.
fn list_pack_names(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.ends_with(".tmp") || name.ends_with(".part") {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() || !(ft.is_dir() || (ft.is_file() && has_zip_ext(&name))) {
            continue;
        }
        if safe_basename(&name).is_ok() {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Modrinth project, version id and version number of a file hash.
fn modrinth_ids(map: &IndexMap<String, Option<Value>>, sha1: Option<&String>) -> (Option<String>, Option<String>, Option<String>) {
    let Some(hash) = sha1 else { return (None, None, None) };
    match map.get(hash).and_then(|v| v.as_ref()) {
        Some(v) => (s(v, "project_id"), s(v, "id"), s(v, "version_number")),
        None => (None, None, None),
    }
}

/// SHA-1 of a pack file, memoized by path, mtime and size.
async fn sha1_of(engine: &Engine, path: &Path) -> Option<String> {
    let md = std::fs::metadata(path).ok()?;
    let stamp = stamp_of(path, &md, md.len());
    if let Some(found) = engine.content_state().pack_sha1.lock().unwrap_or_else(|p| p.into_inner()).get(&stamp) {
        return Some(found.clone());
    }
    let hash = crate::download::sha1_file(path).await.ok()?;
    let mut cache = engine.content_state().pack_sha1.lock().unwrap_or_else(|p| p.into_inner());
    if cache.len() >= MEMO_CAP {
        cache.clear();
    }
    cache.insert(stamp, hash.clone());
    Some(hash)
}

// ---------------------------------------------------------------------------
// options.txt and iris.properties
// ---------------------------------------------------------------------------

fn load_options(game: &Path) -> ParsedOptions {
    match std::fs::read(game.join("options.txt")) {
        Ok(bytes) => parse_options_txt(&String::from_utf8_lossy(&bytes)),
        Err(_) => parse_options_txt(""),
    }
}

/// A JSON string-array option (`resourcePacks`, `incompatibleResourcePacks`), when present and valid.
fn options_pack_list(parsed: &ParsedOptions, key: &str) -> Option<Vec<String>> {
    let value = parsed.entries.iter().find(|e| e.key.as_deref() == Some(key))?.value.as_deref()?;
    serde_json::from_str::<Vec<String>>(value).ok()
}

/// The resource-pack stack as options.txt holds it.
struct PackOrder {
    /// Built-in entries (`vanilla`, `fabric`, …) in their stored order.
    builtins: Vec<String>,
    /// Active pack basenames, highest priority first.
    active_top_first: Vec<String>,
}

fn read_order(parsed: &ParsedOptions) -> PackOrder {
    let list = options_pack_list(parsed, "resourcePacks").unwrap_or_else(|| vec!["vanilla".to_string()]);
    let builtins: Vec<String> = list.iter().filter(|e| !e.starts_with("file/")).cloned().collect();
    let mut active_top_first: Vec<String> = Vec::new();
    for entry in list.iter().rev() {
        if let Some(name) = entry.strip_prefix("file/") {
            if !active_top_first.iter().any(|n| n == name) {
                active_top_first.push(name.to_string());
            }
        }
    }
    PackOrder { builtins, active_top_first }
}

/// Rewrite `resourcePacks` (built-ins first, then the packs lowest priority first) and
/// `incompatibleResourcePacks`, then write options.txt atomically.
fn write_order(
    game: &Path,
    mut parsed: ParsedOptions,
    builtins: &[String],
    active_top_first: &[String],
    incompatible: &[String],
) -> EngineResult<()> {
    let mut list: Vec<String> = builtins.to_vec();
    list.extend(active_top_first.iter().rev().map(|f| format!("file/{f}")));
    set_option_key(&mut parsed, "resourcePacks", &to_json(&list));
    let incompatible: Vec<String> = incompatible.iter().map(|f| format!("file/{f}")).collect();
    set_option_key(&mut parsed, "incompatibleResourcePacks", &to_json(&incompatible));
    write_text_atomic(&game.join("options.txt"), &write_options_txt(&parsed))
}

fn to_json(list: &[String]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".to_string())
}

/// Packs among `names` that the target format cannot load (TooOld / TooNew).
async fn incompatible_packs(
    engine: &Engine,
    dir: &Path,
    names: &[String],
    target: Option<(u32, u32)>,
) -> EngineResult<Vec<String>> {
    let mut out = Vec::new();
    for name in names {
        let inspected = inspect(engine, dir, name, false).await?;
        if matches!(
            compat_of(inspected.meta.as_ref(), inspected.nested_prefix.is_some(), target),
            PackCompatStatus::TooOld | PackCompatStatus::TooNew
        ) {
            out.push(name.clone());
        }
    }
    Ok(out)
}

/// Put `filename` on top of the active stack (auto-activation after an install).
async fn activate_resource_pack(engine: &Engine, game: &Path, mc: &str, filename: &str) -> EngineResult<()> {
    let dir = game.join("resourcepacks");
    let parsed = load_options(game);
    let PackOrder { builtins, active_top_first } = read_order(&parsed);
    let mut top = vec![filename.to_string()];
    top.extend(
        active_top_first
            .into_iter()
            .filter(|f| f.as_str() != filename && safe_basename(f).is_ok() && dir.join(f).exists()),
    );
    let incompatible = incompatible_packs(engine, &dir, &top, target_resource_pack_format(engine, mc)).await?;
    write_order(game, parsed, &builtins, &top, &incompatible)
}

/// Current `shaderPack` / `enableShaders` values of iris.properties (last occurrence wins).
fn iris_value(game: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(crate::mods::iris_properties_path(game)).ok()?;
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') || line.starts_with('!') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
        .last()
}

/// Apply `edit` to iris.properties lines (blank trailing lines dropped) and write it atomically.
fn edit_iris_properties(game: &Path, edit: impl FnOnce(&mut Vec<String>)) -> EngineResult<()> {
    let file = crate::mods::iris_properties_path(game);
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .map(|t| t.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect())
        .unwrap_or_default();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    edit(&mut lines);
    write_text_atomic(&file, &format!("{}\n", lines.join("\n")))
}

fn select_shader(game: &Path, pack: &str) -> EngineResult<()> {
    edit_iris_properties(game, |lines| {
        set_property(lines, "shaderPack", pack);
        set_property(lines, "enableShaders", "true");
    })
}

/// Whether an enabled Iris mod jar is in `<game>/mods/`.
fn iris_installed(game: &Path) -> bool {
    crate::fabric_compat::scan_mods_dir(&game.join("mods")).iter().any(|j| j.enabled && j.root.id == "iris")
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// A file name every OS stores as-is, with no path parts. Used for every name that arrives
/// from a request, a remote API or an archive.
pub(crate) fn safe_basename(name: &str) -> EngineResult<String> {
    let bad = name.is_empty()
        || name.len() > 200
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('.')
        || !crate::mrpack::is_portable_segment(name);
    if bad {
        return Err(EngineError::bad_request("invalid_filename", format!("unsafe pack filename: {name}")));
    }
    Ok(name.to_string())
}

fn check_modrinth_id(id: &str) -> EngineResult<()> {
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(EngineError::bad_request("invalid_project_id", format!("not a Modrinth id: {id}")));
    }
    Ok(())
}

/// `name`, or `name-2`, `name-3`, … when that path is taken.
fn unique_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2..10_000 {
        let candidate = format!("{stem}-{n}{ext}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem}-{}{ext}", std::process::id())
}

/// Write `text` beside `file`, then rename it over `file`.
fn write_text_atomic(file: &Path, text: &str) -> EngineResult<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = PathBuf::from(format!("{}.{}.tmp", file.to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, text)?;
    if let Err(e) = std::fs::rename(&tmp, file) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn remove_path(path: &Path) -> EngineResult<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Recursive copy of a pack folder (symlinks are skipped, never followed).
pub(crate) fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if ft.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn dir_usage(dir: &Path, depth: usize) -> (usize, u64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return (0, 0) };
    let (mut files, mut bytes) = (0usize, 0u64);
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() && depth < 16 {
            let (f, b) = dir_usage(&entry.path(), depth + 1);
            files += f;
            bytes += b;
        } else if ft.is_file() {
            files += 1;
            bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (files, bytes)
}

fn ensure_not_running(engine: &Engine, name: &str) -> EngineResult<()> {
    if engine.active_instances().contains(name) {
        return Err(EngineError::conflict(
            "ALREADY_RUNNING",
            format!("instance '{name}' is running; stop it before changing its packs"),
        ));
    }
    Ok(())
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

// ---------------------------------------------------------------------------
// Modrinth
// ---------------------------------------------------------------------------

fn api_base(engine: &Engine) -> String {
    engine.content_settings().modrinth_api.trim_end_matches('/').to_string()
}

async fn fetch_json(engine: &Engine, url: &str) -> EngineResult<Value> {
    let opts = FetchOptions { timeout: Duration::from_secs(30), ..Default::default() };
    engine.http().fetch_json::<Value>(url, &opts).await
}

fn primary_file(meta: &Value) -> Option<&Value> {
    let files = meta.get("files")?.as_array()?;
    files.iter().find(|f| f.get("primary") == Some(&Value::Bool(true))).or_else(|| files.first())
}

fn file_sha1(file: &Value) -> Option<String> {
    file.get("hashes").and_then(|h| s(h, "sha1"))
}

/// Newest listed build of a pack project for `mc` on `loader`.
async fn best_pack_version(engine: &Engine, project_id: &str, mc: &str, loader: &str) -> EngineResult<Value> {
    let query = format!(
        "game_versions={}&loaders={}",
        encode_uri_component(&json!([mc]).to_string()),
        encode_uri_component(&json!([loader]).to_string())
    );
    let url = format!("{}/project/{}/version?{query}", api_base(engine), encode_uri_component(project_id));
    let list = fetch_json(engine, &url).await?;
    list.as_array()
        .and_then(|a| {
            a.iter()
                .find(|v| {
                    matches!(v.get("status").and_then(Value::as_str), Some("listed") | Some("approved") | None)
                        && primary_file(v).is_some()
                })
                .cloned()
        })
        .ok_or_else(|| {
            EngineError::not_found("PACK_VERSION_NOT_FOUND", format!("no {loader} build of {project_id} for {mc}"))
        })
}

/// `POST /version_files/update`: the newest build per installed file hash for `mc` and `loader`.
async fn pack_update_lookup(
    engine: &Engine,
    hashes: &[String],
    loader: &str,
    mc: &str,
) -> EngineResult<HashMap<String, Value>> {
    let url = format!("{}/version_files/update", api_base(engine));
    let body = json!({ "hashes": hashes, "algorithm": "sha1", "loaders": [loader], "game_versions": [mc] });
    let res = engine
        .http()
        .client()
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| EngineError::api(502, "MODRINTH_UPDATE_FAILED", e.to_string()))?;
    if !res.status().is_success() {
        return Err(EngineError::api(502, "MODRINTH_UPDATE_FAILED", format!("Modrinth answered {}", res.status())));
    }
    let map: Map<String, Value> = match res.json::<Value>().await {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    Ok(map.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Public API: listing
// ---------------------------------------------------------------------------

/// Every resource pack of an instance: enabled ones first (by priority), then the rest by name.
pub async fn list_resource_packs(engine: &Engine, name: &str) -> EngineResult<Vec<ResourcePackEntry>> {
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    let dir = pack_dir(&game, PackKind::ResourcePack);
    let target = target_resource_pack_format(engine, &inst.version);
    let order = read_order(&load_options(&game));

    let names = list_pack_names(&dir);
    let mut rows: Vec<Inspected> = Vec::new();
    let mut out: Vec<ResourcePackEntry> = Vec::new();
    for name in &names {
        match inspect(engine, &dir, name, true).await {
            Ok(row) => rows.push(row),
            Err(e) => {
                tracing::warn!("[packs] {name} unreadable: {e}");
                out.extend(unreadable_entry(&dir, name));
            }
        }
    }

    let present: HashSet<&str> = rows.iter().map(|r| r.filename.as_str()).collect();
    let active: Vec<String> = order.active_top_first.iter().filter(|f| present.contains(f.as_str())).cloned().collect();
    let stacked: Vec<(String, Arc<PackScan>)> = active
        .iter()
        .rev()
        .filter_map(|f| rows.iter().find(|r| &r.filename == f).map(|r| (r.filename.clone(), r.scan.clone())))
        .collect();
    let overlaps: HashMap<String, Overlap> = compute_overlaps(&stacked).into_iter().collect();

    let hashes: Vec<String> = rows.iter().filter_map(|r| r.sha1.clone()).collect();
    let ids = cached_version_files(engine, &hashes).await;

    for row in &rows {
        let priority = active.iter().position(|f| f == &row.filename);
        let (project_id, version_id, version_number) = modrinth_ids(&ids, row.sha1.as_ref());
        let overlap = overlaps.get(&row.filename).cloned().unwrap_or_default();
        let compat = compat_of(row.meta.as_ref(), row.nested_prefix.is_some(), target);
        out.push(ResourcePackEntry {
            filename: row.filename.clone(),
            option_id: format!("file/{}", row.filename),
            is_dir: row.is_dir,
            size: row.size,
            sha1: row.sha1.clone(),
            display_name: display_name(&row.filename),
            description: row.meta.as_ref().map(|m| m.description.clone()).unwrap_or_default(),
            has_icon: row.has_icon,
            enabled: priority.is_some(),
            priority,
            compat,
            format_range: row.meta.as_ref().map(|m| m.range.clone()),
            target_format: target,
            nested_prefix: row.nested_prefix.clone(),
            project_id,
            version_id,
            version_number,
            overrides_count: overlap.overrides,
            overridden_by_count: overlap.overridden_by,
            overridden_by_names: overlap.names,
            texture_tags: row.scan.tags.clone(),
        });
    }
    out.sort_by(|a, b| {
        (a.priority.is_none(), a.priority, a.filename.as_str())
            .cmp(&(b.priority.is_none(), b.priority, b.filename.as_str()))
    });
    Ok(out)
}

/// Iris state and the shader packs of an instance (Modrinth ids via the batched SHA-1 lookup).
pub async fn shaders_state(engine: &Engine, name: &str) -> EngineResult<ShadersState> {
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    let dir = pack_dir(&game, PackKind::Shader);
    let shader_pack = iris_value(&game, "shaderPack")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("off"));
    let enabled = iris_value(&game, "enableShaders").is_some_and(|v| v.eq_ignore_ascii_case("true"));

    let mut rows: Vec<Inspected> = Vec::new();
    for name in list_pack_names(&dir) {
        match inspect(engine, &dir, &name, true).await {
            Ok(row) => rows.push(row),
            Err(e) => tracing::warn!("[packs] shader {name} unreadable: {e}"),
        }
    }
    let hashes: Vec<String> = rows.iter().filter_map(|r| r.sha1.clone()).collect();
    let ids = cached_version_files(engine, &hashes).await;
    let packs = rows
        .iter()
        .map(|row| {
            let (project_id, version_id, version_number) = modrinth_ids(&ids, row.sha1.as_ref());
            ShaderPackEntry {
                filename: row.filename.clone(),
                is_dir: row.is_dir,
                size: row.size,
                sha1: row.sha1.clone(),
                active: enabled && shader_pack.as_deref() == Some(row.filename.as_str()),
                project_id,
                version_id,
                version_number,
            }
        })
        .collect();
    Ok(ShadersState { iris_installed: iris_installed(&game), shaders_enabled: enabled, active_pack: shader_pack, packs })
}

/// `pack.png` of a resource pack (root or nested), at most 2 MiB; None when absent.
pub async fn resource_pack_icon(engine: &Engine, name: &str, filename: &str) -> EngineResult<Option<Vec<u8>>> {
    let inst = get_instance(engine, name).await?;
    let fname = safe_basename(filename)?;
    let path = pack_dir(&effective_game_dir(engine, &inst), PackKind::ResourcePack).join(&fname);
    if !path.exists() {
        return Err(EngineError::not_found("PACK_NOT_FOUND", format!("resource pack not found: {fname}")));
    }
    let src = if path.is_dir() { Source::Dir(path) } else { Source::Zip(path) };
    let entries = src.list()?;
    match layout(&entries).icon {
        Some(icon) => src.read(&icon, MAX_ICON_BYTES),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Public API: order, nested archives, install, delete
// ---------------------------------------------------------------------------

/// Set the active resource-pack stack (`active_top_first`: highest priority first). Packs whose
/// format the instance cannot load go to `incompatibleResourcePacks`, as the game itself writes it.
pub async fn set_resource_packs_order(engine: &Engine, name: &str, active_top_first: &[String]) -> EngineResult<()> {
    ensure_not_running(engine, name)?;
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    let dir = pack_dir(&game, PackKind::ResourcePack);
    let mut seen: HashSet<&str> = HashSet::new();
    for f in active_top_first {
        safe_basename(f)?;
        if !seen.insert(f.as_str()) {
            return Err(EngineError::bad_request("DUPLICATE_PACK", format!("{f} is listed twice")));
        }
        if !dir.join(f).exists() {
            return Err(EngineError::not_found("PACK_NOT_FOUND", format!("resource pack not found: {f}")));
        }
    }
    let incompatible =
        incompatible_packs(engine, &dir, active_top_first, target_resource_pack_format(engine, &inst.version)).await?;
    let parsed = load_options(&game);
    let builtins = read_order(&parsed).builtins;
    write_order(&game, parsed, &builtins, active_top_first, &incompatible)
}

/// Rewrite a nested resource pack (`Pack/pack.mcmeta`) as a root-level one. The original is kept
/// under `<instance>/backups/nested-<filename>`; any entry that would escape the pack root aborts
/// the whole unwrap and leaves the original untouched.
pub async fn unwrap_nested_resource_pack(engine: &Engine, name: &str, filename: &str) -> EngineResult<PackUnwrapResult> {
    ensure_not_running(engine, name)?;
    let inst = get_instance(engine, name).await?;
    let dir = pack_dir(&effective_game_dir(engine, &inst), PackKind::ResourcePack);
    let fname = safe_basename(filename)?;
    let path = dir.join(&fname);
    if !path.is_file() {
        return Err(EngineError::not_found("PACK_NOT_FOUND", format!("resource pack not found: {fname}")));
    }
    let entries = list_zip(&path)?;
    let Some(prefix) = layout(&entries).nested_prefix else {
        return Err(EngineError::bad_request("NOT_NESTED", format!("{fname} has no pack.mcmeta one folder deep")));
    };
    let inside = entries.iter().filter(|e| e.name.starts_with(&prefix)).count();
    if inside > MAX_PACK_ENTRIES {
        return Err(too_many_entries());
    }
    let declared: u64 = entries.iter().filter(|e| e.name.starts_with(&prefix)).map(|e| e.size).sum();
    if declared > MAX_UNWRAP_BYTES {
        return Err(EngineError::bad_request("PACK_TOO_LARGE", "the nested pack expands past 2 GiB"));
    }

    let tmp = dir.join(format!("{fname}.unwrap.{}.tmp", std::process::id()));
    let written = match write_unwrapped(&path, &prefix, &tmp) {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    let verified = list_zip(&tmp)
        .map(|entries| {
            let after = layout(&entries);
            after.mcmeta.is_some() && after.nested_prefix.is_none()
        })
        .unwrap_or(false);
    if !verified {
        let _ = std::fs::remove_file(&tmp);
        return Err(EngineError::other("unwrapped archive has no root pack.mcmeta"));
    }

    let backup_dir = engine.paths().instance_dir(name).join("backups");
    std::fs::create_dir_all(&backup_dir)?;
    let backup = backup_dir.join(unique_name(&backup_dir, &format!("nested-{fname}")));
    if let Err(e) = std::fs::copy(&path, &backup) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(PackUnwrapResult { filename: fname, backup: backup.to_string_lossy().into_owned(), entries: written })
}

/// Copy the entries under `prefix` of `src` into a new zip at `dst`, prefix stripped, every path
/// checked with `safe_join` and every byte counted against the unwrap cap.
fn write_unwrapped(src: &Path, prefix: &str, dst: &Path) -> EngineResult<usize> {
    let mut archive = open_zip(src)?;
    let mut writer = zip::ZipWriter::new(std::fs::File::create(dst)?);
    let root = Path::new("pack-root");
    let mut total: u64 = 0;
    let mut files = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        let name = entry.name().map_err(zip_err)?.into_owned();
        let Some(rel) = name.strip_prefix(prefix) else { continue };
        if entry.is_dir() || rel.is_empty() {
            continue;
        }
        let joined = crate::mrpack::safe_join(root, &rel.replace('\\', "/"))?;
        let rel_out = joined.strip_prefix(root).map_err(|_| EngineError::other("unsafe path in nested pack"))?;
        let out_name =
            rel_out.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        files += 1;
        if files > MAX_PACK_ENTRIES {
            return Err(too_many_entries());
        }
        writer.start_file(out_name, zip::write::SimpleFileOptions::default()).map_err(zip_err)?;
        let remaining = MAX_UNWRAP_BYTES - total;
        let copied = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut writer)?;
        if copied > remaining {
            return Err(EngineError::bad_request("PACK_TOO_LARGE", "the nested pack expands past 2 GiB"));
        }
        total += copied;
    }
    writer.finish().map_err(zip_err)?;
    Ok(files)
}

/// Install a Modrinth resource pack or shader (SHA-1 verified) and optionally activate it.
/// A running instance keeps the file but skips activation (`note: "running"`).
pub async fn install_modrinth_pack(
    engine: &Engine,
    name: &str,
    project_id: &str,
    version_id: Option<&str>,
    kind: PackKind,
    auto_activate: bool,
) -> EngineResult<PackInstallResult> {
    check_modrinth_id(project_id)?;
    if let Some(v) = version_id {
        check_modrinth_id(v)?;
    }
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    let loader = match kind {
        PackKind::ResourcePack => RESOURCE_PACK_LOADER,
        PackKind::Shader => SHADER_LOADER,
    };
    let meta = match version_id {
        Some(v) => fetch_json(engine, &format!("{}/version/{}", api_base(engine), encode_uri_component(v))).await?,
        None => best_pack_version(engine, project_id, &inst.version, loader).await?,
    };
    if s(&meta, "project_id").is_some_and(|p| p != project_id) {
        return Err(EngineError::bad_request("PACK_VERSION_MISMATCH", "that version belongs to another project"));
    }
    let file = primary_file(&meta).ok_or_else(|| EngineError::not_found("PACK_FILE_NOT_FOUND", "no downloadable file"))?;
    let fname = safe_basename(&s(file, "filename").unwrap_or_default())?;
    let url = s(file, "url").filter(|u| !u.is_empty()).ok_or_else(|| EngineError::other("pack file has no URL"))?;
    let sha1 = file_sha1(file).ok_or_else(|| EngineError::api(502, "PACK_NO_SHA1", "Modrinth gave no SHA-1 for the pack"))?;
    let size = file.get("size").and_then(Value::as_u64);

    let dir = pack_dir(&game, kind);
    std::fs::create_dir_all(&dir)?;
    let existing = dir.join(&fname);
    let already = existing.is_file() && sha1_of(engine, &existing).await.as_deref() == Some(sha1.as_str());
    let stored = if already {
        fname
    } else {
        let dest_name = unique_name(&dir, &fname);
        let opts = DownloadOptions { sha1: Some(sha1), size, ..Default::default() };
        engine.http().download_file(&url, &dir.join(&dest_name), &opts).await?;
        dest_name
    };

    let running = engine.active_instances().contains(name);
    let mut activated = false;
    let mut note = None;
    let needs_iris = kind == PackKind::Shader && !iris_installed(&game);
    if auto_activate {
        if running {
            note = Some("running".to_string());
        } else {
            match kind {
                PackKind::ResourcePack => {
                    activate_resource_pack(engine, &game, &inst.version, &stored).await?;
                }
                PackKind::Shader => select_shader(&game, &stored)?,
            }
            activated = true;
        }
    }
    Ok(PackInstallResult { filename: stored, kind, activated, needs_iris, nested: false, note })
}

/// Switch the active shader (`Some`) or turn shaders off (`None`); returns the new state.
pub async fn set_active_shader(engine: &Engine, name: &str, filename: Option<&str>) -> EngineResult<ShadersState> {
    ensure_not_running(engine, name)?;
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    match filename {
        Some(f) => {
            let fname = safe_basename(f)?;
            if !pack_dir(&game, PackKind::Shader).join(&fname).exists() {
                return Err(EngineError::not_found("PACK_NOT_FOUND", format!("shader pack not found: {fname}")));
            }
            select_shader(&game, &fname)?;
        }
        None => edit_iris_properties(&game, |lines| set_property(lines, "enableShaders", "false"))?,
    }
    shaders_state(engine, name).await
}

/// Delete a resource pack and drop its `file/<name>` entries from options.txt.
pub async fn delete_resource_pack(engine: &Engine, name: &str, filename: &str) -> EngineResult<String> {
    ensure_not_running(engine, name)?;
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    let fname = safe_basename(filename)?;
    let path = pack_dir(&game, PackKind::ResourcePack).join(&fname);
    if !path.exists() {
        return Err(EngineError::not_found("PACK_NOT_FOUND", format!("resource pack not found: {fname}")));
    }
    remove_path(&path)?;
    if game.join("options.txt").is_file() {
        let mut parsed = load_options(&game);
        let target = format!("file/{fname}");
        let mut changed = false;
        for key in ["resourcePacks", "incompatibleResourcePacks"] {
            if let Some(list) = options_pack_list(&parsed, key) {
                if list.contains(&target) {
                    let kept: Vec<String> = list.into_iter().filter(|e| *e != target).collect();
                    set_option_key(&mut parsed, key, &to_json(&kept));
                    changed = true;
                }
            }
        }
        if changed {
            write_text_atomic(&game.join("options.txt"), &write_options_txt(&parsed))?;
        }
    }
    Ok(fname)
}

/// Delete a shader pack; if it was the active one, iris.properties is reset (no shader, shaders off).
pub async fn delete_shader_pack(engine: &Engine, name: &str, filename: &str) -> EngineResult<String> {
    ensure_not_running(engine, name)?;
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    let fname = safe_basename(filename)?;
    let path = pack_dir(&game, PackKind::Shader).join(&fname);
    if !path.exists() {
        return Err(EngineError::not_found("PACK_NOT_FOUND", format!("shader pack not found: {fname}")));
    }
    let was_active = iris_value(&game, "shaderPack").as_deref() == Some(fname.as_str());
    remove_path(&path)?;
    if was_active {
        edit_iris_properties(&game, |lines| {
            set_property(lines, "shaderPack", "");
            set_property(lines, "enableShaders", "false");
        })?;
    }
    Ok(fname)
}

/// Newer Modrinth builds of the installed packs of one kind (one batched request).
pub async fn check_pack_updates(engine: &Engine, name: &str, kind: PackKind) -> EngineResult<Vec<PackUpdate>> {
    let inst = get_instance(engine, name).await?;
    let dir = pack_dir(&effective_game_dir(engine, &inst), kind);
    let mut by_hash: HashMap<String, String> = HashMap::new();
    for file in list_pack_names(&dir) {
        let path = dir.join(&file);
        if !path.is_file() {
            continue;
        }
        if let Some(hash) = sha1_of(engine, &path).await {
            by_hash.insert(hash, file);
        }
    }
    if by_hash.is_empty() {
        return Ok(Vec::new());
    }
    let hashes: Vec<String> = by_hash.keys().cloned().collect();
    let loader = match kind {
        PackKind::ResourcePack => RESOURCE_PACK_LOADER,
        PackKind::Shader => SHADER_LOADER,
    };
    let hits = pack_update_lookup(engine, &hashes, loader, &inst.version).await?;
    let current = cached_version_files(engine, &hashes).await;
    let mut out = Vec::new();
    for (hash, filename) in &by_hash {
        let Some(to) = hits.get(hash) else { continue };
        let to_sha1 = primary_file(to).and_then(file_sha1);
        if to_sha1.as_deref() == Some(hash.as_str()) {
            continue;
        }
        let installed = current.get(hash).and_then(|v| v.as_ref());
        out.push(PackUpdate {
            filename: filename.clone(),
            project_id: installed.and_then(|v| s(v, "project_id")),
            version_id: installed.and_then(|v| s(v, "id")),
            to_version_id: s(to, "id").unwrap_or_default(),
            to_version_number: s(to, "version_number"),
            to_filename: primary_file(to).and_then(|f| s(f, "filename")),
        });
    }
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(out)
}

/// Copy packs from `source` into `target` (identical files are skipped). With `preserve_order`,
/// packs the source had active go on top of the target's stack, in the source's order.
pub async fn copy_resource_packs_from_instance(
    engine: &Engine,
    target: &str,
    source: &str,
    filenames: &[String],
    preserve_order: bool,
) -> EngineResult<PackCopyResult> {
    if target == source {
        return Err(EngineError::bad_request("SAME_INSTANCE", "source and target are the same instance"));
    }
    ensure_not_running(engine, target)?;
    let tinst = get_instance(engine, target).await?;
    let sinst = get_instance(engine, source).await?;
    let tgame = effective_game_dir(engine, &tinst);
    let sgame = effective_game_dir(engine, &sinst);
    let sdir = pack_dir(&sgame, PackKind::ResourcePack);
    let tdir = pack_dir(&tgame, PackKind::ResourcePack);

    let mut selected: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for f in filenames {
        let fname = safe_basename(f)?;
        if !seen.insert(fname.clone()) {
            continue;
        }
        let path = sdir.join(&fname);
        if !path.exists() {
            return Err(EngineError::not_found("PACK_NOT_FOUND", format!("resource pack not found: {fname}")));
        }
        selected.push((fname, path));
    }

    let mut copied = Vec::new();
    let mut skipped = Vec::new();
    let mut mapping: HashMap<String, String> = HashMap::new();
    for (fname, src) in &selected {
        let dst = tdir.join(fname);
        if dst.exists() && src.is_file() && dst.is_file() {
            let same = match (sha1_of(engine, src).await, sha1_of(engine, &dst).await) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };
            if same {
                skipped.push(fname.clone());
                mapping.insert(fname.clone(), fname.clone());
                continue;
            }
        }
        std::fs::create_dir_all(&tdir)?;
        let dest_name = unique_name(&tdir, fname);
        let dest = tdir.join(&dest_name);
        if src.is_dir() {
            copy_tree(src, &dest)?;
        } else {
            std::fs::copy(src, &dest)?;
        }
        mapping.insert(fname.clone(), dest_name.clone());
        copied.push(dest_name);
    }

    if preserve_order {
        let mut new_top: Vec<String> = Vec::new();
        for f in read_order(&load_options(&sgame)).active_top_first {
            if let Some(t) = mapping.get(&f) {
                if !new_top.contains(t) {
                    new_top.push(t.clone());
                }
            }
        }
        if !new_top.is_empty() {
            let tparsed = load_options(&tgame);
            let PackOrder { builtins, active_top_first } = read_order(&tparsed);
            let mut top = new_top;
            for f in active_top_first {
                if !top.contains(&f) && tdir.join(&f).exists() {
                    top.push(f);
                }
            }
            let incompatible =
                incompatible_packs(engine, &tdir, &top, target_resource_pack_format(engine, &tinst.version)).await?;
            write_order(&tgame, tparsed, &builtins, &top, &incompatible)?;
        }
    }
    Ok(PackCopyResult { copied, skipped })
}

/// Classify and copy a local `.zip` into `resourcepacks/` or `shaderpacks/`. Modpacks and
/// unrecognized archives are refused with a code the UI can explain.
pub async fn install_local_pack(engine: &Engine, name: &str, path: &Path) -> EngineResult<PackInstallResult> {
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    if !path.is_file() || !has_zip_ext(&path.to_string_lossy()) {
        return Err(EngineError::bad_request("PACK_NOT_A_ZIP", "only .zip packs can be installed from a file"));
    }
    let base = safe_basename(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
    let (kind, nested) = match classify_zip_pack(path)? {
        ZipPackKind::ResourcePack { nested_prefix } => (PackKind::ResourcePack, nested_prefix.is_some()),
        ZipPackKind::ShaderPack => (PackKind::Shader, false),
        ZipPackKind::Mrpack => {
            return Err(EngineError::bad_request("MRPACK_IMPORT", "modpack archives are imported as instances"))
        }
        ZipPackKind::Unknown => {
            return Err(EngineError::bad_request("UNRECOGNIZED_PACK", "the archive has no pack.mcmeta or shader files"))
        }
    };
    let dir = pack_dir(&game, kind);
    std::fs::create_dir_all(&dir)?;
    let stored = unique_name(&dir, &base);
    let tmp = dir.join(format!("{stored}.{}.tmp", std::process::id()));
    std::fs::copy(path, &tmp)?;
    if let Err(e) = std::fs::rename(&tmp, dir.join(&stored)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(PackInstallResult {
        filename: stored,
        kind,
        activated: false,
        needs_iris: kind == PackKind::Shader && !iris_installed(&game),
        nested,
        note: nested.then(|| "nested_zip".to_string()),
    })
}

/// Size of `<game dir>/downloads/` (the server resource-pack cache).
pub async fn server_pack_cache_info(engine: &Engine, name: &str) -> EngineResult<ServerPackCacheInfo> {
    let inst = get_instance(engine, name).await?;
    let (file_count, total_bytes) = dir_usage(&effective_game_dir(engine, &inst).join("downloads"), 0);
    Ok(ServerPackCacheInfo { file_count, total_bytes })
}

/// Empty `<game dir>/downloads/`; returns what was there.
pub async fn clear_server_pack_cache(engine: &Engine, name: &str) -> EngineResult<ServerPackCacheInfo> {
    ensure_not_running(engine, name)?;
    let inst = get_instance(engine, name).await?;
    let game = effective_game_dir(engine, &inst);
    let info = server_pack_cache_info(engine, name).await?;
    let dir = game.join("downloads");
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(info)
}

/// Active resource packs that the target version cannot load, with a newer Modrinth build for
/// that version when one exists. Packs that cannot be read are skipped; the Modrinth lookup
/// is best-effort.
pub async fn version_change_packs(engine: &Engine, inst: &InstanceMeta, target_version: &str) -> Vec<VersionChangePack> {
    let game = effective_game_dir(engine, inst);
    let dir = pack_dir(&game, PackKind::ResourcePack);
    let order = read_order(&load_options(&game));
    let target = target_resource_pack_format(engine, target_version);
    let mut found: Vec<(String, Option<String>, PackCompatStatus)> = Vec::new();
    for filename in order.active_top_first {
        if safe_basename(&filename).is_err() || !dir.join(&filename).exists() {
            continue;
        }
        let row = match inspect(engine, &dir, &filename, true).await {
            Ok(row) => row,
            Err(e) => {
                tracing::warn!("[packs] version change skips {filename}: {e}");
                continue;
            }
        };
        let compat = compat_of(row.meta.as_ref(), row.nested_prefix.is_some(), target);
        if matches!(compat, PackCompatStatus::TooOld | PackCompatStatus::TooNew) {
            found.push((filename, row.sha1, compat));
        }
    }
    if found.is_empty() {
        return Vec::new();
    }
    let hashes: Vec<String> = found.iter().filter_map(|(_, sha1, _)| sha1.clone()).collect();
    let hits = if hashes.is_empty() {
        HashMap::new()
    } else {
        pack_update_lookup(engine, &hashes, RESOURCE_PACK_LOADER, target_version).await.unwrap_or_default()
    };
    found
        .into_iter()
        .map(|(filename, sha1, compat)| {
            let update = sha1.as_deref().and_then(|h| hits.get(h)).filter(|to| {
                primary_file(to).and_then(file_sha1).as_deref() != sha1.as_deref()
            });
            VersionChangePack {
                display_name: display_name(&filename),
                filename,
                compat,
                update_version_id: update.and_then(|v| s(v, "id")),
                update_version_number: update.and_then(|v| s(v, "version_number")),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use std::io::Write;

    const PACK_34: &[u8] = br#"{"pack":{"pack_format":34,"description":"Test pack"}}"#;

    /// An engine with `instance` at `version`, Modrinth pointed at a closed port (no network).
    fn engine_for(instance: &str, version: &str) -> (tempfile::TempDir, Engine) {
        let (d, e) = temp_engine();
        e.update_content_settings(|s| {
            s.modrinth_api = "http://127.0.0.1:9/v2".into();
            s.modrinth_retry_backoff = Duration::from_millis(1);
        });
        crate::paths::write_json_atomic(
            &e.paths().instance_json(instance),
            &json!({ "name": instance, "version": version, "loader": "vanilla" }),
        )
        .unwrap();
        (d, e)
    }

    fn game_dir(e: &Engine, instance: &str) -> PathBuf {
        e.paths().instance_dir(instance)
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn parses_legacy_supported_formats_and_26x_min_max_format() {
        let range = |raw: &str| parse_pack_mcmeta(raw).map(|m| m.range);
        let legacy = |lo: u32, hi: u32| Some(span(lo, hi));
        assert_eq!(range(r#"{"pack":{"pack_format":15,"description":"x"}}"#), legacy(15, 15));
        assert_eq!(range(r#"{"pack":{"supported_formats":[15,18]}}"#), legacy(15, 18));
        assert_eq!(range(r#"{"pack":{"supported_formats":{"min_inclusive":22,"max_inclusive":34}}}"#), legacy(22, 34));
        assert_eq!(range(r#"{"pack":{"supported_formats":34}}"#), legacy(34, 34));
        assert_eq!(
            range("\u{feff}{\"pack\":{\"min_format\":[88,0],\"max_format\":[88,1]}}"),
            Some(PackFormatRange { min: (88, 0), max: (88, 1) })
        );
        assert_eq!(
            range(r#"{"pack":{"min_format":69,"max_format":75}}"#),
            Some(PackFormatRange { min: (69, 0), max: (75, 0) })
        );
        assert_eq!(range(r#"{"pack":{"description":"no format"}}"#), None);
        assert_eq!(range("not json"), None);

        let fixed = PackFormatRange { min: (46, 0), max: (88, u32::MAX) };
        assert_eq!(compare((34, 0), &fixed), PackCompatStatus::TooNew);
        assert_eq!(compare((88, 0), &legacy(15, 18).unwrap()), PackCompatStatus::TooOld);
        assert_eq!(compare((46, 0), &fixed), PackCompatStatus::Compatible);
    }

    #[test]
    fn target_format_from_client_jar_then_table() {
        let (_d, e) = temp_engine();
        assert_eq!(target_resource_pack_format(&e, "1.8.9"), Some((1, 0)));
        assert_eq!(target_resource_pack_format(&e, "1.21.11"), Some((75, 0)));
        assert_eq!(target_resource_pack_format(&e, "26.2"), Some((88, 0)));
        assert_eq!(target_resource_pack_format(&e, "26.3"), None, "26.3 needs its client jar");
        // A downloaded jar is the source of truth, in both shapes.
        write_zip(
            &e.paths().client_jar("26.3"),
            &[("version.json", br#"{"pack_version":{"resource_major":90,"resource_minor":1}}"#)],
        );
        assert_eq!(target_resource_pack_format(&e, "26.3"), Some((90, 1)));
        write_zip(&e.paths().client_jar("1.8.9"), &[("version.json", br#"{"pack_version":{"resource":99}}"#)]);
        assert_eq!(target_resource_pack_format(&e, "1.8.9"), Some((99, 0)));
    }

    #[test]
    fn flattens_section_codes_and_json_text_components() {
        assert_eq!(flatten_text(&json!("§aGreen §lBold §r")), "Green Bold ");
        assert_eq!(flatten_text(&json!({ "text": "Hi ", "extra": [{ "text": "§cthere" }, { "translate": "x" }] })), "Hi therex");
        assert_eq!(flatten_text(&json!(["a", { "text": "b", "color": "red" }, "§4§n!"])), "ab!");
        assert_eq!(flatten_text(&json!("§zkeep")), "§zkeep", "§z is not a format code");
        let meta = parse_pack_mcmeta(
            r#"{"pack":{"pack_format":34,"description":[{"text":"§9Mi ","extra":[{"text":"pack"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(meta.description, "Mi pack");
    }

    #[tokio::test]
    async fn detects_and_unwraps_nested_zip_safely() {
        let (_d, e) = engine_for("nested", "1.21.1");
        let rp = game_dir(&e, "nested").join("resourcepacks");
        write_zip(
            &rp.join("Outer.zip"),
            &[
                ("Pack/pack.mcmeta", PACK_34),
                ("Pack/assets/minecraft/textures/block/stone.png", b"png"),
                ("Pack/pack.png", b"icon"),
            ],
        );
        let packs = list_resource_packs(&e, "nested").await.unwrap();
        assert_eq!(packs[0].compat, PackCompatStatus::NestedArchive);
        assert_eq!(packs[0].nested_prefix.as_deref(), Some("Pack/"));
        assert!(packs[0].has_icon);
        assert_eq!(packs[0].texture_tags, vec!["blocks".to_string()]);
        assert_eq!(
            classify_zip_pack(&rp.join("Outer.zip")).unwrap(),
            ZipPackKind::ResourcePack { nested_prefix: Some("Pack/".into()) }
        );
        assert_eq!(resource_pack_icon(&e, "nested", "Outer.zip").await.unwrap(), Some(b"icon".to_vec()));

        e.active_instances().insert("nested");
        assert!(unwrap_nested_resource_pack(&e, "nested", "Outer.zip").await.unwrap_err().has_code("ALREADY_RUNNING"));
        e.active_instances().remove("nested");

        let res = unwrap_nested_resource_pack(&e, "nested", "Outer.zip").await.unwrap();
        assert_eq!(res.filename, "Outer.zip");
        assert!(Path::new(&res.backup).is_file(), "original kept in backups/");
        assert_eq!(classify_zip_pack(&rp.join("Outer.zip")).unwrap(), ZipPackKind::ResourcePack { nested_prefix: None });

        // An entry that escapes the pack root aborts the unwrap; the original stays as it was.
        write_zip(&rp.join("Evil.zip"), &[("Pack/pack.mcmeta", PACK_34), ("Pack/../escape.txt", b"x")]);
        let before = std::fs::read(rp.join("Evil.zip")).unwrap();
        assert!(unwrap_nested_resource_pack(&e, "nested", "Evil.zip").await.is_err());
        assert_eq!(std::fs::read(rp.join("Evil.zip")).unwrap(), before);
    }

    #[tokio::test]
    async fn options_txt_roundtrip_preserves_vanilla_and_inverts_priority() {
        let (_d, e) = engine_for("opts", "1.21.1");
        let game = game_dir(&e, "opts");
        let rp = game.join("resourcepacks");
        for n in ["Base.zip", "Top.zip"] {
            write_zip(&rp.join(n), &[("pack.mcmeta", PACK_34)]);
        }
        std::fs::write(
            game.join("options.txt"),
            "version:3700\nfov:0.5\nresourcePacks:[\"vanilla\",\"file/Base.zip\",\"file/Top.zip\"]\n",
        )
        .unwrap();
        let packs = list_resource_packs(&e, "opts").await.unwrap();
        let top = packs.iter().find(|p| p.filename == "Top.zip").unwrap();
        let base = packs.iter().find(|p| p.filename == "Base.zip").unwrap();
        assert_eq!((top.priority, top.enabled), (Some(0), true));
        assert_eq!(base.priority, Some(1));

        set_resource_packs_order(&e, "opts", &["Base.zip".into(), "Top.zip".into()]).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(game.join("options.txt")).unwrap(),
            "version:3700\nfov:0.5\nresourcePacks:[\"vanilla\",\"file/Top.zip\",\"file/Base.zip\"]\nincompatibleResourcePacks:[]\n"
        );

        e.active_instances().insert("opts");
        assert!(set_resource_packs_order(&e, "opts", &[]).await.unwrap_err().has_code("ALREADY_RUNNING"));
        e.active_instances().remove("opts");

        delete_resource_pack(&e, "opts", "Top.zip").await.unwrap();
        let text = std::fs::read_to_string(game.join("options.txt")).unwrap();
        assert!(!text.contains("Top.zip") && text.contains("file/Base.zip") && text.contains("\"vanilla\""));
    }

    #[tokio::test]
    async fn overlap_analyzer_counts_overrides_in_priority_order() {
        let (_d, e) = engine_for("ov", "1.21.1");
        let game = game_dir(&e, "ov");
        let rp = game.join("resourcepacks");
        write_zip(
            &rp.join("low.zip"),
            &[
                ("pack.mcmeta", PACK_34),
                ("assets/minecraft/textures/block/a.png", b""),
                ("assets/minecraft/textures/block/b.png", b""),
            ],
        );
        write_zip(&rp.join("mid.zip"), &[("pack.mcmeta", PACK_34), ("assets/minecraft/textures/block/a.png", b"")]);
        write_zip(
            &rp.join("high.zip"),
            &[
                ("pack.mcmeta", PACK_34),
                ("assets/minecraft/textures/block/a.png", b""),
                ("assets/minecraft/textures/item/c.png", b""),
            ],
        );
        std::fs::write(
            game.join("options.txt"),
            "resourcePacks:[\"vanilla\",\"file/low.zip\",\"file/mid.zip\",\"file/high.zip\"]\n",
        )
        .unwrap();
        let packs = list_resource_packs(&e, "ov").await.unwrap();
        let get = |n: &str| packs.iter().find(|p| p.filename == n).unwrap();
        let high = get("high.zip");
        assert_eq!((high.overrides_count, high.overridden_by_count), (1, 0));
        assert!(high.overridden_by_names.is_empty());
        assert_eq!(high.texture_tags, vec!["blocks".to_string(), "items".to_string()]);
        let mid = get("mid.zip");
        assert_eq!((mid.overrides_count, mid.overridden_by_count), (1, 1));
        assert_eq!(mid.overridden_by_names, vec!["high".to_string()]);
        let low = get("low.zip");
        assert_eq!((low.overrides_count, low.overridden_by_count), (0, 1));
        assert_eq!(low.overridden_by_names, vec!["high".to_string(), "mid".to_string()]);
    }

    #[tokio::test]
    async fn shader_install_flags_missing_iris_and_updates_iris_properties() {
        let (d, e) = engine_for("sh", "1.21.1");
        let game = game_dir(&e, "sh");
        let src = d.path().join("BSL.zip");
        write_zip(&src, &[("shaders/gbuffers_basic.vsh", b"void main(){}"), ("shaders/shaders.properties", b"")]);

        let installed = install_local_pack(&e, "sh", &src).await.unwrap();
        assert_eq!((installed.kind, installed.filename.as_str()), (PackKind::Shader, "BSL.zip"));
        assert!(installed.needs_iris && !installed.activated);

        let state = set_active_shader(&e, "sh", Some("BSL.zip")).await.unwrap();
        assert!(!state.iris_installed && state.shaders_enabled);
        assert_eq!(state.active_pack.as_deref(), Some("BSL.zip"));
        assert!(state.packs[0].active);
        let props = std::fs::read_to_string(game.join("config").join("iris.properties")).unwrap();
        assert!(props.contains("shaderPack=BSL.zip") && props.contains("enableShaders=true"));

        write_zip(
            &game.join("mods").join("iris.jar"),
            &[(
                "fabric.mod.json",
                br#"{"schemaVersion":1,"id":"iris","version":"1.8.0","name":"Iris","environment":"*"}"#,
            )],
        );
        assert!(shaders_state(&e, "sh").await.unwrap().iris_installed);

        let off = set_active_shader(&e, "sh", None).await.unwrap();
        assert!(!off.shaders_enabled);
        assert!(set_active_shader(&e, "sh", Some("Missing.zip")).await.unwrap_err().has_code("PACK_NOT_FOUND"));

        e.active_instances().insert("sh");
        assert!(set_active_shader(&e, "sh", None).await.unwrap_err().has_code("ALREADY_RUNNING"));
        e.active_instances().remove("sh");
    }

    #[tokio::test]
    async fn version_change_plan_flags_incompatible_active_resource_packs() {
        let (_d, e) = engine_for("vc-packs", "1.21.1");
        let game = game_dir(&e, "vc-packs");
        let rp = game.join("resourcepacks");
        write_zip(&rp.join("Modern.zip"), &[("pack.mcmeta", PACK_34)]);
        write_zip(&rp.join("Legacy.zip"), &[("pack.mcmeta", br#"{"pack":{"pack_format":1,"description":"old"}}"#)]);
        std::fs::write(
            game.join("options.txt"),
            "resourcePacks:[\"vanilla\",\"file/Modern.zip\",\"file/Legacy.zip\"]\n",
        )
        .unwrap();
        let inst = InstanceMeta { name: "vc-packs".into(), version: "1.21.1".into(), ..Default::default() };
        let flagged = version_change_packs(&e, &inst, "1.8.9").await;
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].filename, "Modern.zip");
        assert_eq!(flagged[0].compat, PackCompatStatus::TooNew);
        assert!(flagged[0].update_version_id.is_none());
    }

    #[test]
    fn classify_zip_distinguishes_resourcepack_shader_mrpack_and_unknown() {
        let d = tempfile::tempdir().unwrap();
        let rp = d.path().join("rp.zip");
        write_zip(&rp, &[("pack.mcmeta", PACK_34), ("assets/x.png", b"")]);
        let nested = d.path().join("nested.zip");
        write_zip(&nested, &[("Pack/pack.mcmeta", PACK_34)]);
        let shader = d.path().join("shader.zip");
        write_zip(&shader, &[("Shader/shaders/gbuffers_basic.vsh", b""), ("Shader/shaders/shaders.properties", b"")]);
        let mrpack = d.path().join("pack.mrpack");
        write_zip(&mrpack, &[("modrinth.index.json", b"{}"), ("overrides/x.txt", b"")]);
        let unknown = d.path().join("u.zip");
        write_zip(&unknown, &[("readme.txt", b"hi")]);

        assert_eq!(classify_zip_pack(&rp).unwrap(), ZipPackKind::ResourcePack { nested_prefix: None });
        assert_eq!(classify_zip_pack(&nested).unwrap(), ZipPackKind::ResourcePack { nested_prefix: Some("Pack/".into()) });
        assert_eq!(classify_zip_pack(&shader).unwrap(), ZipPackKind::ShaderPack);
        assert_eq!(classify_zip_pack(&mrpack).unwrap(), ZipPackKind::Mrpack);
        assert_eq!(classify_zip_pack(&unknown).unwrap(), ZipPackKind::Unknown);
    }
}
