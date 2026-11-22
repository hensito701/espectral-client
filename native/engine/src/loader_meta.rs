//! Multi-loader jar metadata: classify `.jar` / `.jar.disabled` files by the
//! mod descriptors they ship (`fabric.mod.json`, `quilt.mod.json`,
//! `META-INF/neoforge.mods.toml`, `META-INF/mods.toml`, `mcmod.info`,
//! `MANIFEST.MF` coremod markers) and evaluate compatibility for Forge,
//! NeoForge and legacy-Forge instances on top of the shared
//! `fabric_compat` report types.
//!
//! New code (no JS original). Every jar entry read goes through `ziputil`
//! with an explicit decompressed-size cap.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use indexmap::IndexMap;

use crate::fabric_compat::{
    clean_version_display, known_modrinth_for_id, CompatFix, CompatIssueKind, CompatSeverity, FabricModMeta,
    JarModInfo, ModCompatIssue, ModCompatReport,
};
use crate::types::Loader;
use crate::ziputil;

/// Decompressed cap for metadata entries (mods.toml / mcmod.info / MANIFEST.MF).
const META_CAP: u64 = 1 << 20;

/// Ids that name the loader or Minecraft itself rather than a mod. Legacy
/// `requiredMods` lists them routinely; they are never missing mods.
const LOADER_IDS: &[&str] = &["forge", "fml", "forgemodloader", "neoforge", "minecraft", "java"];

/// Which loader family a jar is built for. A jar may ship several descriptors
/// (multi-loader releases), so scans keep a set, never a single kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JarLoaderKind {
    /// `fabric.mod.json` or `quilt.mod.json` (Quilt is accepted by Fabric).
    Fabric,
    /// `META-INF/neoforge.mods.toml`.
    Neoforge,
    /// `META-INF/mods.toml`.
    Forge,
    /// `mcmod.info`, or a bare coremod manifest (`FMLCorePlugin`/`TweakClass`).
    LegacyForge,
    /// No mod descriptor found at all (shaded library, misplaced file).
    Unknown,
}

impl JarLoaderKind {
    fn label(self) -> &'static str {
        match self {
            JarLoaderKind::Fabric => "Fabric",
            JarLoaderKind::Neoforge => "NeoForge",
            JarLoaderKind::Forge => "Forge",
            JarLoaderKind::LegacyForge => "Legacy Forge",
            JarLoaderKind::Unknown => "",
        }
    }
}

/// Whether a jar whose descriptors are `kinds` belongs in an instance running
/// `loader`.
pub fn jar_accepts_loader(kinds: &HashSet<JarLoaderKind>, loader: Loader) -> bool {
    match loader {
        Loader::Fabric => kinds.contains(&JarLoaderKind::Fabric),
        Loader::Neoforge => kinds.contains(&JarLoaderKind::Neoforge),
        // Legacy Forge jars run on any Forge instance.
        Loader::Forge => kinds.iter().any(|k| matches!(k, JarLoaderKind::Forge | JarLoaderKind::LegacyForge)),
        Loader::Vanilla => false,
    }
}

/// `type` of a `[[dependencies.<modId>]]` entry (legacy Forge used
/// `mandatory = true|false` instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepType {
    Required,
    Optional,
    Incompatible,
    Discouraged,
}

impl DepType {
    fn parse(s: &str) -> Option<DepType> {
        match s.to_ascii_lowercase().as_str() {
            "required" => Some(DepType::Required),
            "optional" => Some(DepType::Optional),
            "incompatible" => Some(DepType::Incompatible),
            "discouraged" => Some(DepType::Discouraged),
            _ => None,
        }
    }
}

/// One `[[dependencies.<modId>]]` entry.
#[derive(Debug, Clone)]
pub struct ModDep {
    pub mod_id: String,
    pub dep_type: DepType,
    /// Maven version range as written (`[1.0,2.0)`); empty means "any".
    pub version_range: String,
    /// `BOTH` | `CLIENT` | `SERVER`. A `side = "SERVER"` `minecraft` dep marks
    /// the whole mod as server-only.
    pub side: String,
}

/// One `[[mods]]` entry from `META-INF/mods.toml` / `neoforge.mods.toml`.
#[derive(Debug, Clone)]
pub struct ForgeModInfo {
    pub mod_id: String,
    pub name: String,
    /// `version` with `${file.jarVersion}` resolved from the manifest
    /// `Implementation-Version`.
    pub version: String,
    pub client_side_only: bool,
    pub deps: Vec<ModDep>,
}

/// One entry of `mcmod.info` (legacy Forge, 1.8.9–1.12.2).
#[derive(Debug, Clone)]
pub struct LegacyModInfo {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub mc_version: String,
    /// `requiredMods` + `dependencies` as `modid` / `modid@range` strings.
    pub deps: Vec<String>,
}

/// Loader descriptors and mod metadata found inside one jar.
#[derive(Debug)]
pub struct JarScan {
    /// Base filename without `.disabled`.
    pub filename: String,
    pub enabled: bool,
    /// Every descriptor kind the jar ships.
    pub kinds: HashSet<JarLoaderKind>,
    /// `fabric.mod.json` root metadata (nested jars are owned by `fabric_compat`).
    pub fabric: Option<FabricModMeta>,
    /// `[[mods]]` entries of `neoforge.mods.toml` or `mods.toml`.
    pub forge_mods: Vec<ForgeModInfo>,
    /// `mcmod.info` entries (legacy Forge).
    pub legacy_mods: Vec<LegacyModInfo>,
}

impl JarScan {
    /// Primary mod identity for `list_mods` and issue rows: the Fabric root
    /// first (unchanged for Fabric jars), then the first `[[mods]]` entry,
    /// then the first `mcmod.info` entry.
    pub fn primary_id(&self) -> Option<&str> {
        self.fabric
            .as_ref()
            .map(|f| f.id.as_str())
            .or_else(|| self.forge_mods.first().map(|m| m.mod_id.as_str()))
            .or_else(|| self.legacy_mods.first().map(|m| m.mod_id.as_str()))
    }

    pub fn primary_name(&self) -> Option<&str> {
        self.fabric
            .as_ref()
            .map(|f| f.name.as_str())
            .or_else(|| self.forge_mods.first().map(|m| m.name.as_str()))
            .or_else(|| self.legacy_mods.first().map(|m| m.name.as_str()))
    }

    pub fn primary_version(&self) -> Option<&str> {
        self.fabric
            .as_ref()
            .map(|f| clean_version_display(&f.version))
            .or_else(|| self.forge_mods.first().map(|m| m.version.as_str()))
            .or_else(|| self.legacy_mods.first().map(|m| m.version.as_str()))
    }

    /// Loader(s) this jar is built for, e.g. "Forge + Fabric".
    pub fn loader_label(&self) -> String {
        const ORDER: [JarLoaderKind; 4] =
            [JarLoaderKind::Fabric, JarLoaderKind::Neoforge, JarLoaderKind::Forge, JarLoaderKind::LegacyForge];
        ORDER.iter().filter(|k| self.kinds.contains(*k)).map(|k| k.label()).collect::<Vec<&str>>().join(" + ")
    }
}

/// Instance side of a Forge/NeoForge compat evaluation.
#[derive(Debug, Clone)]
pub struct LoaderEnv {
    pub mc_version: String,
    /// `Loader::Neoforge` or `Loader::Forge`.
    pub loader: Loader,
    /// Pinned/installed loader version; `None` skips `LoaderVersion` checks.
    pub loader_version: Option<String>,
    pub java_major: u32,
}

// ---------------------------------------------------------------------------
// Jar scanning
// ---------------------------------------------------------------------------

fn read_entry(buf: &[u8], name: &str) -> Option<Vec<u8>> {
    ziputil::read_zip_entry_from_capped(buf, name, META_CAP).ok().flatten()
}

/// Parse `META-INF/MANIFEST.MF` main attributes into lowercase keys, joining
/// continuation lines (a line starting with a space extends the previous value).
fn parse_manifest(raw: &[u8]) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let mut last_key: Option<String> = None;
    let text = String::from_utf8_lossy(raw);
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            last_key = None;
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(v) = last_key.as_ref().and_then(|k| out.get_mut(k)) {
                v.push_str(line.trim_start());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            let key = name.trim().to_ascii_lowercase();
            out.insert(key.clone(), value.trim().to_string());
            last_key = Some(key);
        }
    }
    out
}

/// Scan one jar file for loader descriptors and mod metadata. `filename` is
/// the base name without `.disabled`. `None` when the file is not a readable
/// zip at all.
pub fn scan_jar(path: &Path, filename: &str, enabled: bool) -> Option<JarScan> {
    let buf = std::fs::read(path).ok()?;
    let mut kinds: HashSet<JarLoaderKind> = HashSet::new();

    let has_fabric = read_entry(&buf, "fabric.mod.json").is_some() || read_entry(&buf, "quilt.mod.json").is_some();
    let fabric = if has_fabric {
        kinds.insert(JarLoaderKind::Fabric);
        crate::fabric_compat::read_fabric_jar(path, filename, enabled).map(|s| s.root)
    } else {
        None
    };

    let manifest = read_entry(&buf, "META-INF/MANIFEST.MF").map(|b| parse_manifest(&b)).unwrap_or_default();

    let mut forge_mods: Vec<ForgeModInfo> = Vec::new();
    for (entry, kind) in
        [("META-INF/neoforge.mods.toml", JarLoaderKind::Neoforge), ("META-INF/mods.toml", JarLoaderKind::Forge)]
    {
        if let Some(bytes) = read_entry(&buf, entry) {
            kinds.insert(kind);
            if let Ok(text) = String::from_utf8(bytes) {
                forge_mods.extend(parse_mods_toml(&text, &manifest));
            }
        }
    }

    let mut legacy_mods: Vec<LegacyModInfo> = Vec::new();
    if let Some(bytes) = read_entry(&buf, "mcmod.info") {
        kinds.insert(JarLoaderKind::LegacyForge);
        legacy_mods = parse_mcmod_info(&String::from_utf8_lossy(&bytes));
    }

    if kinds.is_empty() {
        // A coremod without a mod descriptor is still a Forge-family jar.
        let coremod = manifest.contains_key("fmlcoreplugin") || manifest.contains_key("tweakclass");
        kinds.insert(if coremod { JarLoaderKind::LegacyForge } else { JarLoaderKind::Unknown });
    }

    Some(JarScan { filename: filename.to_string(), enabled, kinds, fabric, forge_mods, legacy_mods })
}

/// Scan every `.jar` / `.jar.disabled` in `mods_dir` (sorted, deterministic).
pub fn scan_mods_dir(mods_dir: &Path) -> Vec<JarScan> {
    let mut names: Vec<String> = match std::fs::read_dir(mods_dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|f| f.ends_with(".jar") || f.ends_with(".jar.disabled"))
            .collect(),
        Err(_) => return vec![],
    };
    names.sort();
    let mut out = Vec::new();
    for file in names {
        let disabled = file.ends_with(".jar.disabled");
        let base = if disabled { file[..file.len() - ".disabled".len()].to_string() } else { file.clone() };
        if let Some(scan) = scan_jar(&mods_dir.join(&file), &base, !disabled) {
            out.push(scan);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// mods.toml / neoforge.mods.toml
// ---------------------------------------------------------------------------

fn resolve_version_placeholder(version: &str, manifest: &HashMap<String, String>) -> String {
    if version.contains("${file.jarVersion}") {
        let resolved = manifest.get("implementation-version").map(|v| v.trim()).unwrap_or("");
        version.replace("${file.jarVersion}", resolved)
    } else {
        version.to_string()
    }
}

/// Parse `META-INF/mods.toml` / `META-INF/neoforge.mods.toml` into `[[mods]]`
/// entries with their `[[dependencies.<modId>]]` blocks.
pub fn parse_mods_toml(text: &str, manifest: &HashMap<String, String>) -> Vec<ForgeModInfo> {
    let Ok(table) = toml::from_str::<toml::Table>(text) else { return Vec::new() };
    let Some(mods_arr) = table.get("mods").and_then(toml::Value::as_array) else { return Vec::new() };
    let deps_table = table.get("dependencies").and_then(toml::Value::as_table);
    let str_of = |t: &toml::Table, key: &str| t.get(key).and_then(toml::Value::as_str).unwrap_or("").trim().to_string();

    let mut out: Vec<ForgeModInfo> = Vec::new();
    for mv in mods_arr {
        let Some(m) = mv.as_table() else { continue };
        let mod_id = str_of(m, "modId");
        if mod_id.is_empty() {
            continue;
        }
        let name = match str_of(m, "displayName") {
            n if n.is_empty() => mod_id.clone(),
            n => n,
        };
        let version = resolve_version_placeholder(&str_of(m, "version"), manifest);
        let client_side_only = m.get("clientSideOnly").and_then(toml::Value::as_bool).unwrap_or(false);

        let mut deps: Vec<ModDep> = Vec::new();
        if let Some(list) = deps_table.and_then(|t| t.get(&mod_id)).and_then(toml::Value::as_array) {
            for dv in list {
                let Some(d) = dv.as_table() else { continue };
                let target = str_of(d, "modId");
                if target.is_empty() {
                    continue;
                }
                let dep_type = match DepType::parse(&str_of(d, "type")) {
                    Some(t) => t,
                    None if d.get("mandatory").and_then(toml::Value::as_bool) == Some(true) => DepType::Required,
                    None => DepType::Optional,
                };
                let side = match str_of(d, "side") {
                    s if s.is_empty() => "BOTH".to_string(),
                    s => s.to_ascii_uppercase(),
                };
                deps.push(ModDep { mod_id: target, dep_type, version_range: str_of(d, "versionRange"), side });
            }
        }
        out.push(ForgeModInfo { mod_id, name, version, client_side_only, deps });
    }
    out
}

// ---------------------------------------------------------------------------
// mcmod.info (legacy Forge)
// ---------------------------------------------------------------------------

/// Remove commas that directly precede a `]` or `}` (outside string literals);
/// real-world `mcmod.info` files carry them.
fn strip_trailing_commas(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_str = false;
    let mut escape = false;
    while let Some(c) = chars.next() {
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            out.push(c);
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            ',' => {
                let next_significant = chars.clone().find(|n| !n.is_whitespace());
                if !matches!(next_significant, Some(']') | Some('}')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Parse `mcmod.info`: a bare JSON array, `{"modList": [...]}`, or a single
/// object. Tolerates a BOM and trailing commas.
pub fn parse_mcmod_info(raw: &str) -> Vec<LegacyModInfo> {
    let cleaned = strip_trailing_commas(raw.trim_start_matches('\u{feff}'));
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&cleaned) else { return Vec::new() };
    let items: Vec<&serde_json::Value> = match &v {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(_) => match v.get("modList").and_then(serde_json::Value::as_array) {
            Some(a) => a.iter().collect(),
            None => vec![&v],
        },
        _ => return Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|item| {
            let mod_id = item.get("modid").and_then(serde_json::Value::as_str)?.trim().to_string();
            if mod_id.is_empty() {
                return None;
            }
            let str_of = |key: &str| item.get(key).and_then(serde_json::Value::as_str).unwrap_or("").trim().to_string();
            let name = match str_of("name") {
                n if n.is_empty() => mod_id.clone(),
                n => n,
            };
            let mut deps: Vec<String> = Vec::new();
            for key in ["requiredMods", "dependencies"] {
                if let Some(arr) = item.get(key).and_then(serde_json::Value::as_array) {
                    deps.extend(
                        arr.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    );
                }
            }
            Some(LegacyModInfo { mod_id, name, version: str_of("version"), mc_version: str_of("mcversion"), deps })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Maven-ish version comparison & ranges
// ---------------------------------------------------------------------------

/// Rank of a Maven qualifier: alpha < beta < snapshot < rc < release < patch.
/// Unknown qualifiers sort after the release they follow.
fn qualifier_rank(q: &str) -> (u8, String) {
    let l = q.trim_start_matches('-').to_ascii_lowercase();
    if l.is_empty() {
        return (6, String::new());
    }
    for (name, rank) in [
        ("alpha", 1u8),
        ("beta", 2),
        ("snapshot", 3),
        ("milestone", 4),
        ("pre", 4),
        ("rc", 4),
        ("cr", 4),
        ("ga", 6),
        ("final", 6),
        ("release", 6),
        ("sp", 8),
    ] {
        if l == name {
            return (rank, l);
        }
        if let Some(rest) = l.strip_prefix(name) {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return (rank, l);
            }
        }
    }
    (7, l)
}

/// Split `1.21.1-beta` into numeric-ish segments and a qualifier. `_` acts as
/// `.`; `+` build metadata is ignored.
fn split_version(v: &str) -> (Vec<String>, String) {
    let s = v.trim();
    let (core, qual) = s.split_once('-').unwrap_or((s, ""));
    let core = core.split_once('+').map(|(a, _)| a).unwrap_or(core);
    let segs: Vec<String> = core.split(['.', '_']).filter(|p| !p.is_empty()).map(str::to_string).collect();
    (segs, qual.to_string())
}

/// Maven-style comparison: numeric segments first (missing = 0), then the
/// qualifier (`1.0-beta` < `1.0` < `1.0-hotfix`).
pub fn maven_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (sa, qa) = split_version(a);
    let (sb, qb) = split_version(b);
    for i in 0..sa.len().max(sb.len()) {
        let x = sa.get(i).map(String::as_str).unwrap_or("0");
        let y = sb.get(i).map(String::as_str).unwrap_or("0");
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(nx), Ok(ny)) => nx.cmp(&ny),
            // Numeric segments sort before string ones, as in Maven.
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    let (ra, ta) = qualifier_rank(&qa);
    let (rb, tb) = qualifier_rank(&qb);
    ra.cmp(&rb).then_with(|| ta.cmp(&tb))
}

/// One `[lo,hi]`-style interval. `None` bounds are open-ended.
#[derive(Debug, Clone, PartialEq)]
struct Interval {
    lo: Option<String>,
    lo_incl: bool,
    hi: Option<String>,
    hi_incl: bool,
    /// Exact `[1.21.1]`: both bounds present, equal and inclusive.
    exact: bool,
}

/// A parsed Maven range.
#[derive(Debug, Clone, PartialEq)]
enum MavenRange {
    /// Union of intervals (`[1.0,2.0),[3.0,)`); matches if any one does.
    Union(Vec<Interval>),
    /// Bare version (`1.0`): a soft recommendation that any version satisfies.
    Soft,
}

/// Parse a Maven version range. `None` when a bracketed part is malformed;
/// callers treat that as "cannot verify" and allow the jar.
fn parse_maven_range(raw: &str) -> Option<MavenRange> {
    let s = raw.trim();
    if s.is_empty() || s == "*" {
        return Some(MavenRange::Union(vec![Interval {
            lo: None,
            lo_incl: true,
            hi: None,
            hi_incl: true,
            exact: false,
        }]));
    }
    if !s.starts_with(['[', '(']) {
        return Some(MavenRange::Soft);
    }
    // Split unions on top-level commas only; bounds keep theirs.
    let mut parts: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);

    let mut intervals: Vec<Interval> = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.len() < 2 || !p.starts_with(['[', '(']) || !p.ends_with([']', ')']) {
            return None;
        }
        let lo_incl = p.starts_with('[');
        let hi_incl = p.ends_with(']');
        let inner = &p[1..p.len() - 1];
        let (lo, hi) = match inner.split_once(',') {
            Some((l, h)) => {
                let l = l.trim();
                let h = h.trim();
                ((!l.is_empty()).then(|| l.to_string()), (!h.is_empty()).then(|| h.to_string()))
            }
            None => {
                let v = inner.trim();
                if v.is_empty() {
                    (None, None)
                } else {
                    (Some(v.to_string()), Some(v.to_string()))
                }
            }
        };
        let exact = lo.is_some() && lo == hi && lo_incl && hi_incl;
        intervals.push(Interval { lo, lo_incl, hi, hi_incl, exact });
    }
    Some(MavenRange::Union(intervals))
}

/// Whether `version` satisfies the range. Unparseable ranges and unversioned
/// providers (empty or unresolved `${...}` placeholders) are allowed:
/// blocking on unverifiable data is worse than letting a pair through.
pub fn maven_range_satisfies(range: &str, version: &str) -> bool {
    use std::cmp::Ordering;
    if version.trim().is_empty() || version.contains("${") {
        return true;
    }
    match parse_maven_range(range) {
        None | Some(MavenRange::Soft) => true,
        Some(MavenRange::Union(intervals)) => intervals.iter().any(|iv| {
            if iv.exact {
                return maven_cmp(version, iv.lo.as_deref().unwrap_or("")) == Ordering::Equal;
            }
            let lo_ok = iv.lo.as_ref().is_none_or(|l| {
                let o = maven_cmp(version, l);
                o == Ordering::Greater || (iv.lo_incl && o == Ordering::Equal)
            });
            let hi_ok = iv.hi.as_ref().is_none_or(|h| {
                let o = maven_cmp(version, h);
                o == Ordering::Less || (iv.hi_incl && o == Ordering::Equal)
            });
            lo_ok && hi_ok
        }),
    }
}

// ---------------------------------------------------------------------------
// Compat evaluation (Forge / NeoForge / legacy)
// ---------------------------------------------------------------------------

/// The jar that provides a mod id, for fix targets and version checks.
struct ProvRef {
    file: String,
    name: String,
    version: String,
}

/// `(mod_id, name, version, environment)` for every mod a jar declares.
/// `environment` uses the Fabric vocabulary (`*`, `client`, `server`).
fn descriptor_mods(j: &JarScan) -> Vec<(&str, &str, &str, String)> {
    let mut out: Vec<(&str, &str, &str, String)> = Vec::new();
    for m in &j.forge_mods {
        let side = m.deps.iter().find(|d| d.mod_id == "minecraft").map(|d| d.side.as_str()).unwrap_or("BOTH");
        let env = match side {
            "SERVER" => "server",
            "CLIENT" => "client",
            _ if m.client_side_only => "client",
            _ => "*",
        };
        out.push((&m.mod_id, &m.name, &m.version, env.to_string()));
    }
    for m in &j.legacy_mods {
        out.push((&m.mod_id, &m.name, &m.version, "*".to_string()));
    }
    if out.is_empty() {
        if let Some(f) = &j.fabric {
            out.push((&f.id, &f.name, &f.version, f.environment.clone()));
        }
    }
    out
}

fn loader_display(loader: Loader) -> &'static str {
    match loader {
        Loader::Fabric => "Fabric",
        Loader::Neoforge => "NeoForge",
        Loader::Forge => "Forge",
        Loader::Vanilla => "Vanilla",
    }
}

/// `WrongLoader` for every enabled jar whose descriptors do not include
/// `instance_loader`. Unknown jars (no descriptor at all) are never flagged.
/// Fix: switch version when the mod id is a known Modrinth project, else
/// disable.
pub fn wrong_loader_issues(jars: &[JarScan], instance_loader: Loader) -> Vec<ModCompatIssue> {
    jars.iter()
        .filter(|j| j.enabled)
        .filter(|j| !j.kinds.iter().all(|k| *k == JarLoaderKind::Unknown))
        .filter(|j| !jar_accepts_loader(&j.kinds, instance_loader))
        .map(|j| {
            let mod_id = j.primary_id().unwrap_or_default().to_string();
            let fix = if known_modrinth_for_id(&mod_id).is_some() {
                CompatFix::ChangeVersion { filename: j.filename.clone() }
            } else {
                CompatFix::Disable { filename: j.filename.clone() }
            };
            ModCompatIssue {
                filename: j.filename.clone(),
                mod_id,
                mod_name: j.primary_name().unwrap_or(j.filename.as_str()).to_string(),
                severity: CompatSeverity::Error,
                kind: CompatIssueKind::WrongLoader,
                target_id: instance_loader.as_str().into(),
                target_name: loader_display(instance_loader).into(),
                target_filename: None,
                // The message reads: "this mod is for {required}, but this
                // instance uses {actual}".
                required: j.loader_label(),
                actual: Some(loader_display(instance_loader).into()),
                fix: Some(fix),
                reason: None,
            }
        })
        .collect()
}

/// Evaluate compatibility for a Forge or NeoForge instance over scanned jars.
pub fn evaluate_loader_compat(env: &LoaderEnv, jars: &[JarScan]) -> ModCompatReport {
    let mut jar_info: IndexMap<String, JarModInfo> = IndexMap::new();
    let mut issues: Vec<ModCompatIssue> = wrong_loader_issues(jars, env.loader);

    // Mods of jars built for another loader are not evaluated: they are the
    // WrongLoader issue, and their metadata would only add noise.
    let accepted: Vec<&JarScan> = jars.iter().filter(|j| jar_accepts_loader(&j.kinds, env.loader)).collect();

    for j in jars {
        if let Some((id, name, version, environment)) = descriptor_mods(j).into_iter().next() {
            jar_info.insert(
                j.filename.clone(),
                JarModInfo {
                    mod_id: id.to_string(),
                    name: name.to_string(),
                    version: version.to_string(),
                    environment,
                },
            );
        }
    }

    // Enabled jars provide mods; the newest version of an id wins. Disabled
    // providers are kept apart so `Enable` / `UpdateMod` fixes can name them.
    let mut provided: HashMap<String, ProvRef> = HashMap::new();
    let mut disabled_provided: HashMap<String, ProvRef> = HashMap::new();
    let mut by_id: IndexMap<String, Vec<(&JarScan, String, String)>> = IndexMap::new();
    for &j in &accepted {
        for (id, name, version, _) in descriptor_mods(j) {
            if id.is_empty() {
                continue;
            }
            let dest = if j.enabled { &mut provided } else { &mut disabled_provided };
            let newer = dest.get(id).is_none_or(|e| maven_cmp(version, &e.version) == std::cmp::Ordering::Greater);
            if newer {
                dest.insert(
                    id.to_string(),
                    ProvRef { file: j.filename.clone(), name: name.to_string(), version: version.to_string() },
                );
            }
            if j.enabled {
                by_id.entry(id.to_string()).or_default().push((j, name.to_string(), version.to_string()));
            }
        }
    }

    // Duplicate mod ids across enabled jars: keep the newest, block the rest.
    for (mod_id, mut group) in by_id {
        group.dedup_by(|a, b| a.0.filename == b.0.filename);
        if group.len() <= 1 {
            continue;
        }
        group.sort_by(|a, b| maven_cmp(&b.2, &a.2).then_with(|| a.0.filename.cmp(&b.0.filename)));
        let (kept_jar, kept_name, kept_ver) = group[0].clone();
        for (jar, name, _) in &group[1..] {
            issues.push(ModCompatIssue {
                filename: jar.filename.clone(),
                mod_id: mod_id.clone(),
                mod_name: name.clone(),
                severity: CompatSeverity::Error,
                kind: CompatIssueKind::DuplicateMod,
                target_id: mod_id.clone(),
                target_name: kept_name.clone(),
                target_filename: Some(kept_jar.filename.clone()),
                required: String::new(),
                actual: Some(kept_ver.clone()),
                fix: Some(CompatFix::Disable { filename: jar.filename.clone() }),
                reason: None,
            });
        }
    }

    let loader_dep_id = if env.loader == Loader::Neoforge { "neoforge" } else { "forge" };
    let loader_name = loader_display(env.loader);

    for j in accepted.iter().filter(|j| j.enabled) {
        for m in &j.forge_mods {
            for d in &m.deps {
                check_forge_dep(&mut issues, env, loader_dep_id, loader_name, &provided, &disabled_provided, j, m, d);
            }
        }
        for m in &j.legacy_mods {
            check_legacy_mod(&mut issues, env, &provided, &disabled_provided, j, m);
        }
    }

    let error_count = issues.iter().filter(|i| i.severity == CompatSeverity::Error).count();
    let warning_count = issues.iter().filter(|i| i.severity == CompatSeverity::Warning).count();
    ModCompatReport {
        supported: true,
        mc_version: env.mc_version.clone(),
        loader_version: env.loader_version.clone(),
        java_major: env.java_major,
        error_count,
        warning_count,
        issues,
        jar_info,
    }
}

/// One `[[dependencies.<modId>]]` entry of a Forge/NeoForge mod.
#[allow(clippy::too_many_arguments)]
fn check_forge_dep(
    issues: &mut Vec<ModCompatIssue>,
    env: &LoaderEnv,
    loader_dep_id: &str,
    loader_name: &str,
    provided: &HashMap<String, ProvRef>,
    disabled_provided: &HashMap<String, ProvRef>,
    j: &JarScan,
    m: &ForgeModInfo,
    d: &ModDep,
) {
    let issue = |severity, kind, target_id: &str, target_name: String, target_filename, actual, fix| ModCompatIssue {
        filename: j.filename.clone(),
        mod_id: m.mod_id.clone(),
        mod_name: m.name.clone(),
        severity,
        kind,
        target_id: target_id.into(),
        target_name,
        target_filename,
        required: d.version_range.clone(),
        actual,
        fix,
        reason: None,
    };
    let disable_self = Some(CompatFix::Disable { filename: j.filename.clone() });

    match d.mod_id.as_str() {
        "minecraft" => {
            if d.side == "SERVER" {
                issues.push(issue(
                    CompatSeverity::Warning,
                    CompatIssueKind::ServerOnly,
                    "minecraft",
                    "client".into(),
                    None,
                    None,
                    disable_self,
                ));
                return;
            }
            if !maven_range_satisfies(&d.version_range, &env.mc_version) {
                issues.push(issue(
                    CompatSeverity::Error,
                    CompatIssueKind::MinecraftVersion,
                    "minecraft",
                    "Minecraft".into(),
                    None,
                    Some(env.mc_version.clone()),
                    Some(CompatFix::ChangeVersion { filename: j.filename.clone() }),
                ));
            }
        }
        id if id == loader_dep_id => {
            let Some(lv) = &env.loader_version else { return };
            if !maven_range_satisfies(&d.version_range, lv) {
                issues.push(issue(
                    CompatSeverity::Error,
                    CompatIssueKind::LoaderVersion,
                    id,
                    loader_name.into(),
                    None,
                    Some(lv.clone()),
                    disable_self,
                ));
            }
        }
        other_id => {
            let prov = provided.get(other_id);
            match d.dep_type {
                DepType::Incompatible | DepType::Discouraged => {
                    let Some(p) = prov else { return };
                    if p.file == j.filename || !maven_range_satisfies(&d.version_range, &p.version) {
                        return;
                    }
                    let (severity, kind) = if d.dep_type == DepType::Incompatible {
                        (CompatSeverity::Error, CompatIssueKind::BreaksMod)
                    } else {
                        (CompatSeverity::Warning, CompatIssueKind::ConflictsMod)
                    };
                    issues.push(issue(
                        severity,
                        kind,
                        other_id,
                        p.name.clone(),
                        Some(p.file.clone()),
                        Some(clean_version_display(&p.version).to_string()),
                        disable_self,
                    ));
                }
                DepType::Required | DepType::Optional => {
                    let severity =
                        if d.dep_type == DepType::Required { CompatSeverity::Error } else { CompatSeverity::Warning };
                    if let Some(p) = prov {
                        if !maven_range_satisfies(&d.version_range, &p.version) {
                            issues.push(issue(
                                severity,
                                CompatIssueKind::IncompatibleDep,
                                other_id,
                                p.name.clone(),
                                Some(p.file.clone()),
                                Some(clean_version_display(&p.version).to_string()),
                                Some(CompatFix::UpdateMod {
                                    filename: p.file.clone(),
                                    target_id: other_id.into(),
                                    target_name: p.name.clone(),
                                    required: d.version_range.clone(),
                                }),
                            ));
                        }
                    } else if let Some(p) = disabled_provided.get(other_id) {
                        let fix = if maven_range_satisfies(&d.version_range, &p.version) {
                            CompatFix::Enable { filename: p.file.clone(), name: p.name.clone() }
                        } else {
                            CompatFix::UpdateMod {
                                filename: p.file.clone(),
                                target_id: other_id.into(),
                                target_name: p.name.clone(),
                                required: d.version_range.clone(),
                            }
                        };
                        issues.push(issue(
                            severity,
                            CompatIssueKind::DisabledDep,
                            other_id,
                            p.name.clone(),
                            Some(p.file.clone()),
                            Some(clean_version_display(&p.version).to_string()),
                            Some(fix),
                        ));
                    } else if d.dep_type == DepType::Required {
                        let name = known_modrinth_for_id(other_id)
                            .map(|(_, n)| n.to_string())
                            .unwrap_or_else(|| other_id.to_string());
                        issues.push(issue(
                            CompatSeverity::Error,
                            CompatIssueKind::MissingDep,
                            other_id,
                            name.clone(),
                            None,
                            None,
                            Some(CompatFix::InstallMod { mod_id: other_id.into(), name }),
                        ));
                    }
                }
            }
        }
    }
}

/// One `mcmod.info` mod (legacy Forge): mcversion match and `requiredMods`.
fn check_legacy_mod(
    issues: &mut Vec<ModCompatIssue>,
    env: &LoaderEnv,
    provided: &HashMap<String, ProvRef>,
    disabled_provided: &HashMap<String, ProvRef>,
    j: &JarScan,
    m: &LegacyModInfo,
) {
    let issue =
        |kind, target_id: &str, target_name: String, target_filename, required: String, actual, fix| ModCompatIssue {
            filename: j.filename.clone(),
            mod_id: m.mod_id.clone(),
            mod_name: m.name.clone(),
            severity: CompatSeverity::Error,
            kind,
            target_id: target_id.into(),
            target_name,
            target_filename,
            required,
            actual,
            fix,
            reason: None,
        };

    if !m.mc_version.is_empty() && !mc_version_matches(&m.mc_version, &env.mc_version) {
        issues.push(issue(
            CompatIssueKind::MinecraftVersion,
            "minecraft",
            "Minecraft".into(),
            None,
            m.mc_version.clone(),
            Some(env.mc_version.clone()),
            Some(CompatFix::Disable { filename: j.filename.clone() }),
        ));
    }

    for dep in &m.deps {
        let (dep_id, range) = match dep.split_once('@') {
            Some((id, r)) => (id.trim(), r.trim().to_string()),
            None => (dep.trim(), String::new()),
        };
        if dep_id.is_empty() || dep_id == m.mod_id || LOADER_IDS.contains(&dep_id) {
            continue;
        }
        if let Some(p) = provided.get(dep_id) {
            if !maven_range_satisfies(&range, &p.version) {
                issues.push(issue(
                    CompatIssueKind::IncompatibleDep,
                    dep_id,
                    p.name.clone(),
                    Some(p.file.clone()),
                    range.clone(),
                    Some(clean_version_display(&p.version).to_string()),
                    Some(CompatFix::UpdateMod {
                        filename: p.file.clone(),
                        target_id: dep_id.into(),
                        target_name: p.name.clone(),
                        required: range,
                    }),
                ));
            }
        } else if let Some(p) = disabled_provided.get(dep_id) {
            issues.push(issue(
                CompatIssueKind::DisabledDep,
                dep_id,
                p.name.clone(),
                Some(p.file.clone()),
                range,
                Some(clean_version_display(&p.version).to_string()),
                Some(CompatFix::Enable { filename: p.file.clone(), name: p.name.clone() }),
            ));
        } else {
            issues.push(issue(
                CompatIssueKind::MissingDep,
                dep_id,
                dep_id.into(),
                None,
                range,
                None,
                Some(CompatFix::InstallMod { mod_id: dep_id.into(), name: dep_id.into() }),
            ));
        }
    }
}

/// Loose legacy `mcversion` check: equal, or one is a dotted prefix of the
/// other (`1.8` matches `1.8.9`, not `1.12.2`).
fn mc_version_matches(declared: &str, instance: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.').to_string();
    let d = norm(declared);
    let i = norm(instance);
    if d.is_empty() || i.is_empty() {
        return true;
    }
    d == i || i.starts_with(&format!("{d}.")) || d.starts_with(&format!("{i}."))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use serde_json::json;
    use std::io::Write;

    fn write_jar(dir: &Path, name: &str, entries: &[(&str, &str)]) {
        let f = std::fs::File::create(dir.join(name)).unwrap();
        let mut w = zip::ZipWriter::new(f);
        for (n, body) in entries {
            w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    fn manifest(impl_version: &str) -> String {
        format!("Manifest-Version: 1.0\r\nImplementation-Version: {impl_version}\r\n\r\n")
    }

    #[test]
    fn parses_neoforge_mods_toml_and_maven_ranges() {
        let toml = r#"
modLoader = "javafml"
loaderVersion = "[21.1,)"
license = "MIT"

[[mods]]
    modId = "coolmod"
    version = "${file.jarVersion}"
    displayName = "Cool Mod"
    clientSideOnly = true

[[dependencies.coolmod]]
    modId = "neoforge"
    type = "required"
    versionRange = "[21.1,)"
    side = "BOTH"

[[dependencies.coolmod]]
    modId = "minecraft"
    type = "required"
    versionRange = "[1.21.1,1.22)"
    side = "CLIENT"

[[dependencies.coolmod]]
    modId = "badmod"
    type = "incompatible"
    versionRange = "[1.0,2.0)"
"#;
        let mut m = HashMap::new();
        m.insert("implementation-version".to_string(), "3.2.1".to_string());
        let mods = parse_mods_toml(toml, &m);
        assert_eq!(mods.len(), 1);
        let cm = &mods[0];
        assert_eq!(cm.mod_id, "coolmod");
        assert_eq!(cm.name, "Cool Mod");
        assert_eq!(cm.version, "3.2.1");
        assert!(cm.client_side_only);
        assert_eq!(cm.deps.len(), 3);
        assert_eq!(cm.deps[0].mod_id, "neoforge");
        assert_eq!(cm.deps[0].dep_type, DepType::Required);
        assert_eq!(cm.deps[1].version_range, "[1.21.1,1.22)");
        assert_eq!(cm.deps[1].side, "CLIENT");
        assert_eq!(cm.deps[2].dep_type, DepType::Incompatible);

        // Legacy `mandatory = true` form.
        let legacy = concat!(
            "[[mods]]\nmodId = \"oldmod\"\nversion = \"1.0\"\n\n",
            "[[dependencies.oldmod]]\nmodId = \"minecraft\"\nmandatory = true\nversionRange = \"[1.20,)\"\n",
        );
        let mods = parse_mods_toml(legacy, &HashMap::new());
        assert_eq!(mods[0].deps[0].dep_type, DepType::Required);

        // Maven range semantics.
        assert!(maven_range_satisfies("[1.21.1,1.22)", "1.21.5"));
        assert!(!maven_range_satisfies("[1.21.1,1.22)", "1.22"));
        assert!(maven_range_satisfies("[1.21.1,1.22)", "1.21.1"));
        assert!(!maven_range_satisfies("[1.21.1,1.22)", "1.21"));
        assert!(maven_range_satisfies("[21.1.0,)", "21.4"));
        assert!(!maven_range_satisfies("[21.1.0,)", "20.9"));
        assert!(maven_range_satisfies("(,1.20.4]", "1.20.4"));
        assert!(!maven_range_satisfies("(,1.20.4]", "1.20.5"));
        assert!(maven_range_satisfies("[1.21.1]", "1.21.1"));
        assert!(!maven_range_satisfies("[1.21.1]", "1.21.2"));
        assert!(maven_range_satisfies("[1.0,2.0),[3.0,)", "3.5"));
        assert!(maven_range_satisfies("[1.0,2.0),[3.0,)", "1.5"));
        assert!(!maven_range_satisfies("[1.0,2.0),[3.0,)", "2.5"));
        // Bare versions are soft requirements.
        assert!(maven_range_satisfies("1.0", "9.9"));
        // Qualifier ordering.
        assert_eq!(maven_cmp("1.0-alpha", "1.0"), std::cmp::Ordering::Less);
        assert_eq!(maven_cmp("1.0-SNAPSHOT", "1.0"), std::cmp::Ordering::Less);
        assert_eq!(maven_cmp("1.0", "1.0.1"), std::cmp::Ordering::Less);
        assert_eq!(maven_cmp("1.0.0", "1.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn parses_legacy_mcmod_info_1_8_9() {
        let raw = concat!(
            "\u{feff}[\n  {\n    \"modid\": \"optifine\",\n    \"name\": \"OptiFine\",\n",
            "    \"mcversion\": \"1.8.9\",\n    \"version\": \"HD_U_J1\",\n",
            "    \"requiredMods\": [\"forge\",],\n    \"dependencies\": [],\n  },\n]",
        );
        let mods = parse_mcmod_info(raw);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].mod_id, "optifine");
        assert_eq!(mods[0].name, "OptiFine");
        assert_eq!(mods[0].mc_version, "1.8.9");
        assert_eq!(mods[0].deps, vec!["forge".to_string()]);

        let wrapped = concat!(
            r#"{"modList": [{"modid": "patcher", "name": "Patcher", "version": "1.8.9", "#,
            r#""mcversion": "1.8.9", "requiredMods": ["sk1er-core@1.0"]}]}"#,
        );
        let mods = parse_mcmod_info(wrapped);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].deps, vec!["sk1er-core@1.0".to_string()]);
        assert!(mc_version_matches("1.8", "1.8.9"));
        assert!(mc_version_matches("1.8.9", "1.8.9"));
        assert!(!mc_version_matches("1.12.2", "1.8.9"));
    }

    #[test]
    fn detects_wrong_loader_jar_in_fabric_and_neoforge() {
        let (dir, e) = temp_engine();

        // A Forge jar dropped into a Fabric instance.
        let inst = e.paths().instance_dir("fab");
        let mods_dir = inst.join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::write(
            inst.join("instance.json"),
            json!({ "name": "fab", "version": "1.21.1", "loader": "fabric" }).to_string(),
        )
        .unwrap();
        write_jar(
            &mods_dir,
            "forgeonly-1.0.jar",
            &[
                ("META-INF/mods.toml", "[[mods]]\nmodId = \"forgeonly\"\nversion = \"1.0\"\n"),
                ("META-INF/MANIFEST.MF", &manifest("1.0")),
            ],
        );
        write_jar(
            &mods_dir,
            "sodium-fabric.jar",
            &[("fabric.mod.json", r#"{"id":"sodium","name":"Sodium","version":"0.9.2","environment":"client"}"#)],
        );
        let report = crate::fabric_compat::check_instance_mods_compat_sync(&e, "fab").unwrap();
        assert!(report.supported);
        let wrong: Vec<&ModCompatIssue> =
            report.issues.iter().filter(|i| i.kind == CompatIssueKind::WrongLoader).collect();
        assert_eq!(wrong.len(), 1, "issues: {:?}", report.issues);
        assert_eq!(wrong[0].filename, "forgeonly-1.0.jar");
        assert_eq!(wrong[0].mod_id, "forgeonly");
        assert_eq!(wrong[0].severity, CompatSeverity::Error);
        assert!(matches!(wrong[0].fix, Some(CompatFix::Disable { .. })));

        // A Fabric jar dropped into a NeoForge instance, next to a valid NeoForge jar.
        let inst = e.paths().instance_dir("neo");
        let mods_dir = inst.join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::write(
            inst.join("instance.json"),
            json!({ "name": "neo", "version": "1.21.1", "loader": "neoforge", "loader_version": "21.1.180" })
                .to_string(),
        )
        .unwrap();
        write_jar(
            &mods_dir,
            "fabriconly-1.0.jar",
            &[("fabric.mod.json", r#"{"id":"fabriconly","name":"FabricOnly","version":"1.0","environment":"*"}"#)],
        );
        write_jar(
            &mods_dir,
            "neomod-2.0.jar",
            &[
                ("META-INF/neoforge.mods.toml", "[[mods]]\nmodId = \"neomod\"\nversion = \"2.0\"\n"),
                ("META-INF/MANIFEST.MF", &manifest("2.0")),
            ],
        );
        let report = crate::fabric_compat::check_instance_mods_compat_sync(&e, "neo").unwrap();
        assert!(report.supported);
        let wrong: Vec<&ModCompatIssue> =
            report.issues.iter().filter(|i| i.kind == CompatIssueKind::WrongLoader).collect();
        assert_eq!(wrong.len(), 1, "issues: {:?}", report.issues);
        assert_eq!(wrong[0].filename, "fabriconly-1.0.jar");
        assert_eq!(wrong[0].mod_id, "fabriconly");
        assert_eq!(wrong[0].severity, CompatSeverity::Error);
        assert!(report.issues.iter().all(|i| i.filename != "neomod-2.0.jar" || i.kind != CompatIssueKind::WrongLoader));
        drop(dir);
    }
}
