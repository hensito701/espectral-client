//! Port of `src/engine/import.mjs` — detect other launchers (vanilla
//! `.minecraft`, FastClient, Lunar) and import options.txt / servers.dat /
//! Lunar settings into an instance.
//!
//! **Owner: content.** Status: PORTED.
//!
//! Detection roots come from [`ImportRoots`] — the environment by default
//! (`%APPDATA%` / home on Windows, `$HOME` / `$XDG_DATA_HOME` / Flatpak on
//! POSIX, exactly like the JS), or `ContentSettings::import_roots` so tests
//! never read the real home dir.
//!
//! Formats:
//! - options.txt — `version:N` header then `key:value` lines (split on the
//!   FIRST colon); unknown lines round-trip verbatim with the original EOL.
//! - optionsLC.txt — Lunar's JSON mirror; fov in degrees (`/70` -> multiplier).
//! - servers.dat — NBT via crate::nbt.
//!
//! CONTRACT (called by onboarding, instances.createInstance): detect_import_sources, import_profile.

use std::path::{Path, PathBuf, MAIN_SEPARATOR};

use serde_json::{Map, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{
    ImportResult, ImportSkipped, ImportSource, ImportSourceKind, InstanceMeta, LunarInfo, OptionsPair, OverwritePolicy,
};

/// Where launcher installs are looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRoots {
    /// Windows layout (`%APPDATA%` candidates) vs POSIX (`$HOME`).
    pub windows: bool,
    /// `%APPDATA%` (JS `process.env.APPDATA || ~/AppData/Roaming`).
    pub appdata: PathBuf,
    /// `os.homedir()` on Windows / `$HOME || os.homedir()` on POSIX.
    pub home: PathBuf,
    /// `$XDG_DATA_HOME` when set and absolute (POSIX only).
    pub xdg_data_home: Option<PathBuf>,
}

impl ImportRoots {
    /// The JS environment lookup.
    pub fn from_env() -> ImportRoots {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let windows = cfg!(windows);
        let os_home = dirs::home_dir().or_else(crate::paths::home_dir).unwrap_or_else(|| PathBuf::from("."));
        let home = if windows { os_home.clone() } else { env("HOME").unwrap_or_else(|| os_home.clone()) };
        let appdata = env("APPDATA").unwrap_or_else(|| os_home.join("AppData").join("Roaming"));
        let xdg_data_home = env("XDG_DATA_HOME").filter(|p| p.is_absolute());
        ImportRoots { windows, appdata, home, xdg_data_home }
    }
}

fn roots(engine: &Engine) -> ImportRoots {
    engine.content_settings().import_roots.unwrap_or_else(ImportRoots::from_env)
}

fn import_error(status: u16, code: &str, message: impl Into<String>) -> EngineError {
    EngineError::api(status, code, message)
}

/// Node `path.resolve(p)`: absolute (cwd-relative when needed) and normalized.
fn resolve(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.to_path_buf())
    };
    crate::util::normalize_path(&abs)
}

/// Import containment guard verbatim: true when `candidate` is not strictly
/// inside `root` (lexical, after resolution; equality counts as escaping).
pub fn path_escapes(root: &Path, candidate: &Path) -> bool {
    let r = resolve(root).to_string_lossy().into_owned();
    let c = resolve(candidate).to_string_lossy().into_owned();
    !c.starts_with(&format!("{r}{MAIN_SEPARATOR}"))
}

// ---------------------------------------------------------------------------
// options.txt parse / write
// ---------------------------------------------------------------------------

/// One options.txt line. `raw` keeps the original line; entries edited via
/// [`set_option_key`] get `raw = None` and are re-serialized from key/value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsEntry {
    pub key: Option<String>,
    pub value: Option<String>,
    pub raw: Option<String>,
}

/// Parsed options.txt (`header` = the raw `version:N` line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOptions {
    pub header: Option<String>,
    pub entries: Vec<OptionsEntry>,
    pub eol: String,
}

fn is_version_header(line: &str) -> bool {
    line.strip_prefix("version:").map(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)
}

/// Parse options.txt text (see module docs).
pub fn parse_options_txt(text: &str) -> ParsedOptions {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut header = None;
    let mut entries = vec![];
    for line in text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if line.is_empty() {
            continue;
        }
        if header.is_none() && is_version_header(line) {
            header = Some(line.to_string());
            continue;
        }
        match line.find(':') {
            Some(i) if i > 0 => entries.push(OptionsEntry {
                key: Some(line[..i].to_string()),
                value: Some(line[i + 1..].to_string()),
                raw: Some(line.to_string()),
            }),
            _ => entries.push(OptionsEntry { key: None, value: None, raw: Some(line.to_string()) }),
        }
    }
    ParsedOptions { header, entries, eol: eol.to_string() }
}

/// Serialize a parsed options.txt (header first, untouched lines verbatim, original EOL).
pub fn write_options_txt(parsed: &ParsedOptions) -> String {
    let eol = if parsed.eol.is_empty() { "\n" } else { parsed.eol.as_str() };
    let mut parts: Vec<String> = vec![];
    if let Some(h) = parsed.header.as_ref().filter(|h| !h.is_empty()) {
        parts.push(h.clone());
    }
    for e in &parsed.entries {
        parts.push(match &e.raw {
            Some(r) => r.clone(),
            None => format!("{}:{}", e.key.as_deref().unwrap_or("null"), e.value.as_deref().unwrap_or("null")),
        });
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join(eol) + eol
    }
}

/// Set/insert a key (re-serializes only that entry).
pub fn set_option_key(parsed: &mut ParsedOptions, key: &str, value: &str) {
    if let Some(e) = parsed.entries.iter_mut().find(|e| e.key.as_deref() == Some(key)) {
        e.value = Some(value.to_string());
        e.raw = None;
    } else {
        parsed.entries.push(OptionsEntry { key: Some(key.to_string()), value: Some(value.to_string()), raw: None });
    }
}

/// Number of key:value entries (excludes the version header).
pub fn count_options_keys(text: &str) -> u32 {
    parse_options_txt(text).entries.iter().filter(|e| e.key.is_some()).count() as u32
}

/// options.txt -> `[[k, v], …]` for the options endpoint / UI table.
pub fn read_options_txt(text: &str) -> Vec<OptionsPair> {
    parse_options_txt(text).entries.into_iter().filter_map(|e| Some((e.key?, e.value.unwrap_or_default()))).collect()
}

// ---------------------------------------------------------------------------
// Lunar helpers
// ---------------------------------------------------------------------------

fn read_json_object(p: &Path) -> Option<Map<String, Value>> {
    let s = std::fs::read(p).ok()?;
    match serde_json::from_slice::<Value>(&s).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

/// optionsLC.txt from a game dir (None if absent/unparseable/non-object).
fn read_options_lc(game_dir: &Path) -> Option<Map<String, Value>> {
    read_json_object(&game_dir.join("optionsLC.txt"))
}

/// Enabled top-level mod names of the newest `settings/game/<profile>/mods.json`.
fn read_lunar_mod_toggles(settings_dir: &Path) -> Vec<String> {
    let game_dir = settings_dir.join("game");
    let Ok(rd) = std::fs::read_dir(&game_dir) else { return vec![] };
    let mut best: Option<(f64, PathBuf)> = None;
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let file = entry.path().join("mods.json");
        if let Ok(st) = std::fs::metadata(&file) {
            let m = crate::platform::mtime_ms(&st);
            if best.as_ref().map(|(bm, _)| m > *bm).unwrap_or(true) {
                best = Some((m, file));
            }
        }
    }
    let Some((_, file)) = best else { return vec![] };
    let Some(obj) = read_json_object(&file) else { return vec![] };
    obj.iter()
        .filter(|(name, cfg)| *name != "version" && cfg.get("enabled") != Some(&Value::Bool(false)))
        .map(|(name, _)| name.clone())
        .collect()
}

/// optionsLC keys merged into options.txt on a Lunar import (JS LC_MERGE_KEYS order).
const LC_MERGE_KEYS: &[&str] = &[
    // video / quality
    "ao",
    "biomeBlendRadius",
    "chunkSectionFadeInTime",
    "cutoutLeaves",
    "enableVsync",
    "entityDistanceScaling",
    "entityShadows",
    "forceUnicodeFont",
    "japaneseGlyphVariants",
    "fov",
    "fovEffectScale",
    "darknessEffectScale",
    "glintSpeed",
    "glintStrength",
    "damageTiltStrength",
    "gamma",
    "renderDistance",
    "simulationDistance",
    "guiScale",
    "maxFps",
    "mouseSensitivity",
    "rawMouseInput",
    "menuBackgroundBlurriness",
    "particles",
    "mipmapLevels",
    "maxAnisotropyBit",
    "screenEffectScale",
    "fullscreen",
    "exclusiveFullscreen",
    "graphicsPreset",
    "graphicsMode",
    "renderClouds",
    "cloudRange",
    "textureFiltering",
    "preferredGraphicsBackend",
    "improvedTransparency",
    "vignette",
    "weatherRadius",
    "prioritizeChunkUpdates",
    "panoramaScrollSpeed",
    "rotateWithMinecart",
    "highContrast",
    "highContrastBlockOutline",
    // input / controls
    "invertXMouse",
    "invertYMouse",
    "toggleAttack",
    "toggleUse",
    "sprintWindow",
    "allowCursorChanges",
    "saveChatDrafts",
    "touchscreen",
    "discrete_mouse_scroll",
    "bobView",
    "toggleCrouch",
    "toggleSprint",
    "autoJump",
    "mouseWheelSensitivity",
    "useNativeTransport",
    "inactivityFpsLimit",
    "soundDevice",
    // accessibility / chat / UI
    "narratorHotkey",
    "narrator",
    "chatOpacity",
    "chatScale",
    "chatWidth",
    "chatHeightFocused",
    "chatHeightUnfocused",
    "chatLineSpacing",
    "textBackgroundOpacity",
    "backgroundForChatOnly",
    "chatDelay",
    "chatVisibility",
    "chatColors",
    "chatLinks",
    "chatLinksPrompt",
    "showSubtitles",
    "reducedDebugInfo",
    "hideServerAddress",
    "advancedItemTooltips",
    "pauseOnLostFocus",
    "overrideWidth",
    "overrideHeight",
    "notificationDisplayTime",
    "glDebugVerbosity",
    "skipMultiplayerWarning",
    "skipRealms32bitWarning",
    "hideMatchedNames",
    "joinedFirstServer",
    "hideBundleTutorial",
    "syncChunkWrites",
    "showAutosaveIndicator",
    "allowServerListing",
    "onlyShowSecureChat",
    "telemetryOptInExtra",
    "onboardAccessibility",
    "startedCleanly",
    "snooperEnabled",
    "darkMojangStudiosBackground",
    "hideLightningFlashes",
    "hideSplashTexts",
    "realmsNotifications",
    "operatorItemsTab",
    "autoSuggestions",
    "musicToast",
    "musicFrequency",
    "inGameNotification",
    "sharePresence",
    // gameplay / misc
    "difficulty",
    "mainHand",
    "attackIndicator",
    "tutorialStep",
    "heldItemTooltips",
    "lang",
    "lastServer",
];

/// JS `String(n)` for a finite number (integers without a fraction).
fn js_number_string(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// JS `String(v)` for JSON scalars.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => js_number_string(n.as_f64().unwrap_or(0.0)),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        Value::Array(a) => a.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// JS `Number(v)` for JSON scalars (NaN -> None).
fn js_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                Some(0.0)
            } else {
                t.parse::<f64>().ok()
            }
        }
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Null => Some(0.0),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

/// Merge Lunar's optionsLC mirror into a parsed options.txt.
fn apply_lunar_merge(parsed: &mut ParsedOptions, lc: &Map<String, Value>) {
    for key in LC_MERGE_KEYS {
        let Some(v) = lc.get(*key) else { continue };
        if v.is_null() {
            continue;
        }
        if *key == "fov" {
            let Some(deg) = js_number(v).filter(|d| *d > 0.0) else { continue };
            set_option_key(parsed, "fov", &js_number_string(deg / 70.0));
        } else {
            set_option_key(parsed, key, &js_string(v));
        }
    }
}

// ---------------------------------------------------------------------------
// Source detection
// ---------------------------------------------------------------------------

fn vanilla_candidates(r: &ImportRoots) -> Vec<PathBuf> {
    if r.windows {
        return vec![r.appdata.join(".minecraft")];
    }
    let mut out = vec![r.home.join(".minecraft")];
    if let Some(x) = &r.xdg_data_home {
        let p = x.join(".minecraft");
        if !out.contains(&p) {
            out.push(p);
        }
    }
    let flatpak = r.home.join(".var").join("app");
    if let Ok(rd) = std::fs::read_dir(&flatpak) {
        let mut dirs: Vec<PathBuf> =
            rd.flatten().filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            for p in [d.join(".minecraft"), d.join("data").join(".minecraft")] {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn fastclient_profiles_dirs(r: &ImportRoots) -> Vec<PathBuf> {
    if r.windows {
        vec![r.appdata.join(".fastclient").join("profiles")]
    } else {
        vec![r.home.join(".fastclient").join("profiles")]
    }
}

fn lunar_candidates(r: &ImportRoots) -> Vec<PathBuf> {
    if r.windows {
        vec![r.appdata.join(".lunarclient"), r.home.join(".lunarclient")]
    } else {
        vec![r.home.join(".lunarclient")]
    }
}

/// ImportSource for a dir holding vanilla-format options.txt / servers.dat.
fn build_source(id: &str, kind: ImportSourceKind, label: &str, dir: &Path) -> ImportSource {
    let options = dir.join("options.txt");
    let servers = dir.join("servers.dat");
    let options_exists = options.exists();
    let servers_exists = servers.exists();
    let options_key_count = if options_exists {
        std::fs::read(&options).map(|b| count_options_keys(&String::from_utf8_lossy(&b))).unwrap_or(0)
    } else {
        0
    };
    let servers_entry_count = if servers_exists {
        std::fs::read(&servers)
            .ok()
            .and_then(|b| crate::nbt::parse_servers_dat(&b).ok())
            .map(|v| v.len() as u32)
            .unwrap_or(0)
    } else {
        0
    };
    ImportSource {
        id: id.into(),
        kind,
        label: label.into(),
        path: dir.to_string_lossy().into_owned(),
        options_exists,
        servers_exists,
        options_key_count,
        servers_entry_count,
        lunar: None,
    }
}

fn make_vanilla_source(r: &ImportRoots) -> ImportSource {
    let cands = vanilla_candidates(r);
    if !r.windows {
        for dir in &cands {
            if dir.join("options.txt").exists() || dir.join("servers.dat").exists() {
                return build_source("vanilla", ImportSourceKind::Vanilla, "Minecraft (vanilla)", dir);
            }
        }
    }
    build_source("vanilla", ImportSourceKind::Vanilla, "Minecraft (vanilla)", &cands[0])
}

fn make_fastclient_sources(r: &ImportRoots) -> Vec<ImportSource> {
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for profiles in fastclient_profiles_dirs(r) {
        let Ok(rd) = std::fs::read_dir(&profiles) else { continue };
        let mut names: Vec<String> = rd
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            if !seen.insert(name.clone()) {
                continue;
            }
            out.push(build_source(
                &format!("fastclient:{name}"),
                ImportSourceKind::Fastclient,
                &format!("FastClient · {name}"),
                &profiles.join(&name),
            ));
        }
    }
    out
}

/// The Lunar source (None when absent or its gameDirectory escapes the install root).
pub fn make_lunar_source(r: &ImportRoots) -> Option<ImportSource> {
    let settings_dir =
        lunar_candidates(r).into_iter().map(|c| c.join("settings")).find(|s| s.join("launcher.json").exists())?;
    let launcher = read_json_object(&settings_dir.join("launcher.json"))?;
    let settings = launcher.get("settings").and_then(Value::as_object).cloned().unwrap_or_default();
    let game_dir_raw = settings.get("gameDirectory").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let mut game_dir = PathBuf::from(game_dir_raw);
    if !crate::instances::is_absolute_like_node(game_dir_raw) {
        let base = if r.windows { &r.appdata } else { &r.home };
        game_dir = resolve(&base.join(game_dir_raw));
    }
    // Containment: only a gameDirectory under the Lunar install root.
    let lunar_root = settings_dir.parent().map(Path::to_path_buf).unwrap_or_else(|| settings_dir.clone());
    let root_s = lunar_root.to_string_lossy().into_owned();
    let abs = resolve(&game_dir).to_string_lossy().into_owned();
    if abs != root_s && !abs.starts_with(&format!("{root_s}{MAIN_SEPARATOR}")) {
        return None;
    }

    let mut lunar = LunarInfo::default();
    if let Some(mem) = settings.get("allocatedMemory").and_then(js_number).filter(|m| *m > 0.0) {
        lunar.allocated_memory = Some(mem.min(u32::MAX as f64) as u32);
    }
    if let Some(lc) = read_options_lc(&game_dir) {
        if let Some(fov) = lc.get("fov").and_then(js_number) {
            lunar.fov_degrees = Some(fov);
        }
    }
    let toggles = read_lunar_mod_toggles(&settings_dir);
    if !toggles.is_empty() {
        lunar.mods_json_toggles = Some(toggles);
    }
    let mut source = build_source("lunar", ImportSourceKind::Lunar, "Lunar Client", &game_dir);
    if lunar != LunarInfo::default() {
        source.lunar = Some(lunar);
    }
    Some(source)
}

/// Every source for the given roots: vanilla, FastClient profiles, Lunar.
pub fn detect_import_sources_in(r: &ImportRoots) -> Vec<ImportSource> {
    let mut sources = vec![make_vanilla_source(r)];
    sources.extend(make_fastclient_sources(r));
    if let Some(l) = make_lunar_source(r) {
        sources.push(l);
    }
    sources
}

/// Detected import sources (ImportSource rows).
pub async fn detect_import_sources(engine: &Engine) -> EngineResult<Vec<ImportSource>> {
    Ok(detect_import_sources_in(&roots(engine)))
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// The FC two-hop whitelist — exactly these two files.
const IMPORT_FILES: [&str; 2] = ["options.txt", "servers.dat"];

fn mtime(p: &Path) -> f64 {
    std::fs::metadata(p).map(|m| crate::platform::mtime_ms(&m)).unwrap_or(0.0)
}

/// Import a source's profile files into `instance` (its instance dir, like the JS).
pub async fn import_profile(
    engine: &Engine,
    instance: &InstanceMeta,
    source_id: &str,
    policy: OverwritePolicy,
    merge_optionslc: bool,
) -> EngineResult<ImportResult> {
    let target = engine.paths().instance_dir(&instance.name);
    import_profile_into(engine, instance, &target, source_id, policy, merge_optionslc).await
}

/// importProfile with an explicit target dir (JS `instance.dir`).
pub async fn import_profile_into(
    engine: &Engine,
    instance: &InstanceMeta,
    target_dir: &Path,
    source_id: &str,
    policy: OverwritePolicy,
    merge_optionslc: bool,
) -> EngineResult<ImportResult> {
    if instance.name.is_empty() {
        return Err(import_error(400, "BAD_INSTANCE", "instance is required"));
    }
    let sources = detect_import_sources(engine).await?;
    let source = sources
        .into_iter()
        .find(|s| s.id == source_id)
        .ok_or_else(|| import_error(404, "SOURCE_NOT_FOUND", format!("import source not found: {source_id}")))?;
    std::fs::create_dir_all(target_dir)?;

    let source_root = resolve(Path::new(&source.path));
    let dst_root = resolve(target_dir);
    let mut copied: Vec<String> = vec![];
    let mut skipped: Vec<ImportSkipped> = vec![];
    let skip = |file: &str, reason: &str| ImportSkipped { file: file.into(), reason: reason.into() };
    for file in IMPORT_FILES {
        let src = source_root.join(file);
        if path_escapes(&source_root, &src) {
            return Err(import_error(
                400,
                "SOURCE_PATH_ESCAPE",
                format!("import source path escapes its directory: {source_id}"),
            ));
        }
        let dst = dst_root.join(file);
        if path_escapes(&dst_root, &dst) {
            return Err(import_error(400, "TARGET_PATH_ESCAPE", "import target path escapes its directory"));
        }
        if !src.exists() {
            skipped.push(skip(file, "not present in source"));
            continue;
        }
        if dst.exists() {
            if policy == OverwritePolicy::Never {
                skipped.push(skip(file, "target already exists and overwrite_policy is \"never\""));
                continue;
            }
            if mtime(&src) <= mtime(&dst) {
                skipped.push(skip(file, "source is not newer than target"));
                continue;
            }
            if file == "servers.dat" {
                std::fs::copy(&dst, format!("{}.bak", dst.to_string_lossy()))?;
            }
        }
        std::fs::copy(&src, &dst)?;
        copied.push(file.to_string());
    }

    let mut lunar = None;
    if source.kind == ImportSourceKind::Lunar {
        lunar = source.lunar.clone();
        if merge_optionslc && copied.iter().any(|c| c == "options.txt") {
            if let Some(lc) = read_options_lc(Path::new(&source.path)) {
                let options_path = target_dir.join("options.txt");
                let text = String::from_utf8_lossy(&std::fs::read(&options_path)?).into_owned();
                let mut parsed = parse_options_txt(&text);
                apply_lunar_merge(&mut parsed, &lc);
                std::fs::write(&options_path, write_options_txt(&parsed))?;
            }
        }
        if let Some(mem) = source.lunar.as_ref().and_then(|l| l.allocated_memory) {
            if instance.memory_mb.unwrap_or(0) == 0 {
                persist_instance_memory(target_dir, mem);
            }
        }
    }

    let target_dat = target_dir.join("servers.dat");
    let dat = if target_dat.exists() { target_dat } else { Path::new(&source.path).join("servers.dat") };
    let servers_parsed = if dat.exists() {
        std::fs::read(&dat).ok().and_then(|b| crate::nbt::parse_servers_dat(&b).ok()).unwrap_or_default()
    } else {
        vec![]
    };
    let options_path = target_dir.join("options.txt");
    let options_keys = if options_path.exists() {
        std::fs::read(&options_path).map(|b| count_options_keys(&String::from_utf8_lossy(&b))).unwrap_or(0)
    } else {
        0
    };
    Ok(ImportResult { copied, skipped, servers_parsed, options_keys, lunar })
}

/// Persist memory_mb into `<target>/instance.json`, preserving every other key.
fn persist_instance_memory(target_dir: &Path, memory_mb: u32) {
    let p = target_dir.join("instance.json");
    let Some(mut json) = read_json_object(&p) else { return };
    json.insert("memory_mb".into(), Value::from(memory_mb));
    let _ = crate::paths::write_json_atomic(&p, &json);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    /// Sandboxed Windows-layout roots (the JS tests sandbox APPDATA).
    fn sandbox_roots(base: &Path) -> ImportRoots {
        ImportRoots { windows: true, appdata: base.join("appdata"), home: base.join("home"), xdg_data_home: None }
    }

    fn write_lunar_launcher(r: &ImportRoots, game_directory: &str) {
        let settings = r.appdata.join(".lunarclient").join("settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("launcher.json"),
            serde_json::json!({ "settings": { "gameDirectory": game_directory } }).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn path_escapes_semantics() {
        let root = std::env::temp_dir().join("espectral-root");
        assert!(!path_escapes(&root, &root.join("options.txt")));
        assert!(!path_escapes(&root, &root.join("game").join("servers.dat")));
        assert!(path_escapes(&root, &root.join("..").join("evil").join("options.txt")));
        assert!(path_escapes(&root, &root.join("..").join("..").join("windows").join("system32")));
        let sibling = PathBuf::from(format!("{}-evil", root.to_string_lossy()));
        assert!(path_escapes(&root, &sibling.join("options.txt")));
        assert!(path_escapes(&root, &std::env::temp_dir().join("elsewhere").join("options.txt")));
        assert!(path_escapes(&root, &root));
        let fs_root: PathBuf = std::env::temp_dir().components().take(1).collect::<PathBuf>();
        let fs_root = if cfg!(windows) { PathBuf::from(format!("{}\\", fs_root.to_string_lossy())) } else { fs_root };
        assert!(path_escapes(&fs_root, &fs_root.join("options.txt")));
    }

    #[test]
    fn options_txt_round_trip() {
        let text = "version:3700\r\nfov:0.5\r\nkey_key.zoom:key.keyboard.z\r\nlonely\r\n:x\r\nsrv:a:b\r\n";
        let p = parse_options_txt(text);
        assert_eq!(p.header.as_deref(), Some("version:3700"));
        assert_eq!(p.eol, "\r\n");
        assert_eq!(write_options_txt(&p), text);
        assert_eq!(count_options_keys(text), 3);
        assert_eq!(read_options_txt(text)[2], ("srv".to_string(), "a:b".to_string()));
        let mut p = p;
        set_option_key(&mut p, "fov", "1.5");
        set_option_key(&mut p, "gamma", "1");
        assert_eq!(
            write_options_txt(&p),
            "version:3700\r\nfov:1.5\r\nkey_key.zoom:key.keyboard.z\r\nlonely\r\n:x\r\nsrv:a:b\r\ngamma:1\r\n"
        );
        assert_eq!(write_options_txt(&parse_options_txt("")), "");
    }

    #[test]
    fn lunar_merge_converts_fov_and_skips_unknown_keys() {
        let mut p = parse_options_txt("version:1\nfov:0.0\ngamma:0.5\n");
        let lc = serde_json::json!({ "fov": "105", "gamma": 1, "lastLaunchedVersion": "1.8", "maxFPS": "200", "renderDistance": "12" });
        apply_lunar_merge(&mut p, lc.as_object().unwrap());
        let out = write_options_txt(&p);
        assert_eq!(out, "version:1\nfov:1.5\ngamma:1\nrenderDistance:12\n");
    }

    #[test]
    fn lunar_source_containment() {
        let d = tempfile::tempdir().unwrap();
        let r = sandbox_roots(d.path());
        write_lunar_launcher(&r, "../../../../windows/system32");
        assert!(make_lunar_source(&r).is_none());
        write_lunar_launcher(&r, &std::env::temp_dir().join("windows").join("system32").to_string_lossy());
        assert!(make_lunar_source(&r).is_none());
        write_lunar_launcher(&r, &r.appdata.join(".lunarclient_evil").to_string_lossy());
        assert!(make_lunar_source(&r).is_none());
        assert!(!detect_import_sources_in(&r).iter().any(|s| s.id == "lunar"));

        let game = r.appdata.join(".lunarclient").join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("options.txt"), "version:15\ngamma:1.0\nfov:0.7\n").unwrap();
        std::fs::write(game.join("optionsLC.txt"), r#"{"fov":"100"}"#).unwrap();
        write_lunar_launcher(&r, &game.to_string_lossy());
        let s = make_lunar_source(&r).unwrap();
        assert_eq!(s.id, "lunar");
        assert_eq!(resolve(Path::new(&s.path)), resolve(&game));
        assert!(s.options_exists);
        assert_eq!(s.options_key_count, 2);
        assert_eq!(s.lunar.unwrap().fov_degrees, Some(100.0));
    }

    #[test]
    fn detects_vanilla_and_fastclient_profiles_posix() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let r = ImportRoots { windows: false, appdata: d.path().join("nope"), home: home.clone(), xdg_data_home: None };
        // vanilla falls back to ~/.minecraft when nothing exists
        let s = detect_import_sources_in(&r);
        assert_eq!(s.len(), 1);
        assert_eq!(PathBuf::from(&s[0].path), home.join(".minecraft"));
        assert!(!s[0].options_exists);
        // a Flatpak install with options.txt wins
        let flat = home.join(".var").join("app").join("com.mojang.Minecraft").join("data").join(".minecraft");
        std::fs::create_dir_all(&flat).unwrap();
        std::fs::write(flat.join("options.txt"), "a:1\n").unwrap();
        for p in ["b", "a"] {
            std::fs::create_dir_all(home.join(".fastclient").join("profiles").join(p)).unwrap();
        }
        let s = detect_import_sources_in(&r);
        assert_eq!(PathBuf::from(&s[0].path), flat);
        assert_eq!(s[0].options_key_count, 1);
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["vanilla", "fastclient:a", "fastclient:b"]);
        assert_eq!(s[1].label, "FastClient · a");
    }

    #[tokio::test]
    async fn import_profile_copies_whitelist_and_guards_target() {
        let (d, e) = temp_engine();
        let r = sandbox_roots(d.path());
        e.update_content_settings(|s| s.import_roots = Some(r.clone()));
        let game = r.appdata.join(".lunarclient").join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("options.txt"), "version:15\ngamma:1.0\nfov:0.7\n").unwrap();
        write_lunar_launcher(&r, &game.to_string_lossy());

        let inst = InstanceMeta { name: "imported".into(), ..Default::default() };
        let res = import_profile(&e, &inst, "lunar", OverwritePolicy::Never, false).await.unwrap();
        assert_eq!(res.copied, vec!["options.txt"]);
        assert_eq!(res.skipped.len(), 1);
        assert_eq!(res.skipped[0].file, "servers.dat");
        let target = e.paths().instance_dir("imported");
        assert_eq!(std::fs::read_to_string(target.join("options.txt")).unwrap(), "version:15\ngamma:1.0\nfov:0.7\n");
        assert_eq!(res.options_keys, 2);

        // never: an existing target is left alone
        let res = import_profile(&e, &inst, "lunar", OverwritePolicy::Never, false).await.unwrap();
        assert!(res.copied.is_empty());
        assert!(res.skipped[0].reason.contains("never"));

        let missing = import_profile(&e, &inst, "nope", OverwritePolicy::Never, false).await.unwrap_err();
        assert_eq!((missing.status(), missing.envelope().code), (404, "SOURCE_NOT_FOUND".to_string()));

        if !cfg!(windows) {
            let err = import_profile_into(&e, &inst, Path::new("/"), "lunar", OverwritePolicy::Never, false)
                .await
                .unwrap_err();
            assert!(err.has_code("TARGET_PATH_ESCAPE"), "{err}");
        }
    }

    #[tokio::test]
    async fn lunar_import_merges_optionslc_and_memory() {
        let (d, e) = temp_engine();
        let r = sandbox_roots(d.path());
        e.update_content_settings(|s| s.import_roots = Some(r.clone()));
        let game = r.appdata.join(".lunarclient").join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("options.txt"), "version:15\nfov:0.7\n").unwrap();
        std::fs::write(game.join("optionsLC.txt"), r#"{"fov":"140","renderDistance":16}"#).unwrap();
        let settings = r.appdata.join(".lunarclient").join("settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("launcher.json"),
            serde_json::json!({ "settings": { "gameDirectory": game.to_string_lossy(), "allocatedMemory": 6144 } })
                .to_string(),
        )
        .unwrap();
        let target = e.paths().instance_dir("lunar-inst");
        crate::paths::write_json_atomic(
            &target.join("instance.json"),
            &serde_json::json!({ "name": "lunar-inst", "version": "1.21.11", "memory_mb": null, "keep": 1 }),
        )
        .unwrap();
        let inst = InstanceMeta { name: "lunar-inst".into(), ..Default::default() };
        let res = import_profile(&e, &inst, "lunar", OverwritePolicy::Never, true).await.unwrap();
        assert_eq!(res.lunar.as_ref().unwrap().allocated_memory, Some(6144));
        assert_eq!(
            std::fs::read_to_string(target.join("options.txt")).unwrap(),
            "version:15\nfov:2\nrenderDistance:16\n"
        );
        let json: Value =
            serde_json::from_str(&std::fs::read_to_string(target.join("instance.json")).unwrap()).unwrap();
        assert_eq!((json["memory_mb"].clone(), json["keep"].clone()), (Value::from(6144), Value::from(1)));
    }
}
