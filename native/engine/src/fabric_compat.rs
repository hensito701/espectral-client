//! Fabric mod metadata (`fabric.mod.json` + nested jar-in-jar) reader,
//! Fabric Loader version / predicate evaluator, and pre-flight compatibility
//! checker.
//!
//! Runs 100% locally (no network): reads `fabric.mod.json` (and recursively
//! any embedded `jars` inside enabled mods) to catch dependency, version,
//! Minecraft/Loader/Java and `breaks`/`conflicts` problems BEFORE launching
//! the game, matching Fabric Loader's own `SemanticVersionImpl` and
//! `VersionPredicateParser` semantics.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::Engine;
use crate::error::EngineResult;
use crate::instances::effective_mods_dir;
use crate::types::{InstanceMeta, Loader};

const MAX_META_BYTES: u64 = 1 << 20; // 1 MiB cap for fabric.mod.json
const MAX_NESTED_JAR_BYTES: u64 = 16 << 20; // 16 MiB cap per jar-in-jar
const MAX_NESTED_DEPTH: usize = 4;
const MAX_NESTED_COUNT: usize = 128;

// ---------------------------------------------------------------------------
// Fabric Semantic Version
// ---------------------------------------------------------------------------

/// Fabric Loader version: semantic (`1.21.11`, `0.9.2+mc26.3`, `26.3-rc.1`,
/// sentinel `26.3-`) or opaque string fallback (`25w11a`, `1.21.11_Fabric`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FabricVersion {
    Semantic {
        components: Vec<u64>,
        /// `None` = release; `Some("")` = Fabric's `MIN_PRERELEASE` sentinel
        /// (smaller than any real prerelease, used by `~`, `^`, `.x`, `>=V-`);
        /// `Some(s)` = prerelease tag (e.g. `"beta.1"`).
        prerelease: Option<String>,
        /// Build metadata (`+mc26.3`) — ignored during version comparisons.
        build: Option<String>,
    },
    String(String),
}

impl FabricVersion {
    pub fn parse(raw: &str) -> Self {
        let s = raw.trim();
        if s.is_empty() {
            return FabricVersion::String(String::new());
        }
        let (without_build, build) = match s.split_once('+') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (s, None),
        };
        let (core, prerelease) = match without_build.split_once('-') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (without_build, None),
        };
        if let Some(components) = parse_numeric_components(core) {
            FabricVersion::Semantic { components, prerelease, build }
        } else {
            FabricVersion::String(s.to_string())
        }
    }

    pub fn semantic_min(components: Vec<u64>) -> Self {
        FabricVersion::Semantic { components, prerelease: Some(String::new()), build: None }
    }

    /// Compare two versions using Fabric Loader's `SemanticVersionImpl` rules.
    /// Returns `None` when a semantic version is compared with an opaque string.
    pub fn cmp_fabric(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (
                FabricVersion::Semantic { components: ac, prerelease: ap, .. },
                FabricVersion::Semantic { components: bc, prerelease: bp, .. },
            ) => {
                let len = ac.len().max(bc.len());
                for i in 0..len {
                    let av = ac.get(i).copied().unwrap_or(0);
                    let bv = bc.get(i).copied().unwrap_or(0);
                    match av.cmp(&bv) {
                        Ordering::Equal => continue,
                        ord => return Some(ord),
                    }
                }
                Some(cmp_prerelease(ap.as_deref(), bp.as_deref()))
            }
            (FabricVersion::String(a), FabricVersion::String(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }

    /// True if `self` is a dev placeholder like `${version}`.
    pub fn is_placeholder(&self) -> bool {
        matches!(self, FabricVersion::String(s) if s.contains("${"))
    }
}

fn parse_numeric_components(core: &str) -> Option<Vec<u64>> {
    if core.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for part in core.split('.') {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out.push(part.parse::<u64>().ok()?);
    }
    Some(out)
}

fn cmp_prerelease(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater, // release > any prerelease
        (Some(_), None) => Ordering::Less,
        (Some(""), Some("")) => Ordering::Equal,
        (Some(""), Some(_)) => Ordering::Less, // MIN_PRERELEASE < any real prerelease
        (Some(_), Some("")) => Ordering::Greater,
        (Some(a), Some(b)) => {
            let mut ai = a.split('.');
            let mut bi = b.split('.');
            loop {
                match (ai.next(), bi.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(at), Some(bt)) => {
                        let an = at.parse::<u64>().ok();
                        let bn = bt.parse::<u64>().ok();
                        let ord = match (an, bn) {
                            (Some(x), Some(y)) => x.cmp(&y),
                            (Some(_), None) => Ordering::Less, // numeric < alphanumeric (SemVer 2.0)
                            (None, Some(_)) => Ordering::Greater,
                            (None, None) => at.cmp(bt),
                        };
                        if ord != Ordering::Equal {
                            return ord;
                        }
                    }
                }
            }
        }
    }
}

/// Best-effort extraction of a Fabric semantic version from a Modrinth
/// `version_number` string (e.g. `mc26.3-0.9.3-beta.1-fabric` -> `0.9.3-beta.1`,
/// `1.11.7+26.3-fabric` -> `1.11.7`, `9.0.0-fabric` -> `9.0.0`).
pub fn extract_mod_semver(version_number: &str) -> Option<FabricVersion> {
    let raw = version_number.trim();
    if raw.is_empty() {
        return None;
    }
    // Strip leading `mc<ver>[-+_]` prefix (e.g. `mc26.3-0.9.3-beta.1-fabric`).
    let mut s = raw;
    if let Some(rest) = s.strip_prefix("mc").or_else(|| s.strip_prefix("MC")) {
        if let Some(idx) = rest.find(['-', '+', '_']) {
            let mc_part = &rest[..idx];
            if !mc_part.is_empty() && mc_part.chars().all(|c| c.is_ascii_digit() || c == '.') {
                s = &rest[idx + 1..];
            }
        }
    }
    // Strip build metadata `+...` first so `-fabric` inside build doesn't confuse us.
    let s = s.split_once('+').map(|(a, _)| a).unwrap_or(s);
    // Strip trailing loader suffix (`-fabric`, `_fabric`, `-neoforge`, `-forge`, `-quilt`).
    let lower = s.to_ascii_lowercase();
    let mut trimmed = s;
    for suffix in ["-fabric", "_fabric", "-neoforge", "_neoforge", "-forge", "_forge", "-quilt", "_quilt"] {
        if lower.ends_with(suffix) && trimmed.len() > suffix.len() {
            trimmed = &trimmed[..trimmed.len() - suffix.len()];
            break;
        }
    }
    // Also strip leading `v` (e.g. `v1.2.3`).
    let trimmed = trimmed.strip_prefix('v').or_else(|| trimmed.strip_prefix('V')).unwrap_or(trimmed);
    let parsed = FabricVersion::parse(trimmed);
    if matches!(parsed, FabricVersion::Semantic { .. }) {
        Some(parsed)
    } else {
        None
    }
}

/// Strip `+build` metadata for cleaner human display (e.g. `0.9.2+mc26.3` -> `0.9.2`).
pub fn clean_version_display(v: &str) -> &str {
    v.split_once('+').map(|(a, _)| a).unwrap_or(v).trim()
}

/// True for weekly Minecraft snapshots (`24w14a`, `26w01a`) whose Fabric
/// internal semver mapping depends on Mojang release dates.
pub fn is_snapshot_mc(mc: &str) -> bool {
    let b = mc.as_bytes();
    b.len() >= 5
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2] == b'w'
        && b[3].is_ascii_digit()
        && b[4].is_ascii_digit()
}

// ---------------------------------------------------------------------------
// Fabric Version Predicate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    Any,
    Eq(FabricVersion),
    Gt(FabricVersion),
    Gte(FabricVersion),
    Lt(FabricVersion),
    Lte(FabricVersion),
    /// Inclusive lower bound, exclusive upper bound (used by `~`, `^`, `.x`).
    Range { min: FabricVersion, max_excl: FabricVersion },
}

impl Term {
    fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.is_empty() || s == "*" || s.eq_ignore_ascii_case("x") {
            return Some(Term::Any);
        }
        let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
            (">=", r)
        } else if let Some(r) = s.strip_prefix("<=") {
            ("<=", r)
        } else if let Some(r) = s.strip_prefix('>') {
            (">", r)
        } else if let Some(r) = s.strip_prefix('<') {
            ("<", r)
        } else if let Some(r) = s.strip_prefix('=') {
            ("=", r)
        } else if let Some(r) = s.strip_prefix('~') {
            ("~", r)
        } else if let Some(r) = s.strip_prefix('^') {
            ("^", r)
        } else {
            ("", s)
        };
        let rest = rest.trim();
        if rest.is_empty() || rest == "*" || rest.eq_ignore_ascii_case("x") {
            return Some(Term::Any);
        }

        // Check for wildcard components (`0.9.x`, `26.3.X`, `1.21.*`).
        if let Some(prefix) = parse_wildcard_prefix(rest) {
            if prefix.is_empty() {
                return Some(Term::Any);
            }
            let min = FabricVersion::semantic_min(prefix.clone());
            let mut next = prefix;
            if let Some(last) = next.last_mut() {
                *last = last.saturating_add(1);
            }
            let max_excl = FabricVersion::semantic_min(next);
            return match op {
                "" | "=" => Some(Term::Range { min, max_excl }),
                ">=" => Some(Term::Gte(min)),
                ">" => Some(Term::Gte(max_excl)),
                "<=" => Some(Term::Lt(max_excl)),
                "<" => Some(Term::Lt(min)),
                "~" | "^" => Some(Term::Range { min, max_excl }),
                _ => None,
            };
        }

        let v = FabricVersion::parse(rest);
        match op {
            "" | "=" => Some(Term::Eq(v)),
            ">" => Some(Term::Gt(v)),
            ">=" => Some(Term::Gte(v)),
            "<" => Some(Term::Lt(v)),
            "<=" => Some(Term::Lte(v)),
            "~" => {
                // Fabric Loader SAME_TO_NEXT_MINOR: >= V and < [c0, c1 + 1]-
                if let FabricVersion::Semantic { ref components, .. } = v {
                    let c0 = components.first().copied().unwrap_or(0);
                    let c1 = components.get(1).copied().unwrap_or(0);
                    let max_excl = FabricVersion::semantic_min(vec![c0, c1.saturating_add(1)]);
                    Some(Term::Range { min: v, max_excl })
                } else {
                    Some(Term::Eq(v))
                }
            }
            "^" => {
                // Fabric Loader SAME_TO_NEXT_MAJOR: >= V and < [c0 + 1]-
                // (Unlike npm semver, Fabric ALWAYS increments c0 even for 0.x!)
                if let FabricVersion::Semantic { ref components, .. } = v {
                    let c0 = components.first().copied().unwrap_or(0);
                    let max_excl = FabricVersion::semantic_min(vec![c0.saturating_add(1)]);
                    Some(Term::Range { min: v, max_excl })
                } else {
                    Some(Term::Eq(v))
                }
            }
            _ => None,
        }
    }

    fn matches(&self, v: &FabricVersion) -> bool {
        match self {
            Term::Any => true,
            Term::Eq(target) => match (v, target) {
                (FabricVersion::Semantic { .. }, FabricVersion::Semantic { .. }) => {
                    v.cmp_fabric(target) == Some(Ordering::Equal)
                }
                (FabricVersion::String(a), FabricVersion::String(b)) => a == b,
                _ => false,
            },
            Term::Gt(target) => v.cmp_fabric(target) == Some(Ordering::Greater),
            Term::Gte(target) => matches!(v.cmp_fabric(target), Some(Ordering::Greater | Ordering::Equal)),
            Term::Lt(target) => v.cmp_fabric(target) == Some(Ordering::Less),
            Term::Lte(target) => matches!(v.cmp_fabric(target), Some(Ordering::Less | Ordering::Equal)),
            Term::Range { min, max_excl } => {
                matches!(v.cmp_fabric(min), Some(Ordering::Greater | Ordering::Equal))
                    && v.cmp_fabric(max_excl) == Some(Ordering::Less)
            }
        }
    }
}

/// If `s` ends in a wildcard component (`x`, `X`, `*`), return the leading
/// numeric components (`"0.9.x"` -> `Some([0, 9])`, `"26.*"` -> `Some([26])`).
fn parse_wildcard_prefix(s: &str) -> Option<Vec<u64>> {
    // Strip build metadata if any.
    let core = s.split_once('+').map(|(a, _)| a).unwrap_or(s);
    // Wildcards don't carry prereleases in Fabric, strip trailing `-` if present.
    let core = core.strip_suffix('-').unwrap_or(core);
    let parts: Vec<&str> = core.split('.').collect();
    let is_wild = |p: &str| p == "*" || p.eq_ignore_ascii_case("x");
    if !parts.iter().any(|p| is_wild(p)) {
        return None;
    }
    let mut prefix = Vec::new();
    let mut saw_wild = false;
    for part in parts {
        if is_wild(part) {
            saw_wild = true;
            continue;
        }
        if saw_wild {
            // Non-wildcard after wildcard (e.g. `1.x.2`) is not a standard range.
            return None;
        }
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        prefix.push(part.parse::<u64>().ok()?);
    }
    Some(prefix)
}

/// A parsed Fabric version predicate: OR of AND-groups of `Term`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPredicate {
    /// Each inner `Vec<Term>` is an AND-group; outer `Vec` is OR.
    groups: Vec<Vec<Term>>,
    /// Original raw predicate clauses (for display/debugging).
    raw: Vec<String>,
}

impl VersionPredicate {
    pub fn parse_str(s: &str) -> Self {
        Self::parse_list(&[s.to_string()])
    }

    pub fn parse_list(clauses: &[String]) -> Self {
        let mut groups = Vec::new();
        let mut raw = Vec::new();
        for clause in clauses {
            let trimmed = clause.trim();
            if trimmed.is_empty() {
                continue;
            }
            raw.push(trimmed.to_string());
            let terms: Vec<Term> = trimmed.split_ascii_whitespace().filter_map(Term::parse).collect();
            if !terms.is_empty() {
                groups.push(terms);
            }
        }
        if groups.is_empty() {
            groups.push(vec![Term::Any]);
        }
        VersionPredicate { groups, raw }
    }

    pub fn parse_value(v: &Value) -> Option<Self> {
        let clauses = extract_predicate_strings(v)?;
        Some(Self::parse_list(&clauses))
    }

    pub fn matches(&self, version: &FabricVersion) -> bool {
        self.groups.iter().any(|and_group| and_group.iter().all(|t| t.matches(version)))
    }

    pub fn is_any(&self) -> bool {
        self.groups.iter().any(|g| g.len() == 1 && matches!(g[0], Term::Any))
    }

    /// True if every clause is a strict upper bound (`<V` or `<=V`), typical of
    /// `breaks` entries for old versions (e.g. `"iris": "<=1.11.2"`), meaning
    /// updating the target mod can resolve the break!
    pub fn is_upper_bound_only(&self) -> bool {
        !self.is_any()
            && self.groups.iter().all(|g| {
                !g.is_empty() && g.iter().all(|t| matches!(t, Term::Lt(_) | Term::Lte(_)))
            })
    }

    pub fn raw_string(&self) -> String {
        if self.raw.is_empty() {
            "*".to_string()
        } else {
            self.raw.join(" || ")
        }
    }
}

fn extract_predicate_strings(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::String(s) => Some(vec![s.clone()]),
        Value::Array(arr) => {
            let list: Vec<String> = arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            if list.is_empty() {
                None
            } else {
                Some(list)
            }
        }
        _ => None,
    }
}

/// Format a Fabric version predicate for players (Spanish when `es == true`,
/// English otherwise). Examples:
/// - `>=0.9.3` -> `"0.9.3 o más nuevo"` / `"0.9.3 or newer"`
/// - `~1.21.8` -> `"1.21.8+"` / `"1.21.8+"` (or `"1.21.x"` for `1.21.x`)
/// - `<=1.11.2` -> `"1.11.2 o anterior"` / `"1.11.2 or older"`
/// - `26.3.x` -> `"26.3"`
pub fn format_predicate_human(raw: &str, es: bool) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return if es { "cualquier versión".into() } else { "any version".into() };
    }
    if trimmed.contains(" || ") {
        let parts: Vec<String> =
            trimmed.split(" || ").map(|p| format_single_clause(p.trim(), es)).collect();
        let sep = if es { " o " } else { " or " };
        return parts.join(sep);
    }
    format_single_clause(trimmed, es)
}

fn format_single_clause(clause: &str, es: bool) -> String {
    let tokens: Vec<&str> = clause.split_ascii_whitespace().collect();
    if tokens.len() == 1 {
        let t = tokens[0];
        if let Some(v) = t.strip_prefix(">=") {
            let v = clean_pred_ver(v);
            return if es { format!("{v} o más nuevo") } else { format!("{v} or newer") };
        }
        if let Some(v) = t.strip_prefix('>') {
            let v = clean_pred_ver(v);
            return if es { format!("más nuevo que {v}") } else { format!("newer than {v}") };
        }
        if let Some(v) = t.strip_prefix("<=") {
            let v = clean_pred_ver(v);
            return if es { format!("{v} o anterior") } else { format!("{v} or older") };
        }
        if let Some(v) = t.strip_prefix('<') {
            let v = clean_pred_ver(v);
            return if es { format!("anterior a {v}") } else { format!("older than {v}") };
        }
        if let Some(v) = t.strip_prefix('=') {
            return clean_pred_ver(v).to_string();
        }
        if let Some(v) = t.strip_prefix('~') {
            return clean_pred_ver(v).to_string();
        }
        if let Some(v) = t.strip_prefix('^') {
            let v = clean_pred_ver(v);
            return if es { format!("{v} o más nuevo") } else { format!("{v} or newer") };
        }
        return clean_pred_ver(t).to_string();
    }
    if tokens.len() == 2 {
        // Common range like `>=1.21.6 <=1.21.11` or `>=26.1 <27`
        if let (Some(lo), Some(hi)) = (
            tokens[0].strip_prefix(">=").or_else(|| tokens[0].strip_prefix('>')),
            tokens[1].strip_prefix("<=").or_else(|| tokens[1].strip_prefix('<')),
        ) {
            let lo = clean_pred_ver(lo);
            let hi = clean_pred_ver(hi);
            return if tokens[1].starts_with("<=") {
                format!("{lo}–{hi}")
            } else {
                format!("{lo}–<{hi}")
            };
        }
    }
    clause.to_string()
}

fn clean_pred_ver(v: &str) -> &str {
    v.trim().trim_end_matches('-')
}

// ---------------------------------------------------------------------------
// fabric.mod.json scanning (root + nested jar-in-jar)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FabricModMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub environment: String,
    pub provides: Vec<String>,
    pub depends: IndexMap<String, Vec<String>>,
    pub breaks: IndexMap<String, Vec<String>>,
    pub conflicts: IndexMap<String, Vec<String>>,
    pub jars: Vec<String>,
    pub is_generated: bool,
}

#[derive(Debug, Clone)]
pub struct ScannedJar {
    /// Base filename (without `.disabled`).
    pub filename: String,
    pub enabled: bool,
    pub root: FabricModMeta,
    pub nested: Vec<FabricModMeta>,
}

fn parse_dep_map(val: Option<&Value>) -> IndexMap<String, Vec<String>> {
    let mut out = IndexMap::new();
    let Some(obj) = val.and_then(Value::as_object) else { return out };
    for (k, v) in obj {
        if let Some(list) = extract_predicate_strings(v) {
            out.insert(k.clone(), list);
        }
    }
    out
}

fn parse_fabric_mod_json(raw: &str) -> Option<FabricModMeta> {
    let cleaned = raw.trim_start_matches('\u{feff}');
    let v: Value = serde_json::from_str(cleaned).ok()?;
    let id = v.get("id").and_then(Value::as_str)?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let version = v.get("version").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());
    let environment =
        v.get("environment").and_then(Value::as_str).map(str::trim).unwrap_or("*").to_ascii_lowercase();
    let provides = v
        .get("provides")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let jars = v
        .get("jars")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("file").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let is_generated = v
        .get("custom")
        .and_then(|c| c.get("fabric-loom:generated"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(FabricModMeta {
        id,
        name,
        version,
        environment,
        provides,
        depends: parse_dep_map(v.get("depends")),
        breaks: parse_dep_map(v.get("breaks")),
        conflicts: parse_dep_map(v.get("conflicts")),
        jars,
        is_generated,
    })
}

fn read_zip_fabric_meta<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    scan_nested: bool,
    depth: usize,
    nested_out: &mut Vec<FabricModMeta>,
) -> Option<FabricModMeta> {
    let mut raw = String::new();
    {
        let entry = zip.by_name("fabric.mod.json").ok()?;
        entry.take(MAX_META_BYTES).read_to_string(&mut raw).ok()?;
    }
    let meta = parse_fabric_mod_json(&raw)?;
    if scan_nested && depth < MAX_NESTED_DEPTH && !meta.jars.is_empty() {
        for jar_path in &meta.jars {
            if nested_out.len() >= MAX_NESTED_COUNT {
                break;
            }
            let mut buf = Vec::new();
            let read_ok = {
                match zip.by_name(jar_path) {
                    Ok(entry) if entry.size() <= MAX_NESTED_JAR_BYTES => {
                        entry.take(MAX_NESTED_JAR_BYTES).read_to_end(&mut buf).is_ok()
                    }
                    _ => false,
                }
            };
            if !read_ok {
                continue;
            }
            if let Ok(mut inner) = zip::ZipArchive::new(Cursor::new(buf)) {
                if let Some(child) = read_zip_fabric_meta(&mut inner, true, depth + 1, nested_out) {
                    nested_out.push(child);
                }
            }
        }
    }
    Some(meta)
}

/// Read `fabric.mod.json` (and nested `jars` when `enabled`) from a jar on disk.
pub fn read_fabric_jar(path: &Path, filename: &str, enabled: bool) -> Option<ScannedJar> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut nested = Vec::new();
    let root = read_zip_fabric_meta(&mut zip, enabled, 0, &mut nested)?;
    Some(ScannedJar { filename: filename.to_string(), enabled, root, nested })
}

/// Read `fabric.mod.json` (and nested `jars`) from raw jar bytes.
pub fn read_fabric_jar_bytes(bytes: &[u8], filename: &str, enabled: bool) -> Option<ScannedJar> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut nested = Vec::new();
    let root = read_zip_fabric_meta(&mut zip, enabled, 0, &mut nested)?;
    Some(ScannedJar { filename: filename.to_string(), enabled, root, nested })
}

// ---------------------------------------------------------------------------
// Public report / issue types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatSeverity {
    /// Blocks game launch (Fabric Loader refuses to start).
    Error,
    /// Advisory warning (won't prevent startup).
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatIssueKind {
    MinecraftVersion,
    LoaderVersion,
    JavaVersion,
    MissingDep,
    DisabledDep,
    IncompatibleDep,
    BreaksMod,
    ConflictsMod,
    DuplicateMod,
    ServerOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompatFix {
    Disable { filename: String },
    Enable { filename: String, name: String },
    UpdateMod { filename: String, target_id: String, target_name: String, required: String },
    ChangeVersion { filename: String },
    InstallMod { mod_id: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModCompatIssue {
    /// Top-level `.jar` filename this note attaches to.
    pub filename: String,
    pub mod_id: String,
    pub mod_name: String,
    pub severity: CompatSeverity,
    pub kind: CompatIssueKind,
    pub target_id: String,
    pub target_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_filename: Option<String>,
    pub required: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<CompatFix>,
}

/// Lightweight local metadata extracted from a jar's `fabric.mod.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JarModInfo {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub environment: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModCompatReport {
    /// True for Fabric instances; false for Vanilla/Forge/NeoForge.
    pub supported: bool,
    pub mc_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    pub java_major: u32,
    pub error_count: usize,
    pub warning_count: usize,
    pub issues: Vec<ModCompatIssue>,
    /// filename -> local `fabric.mod.json` summary (both enabled and disabled).
    pub jar_info: IndexMap<String, JarModInfo>,
}

/// Environment versions known locally for a Fabric instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricEnv {
    pub mc_version: String,
    pub loader_version: Option<String>,
    pub java_major: u32,
    pub mixinextras_version: Option<String>,
}

// ---------------------------------------------------------------------------
// Well-known Fabric mod IDs & helpers
// ---------------------------------------------------------------------------

/// Well-known Fabric mod IDs -> (Modrinth project_id, human display name).
pub const KNOWN_MODRINTH_IDS: &[(&str, &str, &str)] = &[
    ("fabric-api", "P7dR8mSH", "Fabric API"),
    ("fabric", "P7dR8mSH", "Fabric API"),
    ("sodium", "AANobbMI", "Sodium"),
    ("iris", "YL57xq9U", "Iris"),
    ("lithium", "gvQqBUqZ", "Lithium"),
    ("ferritecore", "uXXizFIs", "FerriteCore"),
    ("krypton", "fQEb0iXm", "Krypton"),
    ("modmenu", "mOgUt4GM", "Mod Menu"),
    ("immediatelyfast", "5ZwdcRci", "ImmediatelyFast"),
    ("fabric-language-kotlin", "Ha28R6CL", "Fabric Language Kotlin"),
    ("cloth-config", "9s6osm5g", "Cloth Config"),
    ("cloth-config2", "9s6osm5g", "Cloth Config"),
    ("architectury", "lhGA9TYQ", "Architectury API"),
    ("yet_another_config_lib_v3", "1eAoo2KR", "YetAnotherConfigLib"),
    ("placeholder-api", "eXts2L7r", "Placeholder API"),
    ("owo", "ccKDOlHs", "oωo (owo-lib)"),
    ("cardinal-components", "K01OU20C", "Cardinal Components API"),
    ("cardinal-components-base", "K01OU20C", "Cardinal Components API"),
    ("collective", "e0M1UDsY", "Collective"),
    ("puzzleslib", "QAGBst4M", "Puzzles Lib"),
    ("forgeconfigapiport", "ohNO6lps", "Forge Config API Port"),
    ("entityculling", "NNAgCjsB", "Entity Culling"),
    ("sodium-extra", "PtjYWJkn", "Sodium Extra"),
    ("reeses-sodium-options", "Bh37bMuy", "Reese's Sodium Options"),
];

pub fn known_modrinth_for_id(mod_id: &str) -> Option<(&'static str, &'static str)> {
    KNOWN_MODRINTH_IDS.iter().find(|(id, _, _)| *id == mod_id).map(|(_, pid, name)| (*pid, *name))
}

/// True if `id` is a Fabric API sub-module (so missing it really means "install Fabric API").
pub fn is_fabric_api_module(id: &str) -> bool {
    if matches!(id, "fabric" | "fabric-api" | "fabric-api-base") {
        return true;
    }
    if !id.starts_with("fabric-") {
        return false;
    }
    if matches!(
        id,
        "fabric-loader"
            | "fabric-language-kotlin"
            | "fabric-language-scala"
            | "fabric-permissions-api-v0"
            | "fabric-carpet"
            | "fabric-seasons"
            | "fabric-waystones"
    ) {
        return false;
    }
    // Fabric API sub-modules end in `-v<digit>` (e.g. fabric-lifecycle-events-v1).
    if let Some((_, tail)) = id.rsplit_once("-v") {
        return !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit());
    }
    false
}

/// Friendly human name for a mod id when the jar itself isn't installed.
pub fn friendly_mod_name(id: &str) -> String {
    if let Some((_, name)) = known_modrinth_for_id(id) {
        return name.to_string();
    }
    if is_fabric_api_module(id) {
        return "Fabric API".to_string();
    }
    match id {
        "minecraft" => "Minecraft".to_string(),
        "fabricloader" => "Fabric Loader".to_string(),
        "java" => "Java".to_string(),
        "optifabric" => "OptiFabric".to_string(),
        "embeddium" => "Embeddium".to_string(),
        "vulkanmod" => "VulkanMod".to_string(),
        "indium" => "Indium".to_string(),
        other => other.to_string(),
    }
}

/// Core performance/loader mods that should NOT be the default "disable" target
/// when a third-party mod breaks with them.
fn is_core_mod(id: &str) -> bool {
    matches!(
        id,
        "sodium" | "iris" | "fabric-api" | "fabric" | "espectral-menu" | "espectral-brand" | "lithium" | "ferritecore"
    )
}

// ---------------------------------------------------------------------------
// Core compatibility evaluator (pure)
// ---------------------------------------------------------------------------

struct ProvidedMod {
    jar_filename: String,
    parent_name: String,
    version: FabricVersion,
    version_raw: String,
}

/// Evaluate Fabric compatibility across all scanned jars (enabled + disabled).
pub fn evaluate_fabric_compat(env: &FabricEnv, jars: &[ScannedJar]) -> ModCompatReport {
    let mut jar_info: IndexMap<String, JarModInfo> = IndexMap::new();
    for j in jars {
        jar_info.insert(
            j.filename.clone(),
            JarModInfo {
                mod_id: j.root.id.clone(),
                name: j.root.name.clone(),
                version: j.root.version.clone(),
                environment: j.root.environment.clone(),
            },
        );
    }

    let mut issues: Vec<ModCompatIssue> = Vec::new();
    let mut provided: HashMap<String, ProvidedMod> = HashMap::new();
    let mut disabled_provided: HashMap<String, ProvidedMod> = HashMap::new();
    let mut top_by_id: IndexMap<String, Vec<&ScannedJar>> = IndexMap::new();

    for j in jars {
        if j.enabled {
            top_by_id.entry(j.root.id.clone()).or_default().push(j);
            if j.root.environment == "server" {
                issues.push(ModCompatIssue {
                    filename: j.filename.clone(),
                    mod_id: j.root.id.clone(),
                    mod_name: j.root.name.clone(),
                    severity: CompatSeverity::Warning,
                    kind: CompatIssueKind::ServerOnly,
                    target_id: "minecraft".into(),
                    target_name: "client".into(),
                    target_filename: None,
                    required: String::new(),
                    actual: None,
                    fix: Some(CompatFix::Disable { filename: j.filename.clone() }),
                });
                continue;
            }
            register_provided(&mut provided, &j.filename, &j.root.name, &j.root);
            for child in &j.nested {
                if child.environment != "server" {
                    register_provided(&mut provided, &j.filename, &j.root.name, child);
                }
            }
        } else {
            register_provided(&mut disabled_provided, &j.filename, &j.root.name, &j.root);
            for child in &j.nested {
                register_provided(&mut disabled_provided, &j.filename, &j.root.name, child);
            }
        }
    }

    // 1. Duplicate top-level mod IDs among enabled jars.
    let mut duplicate_older: HashSet<String> = HashSet::new();
    for (mod_id, mut group) in top_by_id {
        if group.len() <= 1 {
            continue;
        }
        let all_semver = group.iter().all(|j| matches!(FabricVersion::parse(&j.root.version), FabricVersion::Semantic { .. }));
        group.sort_by(|a, b| {
            let va = FabricVersion::parse(&a.root.version);
            let vb = FabricVersion::parse(&b.root.version);
            vb.cmp_fabric(&va).unwrap_or(Ordering::Equal).then_with(|| a.filename.cmp(&b.filename))
        });
        let kept = group[0];
        for stale in &group[1..] {
            duplicate_older.insert(stale.filename.clone());
            issues.push(ModCompatIssue {
                filename: stale.filename.clone(),
                mod_id: mod_id.clone(),
                mod_name: stale.root.name.clone(),
                // launch.rs auto-disables semver duplicates, so warn on semver
                // and block on non-semver duplicates.
                severity: if all_semver { CompatSeverity::Warning } else { CompatSeverity::Error },
                kind: CompatIssueKind::DuplicateMod,
                target_id: mod_id.clone(),
                target_name: kept.root.name.clone(),
                target_filename: Some(kept.filename.clone()),
                required: String::new(),
                actual: Some(clean_version_display(&kept.root.version).to_string()),
                fix: Some(CompatFix::Disable { filename: stale.filename.clone() }),
            });
        }
    }

    // 2. Check depends / breaks / conflicts on each enabled top-level mod.
    let mc_ver = FabricVersion::parse(&env.mc_version);
    let mc_is_snapshot = is_snapshot_mc(&env.mc_version);
    let loader_ver = env.loader_version.as_deref().map(FabricVersion::parse);
    let java_ver = FabricVersion::parse(&env.java_major.to_string());
    let mixinextras_ver = env.mixinextras_version.as_deref().map(FabricVersion::parse);
    let loader_bundles_mixinextras = loader_ver
        .as_ref()
        .map_or(true, |lv| VersionPredicate::parse_str(">=0.15.0").matches(lv));

    // Track already-emitted symmetric break/conflict pairs so we don't spam duplicates.
    let mut seen_pairs: HashSet<(String, String, bool)> = HashSet::new();

    for j in jars {
        if !j.enabled || j.root.environment == "server" || duplicate_older.contains(&j.filename) {
            continue;
        }
        let mut reported_missing_fapi = false;

        // ---- depends (fatal) ----
        for (dep_id, clauses) in &j.root.depends {
            let pred = VersionPredicate::parse_list(clauses);
            match dep_id.as_str() {
                "minecraft" => {
                    if mc_is_snapshot || pred.is_any() {
                        continue;
                    }
                    if !pred.matches(&mc_ver) {
                        issues.push(ModCompatIssue {
                            filename: j.filename.clone(),
                            mod_id: j.root.id.clone(),
                            mod_name: j.root.name.clone(),
                            severity: CompatSeverity::Error,
                            kind: CompatIssueKind::MinecraftVersion,
                            target_id: "minecraft".into(),
                            target_name: "Minecraft".into(),
                            target_filename: None,
                            required: pred.raw_string(),
                            actual: Some(env.mc_version.clone()),
                            fix: Some(CompatFix::ChangeVersion { filename: j.filename.clone() }),
                        });
                    }
                }
                "fabricloader" => {
                    if pred.is_any() {
                        continue;
                    }
                    if let Some(lv) = &loader_ver {
                        if !pred.matches(lv) {
                            issues.push(ModCompatIssue {
                                filename: j.filename.clone(),
                                mod_id: j.root.id.clone(),
                                mod_name: j.root.name.clone(),
                                severity: CompatSeverity::Error,
                                kind: CompatIssueKind::LoaderVersion,
                                target_id: "fabricloader".into(),
                                target_name: "Fabric Loader".into(),
                                target_filename: None,
                                required: pred.raw_string(),
                                actual: env.loader_version.clone(),
                                fix: Some(CompatFix::Disable { filename: j.filename.clone() }),
                            });
                        }
                    }
                }
                "java" => {
                    if pred.is_any() {
                        continue;
                    }
                    if !pred.matches(&java_ver) {
                        issues.push(ModCompatIssue {
                            filename: j.filename.clone(),
                            mod_id: j.root.id.clone(),
                            mod_name: j.root.name.clone(),
                            severity: CompatSeverity::Error,
                            kind: CompatIssueKind::JavaVersion,
                            target_id: "java".into(),
                            target_name: "Java".into(),
                            target_filename: None,
                            required: pred.raw_string(),
                            actual: Some(env.java_major.to_string()),
                            fix: Some(CompatFix::Disable { filename: j.filename.clone() }),
                        });
                    }
                }
                "mixinextras" => {
                    if let Some(prov) = provided.get("mixinextras") {
                        if !prov.version.is_placeholder() && !pred.matches(&prov.version) {
                            issues.push(incompatible_dep_issue(j, dep_id, prov, &pred));
                        }
                    } else if let Some(mv) = &mixinextras_ver {
                        if !pred.matches(mv) {
                            issues.push(ModCompatIssue {
                                filename: j.filename.clone(),
                                mod_id: j.root.id.clone(),
                                mod_name: j.root.name.clone(),
                                severity: CompatSeverity::Error,
                                kind: CompatIssueKind::LoaderVersion,
                                target_id: "mixinextras".into(),
                                target_name: "MixinExtras".into(),
                                target_filename: None,
                                required: pred.raw_string(),
                                actual: env.mixinextras_version.clone(),
                                fix: Some(CompatFix::Disable { filename: j.filename.clone() }),
                            });
                        }
                    } else if !loader_bundles_mixinextras {
                        issues.push(missing_dep_issue(j, dep_id, &pred));
                    }
                }
                other_id => {
                    if let Some(prov) = provided.get(other_id) {
                        if !prov.version.is_placeholder() && !pred.matches(&prov.version) {
                            issues.push(incompatible_dep_issue(j, other_id, prov, &pred));
                        }
                    } else if let Some(dis) = disabled_provided.get(other_id) {
                        let satisfies = dis.version.is_placeholder() || pred.matches(&dis.version);
                        let fix = if satisfies {
                            Some(CompatFix::Enable {
                                filename: dis.jar_filename.clone(),
                                name: dis.parent_name.clone(),
                            })
                        } else {
                            Some(CompatFix::UpdateMod {
                                filename: dis.jar_filename.clone(),
                                target_id: other_id.to_string(),
                                target_name: dis.parent_name.clone(),
                                required: pred.raw_string(),
                            })
                        };
                        issues.push(ModCompatIssue {
                            filename: j.filename.clone(),
                            mod_id: j.root.id.clone(),
                            mod_name: j.root.name.clone(),
                            severity: CompatSeverity::Error,
                            kind: CompatIssueKind::DisabledDep,
                            target_id: other_id.to_string(),
                            target_name: dis.parent_name.clone(),
                            target_filename: Some(dis.jar_filename.clone()),
                            required: pred.raw_string(),
                            actual: Some(clean_version_display(&dis.version_raw).to_string()),
                            fix,
                        });
                    } else if is_fabric_api_module(other_id) {
                        if !reported_missing_fapi {
                            reported_missing_fapi = true;
                            issues.push(ModCompatIssue {
                                filename: j.filename.clone(),
                                mod_id: j.root.id.clone(),
                                mod_name: j.root.name.clone(),
                                severity: CompatSeverity::Error,
                                kind: CompatIssueKind::MissingDep,
                                target_id: "fabric-api".into(),
                                target_name: "Fabric API".into(),
                                target_filename: None,
                                required: "*".into(),
                                actual: None,
                                fix: Some(CompatFix::InstallMod {
                                    mod_id: "fabric-api".into(),
                                    name: "Fabric API".into(),
                                }),
                            });
                        }
                    } else {
                        issues.push(missing_dep_issue(j, other_id, &pred));
                    }
                }
            }
        }

        // ---- breaks (fatal) & conflicts (warning) ----
        for (is_break, map) in [(true, &j.root.breaks), (false, &j.root.conflicts)] {
            for (other_id, clauses) in map {
                if other_id == &j.root.id || matches!(other_id.as_str(), "minecraft" | "fabricloader" | "java") {
                    continue;
                }
                let Some(other) = provided.get(other_id) else { continue };
                if other.jar_filename == j.filename {
                    continue;
                }
                let pred = VersionPredicate::parse_list(clauses);
                if other.version.is_placeholder() && !pred.is_any() {
                    continue;
                }
                if !pred.matches(&other.version) {
                    continue;
                }
                let pair_key = if j.filename < other.jar_filename {
                    (j.filename.clone(), other.jar_filename.clone(), is_break)
                } else {
                    (other.jar_filename.clone(), j.filename.clone(), is_break)
                };
                if !seen_pairs.insert(pair_key) {
                    continue;
                }

                // Decide which row owns the note and what the smartest fix is:
                // - If a core mod (e.g. Sodium) breaks a third-party mod (e.g. OptiFabric),
                //   attach the note to the third-party mod so disabling it is 1 click!
                // - If the break is upper-bound-only (e.g. Sodium breaks `iris <= 1.11.2`),
                //   attach to the outdated mod (`iris`) with an UpdateMod fix!
                let (owner_file, owner_id, owner_name, peer_id, peer_name, peer_file, fix) =
                    if pred.is_upper_bound_only() {
                        (
                            other.jar_filename.clone(),
                            other_id.clone(),
                            other.parent_name.clone(),
                            j.root.id.clone(),
                            j.root.name.clone(),
                            j.filename.clone(),
                            CompatFix::ChangeVersion { filename: other.jar_filename.clone() },
                        )
                    } else if is_core_mod(&j.root.id) && !is_core_mod(other_id) {
                        (
                            other.jar_filename.clone(),
                            other_id.clone(),
                            other.parent_name.clone(),
                            j.root.id.clone(),
                            j.root.name.clone(),
                            j.filename.clone(),
                            CompatFix::Disable { filename: other.jar_filename.clone() },
                        )
                    } else {
                        (
                            j.filename.clone(),
                            j.root.id.clone(),
                            j.root.name.clone(),
                            other_id.clone(),
                            other.parent_name.clone(),
                            other.jar_filename.clone(),
                            CompatFix::Disable { filename: j.filename.clone() },
                        )
                    };

                issues.push(ModCompatIssue {
                    filename: owner_file,
                    mod_id: owner_id,
                    mod_name: owner_name,
                    severity: if is_break { CompatSeverity::Error } else { CompatSeverity::Warning },
                    kind: if is_break { CompatIssueKind::BreaksMod } else { CompatIssueKind::ConflictsMod },
                    target_id: peer_id,
                    target_name: peer_name,
                    target_filename: Some(peer_file),
                    required: pred.raw_string(),
                    actual: Some(clean_version_display(&other.version_raw).to_string()),
                    fix: Some(fix),
                });
            }
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

fn register_provided(
    map: &mut HashMap<String, ProvidedMod>,
    jar_filename: &str,
    parent_name: &str,
    meta: &FabricModMeta,
) {
    let ver = FabricVersion::parse(&meta.version);
    let insert_if_newer = |map: &mut HashMap<String, ProvidedMod>, id: &str| {
        let replace = match map.get(id) {
            None => true,
            Some(existing) => ver.cmp_fabric(&existing.version) == Some(Ordering::Greater),
        };
        if replace {
            map.insert(
                id.to_string(),
                ProvidedMod {
                    jar_filename: jar_filename.to_string(),
                    parent_name: parent_name.to_string(),
                    version: ver.clone(),
                    version_raw: meta.version.clone(),
                },
            );
        }
    };
    insert_if_newer(map, &meta.id);
    for alias in &meta.provides {
        insert_if_newer(map, alias);
    }
}

fn incompatible_dep_issue(
    j: &ScannedJar,
    dep_id: &str,
    prov: &ProvidedMod,
    pred: &VersionPredicate,
) -> ModCompatIssue {
    ModCompatIssue {
        filename: j.filename.clone(),
        mod_id: j.root.id.clone(),
        mod_name: j.root.name.clone(),
        severity: CompatSeverity::Error,
        kind: CompatIssueKind::IncompatibleDep,
        target_id: dep_id.to_string(),
        target_name: prov.parent_name.clone(),
        target_filename: Some(prov.jar_filename.clone()),
        required: pred.raw_string(),
        actual: Some(clean_version_display(&prov.version_raw).to_string()),
        fix: Some(CompatFix::UpdateMod {
            filename: prov.jar_filename.clone(),
            target_id: dep_id.to_string(),
            target_name: prov.parent_name.clone(),
            required: pred.raw_string(),
        }),
    }
}

fn missing_dep_issue(j: &ScannedJar, dep_id: &str, pred: &VersionPredicate) -> ModCompatIssue {
    let name = friendly_mod_name(dep_id);
    ModCompatIssue {
        filename: j.filename.clone(),
        mod_id: j.root.id.clone(),
        mod_name: j.root.name.clone(),
        severity: CompatSeverity::Error,
        kind: CompatIssueKind::MissingDep,
        target_id: dep_id.to_string(),
        target_name: name.clone(),
        target_filename: None,
        required: pred.raw_string(),
        actual: None,
        fix: Some(CompatFix::InstallMod { mod_id: dep_id.to_string(), name }),
    }
}

// ---------------------------------------------------------------------------
// Local instance discovery & helpers
// ---------------------------------------------------------------------------

/// Discover the Fabric environment (`mc_version`, `loader_version`, `java_major`,
/// `mixinextras_version`) strictly from local instance + cache files (zero network).
pub fn discover_fabric_env(engine: &Engine, instance: &InstanceMeta) -> FabricEnv {
    let mc = instance.version.clone();
    let java_major = crate::runtimes::java_major_for_version_id(&mc);

    // 1. Loader version from instance.json if pinned.
    let mut loader_version = instance.loader_version.clone().filter(|s| !s.trim().is_empty());
    let mut mixinextras_version: Option<String> = None;

    // 2. Look for cached `versions/fabric-loader-<mc>-<loader>.json` (gives both
    //    loader version and bundled mixinextras version!).
    let versions_dir = engine.paths().versions_dir();
    let prefix = format!("fabric-loader-{mc}-");
    let mut cached_combos: Vec<(FabricVersion, String, Value)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&versions_dir) {
        for entry in rd.flatten() {
            let fname = entry.file_name().to_string_lossy().into_owned();
            if let Some(rest) = fname.strip_prefix(&prefix).and_then(|s| s.strip_suffix(".json")) {
                if let Ok(raw) = std::fs::read_to_string(entry.path()) {
                    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                        let lv = v
                            .get("loader")
                            .and_then(|l| l.get("version"))
                            .and_then(Value::as_str)
                            .unwrap_or(rest)
                            .to_string();
                        cached_combos.push((FabricVersion::parse(&lv), lv, v));
                    }
                }
            }
        }
    }
    if !cached_combos.is_empty() {
        cached_combos.sort_by(|a, b| b.0.cmp_fabric(&a.0).unwrap_or(Ordering::Equal));
        let chosen = if let Some(ref pinned) = loader_version {
            cached_combos.iter().find(|(_, lv, _)| lv == pinned).or_else(|| cached_combos.first())
        } else {
            cached_combos.first()
        };
        if let Some((_, lv, doc)) = chosen {
            if loader_version.is_none() {
                loader_version = Some(lv.clone());
            }
            mixinextras_version = extract_mixinextras_from_combo(doc);
        }
    }

    // 3. Fallback: `versions/fabric-loaders.json` `latest_stable`.
    if loader_version.is_none() {
        let loaders_file = versions_dir.join("fabric-loaders.json");
        if let Ok(raw) = std::fs::read_to_string(loaders_file) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                loader_version = v.get("latest_stable").and_then(Value::as_str).map(str::to_string);
            }
        }
    }

    FabricEnv { mc_version: mc, loader_version, java_major, mixinextras_version }
}

fn extract_mixinextras_from_combo(doc: &Value) -> Option<String> {
    let libs = doc.get("launcherMeta")?.get("libraries")?;
    for section in ["common", "client", "development"] {
        let Some(arr) = libs.get(section).and_then(Value::as_array) else { continue };
        for lib in arr {
            let Some(name) = lib.get("name").and_then(Value::as_str) else { continue };
            // e.g. "io.github.llamalad7:mixinextras-fabric:0.5.5"
            if name.contains("mixinextras") {
                if let Some(ver) = name.rsplit(':').next().filter(|s| !s.is_empty()) {
                    return Some(ver.to_string());
                }
            }
        }
    }
    None
}

/// Scan every `.jar` and `.jar.disabled` in `mods_dir` for `fabric.mod.json`.
pub fn scan_mods_dir(mods_dir: &Path) -> Vec<ScannedJar> {
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
        if let Some(scanned) = read_fabric_jar(&mods_dir.join(&file), &base, !disabled) {
            out.push(scanned);
        }
    }
    out
}

/// Run the full Fabric compatibility check for an instance (synchronous local I/O).
pub fn check_instance_mods_compat_sync(engine: &Engine, instance_name: &str) -> EngineResult<ModCompatReport> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    if instance.loader != Loader::Fabric {
        let java_major = crate::runtimes::java_major_for_version_id(&instance.version);
        return Ok(ModCompatReport {
            supported: false,
            mc_version: instance.version,
            loader_version: instance.loader_version,
            java_major,
            ..Default::default()
        });
    }
    let env = discover_fabric_env(engine, &instance);
    let mods_dir = effective_mods_dir(engine, &instance);
    let jars = scan_mods_dir(&mods_dir);
    Ok(evaluate_fabric_compat(&env, &jars))
}

/// Async wrapper (runs zip scanning on blocking pool so UI/tokio stay snappy).
pub async fn check_instance_mods_compat(engine: &Engine, instance_name: &str) -> EngineResult<ModCompatReport> {
    let e = engine.clone();
    let name = instance_name.to_string();
    tokio::task::spawn_blocking(move || check_instance_mods_compat_sync(&e, &name))
        .await
        .map_err(|err| crate::error::EngineError::other(format!("compat scan task failed: {err}")))?
}

/// Find names of enabled mods in `jars` whose `depends` reference any mod id
/// provided by `target_filename` (used for delete warnings and companion checks).
pub fn dependents_of_jar(jars: &[ScannedJar], target_filename: &str) -> Vec<(String, String)> {
    let Some(target) = jars.iter().find(|j| j.filename == target_filename) else {
        return vec![];
    };
    let mut provided_ids: HashSet<&str> = HashSet::new();
    provided_ids.insert(target.root.id.as_str());
    for p in &target.root.provides {
        provided_ids.insert(p.as_str());
    }
    for n in &target.nested {
        provided_ids.insert(n.id.as_str());
        for p in &n.provides {
            provided_ids.insert(p.as_str());
        }
    }
    let mut out = Vec::new();
    for j in jars {
        if !j.enabled || j.filename == target_filename {
            continue;
        }
        if j.root.depends.keys().any(|d| provided_ids.contains(d.as_str())) {
            out.push((j.filename.clone(), j.root.name.clone()));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk_jar(filename: &str, enabled: bool, meta_json: Value, nested: Vec<Value>) -> ScannedJar {
        let root = parse_fabric_mod_json(&meta_json.to_string()).expect("valid root");
        let nested = nested
            .into_iter()
            .map(|n| parse_fabric_mod_json(&n.to_string()).expect("valid nested"))
            .collect();
        ScannedJar { filename: filename.into(), enabled, root, nested }
    }

    fn env_26_3() -> FabricEnv {
        FabricEnv {
            mc_version: "26.3".into(),
            loader_version: Some("0.20.1".into()),
            java_major: 25,
            mixinextras_version: Some("0.5.5".into()),
        }
    }

    #[test]
    fn fabric_version_parsing_and_comparison() {
        let v = FabricVersion::parse;
        assert_eq!(v("0.9.2+mc26.3").cmp_fabric(&v("0.9.2")), Some(Ordering::Equal));
        assert_eq!(v("1.21").cmp_fabric(&v("1.21.0")), Some(Ordering::Equal));
        assert_eq!(v("26.3").cmp_fabric(&v("26.3.0")), Some(Ordering::Equal));
        assert_eq!(v("0.9.3-beta.1+mc26.3").cmp_fabric(&v("0.9.3")), Some(Ordering::Less));
        assert_eq!(v("0.9.3-beta.1").cmp_fabric(&v("0.9.2")), Some(Ordering::Greater));
        assert_eq!(v("0.9.3-alpha.10").cmp_fabric(&v("0.9.3-alpha.2")), Some(Ordering::Greater));
        assert_eq!(v("0.9.3-beta.1").cmp_fabric(&v("0.9.3-alpha.9")), Some(Ordering::Greater));
        assert_eq!(v("0.9.3-").cmp_fabric(&v("0.9.3-alpha.1")), Some(Ordering::Less));
        assert_eq!(v("0.9.3-1").cmp_fabric(&v("0.9.3-alpha")), Some(Ordering::Less));
    }

    #[test]
    fn fabric_predicates_match_loader_semantics() {
        let check = |pred: &str, ver: &str| VersionPredicate::parse_str(pred).matches(&FabricVersion::parse(ver));

        // Basic comparisons + build metadata ignored
        assert!(check(">=0.9.3", "0.9.3+mc26.3"));
        assert!(check(">=0.9.3", "0.9.4"));
        assert!(!check(">=0.9.3", "0.9.2+mc26.3"));
        assert!(!check(">=0.9.3", "0.9.3-beta.1+mc26.3"));
        assert!(check(">=0.9.3-", "0.9.3-beta.1+mc26.3"));
        assert!(check(">=26.3-", "26.3"));
        assert!(check(">=26.3-", "26.3-rc.1"));

        // Tilde (~): SAME_TO_NEXT_MINOR
        assert!(check("~26.3", "26.3"));
        assert!(check("~26.3", "26.3.1"));
        assert!(!check("~26.3", "26.4"));
        assert!(!check("~26.3", "26.4-alpha.1"));
        assert!(check("~26.3-", "26.3"));
        assert!(check("~26.3-", "26.3-rc.1"));
        assert!(check("~1.21.8", "1.21.11"));
        assert!(!check("~1.21.8", "1.21.7"));
        assert!(!check("~1.21.8", "1.22.0-alpha.1"));
        assert!(!check("~1.21.8", "26.3"));

        // Caret (^): Fabric SAME_TO_NEXT_MAJOR (even for 0.x!)
        assert!(check("^0.9.2", "0.9.2"));
        assert!(check("^0.9.2", "0.10.0"));
        assert!(!check("^0.9.2", "1.0.0-alpha.1"));
        assert!(check("^1.2.3", "1.99.0"));
        assert!(!check("^1.2.3", "2.0.0"));

        // Wildcards (.x, .X, .*)
        assert!(check("0.9.x", "0.9.2+mc26.3"));
        assert!(check("0.9.x", "0.9.3-beta.1+mc26.3"));
        assert!(!check("0.9.x", "0.10.0-alpha.1"));
        assert!(!check("0.9.x", "0.8.14"));
        assert!(check("26.3.x", "26.3"));
        assert!(check("26.3.*", "26.3.2"));
        assert!(check("1.21.X", "1.21.11"));
        assert!(check("*", "anything-at-all"));

        // Space-separated AND ranges
        assert!(check(">=26.1 <27", "26.3"));
        assert!(!check(">=26.1 <27", "26.0"));
        assert!(!check(">=26.1 <27", "27.0"));
        assert!(check(">=1.21.6 <=1.21.11", "1.21.8"));
        assert!(!check(">=1.21.6 <=1.21.11", "26.3"));

        // Array OR predicates
        let or_pred = VersionPredicate::parse_list(&["~1.21.8".into(), ">=26.2".into()]);
        assert!(or_pred.matches(&FabricVersion::parse("1.21.11")));
        assert!(or_pred.matches(&FabricVersion::parse("26.3")));
        assert!(!or_pred.matches(&FabricVersion::parse("1.21.4")));
    }

    #[test]
    fn extract_modrinth_version_numbers() {
        assert_eq!(extract_mod_semver("mc26.3-0.9.3-beta.1-fabric"), Some(FabricVersion::parse("0.9.3-beta.1")));
        assert_eq!(extract_mod_semver("mc1.21.11-0.8.14-fabric"), Some(FabricVersion::parse("0.8.14")));
        assert_eq!(extract_mod_semver("1.11.7+26.3-fabric"), Some(FabricVersion::parse("1.11.7")));
        assert_eq!(extract_mod_semver("9.0.0-fabric"), Some(FabricVersion::parse("9.0.0")));
        assert_eq!(extract_mod_semver("0.166.0+26.3"), Some(FabricVersion::parse("0.166.0")));
        assert_eq!(extract_mod_semver("v2.3.4"), Some(FabricVersion::parse("2.3.4")));
    }

    #[test]
    fn real_world_26_3_bundle_has_zero_false_positives() {
        let jars = vec![
            mk_jar(
                "espectral-menu-26.3-1.9.4.jar",
                true,
                json!({
                    "id": "espectral-menu", "name": "Espectral Menu", "version": "1.9.4",
                    "environment": "client",
                    "depends": { "fabricloader": ">=0.19.3", "minecraft": "~26.3", "java": ">=25" }
                }),
                vec![],
            ),
            mk_jar(
                "fabric-api-0.165.0+26.3.jar",
                true,
                json!({
                    "id": "fabric-api", "name": "Fabric API", "version": "0.165.0+26.3",
                    "environment": "*",
                    "depends": { "fabricloader": ">=0.19.3", "java": ">=25", "minecraft": "~26.3-" }
                }),
                vec![
                    json!({ "id": "fabric-resource-loader-v0", "version": "3.0.0" }),
                    json!({ "id": "fabric-resource-loader-v1", "version": "1.0.0" }),
                    json!({ "id": "fabric-screen-api-v1", "version": "2.0.0" }),
                    json!({ "id": "fabric-key-mapping-api-v1", "version": "1.0.0" }),
                    json!({ "id": "fabric-lifecycle-events-v1", "version": "2.0.0" }),
                    json!({ "id": "fabric-rendering-fluids-v1", "version": "3.1.0" }),
                ],
            ),
            mk_jar(
                "sodium-fabric-0.9.2+mc26.3.jar",
                true,
                json!({
                    "id": "sodium", "name": "Sodium", "version": "0.9.2+mc26.3",
                    "provides": ["indium"],
                    "depends": {
                        "minecraft": "26.3.x", "fabricloader": ">=0.16.0",
                        "fabric-block-getter-api-v2": "*", "fabric-rendering-fluids-v1": ">=2.0.0",
                        "fabric-resource-loader-v0": "*"
                    },
                    "breaks": { "optifabric": "*", "iris": "<=1.11.2", "fabric-api": "<0.145.1" }
                }),
                vec![json!({ "id": "fabric-block-getter-api-v2", "version": "1.0.0" })],
            ),
            mk_jar(
                "iris-fabric-1.11.7+mc26.3.jar",
                true,
                json!({
                    "id": "iris", "name": "Iris", "version": "1.11.7+mc26.3",
                    "depends": { "fabricloader": ">=0.12.3", "sodium": ["0.9.x"] },
                    "breaks": { "optifabric": "*" }
                }),
                vec![],
            ),
            mk_jar(
                "lithium-fabric-0.26.3+mc26.3.jar",
                true,
                json!({
                    "id": "lithium", "name": "Lithium", "version": "0.26.3+mc26.3",
                    "depends": { "fabricloader": ">=0.19.5", "minecraft": "~26.3", "mixinextras": ">=0.5.5" },
                    "breaks": { "optifabric": "*" }
                }),
                vec![],
            ),
            mk_jar(
                "ferritecore-9.0.0-fabric.jar",
                true,
                json!({
                    "id": "ferritecore", "name": "FerriteCore", "version": "9.0.0",
                    "depends": { "fabricloader": ">=0.14.21", "minecraft": ">=26.1 <27" }
                }),
                vec![],
            ),
            mk_jar(
                "modmenu-21.0.0.jar",
                true,
                json!({
                    "id": "modmenu", "name": "Mod Menu", "version": "21.0.0",
                    "depends": {
                        "fabric-resource-loader-v1": "*", "fabric-screen-api-v1": "*",
                        "fabric-key-mapping-api-v1": "*", "fabric-lifecycle-events-v1": "*",
                        "fabricloader": ">=0.19.2", "minecraft": ">=26.3-"
                    }
                }),
                vec![],
            ),
        ];
        let report = evaluate_fabric_compat(&env_26_3(), &jars);
        assert_eq!(report.error_count, 0, "unexpected errors: {:?}", report.issues);
        assert_eq!(report.warning_count, 0, "unexpected warnings: {:?}", report.issues);
    }

    #[test]
    fn detects_guaka_sodium_mismatch_wrong_mc_breaks_disabled_and_missing_fapi() {
        let jars = vec![
            mk_jar(
                "sodium-fabric-0.9.2+mc26.3.jar",
                true,
                json!({
                    "id": "sodium", "name": "Sodium", "version": "0.9.2+mc26.3",
                    "depends": { "minecraft": "26.3.x" },
                    "breaks": { "optifabric": "*" }
                }),
                vec![],
            ),
            mk_jar(
                "cool-shader-addon.jar",
                true,
                json!({
                    "id": "cool-addon", "name": "Cool Addon", "version": "1.0.0",
                    "depends": { "sodium": ">=0.9.3" }
                }),
                vec![],
            ),
            mk_jar(
                "old-mod-1.21.8.jar",
                true,
                json!({
                    "id": "oldmod", "name": "Old Mod", "version": "1.0.0",
                    "depends": { "minecraft": "~1.21.8" }
                }),
                vec![],
            ),
            mk_jar(
                "optifabric-1.0.jar",
                true,
                json!({ "id": "optifabric", "name": "OptiFabric", "version": "1.0.0" }),
                vec![],
            ),
            mk_jar(
                "cloth-config-15.0.jar",
                false,
                json!({ "id": "cloth-config", "name": "Cloth Config", "version": "15.0.0" }),
                vec![],
            ),
            mk_jar(
                "needs-cloth-and-fapi.jar",
                true,
                json!({
                    "id": "modmenu", "name": "Mod Menu", "version": "21.0.0",
                    "depends": {
                        "cloth-config": ">=15.0.0",
                        "fabric-resource-loader-v1": "*",
                        "fabric-screen-api-v1": "*"
                    }
                }),
                vec![],
            ),
        ];
        let report = evaluate_fabric_compat(&env_26_3(), &jars);
        assert!(report.issues.iter().any(|i| i.filename == "cool-shader-addon.jar"
            && i.kind == CompatIssueKind::IncompatibleDep
            && i.target_id == "sodium"
            && matches!(&i.fix, Some(CompatFix::UpdateMod { filename, .. }) if filename == "sodium-fabric-0.9.2+mc26.3.jar")));
        assert!(report.issues.iter().any(|i| i.filename == "old-mod-1.21.8.jar"
            && i.kind == CompatIssueKind::MinecraftVersion
            && matches!(&i.fix, Some(CompatFix::ChangeVersion { .. }))));
        // Sodium breaking OptiFabric attaches to OptiFabric with Disable OptiFabric!
        assert!(report.issues.iter().any(|i| i.filename == "optifabric-1.0.jar"
            && i.kind == CompatIssueKind::BreaksMod
            && i.target_id == "sodium"
            && matches!(&i.fix, Some(CompatFix::Disable { filename }) if filename == "optifabric-1.0.jar")));
        // Disabled Cloth Config suggests Enable!
        assert!(report.issues.iter().any(|i| i.filename == "needs-cloth-and-fapi.jar"
            && i.kind == CompatIssueKind::DisabledDep
            && matches!(&i.fix, Some(CompatFix::Enable { filename, .. }) if filename == "cloth-config-15.0.jar")));
        // Two missing fabric-* sub-modules collapse into ONE Fabric API MissingDep!
        let fapi_issues: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.filename == "needs-cloth-and-fapi.jar" && i.kind == CompatIssueKind::MissingDep)
            .collect();
        assert_eq!(fapi_issues.len(), 1);
        assert_eq!(fapi_issues[0].target_id, "fabric-api");
    }

    #[test]
    fn human_predicate_formatting_es_and_en() {
        assert_eq!(format_predicate_human(">=0.9.3", true), "0.9.3 o más nuevo");
        assert_eq!(format_predicate_human(">=0.9.3", false), "0.9.3 or newer");
        assert_eq!(format_predicate_human("<=1.11.2", true), "1.11.2 o anterior");
        assert_eq!(format_predicate_human("~1.21.8", true), "1.21.8");
        assert_eq!(format_predicate_human(">=26.1 <27", true), "26.1–<27");
        assert_eq!(format_predicate_human("~1.21.8 || >=26.2", true), "1.21.8 o 26.2 o más nuevo");
    }
}
