//! Port of `src/engine/launch.mjs` — account resolution, JVM/game argument
//! construction, run-dir selection, `resolveLaunch`, dry-run, spawning the
//! game (detached, output piped, latest.log marker watch, boot phases).
//!
//! **Owner: game-launch.** Status: PORTED.
//!
//! Argv order is fixed and covered by the tests below (ported from
//! tests/launch.test.mjs, preferences.test.mjs, personalization.test.mjs and
//! multi-instance.test.mjs).
//!
//! Notes / deliberate differences:
//! - The JS CLI (`node launch.mjs --dry-run …`) is not ported; the dry run is
//!   [`dry_run_result`] (+ [`library_stats`], [`format_command_preview`]).
//! - A spawn failure (java missing, …) is a synchronous `Err` here; in Node it
//!   surfaced later as the child's `error` event. The launch route reports it
//!   through its spawn-error path (launch-exit with the error) either way.
//! - Game output is delivered to `on_log` line by line (Node delivered raw
//!   chunks; the route split them into lines anyway).

pub mod process;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::rules::RuleEnv;
use crate::types::{
    Account, DryRunResult, InstanceMeta, JavaRuntime, LaunchAccount, LaunchMode, Loader, LoaderLaunch, ResolvedLaunch,
    ResolvedVersionRef, TokenKind,
};

pub use process::{ExitStatusInfo, ProcessHandle};

/// `Sound engine started` — the main-menu marker (JS MARKER_RE).
pub const MARKER: &str = "Sound engine started";
/// Initial heap. The game starts small and G1 grows it toward -Xmx only as needed.
pub const INITIAL_HEAP_MB: u32 = 512;
pub const LAUNCHER_NAME: &str = "Espectral Client";
pub const LAUNCHER_VERSION: &str = "0.6.0";

/// Fixed JVM arg set — order is significant and asserted by tests.
pub const FIXED_JVM_ARGS: [&str; 13] = [
    "--add-opens",
    "java.base/java.lang=ALL-UNNAMED",
    "--add-opens",
    "java.base/java.util=ALL-UNNAMED",
    "--add-opens",
    "java.base/java.io=ALL-UNNAMED",
    "--add-opens",
    "java.base/java.nio=ALL-UNNAMED",
    "--add-opens",
    "java.base/sun.nio.ch=ALL-UNNAMED",
    "--add-exports",
    "java.base/sun.nio.ch=ALL-UNNAMED",
    "--enable-native-access=ALL-UNNAMED",
];

/// Boot phases detected from game output (first match only).
const PHASE_MAP: [(&str, &str); 8] = [
    ("loading_mc", "Loading Minecraft"),
    ("mixin", "MIXIN Subsystem"),
    ("gl_probe", "Searching for graphics cards"),
    ("datafixer", "Datafixer"),
    ("mc_ctor", "Completely ignored arguments"),
    ("mc_user", "Setting user:"),
    ("resources", "Reloading ResourceManager"),
    ("openal", "OpenAL initialized"),
];

/// Progress / phase callback (`onProgress(line)`, `onPhase(name)`).
pub type LineFn = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// resolveLaunch() options.
#[derive(Clone, Default)]
pub struct ResolveLaunchOptions {
    pub mode: LaunchMode,
    /// Already-resolved identity (route passes resolveAccountFor's result) or a stored account.
    pub account: Option<LaunchAccountInput>,
    pub dry_run: bool,
    pub force_verify: bool,
    pub on_progress: Option<LineFn>,
    pub on_phase: Option<LineFn>,
    /// Agent launch: adds `-Despectral.agent=1` (+ ffmpeg/hidden) and the
    /// option-derived program args. False on every plain launch.
    pub agent: bool,
    /// Validated quick-play option (`--quickPlay*`).
    pub join: Option<crate::agent::JoinOption>,
    /// Validated window option (`--width/--height`, agent-only `hidden`).
    pub window: Option<crate::agent::WindowOption>,
}

/// Account given to resolveLaunch: a stored account (token resolved inside)
/// or an already-resolved launch identity.
#[derive(Debug, Clone)]
pub enum LaunchAccountInput {
    Stored(Box<Account>),
    Resolved(LaunchAccount),
}

// ---------------------------------------------------------------------------
// Small pure helpers
// ---------------------------------------------------------------------------

/// jvm.mjs parseMajor: leading integer of a version string, legacy `1.x` -> 8.
pub fn parse_major(version: &str) -> u32 {
    let digits: String = version.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 0;
    }
    let rest = &version[digits.len()..];
    if !(rest.is_empty() || rest.starts_with('.')) {
        return 0;
    }
    match digits.parse::<u32>() {
        Ok(1) => 8,
        Ok(n) => n,
        Err(_) => 0,
    }
}

/// The java major a resolved runtime runs (`java.major ?? parseMajor(build ?? version)`).
pub fn java_major_of(java: &JavaRuntime) -> u32 {
    if let Some(m) = java.major {
        return m;
    }
    let src = if !java.build.is_empty() { &java.build } else { &java.version };
    parse_major(src)
}

/// -Xms/-Xmx. -Xmx is the ceiling (whole gigabytes when possible: 3072 -> 3G);
/// -Xms stays at INITIAL_HEAP_MB (never above -Xmx). Invalid -> automatic default.
pub fn memory_args(engine: &Engine, memory_mb: Option<i64>) -> [String; 2] {
    let mb = match memory_mb {
        Some(m) if m > 0 => m,
        _ => engine.config().default_memory_mb() as i64,
    };
    let label = if mb % 1024 == 0 && mb >= 1024 { format!("{}G", mb / 1024) } else { format!("{mb}M") };
    [format!("-Xms{}M", mb.min(INITIAL_HEAP_MB as i64)), format!("-Xmx{label}")]
}

/// Clamp requested memory to 60% of system RAM (`total_bytes`); invalid
/// inputs pass through unchanged.
pub fn effective_memory_mb(requested_mb: i64, total_bytes: u64) -> i64 {
    if requested_mb <= 0 || total_bytes == 0 {
        return requested_mb;
    }
    let cap = ((total_bytes as f64 / (1024.0 * 1024.0)) * 0.6).floor() as i64;
    requested_mb.min(cap)
}

/// launch.mjs `stackFlags` (upstream 67614f8): on Windows commit every Java
/// thread's stack up front. A lost guard page on a worker thread crashed
/// 26.3 silently (0xC0000005, no hs_err); pre-touching makes it harmless.
/// About 1 MB RAM per thread. Diagnostic-gated on JDK 21-25.
pub fn stack_flags(windows: bool) -> &'static [&'static str] {
    if windows {
        &["-XX:+UnlockDiagnosticVMOptions", "-XX:+AlwaysPreTouchStacks"]
    } else {
        &[]
    }
}

/// Boot-time flags: `-XX:-UsePerfData`, `-Xverify:none` (JDK < 27 — a hard
/// error from 27 on), and the opt-in C1-only `-XX:TieredStopAtLevel=1`
/// (instance `fast_boot` wins, absent -> config `fast_boot === true`).
pub fn boot_flags(engine: &Engine, instance: &InstanceMeta, java_major: Option<u32>) -> Vec<String> {
    let mut flags = vec!["-XX:-UsePerfData".to_string()];
    if java_major.map_or(true, |m| m < 27) {
        flags.push("-Xverify:none".into());
    }
    let fast = match instance.extra.get("fast_boot") {
        Some(Value::Bool(true)) => true,
        None => engine.config().is_true("fast_boot"),
        Some(_) => false,
    };
    if fast {
        flags.push("-XX:TieredStopAtLevel=1".into());
    }
    flags
}

fn expand_with(args: &[Value], env: &RuleEnv<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for entry in args {
        match entry {
            Value::String(s) => out.push(s.clone()),
            Value::Object(o) => {
                let Some(Value::Array(rules)) = o.get("rules") else { continue };
                let probe = Value::Object(Map::from_iter([("rules".to_string(), Value::Array(rules.clone()))]));
                if !crate::rules::rules_allow_with(&probe, env) {
                    continue;
                }
                match o.get("value") {
                    Some(Value::Array(vs)) => {
                        out.extend(vs.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
                    }
                    Some(Value::String(s)) => out.push(s.clone()),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// Expand a Mojang jvm argument template (strings pass; `{rules, value}`
/// entries kept when the os/arch rules allow).
pub fn expand_jvm_args(args: &[Value], os: &str, arch: &str) -> Vec<String> {
    expand_with(args, &RuleEnv { os, arch, features: None })
}

/// Same as [`expand_jvm_args`] but evaluates feature rules (game template).
pub fn expand_game_args(args: &[Value], features: &Map<String, Value>) -> Vec<String> {
    let base = RuleEnv::default();
    expand_with(args, &RuleEnv { os: base.os, arch: base.arch, features: Some(features) })
}

/// Replace `${key}` tokens from `map`; unknown keys expand to ''.
pub fn substitute_placeholders(arg: &str, map: &Map<String, Value>) -> String {
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        // `[^{}]*` then `}` — a nested brace breaks the token.
        let end = after.find(['{', '}']);
        match end {
            Some(e) if after.as_bytes()[e] == b'}' => {
                out.push_str(&rest[..start]);
                let key = &after[..e];
                if let Some(v) = map.get(key) {
                    match v {
                        Value::String(s) => out.push_str(s),
                        other => out.push_str(&other.to_string()),
                    }
                }
                rest = &after[e + 1..];
            }
            _ => {
                out.push_str(&rest[..start + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn path_delimiter() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn join_classpath(cp: &[PathBuf]) -> String {
    cp.iter().map(|p| lossy(p)).collect::<Vec<_>>().join(path_delimiter())
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// Offline fallback: OS username (sanitized, max 16), offline UUID, token '0'.
pub fn default_account() -> LaunchAccount {
    let raw = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" })
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "Player".into());
    let mut name: String =
        raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).take(16).collect();
    if name.is_empty() {
        name = "Player".into();
    }
    LaunchAccount {
        uuid: crate::accounts::offline_uuid(&name),
        username: name,
        access_token: "0".into(),
        user_type: "mojang".into(),
    }
}

/// Active account (offline token), else the OS-username fallback.
pub fn pick_account(engine: &Engine) -> LaunchAccount {
    match crate::accounts::get_active_account(engine) {
        Some(a) if !a.username.is_empty() => {
            LaunchAccount { username: a.username, uuid: a.uuid, access_token: "0".into(), user_type: "mojang".into() }
        }
        _ => default_account(),
    }
}

/// Resolve a stored account to a launch identity. MSA: a live MC token
/// (refreshed here); a failure falls back to OFFLINE and returns the warning
/// the launch log must show (silence here was the "Invalid session" bug).
pub async fn resolve_account_for(engine: &Engine, stored: Option<&Account>) -> (LaunchAccount, Option<String>) {
    let Some(acc) = stored.filter(|a| !a.username.is_empty()) else {
        return (default_account(), None);
    };
    let offline = || LaunchAccount {
        username: acc.username.clone(),
        uuid: acc.uuid.clone(),
        access_token: "0".into(),
        user_type: "mojang".into(),
    };
    if acc.token_kind != TokenKind::Msa {
        return (offline(), None);
    }
    let msg = match crate::msauth::ensure_minecraft_token(engine, acc).await {
        Ok(Some(t)) if !t.access_token.is_empty() => {
            return (
                LaunchAccount {
                    username: acc.username.clone(),
                    uuid: acc.uuid.clone(),
                    access_token: t.access_token,
                    user_type: "msa".into(),
                },
                None,
            );
        }
        Ok(_) => format!(
            "Microsoft session for '{}' could not be established (no access token returned) — launching OFFLINE; \
             online servers will reject this session.",
            acc.username
        ),
        Err(e) => format!(
            "Microsoft session for '{}' failed ({}: {}) — launching OFFLINE; online servers will reject this session.",
            acc.username,
            e.raw_code().unwrap_or("UNKNOWN"),
            e
        ),
    };
    tracing::warn!("[launch] {msg}");
    (offline(), Some(msg))
}

/// `target ?? active account`, resolved with [`resolve_account_for`].
pub async fn resolve_account(engine: &Engine, target: Option<&Account>) -> (LaunchAccount, Option<String>) {
    let active;
    let acc = match target {
        Some(a) => Some(a),
        None => {
            active = crate::accounts::get_active_account(engine);
            active.as_ref()
        }
    };
    match acc {
        Some(a) if !a.username.is_empty() => resolve_account_for(engine, Some(a)).await,
        _ => (default_account(), None),
    }
}

// ---------------------------------------------------------------------------
// argv
// ---------------------------------------------------------------------------

/// Pure argv assembly — no I/O. Pushes AOT/memory warnings into
/// `resolved.warnings` (like the JS, which mutated `resolved.warnings`).
pub fn build_argv(engine: &Engine, instance: &InstanceMeta, resolved: &mut ResolvedLaunch) -> Vec<String> {
    let java_major = java_major_of(&resolved.java);
    // Agent launch options (validated at the route). `false`/`None` on every
    // plain launch, so the argv below is byte-identical to before.
    let agent = resolved.agent;
    let join = resolved.join.clone();
    let window = resolved.window.clone();
    let raw = &resolved.account;
    let access_token = if raw.access_token.is_empty() || raw.access_token == "undefined" {
        "0".to_string()
    } else {
        raw.access_token.clone()
    };
    let user_type = if raw.user_type == "msa" { "msa" } else { "mojang" };
    let username = raw.username.clone();
    let uuid = raw.uuid.clone();

    let requested_mb: i64 = if resolved.memory_mb > 0 {
        resolved.memory_mb as i64
    } else if let Some(m) = instance.memory_mb.filter(|m| *m > 0) {
        m as i64
    } else {
        engine.config().default_memory_mb() as i64
    };
    let effective_mb = effective_memory_mb(requested_mb, crate::platform::total_memory_bytes());
    if effective_mb != requested_mb {
        resolved.warnings.push(format!("memory_mb {requested_mb} clamped to {effective_mb} (60% of system RAM)"));
    }

    let classpath = join_classpath(&resolved.classpath);
    let natives = lossy(&resolved.natives_dir);
    let mut subst = Map::new();
    let mut put = |k: &str, v: String| {
        subst.insert(k.to_string(), Value::String(v));
    };
    put("auth_player_name", username.clone());
    put("auth_uuid", uuid.clone());
    put("auth_access_token", access_token.clone());
    put("auth_session", access_token.clone());
    put("user_type", user_type.into());
    put("version_name", resolved.version.id.clone());
    put("game_directory", lossy(&resolved.game_dir));
    put("assets_root", lossy(&resolved.assets_dir));
    put("assets_index_name", resolved.version.asset_index_id.clone().unwrap_or_else(|| "null".into()));
    put("natives_directory", natives.clone());
    put("library_directory", lossy(&engine.paths().libraries_dir()));
    put("classpath", classpath.clone());
    put("classpath_separator", path_delimiter().into());
    put("launcher_name", LAUNCHER_NAME.into());
    put("launcher_version", LAUNCHER_VERSION.into());
    put("resolution_width", window.as_ref().map_or_else(|| "854".to_string(), |w| w.width.to_string()));
    put("resolution_height", window.as_ref().map_or_else(|| "480".to_string(), |w| w.height.to_string()));
    // Quick-play placeholders: only meaningful when the matching feature is
    // enabled below; a missing key substitutes to ''.
    put("quickPlaySingleplayer", join.as_ref().and_then(|j| j.world.clone()).unwrap_or_default());
    put("quickPlayMultiplayer", join.as_ref().and_then(|j| j.server.clone()).unwrap_or_default());
    put("clientid", "0".into());
    put("auth_xuid", "0".into());
    put("version_type", "release".into());

    let mut jvm: Vec<String> = memory_args(engine, Some(effective_mb)).to_vec();
    // --add-opens/--add-exports/--enable-native-access are Java 9+ (the
    // latter 17+) flags; legacy tiers (JDK 8/16/17) reject them at startup.
    if java_major >= 21 {
        jvm.extend(FIXED_JVM_ARGS.iter().map(|s| s.to_string()));
    }
    jvm.push(format!("-Djava.library.path={natives}"));
    if java_major >= 21 {
        jvm.extend(boot_flags(engine, instance, Some(java_major)));
        jvm.extend(stack_flags(cfg!(windows)).iter().map(|s| s.to_string()));
    }
    // Mod bridge: the in-game mod reports visible uuids over loopback.
    jvm.push(format!("-Despectral.bridge.url={}", crate::loopback::bridge_url(engine)));
    if let Some(t) = resolved.bridge_token.as_deref().filter(|t| !t.is_empty()) {
        jvm.push(format!("-Despectral.bridge.token={t}"));
    }
    // Agent mode. Inert unless the route tagged the launch `agent: true`; the
    // mod starts its relay thread only when it sees -Despectral.agent=1.
    if agent {
        jvm.push("-Despectral.agent=1".into());
        // ffmpeg for live recording / clip rendering (resolveLaunch already
        // resolved the absolute path — captures report NO_FFMPEG mod-side when
        // there is none, instead of guessing a binary name).
        if let Some(ff) = resolved.ffmpeg_path.as_ref().filter(|p| !p.as_os_str().is_empty()) {
            jvm.push(format!("-Despectral.ffmpeg={}", lossy(ff)));
        }
        if window.as_ref().is_some_and(|w| w.hidden) {
            jvm.push("-Despectral.agent.hidden=1".into());
        }
    }
    if let Some(loader) = &resolved.loader {
        if !loader.jvm_args.is_empty() {
            let os = crate::platform::os_name();
            let arch = crate::platform::node_arch();
            jvm.extend(expand_jvm_args(&loader.jvm_args, os, arch).iter().map(|a| substitute_placeholders(a, &subst)));
        }
        if loader.kind.as_deref() == Some("neoforge") {
            // bootstraplauncher skips module-izing files whose NAME STARTS WITH
            // an ignoreList entry: the vanilla client jar must be exempt too.
            if let Some(il) = jvm.iter().position(|a| a.starts_with("-DignoreList=")) {
                if !instance.version.is_empty() {
                    let client = engine.paths().client_jar(&instance.version);
                    let client_name = client.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if !jvm[il].contains(&client_name) {
                        jvm[il] = format!("{},{client_name}", jvm[il]);
                    }
                }
            }
        }
    }
    jvm.push("-cp".into());
    jvm.push(classpath);

    let cache = lossy(&resolved.aot_cache_path);
    // AOT (-XX:AOTCache*/JEP 483) exists only on JDK 25+ runtimes.
    if resolved.mode == LaunchMode::Train {
        if java_major >= 25 {
            // The branding mod neutralizes MC's ClientShutdownWatchdog during
            // training (the AOT dump always exceeds its budget).
            jvm.push("-Despectral.aot-training=true".into());
            jvm.push(format!("-XX:AOTCacheOutput={cache}"));
        } else {
            resolved.warnings.push(format!(
                "AOT training requires a JDK 25-tier runtime (current major {java_major}); skipping -XX:AOTCacheOutput"
            ));
        }
    } else if java_major >= 25 {
        if resolved.aot_cache_exists && !resolved.aot_cache_stale {
            jvm.push(format!("-XX:AOTCache={cache}"));
            // RELATIVE proof log — -Xlog splits on ':' so C:\... would break; cwd = gameDir.
            jvm.push("-Xlog:aot=info:file=aot-%p.log".into());
        } else if resolved.aot_cache_exists {
            resolved
                .warnings
                .push(format!("AOT cache {cache} no longer matches the classpath on disk; retraining is required"));
        } else {
            resolved.warnings.push(format!("AOT cache {cache} not found; running without -XX:AOTCache"));
        }
    } else {
        resolved.warnings.push(format!(
            "AOT requires a JDK 25-tier runtime (current major {java_major}); running without -XX:AOTCache"
        ));
    }

    let main_class = resolved
        .loader
        .as_ref()
        .map(|l| l.main_class.clone())
        .filter(|m| !m.is_empty())
        .or_else(|| Some(resolved.version.main_class.clone()).filter(|m| !m.is_empty()))
        .unwrap_or_else(|| "net.minecraft.client.main.Main".into());
    // Features the vanilla/NeoForge game template gates its program args on.
    // Fabric has no game template (it reuses the version's, empty in the loader
    // profile), so its flags are appended explicitly below; NeoForge merges the
    // vanilla game args and expands them here.
    let mut features = Map::new();
    features.insert("has_custom_resolution".into(), Value::Bool(window.is_some()));
    features.insert("is_quick_play_singleplayer".into(), Value::Bool(join.as_ref().is_some_and(|j| j.world.is_some())));
    features.insert("is_quick_play_multiplayer".into(), Value::Bool(join.as_ref().is_some_and(|j| j.server.is_some())));
    let fragment = crate::agent::game_arg_fragment(&crate::agent::LaunchOptions {
        agent,
        join: join.clone(),
        window: window.clone(),
    });
    let prog: Vec<String> = match &resolved.loader {
        Some(l) if !l.game_args.is_empty() => {
            expand_game_args(&l.game_args, &features).iter().map(|a| substitute_placeholders(a, &subst)).collect()
        }
        _ => {
            let mut prog = vec![
                "--username".into(),
                username,
                "--version".into(),
                resolved.version.id.clone(),
                "--gameDir".into(),
                lossy(&resolved.game_dir),
                "--assetsDir".into(),
                lossy(&resolved.assets_dir),
                "--assetIndex".into(),
                resolved.version.asset_index_id.clone().unwrap_or_else(|| "null".into()),
                "--uuid".into(),
                uuid,
                "--accessToken".into(),
                access_token,
                "--userType".into(),
                user_type.into(),
                "--versionType".into(),
                "release".into(),
            ];
            // Fabric/vanilla fixed list: append the agent launch flags directly
            // (there is no feature-gated template to expand here).
            prog.extend(fragment);
            prog
        }
    };
    let mut argv = Vec::with_capacity(jvm.len() + prog.len() + 2);
    argv.push(resolved.java.path.clone());
    argv.extend(jvm);
    argv.push(main_class);
    argv.extend(prog);
    argv
}

fn redact_preview(argv: &[String], access_token: Option<&str>) -> Vec<String> {
    argv.iter()
        .map(|a| {
            if Some(a.as_str()) == access_token || a.starts_with("-Despectral.bridge.token=") {
                "<REDACTED>".to_string()
            } else {
                a.clone()
            }
        })
        .collect()
}

/// Shell-ish one-line preview with the access + bridge tokens redacted.
pub fn format_command_preview(argv: &[String], access_token: Option<&str>) -> String {
    redact_preview(argv, access_token)
        .into_iter()
        .map(|a| if a.chars().any(char::is_whitespace) { format!("\"{a}\"") } else { a })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `{ jars, skippedNatives, skippedRules }` for a version's libraries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LibraryStats {
    pub jars: u64,
    pub skipped_natives: u64,
    pub skipped_rules: u64,
}

/// Harness-style stats for the dry-run report.
pub fn library_stats(libraries: &[Value]) -> LibraryStats {
    let mut s = LibraryStats::default();
    for lib in libraries {
        let name = lib.get("name").and_then(Value::as_str).unwrap_or("");
        let parts: Vec<&str> = name.split(':').collect();
        if parts.len() > 3 && parts[3].starts_with("natives") {
            s.skipped_natives += 1;
            continue;
        }
        if !crate::rules::rules_allow(lib) {
            s.skipped_rules += 1;
            continue;
        }
        s.jars += 1;
    }
    s
}

// ---------------------------------------------------------------------------
// Pre-launch file work
// ---------------------------------------------------------------------------

/// options.txt round-trip (import.mjs parseOptionsTxt/setOptionKey/writeOptionsTxt).
struct OptionsTxt {
    header: Option<String>,
    /// (key, value, raw) — raw None = re-serialize `key:value`.
    entries: Vec<(Option<String>, Option<String>, Option<String>)>,
    eol: &'static str,
}

impl OptionsTxt {
    fn parse(src: &str) -> Self {
        let eol = if src.contains("\r\n") { "\r\n" } else { "\n" };
        let mut header = None;
        let mut entries = Vec::new();
        for line in src.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
            if line.is_empty() {
                continue;
            }
            if header.is_none()
                && line.starts_with("version:")
                && line.len() > 8
                && line[8..].bytes().all(|b| b.is_ascii_digit())
            {
                header = Some(line.to_string());
                continue;
            }
            match line.find(':') {
                Some(i) if i > 0 => {
                    entries.push((Some(line[..i].to_string()), Some(line[i + 1..].to_string()), Some(line.to_string())))
                }
                _ => entries.push((None, None, Some(line.to_string()))),
            }
        }
        OptionsTxt { header, entries, eol }
    }

    fn set(&mut self, key: &str, value: &str) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.0.as_deref() == Some(key)) {
            e.1 = Some(value.to_string());
            e.2 = None;
        } else {
            self.entries.push((Some(key.to_string()), Some(value.to_string()), None));
        }
    }

    fn write(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(h) = &self.header {
            parts.push(h.clone());
        }
        for (k, v, raw) in &self.entries {
            match raw {
                Some(r) => parts.push(r.clone()),
                None => parts.push(format!("{}:{}", k.as_deref().unwrap_or(""), v.as_deref().unwrap_or(""))),
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            parts.join(self.eol) + self.eol
        }
    }
}

/// Integrated-client QoL seeded into options.txt before the game starts:
/// `fullbright_on_launch` -> `gamma:1.0` (vanilla's maximum). Fabric gets its
/// fullbright from the gamma-utils mod, so it is skipped there. Never fails a
/// launch (errors are logged).
pub fn apply_launch_preferences(engine: &Engine, instance: &InstanceMeta, game_dir: Option<&Path>) {
    if instance.loader == Loader::Fabric || !engine.config().is_true("fullbright_on_launch") {
        return;
    }
    let dir = game_dir.map(Path::to_path_buf).unwrap_or_else(|| engine.paths().instance_dir(&instance.name));
    let path = dir.join("options.txt");
    let mut parsed = OptionsTxt::parse(&std::fs::read_to_string(&path).unwrap_or_default());
    parsed.set("gamma", "1.0");
    if let Err(e) = std::fs::write(&path, parsed.write()) {
        tracing::warn!("[launch] could not apply launch preferences: {e}");
    }
}

/// Run-dir decision: a custom `game_dir` wins (mkdir -p unless dry run; no
/// per-account split); otherwise a non-active account gets
/// `<instanceDir>/profiles/<uuid>/` (Contract C). Natives always stay at the
/// instance dir. Returns `(game_dir, natives_dir)`.
pub fn select_game_dir(
    engine: &Engine,
    instance: &InstanceMeta,
    launch_account: Option<&LaunchAccount>,
    dry_run: bool,
    warnings: Option<&mut Vec<String>>,
) -> (PathBuf, PathBuf) {
    let base = engine.paths().instance_dir(&instance.name);
    let natives = engine.paths().instance_natives_dir(&instance.name);
    if let Some(custom) = instance.game_dir.as_deref().filter(|d| !d.is_empty()) {
        if !dry_run {
            if let Err(e) = std::fs::create_dir_all(custom) {
                if let Some(w) = warnings {
                    w.push(format!("could not create game_dir: {e}"));
                }
            }
        }
        return (PathBuf::from(custom), natives);
    }
    let Some(acc) = launch_account else { return (base, natives) };
    let active = crate::accounts::get_active_account(engine);
    if active.as_ref().map_or(true, |a| a.uuid != acc.uuid) {
        let profile = base.join("profiles").join(&acc.uuid);
        if !dry_run {
            let r = std::fs::create_dir_all(profile.join("logs"))
                .and_then(|_| std::fs::create_dir_all(profile.join("config")));
            if let Err(e) = r {
                if let Some(w) = warnings {
                    w.push(format!("could not create profile dir: {e}"));
                }
            }
        }
        return (profile, natives);
    }
    (base, natives)
}

/// The JDK a launch runs on: per-instance override (any JDK) > NeoForge's
/// EXACT 21 > the version's own `javaVersion.majorVersion` tier.
pub async fn java_for_launch(
    engine: &Engine,
    instance: &InstanceMeta,
    version_java_major: u32,
) -> EngineResult<JavaRuntime> {
    if let Some(o) = instance.jdk_path_override.as_deref().filter(|s| !s.is_empty()) {
        return crate::jvm::get_jvm_info(engine, Some(o)).await;
    }
    if instance.loader == Loader::Neoforge {
        return crate::runtimes::java_for_major_exact(engine, 21).await;
    }
    crate::runtimes::java_for_version(engine, version_java_major).await
}

/// Copy the canonical `<effectiveGameDir>/config/espectral-client.json` into
/// the launch's game dir when they differ (per-account profiles) and the
/// bytes are stale.
fn sync_profile_client_config(engine: &Engine, instance: &InstanceMeta, game_dir: &Path) {
    let src = crate::paths::client_config_in(&crate::instances::effective_game_dir(engine, instance));
    let dst = crate::paths::client_config_in(game_dir);
    if crate::util::normalize_path(&src) == crate::util::normalize_path(&dst) || !src.exists() {
        return;
    }
    let stale = match (std::fs::read(&dst), std::fs::read(&src)) {
        (Ok(a), Ok(b)) => a != b,
        _ => true,
    };
    if stale {
        if let Some(p) = dst.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::copy(&src, &dst);
    }
}

/// Contract stub results are tolerated (the owning cluster has not landed):
/// a `NotImplemented` is logged at debug level and ignored.
fn tolerate<T>(what: &str, r: EngineResult<T>) -> EngineResult<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(EngineError::NotImplemented(w)) => {
            tracing::debug!("[launch] {what}: {w} not ported yet");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// instances::get_instance (409 INVALID_NAME / 404 NOT_FOUND like the JS).
pub(crate) async fn load_instance(engine: &Engine, name: &str) -> EngineResult<InstanceMeta> {
    crate::instances::get_instance(engine, name).await
}

/// Resolve everything a launch needs: JVM, version JSON (merged chain for
/// NeoForge), libraries + assets (installed unless `dry_run`), Fabric
/// profile, account, run dir, classpath, AOT key / cache state.
/// `dry_run` never downloads or writes — it only resolves paths.
pub async fn resolve_launch(
    engine: &Engine,
    instance: &InstanceMeta,
    opts: ResolveLaunchOptions,
) -> EngineResult<ResolvedLaunch> {
    let mut warnings: Vec<String> = Vec::new();
    let progress = |m: &str| {
        if let Some(f) = &opts.on_progress {
            f(m);
        }
    };
    let phase = |m: &str| {
        if let Some(f) = &opts.on_phase {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(m)));
        }
    };
    progress(&format!("[espectral] preparing {}…", instance.name));
    phase("version");
    let mut version_json = crate::resolver::get_version_json(engine, &instance.version).await?;
    progress("[espectral] Minecraft version resolved");
    phase("version");

    let mut neoforge_id: Option<String> = None;
    if instance.loader == Loader::Neoforge {
        let lv = instance.loader_version.clone().filter(|s| !s.is_empty()).ok_or_else(|| {
            EngineError::api(400, "LOADER_VERSION_REQUIRED", "loader_version es obligatorio para NeoForge")
        })?;
        let nf_id = crate::neoforge::neoforge_version_id(&lv);
        let nf_json = match crate::resolver::read_local_version_json(engine, &nf_id) {
            None => {
                crate::neoforge::ensure_neoforge_loader(engine, &lv, Some(instance)).await?;
                crate::resolver::read_local_version_json(engine, &nf_id).ok_or_else(|| {
                    EngineError::api(502, "NEOFORGE_PROFILE_MISSING", "NeoForge profile missing after loader install")
                })?
            }
            Some(j) => {
                if !crate::neoforge::launch_artifacts_ready(engine, &lv, &j)? {
                    progress("[espectral] generando artefactos NeoForge (primera vez)…");
                    crate::neoforge::ensure_neoforge_loader(engine, &lv, Some(instance)).await?;
                }
                j
            }
        };
        version_json = crate::resolver::resolve_version_chain(engine, nf_json).await?;
        neoforge_id = Some(nf_id);
        progress("[espectral] perfil NeoForge cargado");
    }
    let asset_index_id = version_json.pointer("/assetIndex/id").and_then(Value::as_str).map(str::to_string);
    let version_main =
        version_json.get("mainClass").and_then(Value::as_str).unwrap_or("net.minecraft.client.main.Main").to_string();
    let version_java = version_json.pointer("/javaVersion/majorVersion").and_then(Value::as_u64).unwrap_or(8) as u32;

    let java = java_for_launch(engine, instance, version_java).await?;
    progress(&format!("[espectral] JDK {} listo", java_major_of(&java)));
    phase("jdk");

    let fabric = if instance.loader == Loader::Fabric {
        Some(crate::resolver::resolve_fabric(engine, &instance.version, None).await?)
    } else {
        None
    };
    phase("fabric");

    phase("account");
    let launch_account = match &opts.account {
        Some(LaunchAccountInput::Resolved(a)) => {
            let mut a = a.clone();
            if a.user_type.is_empty() {
                a.user_type = "mojang".into();
            }
            a
        }
        other => {
            let stored = match other {
                Some(LaunchAccountInput::Stored(a)) => Some(a.as_ref()),
                _ => None,
            };
            let (acc, warn) = resolve_account(engine, stored).await;
            if let Some(w) = warn {
                progress(&format!("[espectral] {w}"));
                warnings.push(w);
            }
            acc
        }
    };
    phase("account");

    let (game_dir, natives_dir) =
        select_game_dir(engine, instance, Some(&launch_account), opts.dry_run, Some(&mut warnings));

    let mut bridge_token: Option<String> = None;
    if !opts.dry_run {
        progress("[espectral] verificando dependencias…");
        phase("fingerprint");
        let mut hit = false;
        let mut reason = "unknown".to_string();
        if !opts.force_verify {
            let (paths, inst, vj, fb) =
                (engine.paths().clone(), instance.clone(), version_json.clone(), fabric.clone());
            match tokio::task::spawn_blocking(move || {
                crate::fingerprint::check_fingerprint(&paths, &inst, &vj, fb.as_ref())
            })
            .await
            {
                Ok(check) => {
                    hit = check.hit;
                    reason = check.reason.unwrap_or("ok").to_string();
                }
                Err(_) => reason = "fingerprint unavailable".into(),
            }
        }
        if hit {
            warnings.push("fingerprint hit — skipping verification".into());
        } else {
            if !opts.force_verify {
                warnings.push(format!("fingerprint miss ({reason}) — verifying"));
            }
            if let Some(f) = &fabric {
                crate::resolver::install_fabric_libraries(engine, instance, f, true).await?;
            }
            let lib_opts = crate::resolver::InstallLibrariesOptions {
                seed_from_fast_client: true,
                force_extract: opts.force_verify,
            };
            crate::resolver::install_libraries(engine, instance, &version_json, lib_opts).await?;
            progress("[espectral] libraries ready");
            phase("libraries");
            crate::resolver::install_assets(engine, instance, &version_json).await?;
            progress("[espectral] game assets ready");
            phase("assets");
            if let Err(e) =
                crate::fingerprint::write_fingerprint_file(engine.paths(), instance, &version_json, fabric.as_ref())
            {
                warnings.push(format!("fingerprint write failed: {e}"));
            }
        }

        let _ = crate::mods::ensure_branding_seeded(engine, instance).await;
        phase("branding");
        apply_launch_preferences(engine, instance, Some(&game_dir));
        phase("prefs");
        tolerate("seed_client_config", crate::client::seed_client_config(engine, &instance.name))?;
        // The game reads <gameDir>/config/espectral-client.json; mirror the
        // canonical file on EVERY profile launch so later toggles reach it.
        sync_profile_client_config(engine, instance, &game_dir);
        // Title-menu background staging (Contract-C gap, same as above).
        let dirs = [engine.paths().instance_dir(&instance.name), game_dir.clone()];
        if crate::backgrounds::migrate_legacy_background(engine, &instance.name, &dirs).await.is_ok() {
            let bg = crate::client::load_client_config(engine, &instance.name).ok().and_then(|c| {
                c.personalization.and_then(|p| p.get("background").and_then(Value::as_str).map(str::to_string))
            });
            let _ = crate::backgrounds::stage_background_pack(engine, bg.as_deref(), &game_dir);
        }
        // Cosmetics + per-launch bridge token (never blocks/fails a launch).
        if let Some(flat) = crate::identity::normalize_uuid(&launch_account.uuid) {
            bridge_token = Some(crate::bridge::mint_token());
            match crate::identity::refresh_grant_soft(engine, &flat, 2000).await {
                Ok(refreshed) => {
                    if !refreshed {
                        warnings.push("identity grant refresh skipped/failed — using cached entitlement".into());
                    }
                    let canonical = crate::instances::effective_game_dir(engine, instance);
                    let _ = crate::cosmetics::write_cosmetics_file(engine, &flat, &game_dir, Some(&canonical));
                }
                Err(e) => tracing::debug!("[launch] cosmetics staging skipped: {e}"),
            }
        }
        // Fire-and-forget AOT proof-log pruning.
        {
            let e = engine.clone();
            let name = instance.name.clone();
            engine.spawn_background("aot", async move {
                crate::aot::prune_aot_proof_logs(&e, &name).await;
                Ok(())
            });
        }
    }

    let classpath = crate::resolver::resolve_classpath(engine, instance, &version_json, fabric.as_ref());
    phase("classpath");
    let assets_dir = engine.paths().assets_dir();

    progress("[espectral] launching game…");
    phase("ready");
    let build = Some(java.build.as_str()).filter(|b| !b.is_empty());
    let aot_key = crate::aot::cache_key(&instance.version, build, None);
    let aot_cache_path = engine.paths().aot_cache_file(&aot_key);
    let aot_cache_exists = std::fs::metadata(&aot_cache_path).map(|m| m.len() > 0).unwrap_or(false);
    // A cache whose recorded classpath identity drifted is REFUSED by the JVM;
    // detect it so the launch skips -XX:AOTCache and the route retrains.
    let aot_cache_stale = aot_cache_exists && crate::aot::is_cache_stale(engine, &aot_key, &classpath);

    let instance_dir = engine.paths().instance_dir(&instance.name);
    let loader = if let Some(f) = &fabric {
        let mut jvm_args = f.jvm_args.clone();
        if game_dir != instance_dir {
            let mods = crate::instances::effective_mods_dir(engine, instance);
            jvm_args.push(Value::String(format!("-Dfabric.modsFolder={}", lossy(&mods))));
        }
        Some(LoaderLaunch { main_class: f.main_class.clone(), jvm_args, game_args: vec![], kind: None })
    } else if neoforge_id.is_some() {
        let arr = |p: &str| version_json.pointer(p).and_then(Value::as_array).cloned().unwrap_or_default();
        Some(LoaderLaunch {
            main_class: version_main.clone(),
            jvm_args: arr("/arguments/jvm"),
            game_args: arr("/arguments/game"),
            kind: Some("neoforge".into()),
        })
    } else {
        None
    };

    let aot_available = java.major.unwrap_or(0) >= 25;

    // ffmpeg for agent captures. Dry runs only look at what is already on disk
    // (a preview must not download 110 MB); a real agent launch provisions the
    // pinned Windows build when the machine has none.
    let mut ffmpeg_path = None;
    if opts.agent {
        let resolved_ff = if opts.dry_run {
            Ok(crate::ffmpeg::resolve_ffmpeg_sync(engine))
        } else {
            crate::ffmpeg::ensure_ffmpeg(engine).await
        };
        match resolved_ff {
            Ok(Some(p)) => ffmpeg_path = Some(p),
            Ok(None) => warnings.push("no ffmpeg found — recording/clip rendering will fail with NO_FFMPEG".into()),
            Err(e) => warnings.push(format!("ffmpeg resolution failed: {e}")),
        }
    }

    Ok(ResolvedLaunch {
        version: ResolvedVersionRef {
            id: neoforge_id.unwrap_or_else(|| instance.version.clone()),
            main_class: version_main,
            asset_index_id,
            json: version_json,
        },
        java,
        loader,
        fabric_profile: fabric,
        classpath,
        natives_dir,
        game_dir,
        assets_dir,
        memory_mb: instance.memory_mb.unwrap_or_else(|| engine.config().default_memory_mb()),
        account: launch_account,
        bridge_token,
        mode: opts.mode,
        aot_key,
        aot_cache_path,
        aot_cache_exists,
        aot_cache_stale,
        aot_available,
        agent: opts.agent,
        join: opts.join.clone(),
        window: opts.window.clone(),
        ffmpeg_path,
        warnings,
    })
}

/// DryRunResult: argv (tokens redacted) + classpath + missing entries +
/// natives + asset index check + java path + warnings. Never spawns/writes.
pub fn dry_run_result(
    engine: &Engine,
    instance: &InstanceMeta,
    resolved: &ResolvedLaunch,
) -> EngineResult<DryRunResult> {
    let mut r = resolved.clone();
    let missing: Vec<String> = r.classpath.iter().filter(|p| !p.exists()).map(|p| lossy(p)).collect();
    let index =
        r.assets_dir.join("indexes").join(format!("{}.json", r.version.asset_index_id.as_deref().unwrap_or("")));
    let argv = build_argv(engine, instance, &mut r);
    Ok(DryRunResult {
        argv: redact_preview(&argv, Some(r.account.access_token.as_str())),
        classpath: r.classpath.iter().map(|p| lossy(p)).collect(),
        classpath_missing: missing,
        natives_dir: lossy(&r.natives_dir),
        assets_ok: index.exists(),
        java_path: r.java.path.clone(),
        warnings: r.warnings.clone(),
    })
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// How a launched game ended (JS onExit `{ code, signal, marker, error }`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GameExit {
    pub code: Option<i32>,
    pub signal: Option<String>,
    pub marker: bool,
    pub error: Option<String>,
}

/// launchInstance() callbacks.
#[derive(Clone, Default)]
pub struct LaunchCallbacks {
    /// One line of game output (stdout or stderr).
    pub on_log: Option<LineFn>,
    /// Menu marker reached (ms epoch).
    pub on_marker: Option<Arc<dyn Fn(i64) + Send + Sync>>,
    /// Exactly once.
    pub on_exit: Option<Arc<dyn Fn(GameExit) + Send + Sync>>,
    /// Right after the process was created (ms epoch).
    pub on_spawned: Option<Arc<dyn Fn(i64) + Send + Sync>>,
    /// First sighting of a boot phase (`loading_mc`, `mixin`, …).
    pub on_phase: Option<LineFn>,
}

/// A spawned game.
#[derive(Debug, Clone)]
pub struct SpawnedGame {
    pub pid: u32,
    pub argv: Vec<String>,
    /// None for a test double that spawned nothing.
    pub process: Option<ProcessHandle>,
}

/// Spawn the game: cwd = gameDir, detached (outlives the launcher), output
/// piped + relayed, size-tracked latest.log marker watch (the output stream
/// is watched too: Windows can hold latest.log unreadable).
pub fn launch_instance(
    engine: &Engine,
    instance: &InstanceMeta,
    resolved: &mut ResolvedLaunch,
    cb: LaunchCallbacks,
) -> EngineResult<SpawnedGame> {
    let argv = build_argv(engine, instance, resolved);
    let game_dir = resolved.game_dir.clone();
    std::fs::create_dir_all(game_dir.join("logs"))?;
    let log_file = game_dir.join("logs").join("latest.log");

    let child = process::spawn_java(&resolved.java.path, &argv, &game_dir, true)?;
    if let Some(f) = &cb.on_spawned {
        f(crate::platform::now_ms());
    }
    let marker = Arc::new(AtomicBool::new(false));
    let fire_marker = {
        let marker = marker.clone();
        let on_marker = cb.on_marker.clone();
        Arc::new(move || {
            if !marker.swap(true, Ordering::SeqCst) {
                if let Some(f) = &on_marker {
                    f(crate::platform::now_ms());
                }
            }
        })
    };
    let seen_phases = Arc::new(Mutex::new(HashSet::<&'static str>::new()));
    let on_line: process::LineSink = {
        let fire_marker = fire_marker.clone();
        let marker = marker.clone();
        let cb = cb.clone();
        Arc::new(move |line: &str| {
            if let Some(on_phase) = &cb.on_phase {
                let mut seen = seen_phases.lock().unwrap_or_else(|p| p.into_inner());
                for (name, needle) in PHASE_MAP {
                    if !seen.contains(name) && line.contains(needle) {
                        seen.insert(name);
                        on_phase(name);
                    }
                }
            }
            if !marker.load(Ordering::SeqCst) && line.contains(MARKER) {
                fire_marker();
            }
            if let Some(f) = &cb.on_log {
                f(line);
            }
        })
    };
    let on_exit = cb.on_exit.clone();
    let marker_for_exit = marker.clone();
    let handle = process::monitor(child, Some(on_line), move |info| {
        if let Some(f) = on_exit {
            f(GameExit {
                code: info.code,
                signal: info.signal,
                marker: marker_for_exit.load(Ordering::SeqCst),
                error: info.error,
            });
        }
    });

    // latest.log watcher (100 ms), stops at the marker or at exit.
    {
        let handle = handle.clone();
        let marker = marker.clone();
        tokio::spawn(async move {
            let mut seen: i64 = std::fs::metadata(&log_file).map(|m| m.len() as i64).unwrap_or(-1);
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if marker.load(Ordering::SeqCst) || handle.has_exited() {
                    break;
                }
                if read_log_growth(&log_file, &mut seen).is_some_and(|chunk| chunk.contains(MARKER)) {
                    fire_marker();
                    break;
                }
            }
        });
    }

    Ok(SpawnedGame { pid: handle.pid(), argv, process: Some(handle) })
}

/// Size-tracked read of a growing log: returns the new bytes since `seen`
/// (`-1` = file absent at start; shrink = truncated -> from 0).
pub(crate) fn read_log_growth(log_file: &Path, seen: &mut i64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let size = std::fs::metadata(log_file).ok()?.len() as i64;
    if *seen == -1 || size < *seen {
        *seen = 0;
    }
    if size <= *seen {
        return None;
    }
    let mut f = std::fs::File::open(log_file).ok()?;
    f.seek(SeekFrom::Start(*seen as u64)).ok()?;
    let mut buf = vec![0u8; (size - *seen) as usize];
    f.read_exact(&mut buf).ok()?;
    *seen = size;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use crate::types::JvmSource;
    use serde_json::json;

    fn java(path: &str, major: u32, build: &str) -> JavaRuntime {
        JavaRuntime {
            path: path.into(),
            build: build.into(),
            major: Some(major),
            source: JvmSource::Path,
            ..Default::default()
        }
    }

    /// A minimal resolveLaunch result with everything buildArgv reads.
    fn make_resolved() -> ResolvedLaunch {
        ResolvedLaunch {
            java: java("/jdk/bin/java.exe", 25, "25.0.4+7"),
            version: ResolvedVersionRef {
                id: "1.21.11".into(),
                main_class: "net.minecraft.client.main.Main".into(),
                asset_index_id: Some("1.21".into()),
                json: json!({}),
            },
            loader: None,
            fabric_profile: None,
            classpath: vec!["/data/libraries/a.jar".into(), "/data/libraries/b.jar".into()],
            natives_dir: "/games/testinst/natives".into(),
            game_dir: "/games/testinst".into(),
            assets_dir: "/data/assets".into(),
            memory_mb: 4096,
            account: LaunchAccount {
                username: "Tester".into(),
                uuid: "00000000-0000-3000-8000-000000000000".into(),
                access_token: "tok".into(),
                user_type: "mojang".into(),
            },
            bridge_token: None,
            mode: LaunchMode::Normal,
            aot_key: "key".into(),
            aot_cache_path: "/data/cache/aot/key/game.aot".into(),
            aot_cache_exists: false,
            aot_cache_stale: false,
            aot_available: true,
            agent: false,
            join: None,
            window: None,
            ffmpeg_path: None,
            warnings: vec![],
        }
    }

    fn inst(name: &str) -> InstanceMeta {
        InstanceMeta { name: name.into(), ..Default::default() }
    }

    /// The memory the tests ask for must not get clamped on small CI boxes.
    fn fits(mb: i64) -> bool {
        effective_memory_mb(mb, crate::platform::total_memory_bytes()) == mb
    }

    #[test]
    fn memory_args_small_xms_ceiling_xmx_and_fallback() {
        let (_d, e) = temp_engine();
        assert_eq!(memory_args(&e, Some(3072)), ["-Xms512M".to_string(), "-Xmx3G".into()]);
        assert_eq!(memory_args(&e, Some(2048)), ["-Xms512M".to_string(), "-Xmx2G".into()]);
        assert_eq!(memory_args(&e, Some(1500)), ["-Xms512M".to_string(), "-Xmx1500M".into()]);
        assert_eq!(memory_args(&e, Some(384)), ["-Xms384M".to_string(), "-Xmx384M".into()]);
        assert_eq!(INITIAL_HEAP_MB, 512);
        let auto = memory_args(&e, Some(e.config().default_memory_mb() as i64));
        for bad in [None, Some(0), Some(-512)] {
            assert_eq!(memory_args(&e, bad), auto, "{bad:?}");
        }
    }

    #[test]
    fn argv0_is_the_java_path() {
        let (_d, e) = temp_engine();
        assert_eq!(build_argv(&e, &inst("t"), &mut make_resolved())[0], "/jdk/bin/java.exe");
    }

    #[test]
    fn full_argv_shape() {
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        if !fits(4096) {
            r.memory_mb = 1024;
        }
        let argv = build_argv(&e, &inst("t"), &mut r);
        let mem = memory_args(&e, Some(r.memory_mb as i64));
        assert_eq!(argv[1..3], mem);
        assert_eq!(argv[3..3 + FIXED_JVM_ARGS.len()], FIXED_JVM_ARGS.map(String::from));
        let natives_idx = 3 + FIXED_JVM_ARGS.len();
        assert_eq!(argv[natives_idx], "-Djava.library.path=/games/testinst/natives");
        let cp = argv.iter().position(|a| a == "-cp").unwrap();
        assert!(cp > natives_idx);
        assert_eq!(argv[cp + 1], format!("/data/libraries/a.jar{}/data/libraries/b.jar", path_delimiter()));
        assert_eq!(argv[cp + 2], "net.minecraft.client.main.Main");
        let expected: Vec<String> = [
            "--username",
            "Tester",
            "--version",
            "1.21.11",
            "--gameDir",
            "/games/testinst",
            "--assetsDir",
            "/data/assets",
            "--assetIndex",
            "1.21",
            "--uuid",
            "00000000-0000-3000-8000-000000000000",
            "--accessToken",
            "tok",
            "--userType",
            "mojang",
            "--versionType",
            "release",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(argv[cp + 3..].to_vec(), expected);
        assert!(
            argv.contains(&"-Despectral.bridge.url=http://127.0.0.1:4199/api/bridge".to_string())
                || std::env::var("ESPECTRAL_PORT").is_ok()
        );
    }

    #[test]
    fn aot_flags_consume_valid_cache_only() {
        let (_d, e) = temp_engine();
        let mut fresh = make_resolved();
        fresh.aot_cache_exists = true;
        let argv = build_argv(&e, &inst("t"), &mut fresh);
        assert!(argv.contains(&"-XX:AOTCache=/data/cache/aot/key/game.aot".to_string()));
        assert!(argv.contains(&"-Xlog:aot=info:file=aot-%p.log".to_string()));
        let cp = argv.iter().position(|a| a == "-cp").unwrap();
        let aot = argv.iter().position(|a| a.starts_with("-XX:AOTCache=")).unwrap();
        assert!(aot > cp, "AOT flags must come after -cp");

        let mut missing = make_resolved();
        let argv = build_argv(&e, &inst("t"), &mut missing);
        assert!(!argv.iter().any(|a| a.starts_with("-XX:AOTCache=") || a.starts_with("-Xlog:aot=")));
        assert!(missing.warnings.iter().any(|w| w.contains("AOT cache") && w.contains("not found")));

        let mut train = make_resolved();
        train.mode = LaunchMode::Train;
        let argv = build_argv(&e, &inst("t"), &mut train);
        assert!(argv.contains(&"-XX:AOTCacheOutput=/data/cache/aot/key/game.aot".to_string()));
        assert!(argv.contains(&"-Despectral.aot-training=true".to_string()));
        assert!(!argv.iter().any(|a| a.starts_with("-XX:AOTCache=") || a.starts_with("-Xlog:aot=")));

        let mut old = make_resolved();
        old.aot_cache_exists = true;
        old.java = java("/jdk17/bin/java.exe", 17, "17.0.12+7");
        let argv = build_argv(&e, &inst("t"), &mut old);
        assert!(!argv.iter().any(|a| a.starts_with("-XX:AOTCache=") || a.starts_with("-Xlog:aot=")));
        assert!(!argv.contains(&"--add-opens".to_string()));
        assert!(old.warnings.iter().any(|w| w.contains("JDK 25-tier")));
    }

    #[test]
    fn stale_cache_is_skipped_and_reported() {
        let (_d, e) = temp_engine();
        let mut stale = make_resolved();
        stale.aot_cache_exists = true;
        stale.aot_cache_stale = true;
        let argv = build_argv(&e, &inst("t"), &mut stale);
        assert!(!argv.iter().any(|a| a.starts_with("-XX:AOTCache=") || a.starts_with("-Xlog:aot=")));
        assert!(stale.warnings.iter().any(|w| w.contains("no longer matches the classpath")));
        let mut fresh = make_resolved();
        fresh.aot_cache_exists = true;
        assert!(build_argv(&e, &inst("t"), &mut fresh).contains(&"-XX:AOTCache=/data/cache/aot/key/game.aot".into()));
    }

    #[test]
    fn memory_fallback_order_and_marker() {
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        r.memory_mb = 0;
        let def = e.config().default_memory_mb() as i64;
        let argv = build_argv(&e, &inst("t"), &mut r);
        if fits(def) {
            assert_eq!(argv[1..3], memory_args(&e, Some(def)));
        }
        let mut r = make_resolved();
        r.memory_mb = 0;
        let i = InstanceMeta { name: "t".into(), memory_mb: Some(2048), ..Default::default() };
        if fits(2048) {
            assert_eq!(build_argv(&e, &i, &mut r)[1..3], ["-Xms512M".to_string(), "-Xmx2G".into()]);
        }
        let mut r = make_resolved();
        r.memory_mb = 1024;
        assert_eq!(build_argv(&e, &i, &mut r)[1..3], ["-Xms512M".to_string(), "-Xmx1G".into()]);
        assert!("[Render thread/INFO]: Sound engine started".contains(MARKER));
        assert!(!"Starting minecraft server".contains(MARKER));
    }

    #[test]
    fn boot_flags_instance_wins_then_config() {
        let (_d, e) = temp_engine();
        e.config().set("fast_boot", json!(false));
        assert_eq!(boot_flags(&e, &inst("t"), Some(25)), ["-XX:-UsePerfData", "-Xverify:none"]);
        let mut on = inst("t");
        on.extra.insert("fast_boot".into(), json!(true));
        assert_eq!(boot_flags(&e, &on, Some(25)), ["-XX:-UsePerfData", "-Xverify:none", "-XX:TieredStopAtLevel=1"]);
        e.config().set("fast_boot", json!(true));
        let mut off = inst("t");
        off.extra.insert("fast_boot".into(), json!(false));
        assert_eq!(boot_flags(&e, &off, Some(25)), ["-XX:-UsePerfData", "-Xverify:none"]);
        assert_eq!(
            boot_flags(&e, &inst("t"), Some(25)),
            ["-XX:-UsePerfData", "-Xverify:none", "-XX:TieredStopAtLevel=1"]
        );
        e.config().set("fast_boot", json!(false));
        // JDK 27 gate
        assert_eq!(boot_flags(&e, &inst("t"), Some(27)), ["-XX:-UsePerfData"]);
        assert!(boot_flags(&e, &inst("t"), Some(26)).contains(&"-Xverify:none".to_string()));
        assert_eq!(boot_flags(&e, &on, Some(27)), ["-XX:-UsePerfData", "-XX:TieredStopAtLevel=1"]);
        assert!(boot_flags(&e, &inst("t"), None).contains(&"-Xverify:none".to_string()));
    }

    #[test]
    fn stack_flags_are_windows_only() {
        assert_eq!(stack_flags(true), ["-XX:+UnlockDiagnosticVMOptions", "-XX:+AlwaysPreTouchStacks"]);
        assert!(stack_flags(false).is_empty());
    }

    #[test]
    fn boot_flags_ride_with_jdk21_fixed_set_only() {
        let (_d, e) = temp_engine();
        let argv = build_argv(&e, &inst("t"), &mut make_resolved());
        assert!(argv.contains(&"-XX:-UsePerfData".to_string()));
        assert!(argv.contains(&"-Xverify:none".to_string()));
        assert!(!argv.contains(&"-XX:TieredStopAtLevel=1".to_string()));
        let mut old = make_resolved();
        old.java = java("/jdk17/bin/java.exe", 17, "17.0.12+7");
        let argv = build_argv(&e, &inst("t"), &mut old);
        assert!(!argv.contains(&"-XX:-UsePerfData".to_string()));
        assert!(!argv.contains(&"-Xverify:none".to_string()));
    }

    #[test]
    fn token_coercion_never_empty_or_undefined() {
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        r.account.access_token = String::new();
        let argv = build_argv(&e, &inst("t"), &mut r);
        let i = argv.iter().position(|a| a == "--accessToken").unwrap();
        assert_eq!(argv[i + 1], "0");
        assert!(!argv.contains(&"undefined".to_string()));
        let mut r = make_resolved();
        r.account.user_type = "msa".into();
        r.account.access_token = "undefined".into();
        let argv = build_argv(&e, &inst("t"), &mut r);
        assert_eq!(argv[argv.iter().position(|a| a == "--userType").unwrap() + 1], "msa");
        assert_eq!(argv[argv.iter().position(|a| a == "--accessToken").unwrap() + 1], "0");
    }

    #[test]
    fn effective_memory_clamp_math() {
        const GB: u64 = 1024 * 1024 * 1024;
        assert_eq!(effective_memory_mb(8192, 8 * GB), 4915);
        assert_eq!(effective_memory_mb(4096, 8 * GB), 4096);
        assert_eq!(effective_memory_mb(4915, 8 * GB), 4915);
        assert_eq!(effective_memory_mb(4916, 8 * GB), 4915);
        assert_eq!(effective_memory_mb(20000, 32 * GB), 19660);
        assert_eq!(effective_memory_mb(1000, 32 * GB), 1000);
        assert_eq!(effective_memory_mb(0, 8 * GB), 0);
        assert_eq!(effective_memory_mb(-3, 8 * GB), -3);
        assert_eq!(effective_memory_mb(5000, 0), 5000);
    }

    #[test]
    fn clamp_via_build_argv_pushes_warning() {
        let (_d, e) = temp_engine();
        let total = crate::platform::total_memory_bytes();
        let cap = ((total as f64 / 1048576.0) * 0.6).floor() as i64;
        let mut r = make_resolved();
        r.memory_mb = (cap + 5000) as u32;
        r.aot_cache_exists = true;
        let argv = build_argv(&e, &inst("t"), &mut r);
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains(&format!("memory_mb {} clamped to {cap}", cap + 5000)));
        let label = if cap % 1024 == 0 { format!("{}G", cap / 1024) } else { format!("{cap}M") };
        assert!(argv.contains(&format!("-Xmx{label}")));
        let mut r2 = make_resolved();
        r2.memory_mb = 1024.min((cap - 1).max(1)) as u32;
        r2.aot_cache_exists = true;
        build_argv(&e, &inst("t"), &mut r2);
        assert!(r2.warnings.is_empty(), "{:?}", r2.warnings);
    }

    #[test]
    fn preview_redacts_access_and_bridge_tokens() {
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        r.account.access_token = "SECRETACCESSTOKEN".into();
        r.bridge_token = Some("a".repeat(64));
        let argv = build_argv(&e, &inst("t"), &mut r);
        assert!(argv.contains(&format!("-Despectral.bridge.token={}", "a".repeat(64))));
        let preview = format_command_preview(&argv, Some("SECRETACCESSTOKEN"));
        assert!(!preview.contains("SECRETACCESSTOKEN"));
        assert!(!preview.contains(&"a".repeat(64)));
        assert!(preview.contains("<REDACTED>"));
    }

    #[test]
    fn dry_run_result_redacts_and_reports_missing() {
        let (d, e) = temp_engine();
        let mut r = make_resolved();
        let jar = d.path().join("present.jar");
        std::fs::write(&jar, b"x").unwrap();
        r.classpath = vec![jar.clone(), d.path().join("missing.jar")];
        r.bridge_token = Some("b".repeat(64));
        let dr = dry_run_result(&e, &inst("t"), &r).unwrap();
        assert_eq!(dr.classpath_missing, vec![lossy(&d.path().join("missing.jar"))]);
        assert!(!dr.assets_ok);
        assert!(!dr.argv.iter().any(|a| a == "tok" || a.contains(&"b".repeat(64))));
        assert_eq!(dr.java_path, "/jdk/bin/java.exe");
    }

    #[test]
    fn placeholders_and_rule_expansion() {
        let mut m = Map::new();
        m.insert("a".into(), json!("X"));
        assert_eq!(substitute_placeholders("-D${a}=${missing}!", &m), "-DX=!");
        assert_eq!(substitute_placeholders("${a", &m), "${a");
        assert_eq!(substitute_placeholders("${{a}", &m), "${{a}");
        let args = vec![
            json!("plain"),
            json!({ "rules": [{ "action": "allow", "os": { "name": "nonexistent" } }], "value": "never" }),
            json!({ "rules": [{ "action": "allow" }], "value": ["one", "two"] }),
            json!({ "value": "no-rules-dropped" }),
        ];
        assert_eq!(expand_jvm_args(&args, "linux", "x64"), vec!["plain", "one", "two"]);
        let feat =
            vec![json!({ "rules": [{ "action": "allow", "features": { "is_demo_user": true } }], "value": "--demo" })];
        assert!(expand_game_args(&feat, &Map::new()).is_empty());
    }

    #[test]
    fn parse_major_parity() {
        assert_eq!(parse_major("25.0.4+7"), 25);
        assert_eq!(parse_major("1.8.0_402"), 8);
        assert_eq!(parse_major("21"), 21);
        assert_eq!(parse_major("x"), 0);
        assert_eq!(parse_major("17-ea"), 0);
    }

    #[test]
    fn library_stats_counts() {
        let libs = vec![
            json!({ "name": "a:b:1" }),
            json!({ "name": "org.lwjgl:lwjgl:3:natives-linux" }),
            json!({ "name": "c:d:2", "rules": [{ "action": "allow", "os": { "name": "nonexistent" } }] }),
        ];
        assert_eq!(library_stats(&libs), LibraryStats { jars: 1, skipped_natives: 1, skipped_rules: 1 });
    }

    // --- preferences.test.mjs ----------------------------------------------

    fn options_path(e: &Engine) -> PathBuf {
        e.paths().instance_dir("testinst").join("options.txt")
    }

    #[test]
    fn fullbright_seeds_gamma_and_keeps_other_keys() {
        let (_d, e) = temp_engine();
        let i = inst("testinst");
        e.config().set("fullbright_on_launch", json!(true));
        std::fs::create_dir_all(options_path(&e).parent().unwrap()).unwrap();
        std::fs::write(options_path(&e), "version:1343\nkey_forward.0:true\n").unwrap();
        apply_launch_preferences(&e, &i, None);
        assert_eq!(std::fs::read_to_string(options_path(&e)).unwrap(), "version:1343\nkey_forward.0:true\ngamma:1.0\n");

        std::fs::remove_file(options_path(&e)).unwrap();
        apply_launch_preferences(&e, &i, None);
        assert_eq!(std::fs::read_to_string(options_path(&e)).unwrap(), "gamma:1.0\n");

        std::fs::write(options_path(&e), "version:1343\ngamma:0.5\nkey_forward.0:true\n").unwrap();
        apply_launch_preferences(&e, &i, None);
        assert_eq!(std::fs::read_to_string(options_path(&e)).unwrap(), "version:1343\ngamma:1.0\nkey_forward.0:true\n");
        // CRLF files keep their EOL.
        std::fs::write(options_path(&e), "version:1343\r\ngamma:0.5\r\n").unwrap();
        apply_launch_preferences(&e, &i, None);
        assert_eq!(std::fs::read_to_string(options_path(&e)).unwrap(), "version:1343\r\ngamma:1.0\r\n");
    }

    #[test]
    fn fullbright_off_or_fabric_leaves_options_untouched() {
        let (_d, e) = temp_engine();
        let i = inst("testinst");
        e.config().set("fullbright_on_launch", json!(false));
        apply_launch_preferences(&e, &i, None);
        assert!(!options_path(&e).exists());
        std::fs::create_dir_all(options_path(&e).parent().unwrap()).unwrap();
        let before = "version:1343\ngamma:0.5\nkey_forward.0:true\n";
        std::fs::write(options_path(&e), before).unwrap();
        apply_launch_preferences(&e, &i, None);
        assert_eq!(std::fs::read_to_string(options_path(&e)).unwrap(), before);
        e.config().set("fullbright_on_launch", json!(true));
        let fabric = InstanceMeta { name: "testinst".into(), loader: Loader::Fabric, ..Default::default() };
        apply_launch_preferences(&e, &fabric, None);
        assert_eq!(std::fs::read_to_string(options_path(&e)).unwrap(), before);
    }

    // --- personalization.test.mjs (selectGameDir) --------------------------

    #[test]
    fn select_game_dir_custom_wins() {
        let (d, e) = temp_engine();
        let other = LaunchAccount {
            username: "Other".into(),
            uuid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            access_token: "0".into(),
            user_type: "mojang".into(),
        };
        let custom = d.path().join("launch-custom");
        let i = InstanceMeta {
            name: "whatever".into(),
            game_dir: Some(custom.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let mut w = vec![];
        let (g, n) = select_game_dir(&e, &i, Some(&other), false, Some(&mut w));
        assert_eq!(g, custom);
        assert_eq!(n, e.paths().instance_natives_dir("whatever"));
        assert!(custom.exists());
        assert!(!e.paths().instance_dir("whatever").join("profiles").exists());
        let dry = d.path().join("launch-custom-dry");
        let i = InstanceMeta {
            name: "whatever".into(),
            game_dir: Some(dry.to_string_lossy().into()),
            ..Default::default()
        };
        assert_eq!(select_game_dir(&e, &i, Some(&other), true, None).0, dry);
        assert!(!dry.exists());
    }

    #[test]
    fn select_game_dir_active_vs_profile_split() {
        let (_d, e) = temp_engine();
        e.config().set(
            "accounts",
            json!([{ "username": "LaunchActive", "uuid": crate::accounts::offline_uuid("LaunchActive"), "token_kind": "offline" }]),
        );
        e.config().set("active_username", json!("LaunchActive"));
        let i = inst("split-inst");
        let active = pick_account(&e);
        assert_eq!(active.username, "LaunchActive");
        assert_eq!(select_game_dir(&e, &i, Some(&active), false, None).0, e.paths().instance_dir("split-inst"));
        let alt = LaunchAccount {
            username: "LaunchAlt".into(),
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            access_token: "0".into(),
            user_type: "mojang".into(),
        };
        let (g, n) = select_game_dir(&e, &i, Some(&alt), true, None);
        assert_eq!(g, e.paths().instance_dir("split-inst").join("profiles").join(&alt.uuid));
        assert_eq!(n, e.paths().instance_natives_dir("split-inst"));
        assert!(!g.exists(), "dry run never creates the profile dir");
        let (g, _) = select_game_dir(&e, &i, Some(&alt), false, None);
        assert!(g.join("logs").is_dir() && g.join("config").is_dir());
    }

    // --- launch.test.mjs resolveAccountFor ---------------------------------

    #[tokio::test]
    async fn resolve_account_for_offline_and_default() {
        let (_d, e) = temp_engine();
        let stored = Account {
            username: "OfflineUser".into(),
            uuid: "11111111-1111-3111-8111-111111111111".into(),
            token_kind: TokenKind::Offline,
            ..Default::default()
        };
        let (acc, warn) = resolve_account_for(&e, Some(&stored)).await;
        assert_eq!(
            (acc.username.as_str(), acc.access_token.as_str(), acc.user_type.as_str()),
            ("OfflineUser", "0", "mojang")
        );
        assert_eq!(acc.uuid, stored.uuid);
        assert!(warn.is_none());
        let (def, _) = resolve_account_for(&e, None).await;
        assert!(!def.username.is_empty() && def.username.len() <= 16);
        assert_eq!(def.uuid, crate::accounts::offline_uuid(&def.username));
    }

    #[tokio::test]
    async fn resolve_account_for_msa_failure_falls_back_offline_with_warning() {
        let (_d, e) = temp_engine();
        let stored = Account {
            username: "MsaNoRefresh".into(),
            uuid: "22222222-2222-3222-8222-222222222222".into(),
            token_kind: TokenKind::Msa,
            microsoft: Some(Default::default()),
            ..Default::default()
        };
        let (acc, warn) = resolve_account_for(&e, Some(&stored)).await;
        assert_eq!(acc.access_token, "0");
        assert_eq!(acc.user_type, "mojang");
        let w = warn.expect("warning for a failed MSA session");
        assert!(w.contains("MsaNoRefresh") && w.contains("OFFLINE"), "{w}");
    }

    #[test]
    fn default_account_is_sanitized() {
        let a = default_account();
        assert!(a.username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        assert_eq!(a.access_token, "0");
    }

    #[test]
    fn neoforge_ignore_list_gets_client_jar() {
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        r.loader = Some(LoaderLaunch {
            main_class: "cpw.mods.bootstraplauncher.BootstrapLauncher".into(),
            jvm_args: vec![json!("-DignoreList=a.jar,b"), json!("-DlibraryDirectory=${library_directory}")],
            game_args: vec![
                json!("--username"),
                json!("${auth_player_name}"),
                json!("--accessToken"),
                json!("${auth_access_token}"),
            ],
            kind: Some("neoforge".into()),
        });
        let i = InstanceMeta { name: "nf".into(), version: "1.21.1".into(), ..Default::default() };
        let argv = build_argv(&e, &i, &mut r);
        assert!(argv.contains(&"-DignoreList=a.jar,b,1.21.1.jar".to_string()), "{argv:?}");
        assert!(argv.contains(&format!("-DlibraryDirectory={}", lossy(&e.paths().libraries_dir()))));
        let main = argv.iter().position(|a| a == "cpw.mods.bootstraplauncher.BootstrapLauncher").unwrap();
        assert_eq!(argv[main + 1..], ["--username", "Tester", "--accessToken", "tok"].map(String::from));
    }

    #[test]
    fn log_growth_reader() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("latest.log");
        let mut seen = -1;
        assert!(read_log_growth(&f, &mut seen).is_none());
        std::fs::write(&f, "a\n").unwrap();
        assert_eq!(read_log_growth(&f, &mut seen).as_deref(), Some("a\n"));
        assert!(read_log_growth(&f, &mut seen).is_none());
        std::fs::write(&f, "abc\nSound engine started\n").unwrap();
        assert_eq!(read_log_growth(&f, &mut seen).as_deref(), Some("c\nSound engine started\n"));
        std::fs::write(&f, "x\n").unwrap();
        assert_eq!(read_log_growth(&f, &mut seen).as_deref(), Some("x\n"), "truncation restarts at 0");
    }
}
