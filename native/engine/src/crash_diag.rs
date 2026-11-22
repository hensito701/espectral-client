//! Post-crash diagnostics (smart-mods plan 1.4): a deterministic analyzer over
//! the launch log, the newest `crash-reports/crash-*.txt` and the JVM's
//! `hs_err_pid*.log`, plus the user-triggered mclo.gs upload.
//!
//! [`Engine::diagnose_crash`] only reads local files. [`Engine::upload_crash_log`]
//! is the one network call here and only runs when the player asks for it.
//! Nothing is sent automatically.

use std::collections::{BTreeSet, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::fabric_compat::CompatSeverity;
use crate::launch::effective_memory_mb;
use crate::ziputil::{list_zip_entries, read_zip_entry_capped};

/// Log lines the analyzer reads (the launch ring keeps 2000).
const ANALYZED_LINES: usize = 400;
/// Byte cap for one crash report or hs_err file.
const MAX_REPORT_BYTES: u64 = 1 << 20;
/// Byte cap for one `fabric.mod.json` / `mods.toml` entry.
const MAX_META_BYTES: u64 = 1 << 20;
/// RAM step the "increase RAM" action offers, in MB (clamped to the RAM cap).
const MEMORY_STEP_MB: i64 = 2048;
/// Stack-trace lines scanned after an ABI error for the culprit's class.
const STACK_WINDOW: usize = 40;
/// Packages the culprit search never blames: the game, the JDK and the loaders.
const NEVER_BLAMED: &[&str] = &[
    "net.minecraft.",
    "com.mojang.",
    "java.",
    "javax.",
    "jdk.",
    "sun.",
    "com.sun.",
    "net.fabricmc.",
    "net.minecraftforge.",
    "net.neoforged.",
    "cpw.mods.",
    "org.lwjgl.",
    "org.spongepowered.",
    "org.objectweb.asm.",
];
const MCLOGS_API: &str = "https://api.mclo.gs/1/log";

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("valid regex"));
    };
}

re!(MOD_RESOLUTION, r"ModResolutionException|Incompatible mods found|ModLoadingException|Could not find required mod");
re!(FABRIC_MOD_ID, r"Mod '[^']*' \(([a-z0-9_.\-]+)\)");
re!(FORGE_MOD_ID, r"(?i)mod ?id\W{1,3}([a-z0-9_.\-]+)");
re!(REQUIRED_MISSING, r"Could not find required mod: ([a-z0-9_.\-]+)");
re!(MOD_FILE, r"(?i)mod file\W{1,3}(?:\S*[\\/])?([^\\/\s]+\.jar)");
re!(
    MIXIN_SIG,
    r"MixinApplyError|InvalidInjectionException|MixinTransformerError|InvalidMixinException|Mixin apply for mod"
);
re!(MIXIN_APPLY_MOD, r"Mixin apply for mod ([a-z0-9_.\-]+)");
re!(MIXIN_FROM_MOD, r"from mod ([a-z0-9_.\-]+)");
re!(MIXIN_CONFIG, r"([A-Za-z0-9_.\-]+\.mixins\.json)");
re!(
    ABI_SIG,
    r"NoSuchMethodError|NoSuchFieldError|IncompatibleClassChangeError|AbstractMethodError|ClassNotFoundException|NoClassDefFoundError"
);
re!(ABI_CLASS_IN_MSG, r"'[^']*?\s([a-z][A-Za-z0-9_$]*(?:\.[A-Za-z0-9_$]+)+)\.[A-Za-z0-9_$<>]+\(");
re!(CNF_CLASS, r"(?:ClassNotFoundException|NoClassDefFoundError)[:\s]+([a-z][A-Za-z0-9_$]*(?:[./][A-Za-z0-9_$]+)+)");
re!(FRAME_CLASS, r"\bat ([a-z][A-Za-z0-9_$]*(?:\.[A-Za-z0-9_$]+)+)\.[A-Za-z0-9_$<>]+\(");
re!(OOM, r"java\.lang\.OutOfMemoryError");
re!(GFX_SIG, r"GLFW error 6554[23]|Pixel format not accelerated|driver does not appear to support OpenGL");
re!(GFX_DRIVER, r"(?i)atio6axx|nvoglv(?:32|64)|igxelpicd(?:32|64)|amdvlk(?:64)?");
re!(ACCESS_VIOLATION, r"EXCEPTION_ACCESS_VIOLATION");
re!(RP_RELOAD, r"ResourceReloadException|ResourcePackLoadException");
re!(RP_JSON, r"JsonSyntaxException");
re!(RP_ASSETS, r"assets/");
re!(RP_CONTEXT, r"(?i)resource ?packs?|ResourceReload|assets/");
re!(RP_FILE, r#"file/([^\s'"\[\](),]+)"#);
re!(RP_ZIP, r#"([^\s'"\\/\[\]()]+\.zip)"#);
re!(UNSUPPORTED_CLASS, r"UnsupportedClassVersionError");
re!(CLASS_FILE_VERSION, r"class file version (\d+)\.\d+");
re!(RUNNING_MAX_VERSION, r"versions up to (\d+)\.\d+");
re!(EXCERPT_START, r"Exception|Error|Caused by|Description:");
re!(TOML_MOD_ID, r#"(?m)^\s*modId\s*=\s*"([^"]+)""#);
re!(TOML_DISPLAY, r#"(?m)^\s*displayName\s*=\s*"([^"]+)""#);
re!(RED_SECRET_ARG, r#"(?i)((?:--accessToken|--session|auth_access_token|auth_session|bearer)[\s=:]+)[^\s"',;]+"#);
re!(RED_JWT, r"eyJ[A-Za-z0-9_\-]{5,}\.[A-Za-z0-9_\-]{5,}\.[A-Za-z0-9_\-]{5,}");
re!(RED_HEX64, r"\b[0-9a-fA-F]{64}\b");
re!(RED_WIN_USER, r#"(?i)([A-Z]:[\\/]+Users[\\/]+)[^\\/"':\r\n]+"#);
re!(RED_UNIX_USER, r#"(/home/|/Users/)[^/\s"':]+"#);
re!(RED_IPV4, r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b");
re!(RED_IPV6_FULL, r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b");
re!(RED_IPV6_COMPRESSED, r"(?i)\b(?:[0-9a-f]{1,4}:){1,7}:(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,6})?");
re!(RED_IPV6_LEADING, r"(?i)::[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,6}\b");

// ---------------------------------------------------------------------------
// Public contract
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashCategory {
    ModIncompatible,
    MixinConflict,
    MissingClassOrMethod,
    OutOfMemory,
    GraphicsDriver,
    ResourcePack,
    JavaMismatch,
    NativeCrash,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspectConfidence {
    High,
    Medium,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuspectedMod {
    pub filename: String,
    pub mod_id: Option<String>,
    pub display_name: String,
    pub confidence: SuspectConfidence,
    /// The log or stack line that named this mod (technical text, not translated).
    pub reason: String,
}

/// What the player can do about a crash. The first fix action is the primary
/// button; the others (`OpenMods`, `OpenCrashReport`, `UploadLogs`, `ViewLog`)
/// are navigation or sharing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashAction {
    DisableMod(String),
    IncreaseMemory(u32),
    DisableShaders,
    DisableResourcePack(String),
    OpenMods,
    OpenCrashReport,
    UploadLogs,
    ViewLog,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrashDiagnosis {
    pub instance: String,
    pub exit_code: Option<i32>,
    pub category: CrashCategory,
    /// i18n key (`crash.cause.*`) the app translates with `summary_args`.
    pub summary_key: String,
    pub summary_args: IndexMap<String, String>,
    pub suspected_mods: Vec<SuspectedMod>,
    pub actions: Vec<CrashAction>,
    /// The crash report (or hs_err) file the diagnosis read.
    pub crash_report_path: Option<String>,
    /// 8-15 consecutive lines around the first error, for the modal.
    pub excerpt: Vec<String>,
}

/// mclo.gs reply for an uploaded log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadedLog {
    pub id: String,
    pub url: String,
}

/// One mod jar as the analyzer sees it. Built from the jar on disk by
/// [`Engine::diagnose_crash`]; tests build them directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModCandidate {
    /// Enabled jar filename (`sodium-0.5.8.jar`), the id the facade toggles by.
    pub filename: String,
    pub mod_id: Option<String>,
    pub display_name: Option<String>,
    /// `mixins` config names from `fabric.mod.json`.
    pub mixin_configs: Vec<String>,
    /// Entrypoint classes from `fabric.mod.json` (`::method` stripped).
    pub entrypoint_classes: Vec<String>,
    /// Dotted packages of the jar's classes (culprit lookup for ABI errors).
    pub packages: BTreeSet<String>,
    /// The compatibility check reports an error for this jar.
    pub compat_flagged: bool,
}

/// Everything [`diagnose`] reads. The caller does all file I/O.
#[derive(Debug, Clone, Copy)]
pub struct CrashInput<'a> {
    pub instance: &'a str,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<&'a str>,
    /// Launch output, oldest first.
    pub lines: &'a [String],
    pub crash_report: Option<&'a str>,
    /// The file the crash report (or hs_err) was read from.
    pub report_path: Option<&'a str>,
    pub hs_err: Option<&'a str>,
    pub mods: &'a [ModCandidate],
    pub current_memory_mb: u32,
    /// The RAM a bump would grant, already clamped to the RAM cap. Equal to the
    /// current amount (or lower) when no bump is possible.
    pub memory_ceiling_mb: u32,
}

/// `4` or `4.5` for a RAM amount in MB (plain text, no locale).
pub fn memory_label_gb(mb: u32) -> String {
    if mb % 1024 == 0 {
        (mb / 1024).to_string()
    } else {
        format!("{:.1}", f64::from(mb) / 1024.0)
    }
}

/// Strip what must not leave the machine before a log is uploaded: OS user
/// names in paths, access and bridge tokens, IP addresses. Everything else is
/// kept byte-for-byte so the log still reads like the original.
pub fn redact_log(text: &str) -> String {
    let s = RED_SECRET_ARG.replace_all(text, "${1}<REDACTED>");
    let s = RED_JWT.replace_all(&s, "<REDACTED>");
    let s = RED_HEX64.replace_all(&s, "<REDACTED>");
    let s = RED_WIN_USER.replace_all(&s, "${1}<USER>");
    let s = RED_UNIX_USER.replace_all(&s, "${1}<USER>");
    let s = RED_IPV4.replace_all(&s, "<IP>");
    let s = RED_IPV6_FULL.replace_all(&s, "<IP>");
    let s = RED_IPV6_COMPRESSED.replace_all(&s, "<IP>");
    RED_IPV6_LEADING.replace_all(&s, "<IP>").into_owned()
}

/// Deterministic diagnosis. Pure: no I/O, no network.
pub fn diagnose(input: &CrashInput<'_>) -> CrashDiagnosis {
    let mut all: Vec<&str> = input.lines.iter().map(String::as_str).collect();
    if let Some(report) = input.crash_report {
        all.extend(report.lines());
    }
    if let Some(hs_err) = input.hs_err {
        all.extend(hs_err.lines());
    }
    // Priority order from the plan: the first matcher that fires names the cause.
    let verdict = match_mod_incompatible(input, &all)
        .or_else(|| match_mixin(input, &all))
        .or_else(|| match_missing_class(input, &all))
        .or_else(|| match_out_of_memory(input, &all))
        .or_else(|| match_graphics(&all))
        .or_else(|| match_resource_pack(&all))
        .or_else(|| match_java_mismatch(&all))
        .or_else(|| match_native_crash(input))
        .unwrap_or_else(|| Verdict::new(CrashCategory::Unknown, "crash.cause.unknown"));

    let mut actions = verdict.actions;
    if matches!(
        verdict.category,
        CrashCategory::ModIncompatible | CrashCategory::MixinConflict | CrashCategory::MissingClassOrMethod
    ) {
        actions.push(CrashAction::OpenMods);
    }
    if input.report_path.is_some() {
        actions.push(CrashAction::OpenCrashReport);
    }
    actions.push(CrashAction::UploadLogs);
    actions.push(CrashAction::ViewLog);

    let mut suspected_mods = verdict.suspects;
    // High-confidence culprits first; the sort is stable within a confidence.
    suspected_mods.sort_by_key(|s| s.confidence == SuspectConfidence::Medium);

    CrashDiagnosis {
        instance: input.instance.to_string(),
        exit_code: input.exit_code,
        category: verdict.category,
        summary_key: verdict.summary_key.to_string(),
        summary_args: verdict.args,
        suspected_mods,
        actions,
        crash_report_path: input.report_path.map(str::to_string),
        excerpt: excerpt(&all),
    }
}

// ---------------------------------------------------------------------------
// Matchers
// ---------------------------------------------------------------------------

/// What one matcher found, before the shared tail adds navigation actions.
struct Verdict {
    category: CrashCategory,
    summary_key: &'static str,
    args: IndexMap<String, String>,
    suspects: Vec<SuspectedMod>,
    actions: Vec<CrashAction>,
}

impl Verdict {
    fn new(category: CrashCategory, summary_key: &'static str) -> Self {
        Verdict { category, summary_key, args: IndexMap::new(), suspects: Vec::new(), actions: Vec::new() }
    }

    fn arg(mut self, key: &str, value: impl Into<String>) -> Self {
        self.args.insert(key.to_string(), value.into());
        self
    }
}

/// A verdict that may name a culprit: the first suspect fills `{mod}` and its
/// jar becomes the primary "disable" action.
fn culprit(
    category: CrashCategory,
    named_key: &'static str,
    generic_key: &'static str,
    suspects: Vec<SuspectedMod>,
) -> Verdict {
    let first = suspects.first().map(|s| (s.display_name.clone(), s.filename.clone()));
    let mut verdict = match &first {
        Some((name, _)) => Verdict::new(category, named_key).arg("mod", name.as_str()),
        None => Verdict::new(category, generic_key),
    };
    if let Some((_, file)) = first {
        verdict.actions.push(CrashAction::DisableMod(file));
    }
    verdict.suspects = suspects;
    verdict
}

/// Fabric `ModResolutionException` / Forge `ModLoadingException`: the offending
/// mod ids come straight from the formatted error block. The compatibility
/// check is the fallback when the block names no mod.
fn match_mod_incompatible(input: &CrashInput<'_>, all: &[&str]) -> Option<Verdict> {
    if !any_line(all, &MOD_RESOLUTION) {
        return None;
    }
    let mut suspects = Vec::new();
    let ids = captures(all, &FABRIC_MOD_ID)
        .into_iter()
        .chain(captures(all, &FORGE_MOD_ID))
        .chain(captures(all, &REQUIRED_MISSING));
    for (id, line) in ids {
        if let Some(m) = input.mods.iter().find(|m| m.mod_id.as_deref() == Some(id.as_str())) {
            push_suspect(&mut suspects, suspect(m, SuspectConfidence::High, line));
        }
    }
    for (file, line) in captures(all, &MOD_FILE) {
        if let Some(m) = input.mods.iter().find(|m| m.filename == file) {
            push_suspect(&mut suspects, suspect(m, SuspectConfidence::High, line));
        }
    }
    if suspects.is_empty() {
        for m in input.mods.iter().filter(|m| m.compat_flagged).take(3) {
            push_suspect(
                &mut suspects,
                suspect(m, SuspectConfidence::Medium, "mod-resolution error without a mod id in the log"),
            );
        }
    }
    Some(culprit(
        CrashCategory::ModIncompatible,
        "crash.cause.modIncompatible",
        "crash.cause.modIncompatibleGeneric",
        suspects,
    ))
}

/// Mixin failures name the mod (`Mixin apply for mod X`) or its config file
/// (`X.mixins.json`); both resolve to a jar with High confidence.
fn match_mixin(input: &CrashInput<'_>, all: &[&str]) -> Option<Verdict> {
    let lines: Vec<&str> = all.iter().copied().filter(|l| MIXIN_SIG.is_match(l)).collect();
    if lines.is_empty() {
        return None;
    }
    let mut suspects = Vec::new();
    let ids = captures(&lines, &MIXIN_APPLY_MOD).into_iter().chain(captures(&lines, &MIXIN_FROM_MOD));
    for (id, line) in ids {
        if let Some(m) = input.mods.iter().find(|m| m.mod_id.as_deref() == Some(id.as_str())) {
            push_suspect(&mut suspects, suspect(m, SuspectConfidence::High, line));
        }
    }
    for (config, line) in captures(&lines, &MIXIN_CONFIG) {
        if let Some(m) = input.mods.iter().find(|m| m.mixin_configs.contains(&config)) {
            push_suspect(&mut suspects, suspect(m, SuspectConfidence::High, line));
        }
    }
    Some(culprit(
        CrashCategory::MixinConflict,
        "crash.cause.mixinConflict",
        "crash.cause.mixinConflictGeneric",
        suspects,
    ))
}

/// ABI errors: the first third-party class in the error and the frames after
/// it names the jar that owns it. A class in a declared entrypoint is High,
/// any other class from that jar Medium.
fn match_missing_class(input: &CrashInput<'_>, all: &[&str]) -> Option<Verdict> {
    let start = all.iter().position(|l| ABI_SIG.is_match(l))?;
    let mut classes: Vec<(String, &str)> = Vec::new();
    for line in all.iter().skip(start).take(STACK_WINDOW).copied() {
        for re in [&*CNF_CLASS, &*ABI_CLASS_IN_MSG, &*FRAME_CLASS] {
            for (class, _) in captures(&[line], re) {
                let class = class.replace('/', ".");
                if !NEVER_BLAMED.iter().any(|p| class.starts_with(*p)) {
                    classes.push((class, line));
                }
            }
        }
    }
    let first_class = classes.first().map(|(c, _)| c.clone());
    let mut suspects = Vec::new();
    for (class, line) in &classes {
        if let Some((m, entrypoint)) = owner_of(class, input.mods) {
            let confidence = if entrypoint { SuspectConfidence::High } else { SuspectConfidence::Medium };
            push_suspect(&mut suspects, suspect(m, confidence, line));
            break;
        }
    }
    let mut verdict = culprit(
        CrashCategory::MissingClassOrMethod,
        "crash.cause.missingClassOrMethod",
        "crash.cause.missingClassOrMethodGeneric",
        suspects,
    );
    if let Some(class) = first_class {
        verdict.args.insert("class".to_string(), class);
    }
    Some(verdict)
}

fn match_out_of_memory(input: &CrashInput<'_>, all: &[&str]) -> Option<Verdict> {
    if !any_line(all, &OOM) {
        return None;
    }
    let mut verdict = Verdict::new(CrashCategory::OutOfMemory, "crash.cause.outOfMemory");
    if input.memory_ceiling_mb > input.current_memory_mb {
        verdict.actions.push(CrashAction::IncreaseMemory(input.memory_ceiling_mb));
    }
    Some(verdict)
}

/// GLFW / pixel-format failures, or an access violation inside a GPU driver DLL.
fn match_graphics(all: &[&str]) -> Option<Verdict> {
    let driver = any_line(all, &GFX_DRIVER);
    let violation = any_line(all, &ACCESS_VIOLATION);
    if !any_line(all, &GFX_SIG) && !(violation && driver) {
        return None;
    }
    let mut verdict = Verdict::new(CrashCategory::GraphicsDriver, "crash.cause.graphicsDriver");
    verdict.actions.push(CrashAction::DisableShaders);
    Some(verdict)
}

/// Resource-pack reload failures; the pack is named on the same lines.
fn match_resource_pack(all: &[&str]) -> Option<Verdict> {
    let reload = any_line(all, &RP_RELOAD);
    let json = any_line(all, &RP_JSON) && any_line(all, &RP_ASSETS);
    if !reload && !json {
        return None;
    }
    let context: Vec<&str> = all.iter().copied().filter(|l| RP_CONTEXT.is_match(l)).collect();
    let pack = captures(&context, &RP_FILE)
        .into_iter()
        .next()
        .or_else(|| captures(&context, &RP_ZIP).into_iter().next())
        .map(|(name, _)| name);
    let mut verdict = match &pack {
        Some(name) => Verdict::new(CrashCategory::ResourcePack, "crash.cause.resourcePack").arg("pack", name.as_str()),
        None => Verdict::new(CrashCategory::ResourcePack, "crash.cause.resourcePackUnknown"),
    };
    if let Some(name) = pack {
        verdict.actions.push(CrashAction::DisableResourcePack(name));
    }
    Some(verdict)
}

/// `UnsupportedClassVersionError`: class file major minus 44 is the Java
/// feature release the code needs; the "up to" major is what the JVM runs.
fn match_java_mismatch(all: &[&str]) -> Option<Verdict> {
    let line = all.iter().copied().find(|l| UNSUPPORTED_CLASS.is_match(l))?;
    let needed = java_feature_release(line, &CLASS_FILE_VERSION);
    let running = java_feature_release(line, &RUNNING_MAX_VERSION);
    Some(
        Verdict::new(CrashCategory::JavaMismatch, "crash.cause.javaMismatch")
            .arg("required", needed.map_or_else(|| "?".to_string(), |v| v.to_string()))
            .arg("running", running.map_or_else(|| "?".to_string(), |v| v.to_string())),
    )
}

/// JVM fatal error: an hs_err file, a signal, or a native exit code.
fn match_native_crash(input: &CrashInput<'_>) -> Option<Verdict> {
    let native = input.hs_err.is_some()
        || input.exit_signal.is_some()
        || matches!(input.exit_code, Some(-1_073_741_819 | 134 | 139));
    native.then(|| Verdict::new(CrashCategory::NativeCrash, "crash.cause.nativeCrash"))
}

// ---------------------------------------------------------------------------
// Matcher helpers
// ---------------------------------------------------------------------------

fn any_line(all: &[&str], re: &Regex) -> bool {
    all.iter().any(|l| re.is_match(l))
}

/// Group 1 of every match, paired with the line it came from.
fn captures<'a>(lines: &[&'a str], re: &Regex) -> Vec<(String, &'a str)> {
    let mut out = Vec::new();
    for &line in lines {
        for c in re.captures_iter(line) {
            if let Some(m) = c.get(1) {
                out.push((m.as_str().to_string(), line));
            }
        }
    }
    out
}

fn java_feature_release(text: &str, re: &Regex) -> Option<u32> {
    re.captures(text)?.get(1)?.as_str().parse::<u32>().ok().map(|major| major.saturating_sub(44))
}

/// The mod whose jar holds the most specific package of `class`, and whether
/// `class` is one of its declared entrypoints.
fn owner_of<'a>(class: &str, mods: &'a [ModCandidate]) -> Option<(&'a ModCandidate, bool)> {
    let mut best: Option<(&ModCandidate, usize)> = None;
    for m in mods {
        for pkg in &m.packages {
            let inside = class.strip_prefix(pkg.as_str()).is_some_and(|rest| rest.starts_with('.'));
            if inside && best.is_none_or(|(_, len)| pkg.len() > len) {
                best = Some((m, pkg.len()));
            }
        }
    }
    best.map(|(m, _)| (m, m.entrypoint_classes.iter().any(|e| e == class)))
}

fn label(m: &ModCandidate) -> String {
    m.display_name.clone().or_else(|| m.mod_id.clone()).unwrap_or_else(|| m.filename.clone())
}

fn suspect(m: &ModCandidate, confidence: SuspectConfidence, evidence: &str) -> SuspectedMod {
    let mut reason: String = evidence.chars().take(300).collect();
    if m.compat_flagged {
        reason.push_str(" [compat check: error]");
    }
    SuspectedMod { filename: m.filename.clone(), mod_id: m.mod_id.clone(), display_name: label(m), confidence, reason }
}

fn push_suspect(out: &mut Vec<SuspectedMod>, s: SuspectedMod) {
    if !out.iter().any(|x| x.filename == s.filename) {
        out.push(s);
    }
}

/// 8-15 consecutive lines starting at the first error line (the stack trace
/// the player most likely wants to read). Falls back to the last 8 lines.
fn excerpt(all: &[&str]) -> Vec<String> {
    let start = all.iter().position(|l| EXCERPT_START.is_match(l)).unwrap_or(all.len().saturating_sub(8));
    all.iter().skip(start).take(15).map(|l| l.chars().take(400).collect::<String>()).collect()
}

// ---------------------------------------------------------------------------
// File gathering (I/O; the analyzer above stays pure)
// ---------------------------------------------------------------------------

/// Newest file in `dir` whose name matches and that was modified at or after `since_ms`.
fn newest_since(dir: &Path, prefix: &str, suffix: &str, since_ms: i64) -> Option<PathBuf> {
    let mut best: Option<(i64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(prefix) || !name.ends_with(suffix) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        let ms = modified.duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        if ms >= since_ms && best.as_ref().is_none_or(|(b, _)| ms > *b) {
            best = Some((ms, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

fn read_capped(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(MAX_REPORT_BYTES).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn find_crash_report(game_dir: &Path, started_at: i64) -> Option<(PathBuf, String)> {
    let path = newest_since(&game_dir.join("crash-reports"), "crash-", ".txt", started_at)?;
    let text = read_capped(&path)?;
    Some((path, text))
}

fn read_json_entry(jar: &Path, entry: &str) -> Option<Value> {
    let bytes = read_zip_entry_capped(jar, entry, MAX_META_BYTES).ok()??;
    let text = String::from_utf8_lossy(&bytes);
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// Dotted package of every class in the jar (META-INF and root classes excluded).
fn class_packages(jar: &Path) -> BTreeSet<String> {
    let Ok(entries) = list_zip_entries(jar) else { return BTreeSet::new() };
    entries
        .iter()
        .filter_map(|e| e.strip_suffix(".class"))
        .filter_map(|e| e.rsplit_once('/'))
        .filter(|(dir, _)| !dir.starts_with("META-INF"))
        .map(|(dir, _)| dir.replace('/', "."))
        .collect()
}

fn read_mod_meta(jar: &Path, c: &mut ModCandidate) {
    if let Some(v) = read_json_entry(jar, "fabric.mod.json") {
        c.mod_id = v.get("id").and_then(Value::as_str).map(str::to_string);
        c.display_name = v.get("name").and_then(Value::as_str).map(str::to_string);
        c.mixin_configs = v
            .get("mixins")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|m| m.as_str().or_else(|| m.get("config").and_then(Value::as_str)).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        c.entrypoint_classes = entrypoint_classes(&v);
        return;
    }
    // Forge / NeoForge: the first [[mods]] entry is enough for matching.
    for entry in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Ok(Some(bytes)) = read_zip_entry_capped(jar, entry, MAX_META_BYTES) {
            let text = String::from_utf8_lossy(&bytes);
            c.mod_id = TOML_MOD_ID.captures(&text).and_then(|m| m.get(1)).map(|m| m.as_str().to_string());
            c.display_name = TOML_DISPLAY.captures(&text).and_then(|m| m.get(1)).map(|m| m.as_str().to_string());
            return;
        }
    }
}

/// `entrypoints` values are a class, an array of classes, or `{ "value": class }`.
fn entrypoint_classes(v: &Value) -> Vec<String> {
    let Some(entrypoints) = v.get("entrypoints").and_then(Value::as_object) else { return Vec::new() };
    let mut out = Vec::new();
    for value in entrypoints.values() {
        let items: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        for item in items {
            let class = match item {
                Value::String(s) => Some(s.as_str()),
                Value::Object(o) => o.get("value").and_then(Value::as_str),
                _ => None,
            };
            if let Some(class) = class {
                out.push(class.split("::").next().unwrap_or(class).to_string());
            }
        }
    }
    out
}

/// Every enabled `.jar` in `mods_dir`, read for its id, mixins, entrypoints and packages.
fn scan_mod_candidates(mods_dir: &Path, flagged: &HashSet<String>) -> Vec<ModCandidate> {
    let Ok(read) = std::fs::read_dir(mods_dir) else { return Vec::new() };
    let mut names: Vec<String> =
        read.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".jar")).collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let path = mods_dir.join(&name);
            let mut candidate = ModCandidate {
                compat_flagged: flagged.contains(&name),
                packages: class_packages(&path),
                filename: name,
                ..Default::default()
            };
            read_mod_meta(&path, &mut candidate);
            candidate
        })
        .collect()
}

/// Write `content` beside `file` and rename over it, so an interrupted write
/// never leaves a truncated config behind.
fn write_atomic(file: &Path, content: &str) -> EngineResult<()> {
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

/// Set `key=value` in a Java-properties file: the last existing line for the
/// key is replaced in place, other duplicates are dropped (Java reads last-wins),
/// and the line is appended when absent.
fn set_property_file(file: &Path, key: &str, value: &str) -> EngineResult<()> {
    let is_key = |line: &str| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('#')
            && !trimmed.starts_with('!')
            && line.split_once('=').is_some_and(|(k, _)| k.trim() == key)
    };
    let mut lines: Vec<String> =
        std::fs::read_to_string(file).unwrap_or_default().lines().map(str::to_string).collect();
    let entry = format!("{key}={value}");
    match lines.iter().rposition(|l| is_key(l.as_str())) {
        Some(idx) => {
            lines = lines
                .into_iter()
                .enumerate()
                .filter_map(|(i, l)| {
                    if i == idx {
                        Some(entry.clone())
                    } else if is_key(l.as_str()) {
                        None
                    } else {
                        Some(l)
                    }
                })
                .collect();
        }
        None => lines.push(entry),
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_atomic(file, &format!("{}\n", lines.join("\n")))
}

/// Remove `file/<pack>` from the `resourcePacks:` JSON list in `options.txt`.
/// Returns false when the pack is not listed.
fn drop_resource_pack(options: &Path, pack: &str) -> EngineResult<bool> {
    let Ok(text) = std::fs::read_to_string(options) else { return Ok(false) };
    let Some(current) = text.lines().find_map(|l| l.strip_prefix("resourcePacks:")) else { return Ok(false) };
    let mut packs: Vec<String> = serde_json::from_str(current)
        .map_err(|e| EngineError::other(format!("options.txt resourcePacks is not a JSON list: {e}")))?;
    let entry = format!("file/{pack}");
    let before = packs.len();
    packs.retain(|p| *p != entry);
    if packs.len() == before {
        return Ok(false);
    }
    let value = serde_json::to_string(&packs).map_err(|e| EngineError::other(e.to_string()))?;
    let out: Vec<String> = text
        .lines()
        .map(|l| if l.starts_with("resourcePacks:") { format!("resourcePacks:{value}") } else { l.to_string() })
        .collect();
    write_atomic(options, &format!("{}\n", out.join("\n")))?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// mclo.gs upload
// ---------------------------------------------------------------------------

fn mclogs_form_body(content: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new()).append_pair("content", content).finish()
}

fn parse_mclogs_reply(status: u16, body: &str) -> EngineResult<UploadedLog> {
    let v: Value = serde_json::from_str(body)
        .map_err(|_| EngineError::other(format!("mclo.gs replied HTTP {status} without JSON")))?;
    if v.get("success").and_then(Value::as_bool) == Some(true) {
        let id = v.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let url = v.get("url").and_then(Value::as_str).unwrap_or_default().to_string();
        if url.is_empty() {
            return Err(EngineError::other("mclo.gs reply has no url"));
        }
        return Ok(UploadedLog { id, url });
    }
    let why = v.get("error").and_then(Value::as_str).unwrap_or("unknown error");
    Err(EngineError::other(format!("mclo.gs upload failed: {why}")))
}

async fn upload_to(client: &reqwest::Client, api: &str, content: &str) -> EngineResult<UploadedLog> {
    let res = client
        .post(api)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(mclogs_form_body(content))
        .send()
        .await
        .map_err(|e| EngineError::other(format!("mclo.gs upload failed: {e}")))?;
    let status = res.status().as_u16();
    let body = res.text().await.map_err(|e| EngineError::other(format!("mclo.gs upload failed: {e}")))?;
    parse_mclogs_reply(status, &body)
}

// ---------------------------------------------------------------------------
// Engine facade
// ---------------------------------------------------------------------------

impl Engine {
    /// Diagnose the game that ended under `launch_key`. None for a clean exit,
    /// a stop the player asked for, a launch that never started a game, or a
    /// launch still running. Reads local files only.
    pub async fn diagnose_crash(&self, launch_key: &str) -> EngineResult<Option<CrashDiagnosis>> {
        let Some(src) = self.crash_source(launch_key) else { return Ok(None) };
        if src.running || src.user_stopped || src.exit_code == Some(0) {
            return Ok(None);
        }
        let Some(game_dir) = src.game_dir.as_deref() else { return Ok(None) };

        let instance = crate::launch::load_instance(self, &src.instance).await?;
        let current_mb = instance.memory_mb.filter(|mb| *mb > 0).unwrap_or_else(|| self.config().default_memory_mb());
        let ceiling =
            effective_memory_mb(i64::from(current_mb) + MEMORY_STEP_MB, crate::platform::total_memory_bytes());
        let memory_ceiling_mb = u32::try_from(ceiling).unwrap_or(current_mb);

        let flagged: HashSet<String> = match crate::fabric_compat::check_instance_mods_compat(self, &src.instance).await
        {
            Ok(report) => report
                .issues
                .iter()
                .filter(|i| i.severity == CompatSeverity::Error)
                .map(|i| i.filename.clone())
                .collect(),
            Err(_) => HashSet::new(),
        };
        let mods = scan_mod_candidates(&game_dir.join("mods"), &flagged);

        let report = find_crash_report(game_dir, src.started_at);
        let hs_err = newest_since(game_dir, "hs_err_pid", ".log", src.started_at)
            .and_then(|p| read_capped(&p).map(|text| (p, text)));
        let report_path = report.as_ref().or(hs_err.as_ref()).map(|(p, _)| p.to_string_lossy().into_owned());
        let lines: Vec<String> = src.lines.iter().rev().take(ANALYZED_LINES).rev().cloned().collect();

        let input = CrashInput {
            instance: &src.instance,
            exit_code: src.exit_code,
            exit_signal: src.exit_signal.as_deref(),
            lines: &lines,
            crash_report: report.as_ref().map(|(_, text)| text.as_str()),
            report_path: report_path.as_deref(),
            hs_err: hs_err.as_ref().map(|(_, text)| text.as_str()),
            mods: &mods,
            current_memory_mb: current_mb,
            memory_ceiling_mb,
        };
        Ok(Some(diagnose(&input)))
    }

    /// Upload the game's log (plus its crash report) to mclo.gs after redacting
    /// user paths, tokens and IP addresses. Runs only when the player asks.
    pub async fn upload_crash_log(&self, launch_key: &str) -> EngineResult<UploadedLog> {
        let src = self
            .crash_source(launch_key)
            .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("unknown launch: {launch_key}")))?;
        let mut text = src.lines.join("\n");
        if let Some((_, report)) = src.game_dir.as_deref().and_then(|d| find_crash_report(d, src.started_at)) {
            text.push_str("\n\n----- crash report -----\n");
            text.push_str(&report);
        }
        upload_to(self.http().client(), MCLOGS_API, &redact_log(&text)).await
    }

    /// Crash fix: turn Iris shaders off (`enableShaders=false`) in the game dir
    /// of the crashed launch. 409 ALREADY_RUNNING while the instance runs.
    pub fn disable_shaders_for_launch(&self, launch_key: &str) -> EngineResult<()> {
        let game_dir = self.launch_game_dir(launch_key)?;
        set_property_file(&crate::mods::iris_properties_path(&game_dir), "enableShaders", "false")
    }

    /// Crash fix: remove a resource pack from the crashed launch's
    /// `options.txt`. Returns false when the pack is not listed.
    /// 409 ALREADY_RUNNING while the instance runs.
    pub fn disable_resource_pack_for_launch(&self, launch_key: &str, pack: &str) -> EngineResult<bool> {
        let game_dir = self.launch_game_dir(launch_key)?;
        drop_resource_pack(&game_dir.join("options.txt"), pack)
    }

    /// The game dir of a finished launch, after the running-instance guard.
    fn launch_game_dir(&self, launch_key: &str) -> EngineResult<PathBuf> {
        let src = self
            .crash_source(launch_key)
            .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("unknown launch: {launch_key}")))?;
        if self.active_instances().contains(&src.instance) {
            return Err(EngineError::conflict(
                "ALREADY_RUNNING",
                format!("instance '{}' is running; stop it first", src.instance),
            ));
        }
        src.game_dir.ok_or_else(|| EngineError::bad_request("NO_GAME", "this launch never started a game"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use serde_json::json;

    fn run(lines: &[&str], mods: &[ModCandidate], current: u32, ceiling: u32) -> CrashDiagnosis {
        let owned: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
        diagnose(&CrashInput {
            instance: "test",
            exit_code: Some(1),
            exit_signal: None,
            lines: &owned,
            crash_report: None,
            report_path: None,
            hs_err: None,
            mods,
            current_memory_mb: current,
            memory_ceiling_mb: ceiling,
        })
    }

    #[test]
    fn identifies_mixin_culprit_oom_abi_and_redacts_mclogs() {
        let mods = vec![
            ModCandidate {
                filename: "sodium-0.5.8.jar".into(),
                mod_id: Some("sodium".into()),
                display_name: Some("Sodium".into()),
                mixin_configs: vec!["sodium.mixins.json".into()],
                ..Default::default()
            },
            ModCandidate {
                filename: "addon-1.0.jar".into(),
                mod_id: Some("addon".into()),
                display_name: Some("Addon".into()),
                packages: BTreeSet::from(["com.addon.core".to_string()]),
                ..Default::default()
            },
        ];

        let mixin = run(
            &[
                "[Render thread/ERROR]: Mixin apply for mod sodium failed sodium.mixins.json:render.MixinChunk from mod sodium",
                "org.spongepowered.asm.mixin.transformer.throwables.MixinApplyError: Mixin [sodium.mixins.json] failed",
            ],
            &mods,
            4096,
            4096,
        );
        assert_eq!(mixin.category, CrashCategory::MixinConflict);
        assert_eq!(mixin.suspected_mods[0].filename, "sodium-0.5.8.jar");
        assert_eq!(mixin.suspected_mods[0].confidence, SuspectConfidence::High);
        assert_eq!(mixin.actions[0], CrashAction::DisableMod("sodium-0.5.8.jar".into()));

        let oom = run(&["java.lang.OutOfMemoryError: Java heap space"], &mods, 2048, 4096);
        assert_eq!(oom.category, CrashCategory::OutOfMemory);
        assert_eq!(oom.actions[0], CrashAction::IncreaseMemory(4096));
        let no_bump = run(&["java.lang.OutOfMemoryError: Java heap space"], &mods, 4096, 4096);
        assert!(!no_bump.actions.contains(&CrashAction::IncreaseMemory(4096)));

        let abi = run(
            &[
                "java.lang.NoSuchMethodError: 'void com.addon.core.Engine.tick(int)'",
                "\tat com.addon.core.Engine.tick(Engine.java:42)",
                "\tat net.minecraft.client.Minecraft.tick(Minecraft.java:1)",
            ],
            &mods,
            4096,
            4096,
        );
        assert_eq!(abi.category, CrashCategory::MissingClassOrMethod);
        assert_eq!(abi.suspected_mods[0].filename, "addon-1.0.jar");
        assert_eq!(abi.suspected_mods[0].confidence, SuspectConfidence::Medium);
        assert_eq!(abi.summary_args.get("class").map(String::as_str), Some("com.addon.core.Engine"));

        let raw = "Loading C:\\Users\\aitor\\AppData\\x and /home/aitor/.omp; --accessToken eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl \
                   0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef from 192.168.1.20 and 2001:db8::1 at [12:34:56]";
        let red = redact_log(raw);
        assert!(!red.contains("aitor"), "{red}");
        assert!(!red.contains("eyJhbGciOi"), "{red}");
        assert!(!red.contains("0123456789abcdef0123"), "{red}");
        assert!(!red.contains("192.168.1.20") && !red.contains("2001:db8::1"), "{red}");
        assert!(red.contains("C:\\Users\\<USER>\\AppData"), "{red}");
        assert!(red.contains("[12:34:56]"), "{red}");
        assert_eq!(mclogs_form_body("a b&c=d"), "content=a+b%26c%3Dd");
    }

    #[tokio::test]
    async fn uploads_redacted_form_to_mclogs() {
        let server = MockServer::start(|_| {
            MockResponse::json(&json!({"success": true, "id": "abc", "url": "https://mclo.gs/abc"}))
        })
        .await;
        let (_dir, engine) = temp_engine();
        let text = "user C:\\Users\\aitor\\x --accessToken SECRETTOKEN123 ip 10.0.0.7";
        let up = upload_to(engine.http().client(), &server.url("/1/log"), &redact_log(text)).await.expect("upload");
        assert_eq!(up.url, "https://mclo.gs/abc");
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        let content = url::form_urlencoded::parse(&requests[0].body)
            .find(|(k, _)| k == "content")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        assert!(!content.contains("aitor") && !content.contains("SECRETTOKEN123") && !content.contains("10.0.0.7"));
        assert!(content.contains("<USER>") && content.contains("<IP>"), "{content}");
    }
}
