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
/// About 1 MB RAM per thread. Diagnostic-gated on JDK 21-25. The flag Mojang
/// ships for that crash, `-XX:StackShadowPages=32`, comes from [`vanilla_vm_flags`].
pub fn stack_flags(windows: bool) -> &'static [&'static str] {
    if windows {
        &["-XX:+UnlockDiagnosticVMOptions", "-XX:+AlwaysPreTouchStacks"]
    } else {
        &[]
    }
}

/// Low-memory heap policy: on machines in the launcher's lowest automatic heap
/// tier (total RAM <= 8.5 GiB, the first tier of
/// `config::recommended_memory_mb_for`), keep the heap from growing back after a
/// spike. Measured in a world at `-Xmx2G` on 4 threads, with compact headers:
/// RSS 1804 -> 1700 MB, committed heap 849 -> 717 MB, p50 frame time -0.1 %, no
/// frame over 50 ms, CPU +2 % (n=8 vs 6). Without compact headers the same pair
/// cost +10 % p99 in one scene, and a tighter 10/30 pair cost +10 % CPU and
/// frames over 50 ms, so neither ships: the flags need `compact_headers`.
pub fn low_memory_heap_flags(total_bytes: u64, compact_headers: bool) -> &'static [&'static str] {
    /// `config::recommended_memory_mb_for`'s lowest tier boundary (<= 8.5 GB).
    const LOWEST_TIER_MAX_GB: f64 = 8.5;
    let total_gb = total_bytes as f64 / 1024f64.powi(3);
    if compact_headers && total_bytes > 0 && total_gb <= LOWEST_TIER_MAX_GB {
        &["-XX:MinHeapFreeRatio=20", "-XX:MaxHeapFreeRatio=40"]
    } else {
        &[]
    }
}

/// `-XX:+UseCompactObjectHeaders` gate: JDK 25+, an AOT-capable loader
/// (vanilla/Fabric — Forge/NeoForge are unmeasured), the version profile's
/// `default-user-jvm` template asking for it (Mojang ships it on 26.x), and a
/// runtime with a compact-headers base CDS archive (`classes_coh.jsa`). Compact
/// headers alone lower the live heap by 2-8 %; the RSS gain comes together with
/// [`low_memory_heap_flags`]. The archive check keeps the JVM's base CDS archive
/// in use — without `classes_coh.jsa` the JVM just runs without a base archive
/// and starts slower.
pub fn compact_object_headers(java_major: u32, loader: Loader, version_json: &Value, java_path: &str) -> bool {
    java_major >= 25
        && crate::aot::loader_supports_aot(loader)
        && profile_wants_compact_headers(version_json)
        && has_compact_headers_archive(java_path)
}

/// The profile's `arguments.default-user-jvm` template asks for
/// `-XX:+UseCompactObjectHeaders`.
///
/// This walks the template itself instead of going through [`expand_jvm_args`]:
/// Mojang's 26.x template carries the flag in an object entry with `value` but
/// NO `rules` key, and `expand_with` treats a rule-less object as "not
/// applicable" (it is written for `arguments.jvm`, where every object has
/// rules). Here a rule-less object is unconditional, as the argument format
/// says; only entries WITH rules are gated on them.
fn profile_wants_compact_headers(version_json: &Value) -> bool {
    let Some(template) = version_json.pointer("/arguments/default-user-jvm").and_then(Value::as_array) else {
        return false;
    };
    let env = RuleEnv { os: crate::platform::os_name(), arch: crate::platform::node_arch(), features: None };
    template.iter().any(|entry| {
        let values: Vec<&Value> = match entry {
            Value::String(_) => vec![entry],
            Value::Object(o) => match o.get("rules") {
                // Rule-less object entries are unconditional.
                None => match o.get("value") {
                    Some(Value::Array(vs)) => vs.iter().collect(),
                    Some(v) => vec![v],
                    None => vec![],
                },
                Some(rules) => {
                    let probe = Value::Object(Map::from_iter([("rules".to_string(), rules.clone())]));
                    if !crate::rules::rules_allow_with(&probe, &env) {
                        return false;
                    }
                    match o.get("value") {
                        Some(Value::Array(vs)) => vs.iter().collect(),
                        Some(v) => vec![v],
                        None => vec![],
                    }
                }
            },
            _ => vec![],
        };
        values.iter().any(|v| v.as_str() == Some("-XX:+UseCompactObjectHeaders"))
    })
}

/// `<java_home>/{bin,lib}/server/classes_coh.jsa`, where `<java_home>` is the
/// parent of the directory holding the resolved java executable.
fn has_compact_headers_archive(java_path: &str) -> bool {
    let Some(home) = Path::new(java_path).parent().and_then(|bin| bin.parent()) else {
        return false;
    };
    ["bin/server", "lib/server"].iter().any(|dir| home.join(dir).join("classes_coh.jsa").is_file())
}

/// Boot-time flags (build_argv adds them on JDK 8 and 21+): `-XX:-UsePerfData`,
/// `-Xverify:none` (JDK < 27 — a hard error from 27 on), on JDK 8
/// `-XX:MetaspaceSize=128M` (its ~21 MB default stops a Forge 1.8.9 boot for
/// two full collections), and the opt-in C1-only `-XX:TieredStopAtLevel=1`
/// (instance `fast_boot` wins, absent -> config `fast_boot === true`). Forge
/// 1.8.9 measured −0.6 s to the main menu with the first three.
pub fn boot_flags(engine: &Engine, instance: &InstanceMeta, java_major: Option<u32>) -> Vec<String> {
    let mut flags = vec!["-XX:-UsePerfData".to_string()];
    if java_major.map_or(true, |m| m < 27) {
        flags.push("-Xverify:none".into());
    }
    if java_major == Some(8) {
        flags.push("-XX:MetaspaceSize=128M".into());
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

/// The VM options of vanilla's `arguments.jvm` template (rules evaluated): `-X…`
/// flags and the module/access options (`--add-exports X`, `--add-opens X`,
/// `--enable-native-access=…`, `--sun-misc-unsafe-memory-access=…`). The
/// `-D` properties and `-cp` are left out: the launcher sets its own natives
/// path and classpath. Mojang ships flags the game needs to stay alive here —
/// 26.3 requires `-XX:StackShadowPages=32` (LWJGL/SDL native upcalls overrun the
/// default shadow zone and kill the JVM in jvm.dll during resource reloads, e.g.
/// a server resource pack on join). Options already in `present` are skipped.
pub fn vanilla_vm_flags(version_json: &Value, os: &str, arch: &str, present: &[String]) -> Vec<String> {
    let Some(template) = version_json.pointer("/arguments/jvm").and_then(Value::as_array) else {
        return vec![];
    };
    let has_pair = |list: &[String], opt: &str, val: &str| list.windows(2).any(|w| w[0] == opt && w[1] == val);
    let args = expand_jvm_args(template, os, arch);
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if matches!(a.as_str(), "--add-exports" | "--add-opens") {
            if let Some(v) = args.get(i + 1) {
                if !has_pair(present, a, v) && !has_pair(&out, a, v) {
                    out.extend([a.clone(), v.clone()]);
                }
            }
            i += 2;
            continue;
        }
        let vm_option = a.starts_with("-X")
            || a.starts_with("--enable-native-access=")
            || a.starts_with("--sun-misc-unsafe-memory-access=");
        if vm_option && !a.contains("${") && !present.contains(a) && !out.contains(a) {
            out.push(a.clone());
        }
        i += 1;
    }
    out
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

/// The classpath the JVM actually receives. Fabric on a JDK 25+ AOT runtime
/// keeps Mojang's signed client jar off `-cp`: only that jar carries a 5 MB
/// `META-INF/MANIFEST.MF` with 40 953 continuation lines, and JDK 25 reads it
/// once in the AOT trainer and once in the cache-assembly child — ~160 s of one
/// core each time, while the other 61 jars together cost 3 s. The client jar
/// alone pushed a 32 s training run to 351 s. Fabric Loader locates the game jar
/// through `-Dfabric.gameJarPath` instead, so the game still loads it and the
/// cache is accepted. Every other loader / JVM tier keeps the resolved
/// classpath byte for byte.
pub fn launch_classpath(
    engine: &Engine,
    instance: &InstanceMeta,
    java_major: u32,
    classpath: &[PathBuf],
) -> Vec<PathBuf> {
    if fabric_jar_off_classpath(instance.loader, java_major) {
        let client = engine.paths().client_jar(&instance.version);
        return classpath.iter().filter(|p| **p != client).cloned().collect();
    }
    classpath.to_vec()
}

/// Whether a Fabric launch on this JVM keeps the client jar off `-cp`.
fn fabric_jar_off_classpath(loader: Loader, java_major: u32) -> bool {
    loader == Loader::Fabric && java_major >= 25
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

/// Pure argv assembly — no I/O beyond the one stat+header read that decides
/// whether `-Despectral.geoip` is advertised. Pushes AOT/memory warnings into
/// `resolved.warnings` (like the JS, which mutated `resolved.warnings`).
pub fn build_argv(engine: &Engine, instance: &InstanceMeta, resolved: &mut ResolvedLaunch) -> Vec<String> {
    let java_major = java_major_of(&resolved.java);
    // Agent launch options (validated at the route). `false`/`None` on every
    // plain launch, so the argv below is byte-identical to before.
    let agent = resolved.agent;
    let join = resolved.join.clone();
    let window = resolved.window.clone();

    let classpath_paths = launch_classpath(engine, instance, java_major, &resolved.classpath);
    let classpath = join_classpath(&classpath_paths);
    // Media Player: one music folder shared by every instance (the launcher's
    // data dir, not the instance gameDir), created here so the mod only ever
    // reads it. The mod falls back to <gameDir>/music when absent. Lives here,
    // not in `jvm_args` (which `resolve_launch` also calls): resolving paths
    // must not write.
    let music = engine.paths().data_dir().join("music");
    if let Err(e) = std::fs::create_dir_all(&music) {
        tracing::warn!("could not create music folder {}: {e}", music.display());
    }
    let JvmArgs { args: mut jvm, subst, warnings } = jvm_args(engine, instance, resolved, &classpath);
    resolved.warnings.extend(warnings);
    // Fabric on an AOT-capable JVM keeps the signed client jar off `-cp`; Fabric
    // Loader reads it from this property instead.
    if fabric_jar_off_classpath(instance.loader, java_major) {
        let client = engine.paths().client_jar(&instance.version);
        let client = std::path::absolute(&client).unwrap_or(client);
        jvm.push(format!("-Dfabric.gameJarPath={}", lossy(&client)));
    }
    jvm.push("-cp".into());
    jvm.push(classpath);

    let cache = lossy(&resolved.aot_cache_path);
    // AOT (-XX:AOTCache*/JEP 483) exists only on JDK 25+ runtimes, and only a
    // plain launcher classpath can be cached: NeoForge/Forge launch through
    // their own module/bootstrap layer (resolve_launch already reports
    // `aot_available: false` for them).
    let aot_jdk = java_major >= 25;
    if resolved.mode == LaunchMode::Train {
        if aot_jdk && resolved.aot_available {
            // The branding mod neutralizes MC's ClientShutdownWatchdog during
            // training (the AOT dump always exceeds its budget).
            jvm.push("-Despectral.aot-training=true".into());
            jvm.push(format!("-XX:AOTCacheOutput={cache}"));
        } else if !resolved.aot_available {
            resolved
                .warnings
                .push(format!("AOT is not available for {} launches; skipping -XX:AOTCacheOutput", instance.loader));
        } else {
            resolved.warnings.push(format!(
                "AOT training requires a JDK 25-tier runtime (current major {java_major}); skipping -XX:AOTCacheOutput"
            ));
        }
    } else if aot_jdk && resolved.aot_available {
        if resolved.aot_cache_exists && !resolved.aot_cache_stale {
            jvm.push(format!("-XX:AOTCache={cache}"));
            // RELATIVE proof log — -Xlog splits on ':' so C:\... would break; cwd = gameDir.
            jvm.push("-Xlog:aot=info:file=aot-%p.log".into());
        } else if resolved.aot_cache_exists {
            resolved.warnings.push(format!(
                "AOT cache {cache} no longer matches the classpath or VM flags on disk; retraining is required"
            ));
        } else {
            resolved.warnings.push(format!("AOT cache {cache} not found; running without -XX:AOTCache"));
        }
    } else if !resolved.aot_available {
        resolved
            .warnings
            .push(format!("AOT is not available for {} launches; running without -XX:AOTCache", instance.loader));
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
    let legacy_client = quick_play_unsupported(&resolved.version.json);
    let mut fragment = crate::agent::game_arg_fragment(&crate::agent::LaunchOptions {
        agent,
        join: join.clone(),
        window: window.clone(),
    });
    // Pre-1.20 clients have no `--quickPlay*` args at all: a server join becomes
    // `--server/--port`, a world join is handed to the branded mod through
    // `-Despectral.join.world=<save>` (the mod opens it once the title loads).
    if legacy_client {
        fragment = legacy_fragment(fragment);
        if let Some(world) = join.as_ref().and_then(|j| j.world.clone()).filter(|w| !w.is_empty()) {
            // Before `-cp` so the property is not parsed as a class name.
            let at = jvm.iter().position(|a| a == "-cp").unwrap_or(jvm.len());
            jvm.insert(at, format!("-Despectral.join.world={world}"));
        }
    }
    // Pre-1.13 profiles have no `arguments.game` template, only a flat string.
    let legacy_args = resolved
        .version
        .json
        .get("minecraftArguments")
        .and_then(Value::as_str)
        .filter(|_| {
            resolved.version.json.pointer("/arguments/game").and_then(Value::as_array).map_or(true, |a| a.is_empty())
        })
        .map(str::to_string);
    let sub = |k: &str| subst.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let prog: Vec<String> = match &resolved.loader {
        Some(l) if !l.game_args.is_empty() => {
            expand_game_args(&l.game_args, &features).iter().map(|a| substitute_placeholders(a, &subst)).collect()
        }
        _ if legacy_args.is_some() => {
            // Pre-1.13 versions (and legacy Forge, whose profile string carries
            // --tweakClass): the version's own `minecraftArguments` string.
            let mut prog: Vec<String> = legacy_args
                .unwrap_or_default()
                .split_whitespace()
                .map(|a| substitute_placeholders(a, &subst))
                .collect();
            prog.extend(fragment);
            prog
        }
        _ => {
            let mut prog = vec![
                "--username".into(),
                sub("auth_player_name"),
                "--version".into(),
                resolved.version.id.clone(),
                "--gameDir".into(),
                lossy(&resolved.game_dir),
                "--assetsDir".into(),
                lossy(&resolved.assets_dir),
                "--assetIndex".into(),
                resolved.version.asset_index_id.clone().unwrap_or_else(|| "null".into()),
                "--uuid".into(),
                sub("auth_uuid"),
                "--accessToken".into(),
                sub("auth_access_token"),
                "--userType".into(),
                sub("user_type"),
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

/// The JVM argument list before `-cp`, the placeholder map shared with the
/// program args, and any warnings raised while assembling. One assembly shared
/// by [`build_argv`] (which appends `-cp`, the AOT flags and the main class) and
/// `resolve_launch` (which stamps the AOT `vm_flags`), so training and launch
/// cannot drift apart.
struct JvmArgs {
    args: Vec<String>,
    subst: Map<String, Value>,
    warnings: Vec<String>,
}

fn jvm_args(engine: &Engine, instance: &InstanceMeta, resolved: &ResolvedLaunch, classpath: &str) -> JvmArgs {
    let mut warnings = Vec::new();
    let java_major = java_major_of(&resolved.java);
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
        warnings.push(format!("memory_mb {requested_mb} clamped to {effective_mb} (60% of system RAM)"));
    }

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
    put("classpath", classpath.to_string());
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
    put("user_properties", "{}".into());

    let mut jvm: Vec<String> = memory_args(engine, Some(effective_mb)).to_vec();
    // --add-opens/--add-exports/--enable-native-access are Java 9+ (the
    // latter 17+) flags; legacy tiers (JDK 8/16/17) reject them at startup.
    if java_major >= 21 {
        jvm.extend(FIXED_JVM_ARGS.iter().map(|s| s.to_string()));
    }
    jvm.push(format!("-Djava.library.path={natives}"));
    // Built by `build_argv`, which also creates the folder.
    jvm.push(format!("-Despectral.music.dir={}", lossy(&engine.paths().data_dir().join("music"))));
    if let Some(a) = logging_config_arg(engine, &resolved.version.json) {
        jvm.push(a);
    }
    // Measured tiers only: JDK 21+ (Fabric/NeoForge/Forge 1.21+) and JDK 8
    // (Forge 1.8.9/1.12.2); 16/17 keep the JVM defaults.
    if java_major == 8 || java_major >= 21 {
        jvm.extend(boot_flags(engine, instance, Some(java_major)));
    }
    if java_major >= 21 {
        jvm.extend(stack_flags(cfg!(windows)).iter().map(|s| s.to_string()));
    }
    // Compact object headers: only when the profile asks and the runtime ships
    // the compact-headers CDS archive. Decided once: the heap ratio flags are
    // only measured clean together with it.
    let compact_headers =
        compact_object_headers(java_major, instance.loader, &resolved.version.json, &resolved.java.path);
    if compact_headers {
        jvm.push("-XX:+UseCompactObjectHeaders".into());
    }
    // Low-memory heap policy (the <= 8.5 GiB tier of the automatic heap).
    let heap = low_memory_heap_flags(crate::platform::total_memory_bytes(), compact_headers);
    jvm.extend(heap.iter().map(|s| s.to_string()));
    // Vanilla and Fabric launches build their own argv, so they would lose the
    // VM options Mojang ships in the version JSON. NeoForge/Forge profiles merge
    // that template already (their loader jvm_args below).
    let merged_profile =
        resolved.loader.as_ref().is_some_and(|l| matches!(l.kind.as_deref(), Some("neoforge" | "forge")));
    if !merged_profile {
        let flags =
            vanilla_vm_flags(&resolved.version.json, crate::platform::os_name(), crate::platform::node_arch(), &jvm);
        jvm.extend(flags);
    }
    // Mod bridge: the in-game mod reports visible uuids over loopback. Its
    // token is NOT an argument (other users can read the argv): spawn_java
    // passes it as ESPECTRAL_BRIDGE_TOKEN.
    jvm.push(format!("-Despectral.bridge.url={}", crate::loopback::bridge_url(engine)));
    // Country table the server list reads (flags). Only a table that passes
    // the header/size check is advertised: a missing or half-written one
    // means the mod shows no flags, never a crash. The game's working
    // directory is the instance dir, so the path is always absolute.
    if let Some(table) = crate::geoip::table_path(engine.paths()) {
        let table = std::path::absolute(&table).unwrap_or(table);
        jvm.push(format!("-Despectral.geoip={}", lossy(&table)));
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
            // The launcher owns java.library.path (natives are extracted into
            // `<instance>/natives`). Merged 26.x profiles carry vanilla's
            // `-Djava.library.path=${natives_directory}/java`, which would win
            // (last definition) and hide liblwjgl.so from LWJGL.
            jvm.extend(
                expand_jvm_args(&loader.jvm_args, os, arch)
                    .iter()
                    .filter(|a| !a.starts_with("-Djava.library.path="))
                    .map(|a| substitute_placeholders(a, &subst)),
            );
        }
        if matches!(loader.kind.as_deref(), Some("neoforge" | "forge")) {
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
    JvmArgs { args: jvm, subst, warnings }
}

/// True when the client has no `--quickPlay*` arguments (quick play shipped in
/// 1.20). Decided from the merged version JSON: an `arguments.game` template is
/// 1.13+, and the quick-play feature keys only exist on 1.20+; the loader's
/// own `mcVersion`/`inheritsFrom` is the fallback for loader profiles that do
/// not re-list the vanilla template.
pub fn quick_play_unsupported(version_json: &Value) -> bool {
    let id = version_json
        .get("inheritsFrom")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| version_json.get("id").and_then(Value::as_str))
        .unwrap_or_default();
    let game = version_json.pointer("/arguments/game").and_then(Value::as_array);
    if let Some(entries) = game {
        let has_quick_play = entries.iter().any(|e| {
            let vals: Vec<&Value> = match e {
                Value::Object(o) => {
                    o.get("value").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default()
                }
                _ => vec![e],
            };
            vals.iter().any(|v| v.as_str().is_some_and(|s| s.starts_with("--quickPlay")))
        });
        if has_quick_play {
            return false;
        }
    }
    match mc_release_number(id) {
        Some((major, minor)) => (major, minor) < (1, 20),
        None => version_json.get("minecraftArguments").is_some(),
    }
}

/// `(1, 21)` for `1.21.11`, `(26, 3)` for `26.3`; None for snapshots/odd ids.
fn mc_release_number(id: &str) -> Option<(u32, u32)> {
    let core = id.split(['-', ' ']).next().unwrap_or("");
    let mut parts = core.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Pre-1.20 clients have no quick play: a server join becomes the old
/// `--server host --port N` pair; a singleplayer world join is handed to the
/// mod via `-Despectral.join.world` (built in [`build_argv`]). `--width/--height`
/// exist since 1.6.
fn legacy_fragment(fragment: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(fragment.len() + 2);
    let mut it = fragment.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--quickPlaySingleplayer" => {
                it.next();
            }
            "--quickPlayMultiplayer" => {
                let target = it.next().unwrap_or_default();
                let (host, port) = match target.rsplit_once(':') {
                    Some((h, p)) if p.parse::<u16>().is_ok() => (h.to_string(), p.to_string()),
                    _ => (target, "25565".into()),
                };
                out.extend(["--server".into(), host, "--port".into(), port]);
            }
            _ => out.push(a),
        }
    }
    out
}

/// Log4Shell mitigation for the log4j 2.0-beta9 versions (1.7–1.11.2, whose
/// `logging.client` is `client-1.7.xml`). Mojang's file has the same filter
/// but an `<XMLLayout/>` console, which would break the plain-text stdout
/// the launcher parses ("Sound engine started", presence markers), so the
/// console keeps the default pattern here.
const LEGACY_LOG4J_ID: &str = "client-1.7.xml";
const LEGACY_LOG4J_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
    <Appenders>
        <Console name="SysOut" target="SYSTEM_OUT">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg%n" />
        </Console>
        <RollingRandomAccessFile name="File" fileName="logs/latest.log" filePattern="logs/%d{yyyy-MM-dd}-%i.log.gz">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg%n" />
            <Policies>
                <TimeBasedTriggeringPolicy />
                <OnStartupTriggeringPolicy />
            </Policies>
        </RollingRandomAccessFile>
    </Appenders>
    <Loggers>
        <Root level="info">
            <filters>
                <MarkerFilter marker="NETWORK_PACKETS" onMatch="DENY" onMismatch="NEUTRAL" />
                <RegexFilter regex="(?s).*\$\{[^}]*\}.*" onMatch="DENY" onMismatch="NEUTRAL"/>
            </filters>
            <AppenderRef ref="SysOut"/>
            <AppenderRef ref="File"/>
        </Root>
    </Loggers>
</Configuration>
"#;

fn legacy_log4j_path(engine: &Engine) -> PathBuf {
    engine.paths().assets_dir().join("log_configs").join("espectral-client-1.7.xml")
}

fn uses_legacy_log4j(version_json: &Value) -> bool {
    version_json.pointer("/logging/client/file/id").and_then(Value::as_str) == Some(LEGACY_LOG4J_ID)
}

/// Write the patched config (non-dry launches only).
fn ensure_legacy_log4j(engine: &Engine, version_json: &Value) -> std::io::Result<()> {
    if !uses_legacy_log4j(version_json) {
        return Ok(());
    }
    let path = legacy_log4j_path(engine);
    if std::fs::read_to_string(&path).is_ok_and(|s| s == LEGACY_LOG4J_XML) {
        return Ok(());
    }
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, LEGACY_LOG4J_XML)
}

fn logging_config_arg(engine: &Engine, version_json: &Value) -> Option<String> {
    let path = legacy_log4j_path(engine);
    (uses_legacy_log4j(version_json) && path.is_file()).then(|| format!("-Dlog4j.configurationFile={}", lossy(&path)))
}

fn redact_preview(argv: &[String], access_token: Option<&str>) -> Vec<String> {
    argv.iter().map(|a| if Some(a.as_str()) == access_token { "<REDACTED>".to_string() } else { a.clone() }).collect()
}

/// Shell-ish one-line preview with the access token redacted (the bridge
/// token is never in the argv).
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

/// The loader launch profile (main class + jvm/game argument templates) an
/// instance launches with: Fabric's own profile, or the merged NeoForge/Forge
/// templates. Shared by `resolve_launch` and [`aot_identity`] so the AOT
/// identity cannot be derived from a different profile than the launch.
fn loader_launch(
    engine: &Engine,
    instance: &InstanceMeta,
    fabric: Option<&crate::types::FabricProfile>,
    version_json: &Value,
    version_main: &str,
    game_dir: &Path,
) -> Option<LoaderLaunch> {
    if let Some(f) = fabric {
        let mut jvm_args = f.jvm_args.clone();
        if game_dir != engine.paths().instance_dir(&instance.name) {
            let mods = crate::instances::effective_mods_dir(engine, instance);
            jvm_args.push(Value::String(format!("-Dfabric.modsFolder={}", lossy(&mods))));
        }
        return Some(LoaderLaunch { main_class: f.main_class.clone(), jvm_args, game_args: vec![], kind: None });
    }
    if !matches!(instance.loader, Loader::Neoforge | Loader::Forge) {
        return None;
    }
    // NeoForge and modern Forge: the merged profile's argument templates.
    // Legacy Forge has none (empty arrays): build_argv then expands its
    // `minecraftArguments` string, which carries `--tweakClass`.
    let arr = |p: &str| version_json.pointer(p).and_then(Value::as_array).cloned().unwrap_or_default();
    Some(LoaderLaunch {
        main_class: version_main.to_string(),
        jvm_args: arr("/arguments/jvm"),
        game_args: arr("/arguments/game"),
        kind: Some(if instance.loader == Loader::Neoforge { "neoforge" } else { "forge" }.into()),
    })
}

/// What an AOT cache must match: the classpath the JVM actually receives and
/// the AOT-relevant VM flags this launch passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AotLaunchIdentity {
    pub classpath: Vec<PathBuf>,
    pub vm_flags: Vec<String>,
}

/// The AOT identity of an already-resolved launch (the launch path itself).
pub fn aot_identity_of(engine: &Engine, instance: &InstanceMeta, resolved: &ResolvedLaunch) -> AotLaunchIdentity {
    let classpath = launch_classpath(engine, instance, java_major_of(&resolved.java), &resolved.classpath);
    let JvmArgs { args, .. } = jvm_args(engine, instance, resolved, &join_classpath(&classpath));
    AotLaunchIdentity { classpath, vm_flags: crate::aot::aot_vm_flags(&args) }
}

/// The AOT identity the NEXT launch will have, resolved the way
/// [`resolve_launch`] resolves it but without downloading, installing or
/// touching any game artifact (only the resolver's own caches are used).
/// `instances::aot_status` reports this so the UI chip can never disagree with
/// the launch decision.
pub async fn aot_identity(engine: &Engine, instance: &InstanceMeta) -> EngineResult<AotLaunchIdentity> {
    let version_json = crate::resolver::get_version_json(engine, &instance.version).await?;
    let version_main =
        version_json.get("mainClass").and_then(Value::as_str).unwrap_or("net.minecraft.client.main.Main").to_string();
    let version_java = version_json.pointer("/javaVersion/majorVersion").and_then(Value::as_u64).unwrap_or(8) as u32;
    let java = status_java_for_launch(engine, instance, version_java).await?;
    let fabric = if instance.loader == Loader::Fabric {
        Some(crate::resolver::resolve_fabric(engine, &instance.version, None).await?)
    } else {
        None
    };
    let instance_dir = engine.paths().instance_dir(&instance.name);
    let loader = loader_launch(engine, instance, fabric.as_ref(), &version_json, &version_main, &instance_dir);
    let classpath = crate::resolver::resolve_classpath(engine, instance, &version_json, fabric.as_ref());
    let resolved = ResolvedLaunch {
        java,
        version: ResolvedVersionRef {
            id: instance.version.clone(),
            main_class: version_main,
            asset_index_id: None,
            json: version_json,
        },
        loader,
        fabric_profile: fabric,
        classpath,
        natives_dir: engine.paths().instance_natives_dir(&instance.name),
        game_dir: instance_dir,
        assets_dir: engine.paths().assets_dir(),
        memory_mb: instance.memory_mb.unwrap_or_else(|| engine.config().default_memory_mb()),
        account: default_account(),
        bridge_token: None,
        mode: LaunchMode::Normal,
        aot_key: String::new(),
        aot_cache_path: PathBuf::new(),
        aot_cache_exists: false,
        aot_cache_stale: false,
        aot_available: false,
        agent: false,
        join: None,
        window: None,
        ffmpeg_path: None,
        warnings: vec![],
    };
    Ok(aot_identity_of(engine, instance, &resolved))
}

/// [`java_for_launch`] for the AOT status read: the same ladder, step for step,
/// with one intentional difference — it never performs the Adoptium download
/// (`runtimes::java_for_version` step 6). Every other step can therefore pick
/// the same runtime the next launch will: the per-instance override, the GLOBAL
/// `config.jdk_path_override` (probed, accepted only inside the tier, never
/// cached), `config.jvm`, the tier's downloaded runtime, FastClient's bundled
/// JDK and PATH java. When nothing fits, the tier's own major is predicted (what
/// a download would install) with an empty path, so the compact-headers archive
/// is not found and the read can only over-report staleness until the first
/// launch provisions the JDK.
async fn status_java_for_launch(
    engine: &Engine,
    instance: &InstanceMeta,
    version_java: u32,
) -> EngineResult<JavaRuntime> {
    if let Some(o) = instance.jdk_path_override.as_deref().filter(|s| !s.is_empty()) {
        return crate::jvm::get_jvm_info(engine, Some(o)).await;
    }
    let tier = if instance.loader == Loader::Neoforge {
        21
    } else if instance.loader == Loader::Forge {
        crate::forge::java_tier(&instance.version, version_java)
    } else {
        version_java
    };
    let need = if tier == 0 { 17 } else { tier };
    let max = crate::runtimes::max_major_for(need);

    // NeoForge goes through `java_for_major_exact` (EXACTLY 21, no override and
    // no `config.jvm`): mirror that ladder instead of the tier one.
    if instance.loader == Loader::Neoforge {
        if let Some(c) = crate::runtimes::cached_major(engine, 21).filter(|j| java_major_of(j) == 21) {
            return Ok(c);
        }
        if let Some(pj) = crate::jvm::path_java(engine) {
            if let Ok(probe) = crate::jvm::probe_java(engine, &pj).await {
                if probe.ok && probe.major == 21 {
                    return Ok(crate::jvm::to_jvm_info(&pj, &probe, crate::types::JvmSource::Path));
                }
            }
        }
        return Ok(JavaRuntime { major: Some(21), ..Default::default() });
    }

    // 1. Settings -> Java path: probed, must fit the tier, never cached.
    if let Some(p) = engine.config().jdk_path_override() {
        let pp = Path::new(&p);
        if pp.exists() {
            // An unusable override makes a real launch fail; a status read just
            // falls through and reports what the rest of the ladder decides.
            if let Ok(probe) = crate::jvm::probe_java(engine, pp).await {
                if probe.ok && (need..=max).contains(&probe.major) {
                    return Ok(crate::jvm::to_jvm_info(pp, &probe, crate::types::JvmSource::Path));
                }
            }
        }
    }
    // 2. `config.jvm`: the launch ranks it by the major parsed from
    // `build`/`version` (a record with neither is skipped), not by its stored
    // `major` field — mirrored here so both pick the same runtime.
    if let Some(c) = engine.config().jvm().filter(|j| !j.path.is_empty() && Path::new(&j.path).exists()) {
        let cm = parse_major(if !c.build.is_empty() { &c.build } else { &c.version });
        if cm > 0 && (need..=max).contains(&cm) {
            return Ok(JavaRuntime { major: Some(cm), ..c });
        }
    }
    // 3. The tier's downloaded runtime (its own probe recorded the major).
    if let Some(c) = crate::runtimes::cached_major(engine, need) {
        return Ok(c);
    }
    // 4./5. FastClient's bundled JDK, then PATH java — same order, same gate.
    for p in [crate::runtimes::fast_client_java(engine), crate::jvm::path_java(engine)].into_iter().flatten() {
        // An unusable candidate is skipped, never fatal: this is a status read.
        if let Ok(probe) = crate::jvm::probe_java(engine, &p).await {
            if probe.ok && (need..=max).contains(&probe.major) {
                return Ok(crate::jvm::to_jvm_info(&p, &probe, crate::types::JvmSource::Path));
            }
        }
    }
    Ok(JavaRuntime { major: Some(need), ..Default::default() })
}

/// The JDK a launch runs on: per-instance override (any JDK) > NeoForge's
/// EXACT 21 > the version's own `javaVersion.majorVersion` tier.
///
/// Forge follows the version's tier (1.8.9/1.12.2 = 8, 1.21.x = 21, 26.x =
/// 25) but never on an older JVM than the loader can boot: modern Forge's
/// bootstrap needs 21+, so an old version reported by a hand-edited profile
/// cannot drag the launch down to 8.
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
    if instance.loader == Loader::Forge {
        return crate::runtimes::java_for_version(
            engine,
            crate::forge::java_tier(&instance.version, version_java_major),
        )
        .await;
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
    crate::instances::validate_mc_version(&instance.version)?;
    phase("version");
    let mut version_json = crate::resolver::get_version_json(engine, &instance.version).await?;
    progress("[espectral] Minecraft version resolved");
    phase("version");

    let mut neoforge_id: Option<String> = None;
    if instance.loader == Loader::Neoforge {
        let lv = instance.loader_version.clone().filter(|s| !s.is_empty()).ok_or_else(|| {
            EngineError::api(400, "LOADER_VERSION_REQUIRED", "loader_version es obligatorio para NeoForge")
        })?;
        crate::instances::validate_loader_version(&lv)?;
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
    let mut forge_id: Option<String> = None;
    if instance.loader == Loader::Forge {
        // Missing build (older instances / hand-written JSON): the promotions
        // default, exactly like instance creation.
        let build =
            crate::forge::resolve_forge_version(engine, &instance.version, instance.loader_version.as_deref()).await?;
        let fj = crate::forge::ensure_forge_loader(engine, &instance.version, &build).await?;
        version_json = crate::resolver::resolve_version_chain(engine, fj).await?;
        forge_id = Some(crate::forge::forge_version_id(&instance.version, &build));
        progress("[espectral] perfil Forge cargado");
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

        if let Err(e) = ensure_legacy_log4j(engine, &version_json) {
            warnings.push(format!("legacy log4j config write failed: {e}"));
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
            let canonical = crate::instances::effective_game_dir(engine, instance);
            // A background refresh (the stored grant was already usable) rewrites
            // the same file; the in-game mod hot-reloads it by mtime. It may land
            // after the user switched accounts and another launch wrote its own
            // cosmetics into the same gameDir, so it only touches copies that
            // still name this account.
            let on_refreshed = {
                let (engine, flat, game_dir, canonical) =
                    (engine.clone(), flat.clone(), game_dir.clone(), canonical.clone());
                move || {
                    crate::cosmetics::rewrite_cosmetics_file_if_owned(&engine, &flat, &game_dir, Some(&canonical));
                }
            };
            match crate::identity::refresh_grant_for_launch(engine, &flat, 2000, on_refreshed).await {
                Ok(refreshed) => {
                    if !refreshed {
                        warnings.push("identity grant refresh skipped/failed — using cached entitlement".into());
                    }
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
    let classpath = launch_classpath(engine, instance, java_major_of(&java), &classpath);
    phase("classpath");
    let assets_dir = engine.paths().assets_dir();

    progress("[espectral] launching game…");
    phase("ready");
    let build = Some(java.build.as_str()).filter(|b| !b.is_empty());
    let aot_key = crate::aot::cache_key(&instance.version, build, None);
    let aot_cache_path = engine.paths().aot_cache_file(&aot_key);
    let aot_cache_exists = std::fs::metadata(&aot_cache_path).map(|m| m.len() > 0).unwrap_or(false);

    let loader = loader_launch(engine, instance, fabric.as_ref(), &version_json, &version_main, &game_dir);
    let aot_available = java.major.unwrap_or(0) >= 25 && crate::aot::loader_supports_aot(instance.loader);

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

    let mut resolved = ResolvedLaunch {
        version: ResolvedVersionRef {
            id: neoforge_id.or(forge_id).unwrap_or_else(|| instance.version.clone()),
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
        aot_cache_stale: false,
        aot_available,
        agent: opts.agent,
        join: opts.join.clone(),
        window: opts.window.clone(),
        ffmpeg_path,
        warnings,
    };

    // A cache is REFUSED by the JVM when its classpath identity or its VM flags
    // differ from training; detect it here (before the route reads
    // `aot_cache_stale`) so the launch skips -XX:AOTCache and the route retrains.
    let identity = aot_identity_of(engine, instance, &resolved);
    resolved.aot_cache_stale = aot_cache_exists
        && crate::aot::is_cache_stale_with_flags(engine, &resolved.aot_key, &identity.classpath, &identity.vm_flags);

    Ok(resolved)
}

/// DryRunResult: argv (tokens redacted) + classpath + missing entries +
/// natives + asset index check + java path + warnings. Never spawns/writes.
pub fn dry_run_result(
    engine: &Engine,
    instance: &InstanceMeta,
    resolved: &ResolvedLaunch,
) -> EngineResult<DryRunResult> {
    let mut r = resolved.clone();
    // The list the JVM will actually receive (Fabric on JDK 25+ drops the
    // client jar from `-cp`), so the report matches the argv it prints.
    let java_major = java_major_of(&r.java);
    let classpath = launch_classpath(engine, instance, java_major, &r.classpath);
    let mut missing: Vec<String> = classpath.iter().filter(|p| !p.exists()).map(|p| lossy(p)).collect();
    // The game still needs the client jar there — it is passed as
    // `-Dfabric.gameJarPath` — so a preview before the first download must
    // report it. The `classpath` field stays the list the JVM receives.
    if fabric_jar_off_classpath(instance.loader, java_major) {
        let client = engine.paths().client_jar(&instance.version);
        if !client.exists() {
            missing.push(lossy(&client));
        }
    }
    let index =
        r.assets_dir.join("indexes").join(format!("{}.json", r.version.asset_index_id.as_deref().unwrap_or("")));
    let argv = build_argv(engine, instance, &mut r);
    Ok(DryRunResult {
        argv: redact_preview(&argv, Some(r.account.access_token.as_str())),
        classpath: classpath.iter().map(|p| lossy(p)).collect(),
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

    let child = process::spawn_java(&resolved.java.path, &argv, &game_dir, true, resolved.bridge_token.as_deref())?;
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
    use crate::agent::JoinOption;
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

    /// Program args after the main class.
    fn prog_tail(argv: &[String], main_class: &str) -> Vec<String> {
        let i = argv.iter().position(|a| a == main_class).unwrap_or_else(|| panic!("main class {main_class} present"));
        argv[i + 1..].to_vec()
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
        // JDK 8 (Forge 1.8.9/1.12.2): metaspace headroom on top.
        assert_eq!(
            boot_flags(&e, &inst("t"), Some(8)),
            ["-XX:-UsePerfData", "-Xverify:none", "-XX:MetaspaceSize=128M"]
        );
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
    fn low_memory_heap_flags_need_the_lowest_tier_and_compact_headers() {
        const GB: u64 = 1024 * 1024 * 1024;
        let on = ["-XX:MinHeapFreeRatio=20", "-XX:MaxHeapFreeRatio=40"];
        let bound = (8.5 * GB as f64) as u64;
        let totals = [0, 4 * GB, 8 * GB, bound, bound + 1, 16 * GB];
        for total in totals {
            // Never without compact headers (that arm measured +10% p99).
            assert!(low_memory_heap_flags(total, false).is_empty(), "{total}, no compact headers");
        }
        // Unknown RAM (0) and above the lowest tier never apply.
        assert!(low_memory_heap_flags(0, true).is_empty(), "0 bytes");
        assert!(low_memory_heap_flags(bound + 1, true).is_empty(), "8.5 GiB + 1 byte");
        assert!(low_memory_heap_flags(16 * GB, true).is_empty(), "16 GiB");
        // Inside the lowest tier with compact headers: the measured pair.
        for total in [4 * GB, 8 * GB, bound] {
            assert_eq!(low_memory_heap_flags(total, true), on, "{total}");
        }
    }

    #[test]
    fn compact_object_headers_needs_every_condition() {
        let d = tempfile::tempdir().unwrap();
        let jdk = d.path().join("jdk");
        let bin = jdk.join("bin");
        std::fs::create_dir_all(bin.join("server")).unwrap();
        let java = bin.join("java.exe");
        std::fs::write(&java, "x").unwrap();
        std::fs::write(bin.join("server").join("classes_coh.jsa"), "x").unwrap();
        let with_archive = java.to_string_lossy().into_owned();

        // Mojang's real 26.3 `arguments.default-user-jvm`, verbatim: the flag
        // sits in a rule-LESS object entry (the reason a plain
        // `expand_jvm_args` walk can never see it).
        let profile = json!({ "arguments": { "default-user-jvm": [
            { "value": ["-Xms2G", "-Xmx4G", "-XX:+UseCompactObjectHeaders", "-XX:+AlwaysPreTouch",
                        "-XX:+UseStringDeduplication"] },
            { "rules": [
                { "action": "allow", "os": { "name": "osx" } },
                { "action": "allow", "os": { "name": "linux" } },
                { "action": "allow", "os": { "name": "windows", "versionRange": { "min": "10.0.17134" } } }
            ], "value": ["-XX:+UseZGC"] },
            { "rules": [
                { "action": "allow", "os": { "name": "windows", "versionRange": { "max": "10.0.17134" } } }
            ], "value": ["-XX:+UnlockExperimentalVMOptions", "-XX:+UseG1GC", "-XX:G1NewSizePercent=20",
                        "-XX:G1ReservePercent=20", "-XX:MaxGCPauseMillis=50", "-XX:G1HeapRegionSize=32M"] }
        ] } });
        let no_profile = json!({});
        // The current walker must see the real shape...
        assert!(profile_wants_compact_headers(&profile), "rule-less entry counts");
        // ...and the old expansion path must not: this is the regression guard.
        assert!(
            !expand_jvm_args(
                profile.pointer("/arguments/default-user-jvm").and_then(Value::as_array).unwrap(),
                crate::platform::os_name(),
                crate::platform::node_arch()
            )
            .iter()
            .any(|a| a == "-XX:+UseCompactObjectHeaders"),
            "expand_jvm_args drops rule-less entries"
        );
        // A flag only inside an entry whose rules exclude this machine (its
        // allow rule names an OS that is not the current one) does not count.
        let gated = json!({ "arguments": { "default-user-jvm": [
            { "rules": [{ "action": "allow", "os": { "name": "plan9" } }], "value": ["-XX:+UseCompactObjectHeaders"] }
        ] } });
        assert!(!profile_wants_compact_headers(&gated), "gated entry does not count");
        // The flag in a rule-less STRING entry counts too (the other legal shape).
        assert!(profile_wants_compact_headers(
            &json!({ "arguments": { "default-user-jvm": ["-XX:+UseCompactObjectHeaders"] } })
        ));
        // The flag absent entirely.
        assert!(!profile_wants_compact_headers(
            &json!({ "arguments": { "default-user-jvm": [{ "value": ["-Xms2G"] }] } })
        ));

        // All four conditions hold.
        assert!(compact_object_headers(25, Loader::Vanilla, &profile, &with_archive));
        assert!(compact_object_headers(25, Loader::Fabric, &profile, &with_archive));
        // JDK < 25.
        assert!(!compact_object_headers(21, Loader::Vanilla, &profile, &with_archive));
        // Loader not AOT-capable.
        assert!(!compact_object_headers(25, Loader::Neoforge, &profile, &with_archive));
        assert!(!compact_object_headers(25, Loader::Forge, &profile, &with_archive));
        // Profile does not ask.
        assert!(!compact_object_headers(25, Loader::Vanilla, &no_profile, &with_archive));
        // Runtime without the compact-headers archive.
        let no_archive = d.path().join("jdk2").join("bin").join("java.exe");
        std::fs::create_dir_all(no_archive.parent().unwrap()).unwrap();
        std::fs::write(&no_archive, "x").unwrap();
        assert!(!compact_object_headers(25, Loader::Vanilla, &profile, &no_archive.to_string_lossy()));
        // The archive is also accepted under lib/server.
        let lib_jdk = d.path().join("jdk3");
        std::fs::create_dir_all(lib_jdk.join("lib").join("server")).unwrap();
        let lib_java = lib_jdk.join("bin").join("java");
        std::fs::create_dir_all(lib_java.parent().unwrap()).unwrap();
        std::fs::write(&lib_java, "x").unwrap();
        std::fs::write(lib_jdk.join("lib").join("server").join("classes_coh.jsa"), "x").unwrap();
        assert!(compact_object_headers(25, Loader::Vanilla, &profile, &lib_java.to_string_lossy()));
        // A bare executable name has no java_home to inspect.
        assert!(!compact_object_headers(25, Loader::Vanilla, &profile, "java"));
    }

    #[test]
    fn fabric_jdk25_keeps_the_client_jar_off_the_classpath() {
        let (_d, e) = temp_engine();
        let client = e.paths().client_jar("26.3");
        let libs = vec![PathBuf::from("/data/libraries/a.jar"), client.clone()];
        let fabric =
            InstanceMeta { name: "f".into(), version: "26.3".into(), loader: Loader::Fabric, ..Default::default() };
        let vanilla =
            InstanceMeta { name: "v".into(), version: "26.3".into(), loader: Loader::Vanilla, ..Default::default() };

        // Fabric on JDK 25: the client jar moves to -Dfabric.gameJarPath.
        assert_eq!(launch_classpath(&e, &fabric, 25, &libs), vec![PathBuf::from("/data/libraries/a.jar")]);
        // Fabric on JDK 21 and vanilla on JDK 25 keep the resolved classpath.
        assert_eq!(launch_classpath(&e, &fabric, 21, &libs), libs);
        assert_eq!(launch_classpath(&e, &vanilla, 25, &libs), libs);

        // build_argv agrees: the property is present before -cp, which lacks the jar.
        let mut r = make_resolved();
        r.java = java("/jdk/bin/java.exe", 25, "25.0.4+7");
        r.classpath = libs;
        r.version = ResolvedVersionRef {
            id: "26.3".into(),
            main_class: "net.minecraft.client.main.Main".into(),
            asset_index_id: Some("26.3".into()),
            json: json!({}),
        };
        let argv = build_argv(&e, &fabric, &mut r);
        let prop = format!("-Dfabric.gameJarPath={}", std::path::absolute(&client).unwrap().display());
        let prop_idx = argv.iter().position(|a| *a == prop).expect("fabric.gameJarPath present");
        let cp_idx = argv.iter().position(|a| a == "-cp").unwrap();
        assert!(prop_idx < cp_idx, "property before -cp: {argv:?}");
        assert!(!argv[cp_idx + 1].contains("26.3.jar"), "client jar off -cp: {argv:?}");
    }

    #[test]
    fn dry_run_reports_the_missing_client_jar_on_fabric_jdk25() {
        let (_d, e) = temp_engine();
        let client = e.paths().client_jar("26.3");
        let fabric =
            InstanceMeta { name: "f".into(), version: "26.3".into(), loader: Loader::Fabric, ..Default::default() };
        let mut r = make_resolved();
        r.java = java("/jdk/bin/java.exe", 25, "25.0.4+7");
        // The resolver's list, as `resolve_classpath` builds it: just the jar.
        r.classpath = vec![client.clone()];
        r.version = ResolvedVersionRef {
            id: "26.3".into(),
            main_class: "net.minecraft.client.main.Main".into(),
            asset_index_id: Some("26.3".into()),
            json: json!({}),
        };

        // Absent: the jar is off `-cp` but the game still needs it, so the
        // preview reports it while `classpath` stays the JVM's list.
        let dr = dry_run_result(&e, &fabric, &r).unwrap();
        assert_eq!(dr.classpath_missing, vec![lossy(&client)], "{dr:?}");
        assert!(!dr.classpath.iter().any(|c| c.ends_with("26.3.jar")), "{dr:?}");

        // Present: no longer missing.
        std::fs::create_dir_all(client.parent().unwrap()).unwrap();
        std::fs::write(&client, b"jar").unwrap();
        let dr = dry_run_result(&e, &fabric, &r).unwrap();
        assert!(dr.classpath_missing.is_empty(), "{dr:?}");

        // Vanilla keeps the jar on `-cp`, where the ordinary check covers it.
        std::fs::remove_file(&client).unwrap();
        let vanilla =
            InstanceMeta { name: "v".into(), version: "26.3".into(), loader: Loader::Vanilla, ..Default::default() };
        let dr = dry_run_result(&e, &vanilla, &r).unwrap();
        assert_eq!(dr.classpath_missing, vec![lossy(&client)], "{dr:?}");
        assert!(dr.classpath.iter().any(|c| c.ends_with("26.3.jar")), "{dr:?}");
    }

    #[tokio::test]
    async fn status_jdk_ladder_honours_the_settings_java_path() {
        let (d, e) = temp_engine();
        e.game_resolve_state().isolate();
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on("http://127.0.0.1:9"));
        // A cached flat version JSON + manifest naming it: no network. The
        // profile asks for compact headers (Mojang's rule-less entry shape), so
        // the flag depends only on which JDK the ladder picks.
        let vj = json!({ "id": "26.3", "mainClass": "net.minecraft.client.main.Main",
                         "javaVersion": { "majorVersion": 25 }, "libraries": [],
                         "arguments": { "default-user-jvm": [{ "value": ["-XX:+UseCompactObjectHeaders"] }] } });
        std::fs::create_dir_all(e.paths().versions_dir()).unwrap();
        std::fs::write(e.paths().versions_dir().join("26.3.json"), vj.to_string()).unwrap();
        std::fs::write(
            e.paths().version_manifest_cache(),
            json!({ "fetched_at": crate::platform::now_ms(), "latest_release": "26.3", "latest_snapshot": "26.3",
                    "versions": [{ "id": "26.3", "type": "release",
                                   "release_time": "2026-01-01T00:00:00+00:00",
                                   "url": "http://127.0.0.1:9/26.3.json" }] })
            .to_string(),
        )
        .unwrap();
        e.game_resolve_state().clear_manifest_memo();
        // Both candidate JDKs probe as a fitting 25; only their paths differ.
        e.game_resolve_state().set_probe_override(Some(std::sync::Arc::new(|_: &Path| crate::types::JavaProbe {
            ok: true,
            major: 25,
            ..Default::default()
        })));
        let inst =
            InstanceMeta { name: "s".into(), version: "26.3".into(), loader: Loader::Vanilla, ..Default::default() };

        // Settings -> Java path: a 25 whose home carries the compact-headers
        // archive, so the flag's presence in `vm_flags` proves which JDK the
        // status read picked.
        let chosen = d.path().join("settings-jdk").join("bin").join("java.exe");
        std::fs::create_dir_all(chosen.parent().unwrap().join("server")).unwrap();
        std::fs::write(&chosen, "x").unwrap();
        std::fs::write(chosen.parent().unwrap().join("server").join("classes_coh.jsa"), "x").unwrap();
        e.config().set("jdk_path_override", json!(chosen.to_string_lossy()));

        let id = aot_identity(&e, &inst).await.unwrap();
        assert!(
            id.vm_flags.contains(&"-XX:+UseCompactObjectHeaders".into()),
            "the status read honours the global override: {:?}",
            id.vm_flags
        );
        // The launch ladder agrees for the same instance.
        let java = java_for_launch(&e, &inst, 25).await.unwrap();
        assert_eq!(java.path, chosen.to_string_lossy());

        // An override OUTSIDE the tier is skipped by the launch (JDK 21 for
        // 26.3), so the status read must skip it too and lose the flag.
        let old = d.path().join("old-jdk").join("bin").join("java.exe");
        std::fs::create_dir_all(old.parent().unwrap().join("server")).unwrap();
        std::fs::write(&old, "x").unwrap();
        std::fs::write(old.parent().unwrap().join("server").join("classes_coh.jsa"), "x").unwrap();
        e.config().set("jdk_path_override", json!(old.to_string_lossy()));
        e.game_resolve_state().set_probe_override(Some(std::sync::Arc::new(|_: &Path| crate::types::JavaProbe {
            ok: true,
            major: 21,
            ..Default::default()
        })));
        let java = java_for_launch(&e, &inst, 25).await;
        assert!(
            java.map(|j| j.path != old.to_string_lossy().as_ref()).unwrap_or(true),
            "launch skips an out-of-tier override (an error means it found no JDK at all)"
        );
        let id = aot_identity(&e, &inst).await.unwrap();
        assert!(
            !id.vm_flags.contains(&"-XX:+UseCompactObjectHeaders".into()),
            "the status read skips it as well: {:?}",
            id.vm_flags
        );
    }

    #[test]
    fn boot_flags_reach_jdk8_and_jdk21_plus_only() {
        let (_d, e) = temp_engine();
        let argv = build_argv(&e, &inst("t"), &mut make_resolved());
        assert!(argv.contains(&"-XX:-UsePerfData".to_string()));
        assert!(argv.contains(&"-Xverify:none".to_string()));
        assert!(!argv.contains(&"-XX:TieredStopAtLevel=1".to_string()));
        assert!(!argv.contains(&"-XX:MetaspaceSize=128M".to_string()));
        let mut legacy = make_resolved();
        legacy.java = java("/jdk8/bin/java.exe", 8, "1.8.0_504-b07");
        let argv = build_argv(&e, &inst("t"), &mut legacy);
        for flag in ["-XX:-UsePerfData", "-Xverify:none", "-XX:MetaspaceSize=128M"] {
            assert!(argv.contains(&flag.to_string()), "{flag}");
        }
        assert!(!argv.contains(&"--add-opens".to_string()), "no JDK 9+ flags on 8");
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
    fn argv_never_carries_the_bridge_token_and_preview_redacts_access() {
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        r.account.access_token = "SECRETACCESSTOKEN".into();
        r.bridge_token = Some("a".repeat(64));
        let argv = build_argv(&e, &inst("t"), &mut r);
        assert!(!argv.iter().any(|a| a.contains(&"a".repeat(64))), "bridge token goes in the environment");
        assert!(argv.iter().any(|a| a.starts_with("-Despectral.bridge.url=")));
        let preview = format_command_preview(&argv, Some("SECRETACCESSTOKEN"));
        assert!(!preview.contains("SECRETACCESSTOKEN"));
        assert!(preview.contains("<REDACTED>"));
    }

    #[test]
    fn geoip_property_is_added_only_for_a_valid_table() {
        let (_d, e) = temp_engine();
        let has = |argv: &[String]| argv.iter().any(|a| a.starts_with("-Despectral.geoip="));
        // Nothing downloaded yet: no property, the mod simply shows no flag.
        assert!(!has(&build_argv(&e, &inst("t"), &mut make_resolved())));
        let table = crate::geoip::encode(&crate::geoip::parse_csv("8.8.8.0,8.8.8.255,US\n")).unwrap();
        let file = e.paths().geoip_table("2026-11");
        crate::paths::write_private(&file, &table).unwrap();
        // A table without meta.json is not trusted either.
        assert!(!has(&build_argv(&e, &inst("t"), &mut make_resolved())));
        crate::paths::write_json_atomic(
            &e.paths().geoip_meta(),
            &json!({ "month": "2026-11", "file": "countries-2026-11.bin" }),
        )
        .unwrap();
        let argv = build_argv(&e, &inst("t"), &mut make_resolved());
        let expected = format!("-Despectral.geoip={}", std::path::absolute(&file).unwrap().display());
        let prop = argv.iter().position(|a| *a == expected).unwrap();
        assert!(prop < argv.iter().position(|a| a == "-cp").unwrap(), "property before -cp");
        assert!(std::path::Path::new(&expected["-Despectral.geoip=".len()..]).is_absolute());
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

    // -----------------------------------------------------------------
    // Legacy (pre-1.13 / pre-1.20) launches: minecraftArguments, quick play,
    // log4j, Forge java tiers
    // -----------------------------------------------------------------

    fn legacy_resolved(json: Value) -> ResolvedLaunch {
        let mut r = make_resolved();
        r.java = java("/jdk8/bin/java", 8, "1.8.0_504");
        r.version = ResolvedVersionRef {
            id: "1.8.9".into(),
            main_class: "net.minecraft.launchwrapper.Launch".into(),
            asset_index_id: Some("1.8".into()),
            json,
        };
        r.aot_available = false;
        r
    }

    #[test]
    fn legacy_minecraft_arguments_expand_placeholders_and_keep_tweakclass() {
        let (_d, e) = temp_engine();
        let mut r = legacy_resolved(json!({
            "id": "forge-1.8.9-11.15.1.2318",
            "minecraftArguments": "--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory} --uuid ${auth_uuid} --accessToken ${auth_access_token} --userProperties ${user_properties} --tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker",
            "libraries": []
        }));
        let i = InstanceMeta {
            name: "f189".into(),
            version: "1.8.9".into(),
            loader: crate::types::Loader::Forge,
            ..Default::default()
        };
        let argv = build_argv(&e, &i, &mut r);
        let tail = prog_tail(&argv, "net.minecraft.launchwrapper.Launch");
        assert_eq!(
            tail,
            [
                "--username",
                "Tester",
                "--version",
                "1.8.9",
                "--gameDir",
                "/games/testinst",
                "--uuid",
                "00000000-0000-3000-8000-000000000000",
                "--accessToken",
                "tok",
                "--userProperties",
                "{}",
                "--tweakClass",
                "net.minecraftforge.fml.common.launcher.FMLTweaker",
            ]
        );
        // No AOT flags on a JDK 8 legacy launch, and the warning says why.
        assert!(!argv.iter().any(|a| a.starts_with("-XX:AOTCache")));
        assert!(r.warnings.iter().any(|w| w.contains("AOT is not available for forge")), "{:?}", r.warnings);
    }

    #[test]
    fn legacy_server_join_becomes_server_and_port() {
        let (_d, e) = temp_engine();
        let mut r = legacy_resolved(json!({ "id": "1.8.9", "minecraftArguments": "--username ${auth_player_name}" }));
        r.agent = true;
        r.join = Some(JoinOption { world: None, server: Some("play.example.com:25566".into()) });
        let i = InstanceMeta { name: "srv".into(), version: "1.8.9".into(), ..Default::default() };
        let argv = build_argv(&e, &i, &mut r);
        let tail = prog_tail(&argv, "net.minecraft.launchwrapper.Launch");
        assert_eq!(tail, ["--username", "Tester", "--server", "play.example.com", "--port", "25566"]);
        // Without a port the default 25565 is used.
        let mut r2 = legacy_resolved(json!({ "id": "1.8.9", "minecraftArguments": "--username ${auth_player_name}" }));
        r2.agent = true;
        r2.join = Some(JoinOption { world: None, server: Some("localhost".into()) });
        let argv = build_argv(&e, &i, &mut r2);
        let tail = prog_tail(&argv, "net.minecraft.launchwrapper.Launch");
        assert_eq!(tail[tail.len() - 4..], ["--server", "localhost", "--port", "25565"]);
    }

    #[test]
    fn legacy_world_join_uses_the_jvm_property_not_a_quick_play_arg() {
        let (_d, e) = temp_engine();
        let mut r = legacy_resolved(json!({ "id": "1.8.9", "minecraftArguments": "--username ${auth_player_name}" }));
        r.agent = true;
        r.join = Some(JoinOption { world: Some("Mundo nuevo".into()), server: None });
        let i = InstanceMeta { name: "w".into(), version: "1.8.9".into(), ..Default::default() };
        let argv = build_argv(&e, &i, &mut r);
        assert!(argv.contains(&"-Despectral.join.world=Mundo nuevo".to_string()), "{argv:?}");
        // The property sits before -cp, never in the program args.
        let prop = argv.iter().position(|a| a == "-Despectral.join.world=Mundo nuevo").unwrap();
        let cp = argv.iter().position(|a| a == "-cp").unwrap();
        assert!(prop < cp);
        let tail = prog_tail(&argv, "net.minecraft.launchwrapper.Launch");
        assert_eq!(tail, ["--username", "Tester"], "no --quickPlay* on 1.8.9");
    }

    #[test]
    fn quick_play_support_is_decided_from_the_profile() {
        // 1.20+ vanilla template carries --quickPlay args -> supported.
        let modern = json!({ "id": "26.3", "arguments": { "game": ["--username", { "rules": [], "value": ["--quickPlayMultiplayer"] }] } });
        assert!(!quick_play_unsupported(&modern));
        // 1.13..1.19: an arguments.game template without quick play.
        assert!(quick_play_unsupported(&json!({ "id": "1.19.4", "arguments": { "game": ["--username"] } })));
        // Legacy profile with a loader's inheritsFrom: the MC version decides.
        assert!(quick_play_unsupported(&json!({ "inheritsFrom": "1.12.2", "minecraftArguments": "--tweakClass x" })));
        assert!(!quick_play_unsupported(
            &json!({ "inheritsFrom": "1.21.11", "arguments": { "game": ["--launchTarget", "forge_client"] } })
        ));
        assert!(
            !quick_play_unsupported(&json!({ "id": "26.3" })),
            "a 1.20+ profile without a template is still modern"
        );
        assert!(!quick_play_unsupported(&json!({ "id": "26.3", "inheritsFrom": "26.3" })));
        assert!(quick_play_unsupported(&json!({ "id": "1.8.9" })));
        // An unrecognizable profile keeps the previous behaviour (the fixed
        // program list, which has no quick-play placeholders to lose).
        assert!(!quick_play_unsupported(&json!({})));
    }

    #[test]
    fn modern_join_world_stays_a_quick_play_argument() {
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        r.agent = true;
        r.join = Some(JoinOption { world: Some("World".into()), server: None });
        let i = InstanceMeta { name: "m".into(), version: "26.3".into(), ..Default::default() };
        let argv = build_argv(&e, &i, &mut r);
        assert!(!argv.iter().any(|a| a.starts_with("-Despectral.join.world=")), "no JVM prop on 1.20+");
        let tail = prog_tail(&argv, "net.minecraft.client.main.Main");
        assert_eq!(tail[tail.len() - 2..], ["--quickPlaySingleplayer", "World"]);
    }

    #[test]
    fn legacy_log4j_config_arg_only_for_the_1_7_profile() {
        let (_d, e) = temp_engine();
        std::fs::create_dir_all(e.paths().assets_dir().join("log_configs")).unwrap();
        std::fs::write(legacy_log4j_path(&e), LEGACY_LOG4J_XML).unwrap();
        let legacy = json!({ "logging": { "client": { "file": { "id": "client-1.7.xml" } } } });
        assert!(uses_legacy_log4j(&legacy));
        assert_eq!(
            logging_config_arg(&e, &legacy).as_deref(),
            Some(format!("-Dlog4j.configurationFile={}", lossy(&legacy_log4j_path(&e))).as_str())
        );
        // Modern profiles (client-1.12.xml+) keep Mojang's own config.
        for modern in [json!({ "logging": { "client": { "file": { "id": "client-1.12.xml" } } } }), json!({})] {
            assert!(!uses_legacy_log4j(&modern));
            assert!(logging_config_arg(&e, &modern).is_none());
        }
        // The written file carries the Log4Shell filters and a plain console layout.
        assert!(LEGACY_LOG4J_XML.contains("RegexFilter"));
        assert!(LEGACY_LOG4J_XML.contains("<!--%m -->") || LEGACY_LOG4J_XML.contains("PatternLayout"));
    }

    #[test]
    fn forge_java_tier_follows_the_loader_generation() {
        assert_eq!(crate::forge::java_tier("1.8.9", 8), 8);
        assert_eq!(crate::forge::java_tier("1.12.2", 8), 8);
        assert_eq!(crate::forge::java_tier("1.21.11", 21), 21);
        assert_eq!(crate::forge::java_tier("1.21.1", 8), 21, "modern Forge never runs on 8");
        assert_eq!(crate::forge::java_tier("26.3", 25), 25);
        assert_eq!(crate::forge::installer_java_tier("1.12.2", 8), 8);
        assert_eq!(crate::forge::installer_java_tier("1.21.11", 21), 21);
        assert_eq!(crate::forge::installer_java_tier("26.3", 25), 25);
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
    fn loader_profile_cannot_override_java_library_path() {
        // Forge/NeoForge 26.x merge vanilla's `-Djava.library.path=${natives_directory}/java`;
        // as the later -D it would win and LWJGL could not find liblwjgl.so.
        let (_d, e) = temp_engine();
        let mut r = make_resolved();
        r.loader = Some(LoaderLaunch {
            main_class: "net.minecraftforge.bootstrap.ForgeBootstrap".into(),
            jvm_args: vec![
                json!("-Djava.library.path=${natives_directory}/java"),
                json!("-Djna.tmpdir=${natives_directory}/jna"),
            ],
            game_args: vec![json!("--username"), json!("${auth_player_name}")],
            kind: Some("forge".into()),
        });
        let i = InstanceMeta { name: "f263".into(), version: "26.3".into(), ..Default::default() };
        let argv = build_argv(&e, &i, &mut r);
        let lib_paths: Vec<&String> = argv.iter().filter(|a| a.starts_with("-Djava.library.path=")).collect();
        let natives = lossy(&r.natives_dir);
        assert_eq!(lib_paths, [&format!("-Djava.library.path={natives}")]);
        assert!(argv.contains(&format!("-Djna.tmpdir={natives}/jna")), "{argv:?}");
    }

    #[test]
    fn vanilla_vm_flags_reach_fabric_and_vanilla_launches() {
        // 26.3 dies in jvm.dll on resource reloads without -XX:StackShadowPages=32,
        // which only vanilla's version JSON carries.
        let (_d, e) = temp_engine();
        let template = json!({ "arguments": { "jvm": [
            { "rules": [{ "action": "allow", "os": { "arch": "x86" } }], "value": "-Xss1M" },
            "-XX:StackShadowPages=32",
            "--enable-native-access=ALL-UNNAMED",
            "--add-exports", "java.base/jdk.internal.misc=ALL-UNNAMED",
            "--add-exports", "java.base/sun.nio.ch=ALL-UNNAMED",
            "-Djava.library.path=${natives_directory}/java",
            "-Dminecraft.launcher.brand=${launcher_name}",
            "-cp", "${classpath}"
        ]}});
        let i = InstanceMeta { name: "f263".into(), version: "26.3".into(), ..Default::default() };
        for fabric in [false, true] {
            let mut r = make_resolved();
            r.version.json = template.clone();
            if fabric {
                r.loader = Some(LoaderLaunch {
                    main_class: "net.fabricmc.loader.impl.launch.knot.KnotClient".into(),
                    jvm_args: vec![],
                    game_args: vec![],
                    kind: None,
                });
            }
            let argv = build_argv(&e, &i, &mut r);
            let count = |s: &str| argv.iter().filter(|a| *a == s).count();
            assert_eq!(count("-XX:StackShadowPages=32"), 1, "{argv:?}");
            assert_eq!(count("-Xss1M"), 0, "x86-only rule");
            assert!(argv.windows(2).any(|w| w == ["--add-exports", "java.base/jdk.internal.misc=ALL-UNNAMED"]));
            assert_eq!(
                argv.windows(2).filter(|w| *w == ["--add-exports", "java.base/sun.nio.ch=ALL-UNNAMED"]).count(),
                1
            );
            assert_eq!(count("--enable-native-access=ALL-UNNAMED"), 1);
            assert_eq!(count("-cp"), 1);
            assert!(!argv.iter().any(|a| a.starts_with("-Dminecraft.launcher.brand=")));
            assert_eq!(argv.iter().filter(|a| a.starts_with("-Djava.library.path=")).count(), 1);
        }
        // NeoForge/Forge merged the template into their own jvm_args: not added twice.
        let mut r = make_resolved();
        r.version.json = template;
        r.loader = Some(LoaderLaunch {
            main_class: "net.minecraftforge.bootstrap.ForgeBootstrap".into(),
            jvm_args: vec![json!("-XX:StackShadowPages=32")],
            game_args: vec![],
            kind: Some("forge".into()),
        });
        let argv = build_argv(&e, &i, &mut r);
        assert_eq!(argv.iter().filter(|a| *a == "-XX:StackShadowPages=32").count(), 1, "{argv:?}");
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
