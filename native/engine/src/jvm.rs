//! Port of `src/engine/jvm.mjs` — JVM discovery / probing (`java
//! -XshowSettings:properties -version`), primary JVM cache in `config.jvm`.
//!
//! **Owner: game-resolve.**
//!
//! Discovery order (`discover_jvm`), every candidate verified by a probe:
//!   1. `config.jdk_path_override` (must verify as >= 21)
//!   2. cached winner (`config.jvm`, when the file still exists)
//!   3. FastClient bundled Temurin 25 (Windows only)
//!   4. PATH `java` (only if >= 21)
//!   5. Adoptium Temurin 25 download (shared pipeline in [`crate::runtimes`])
//!
//! The probe is memoized per `path:mtimeMs:size` in `GameResolveState`. The
//! PATH lookup is done natively (walks `PATH`, honours `PATHEXT` on Windows)
//! instead of spawning `which` / `where`. Helper processes are spawned with
//! `CREATE_NO_WINDOW` on Windows (JS `windowsHide`).
//!
//! CONTRACT (called by game-launch / api): get_jvm_info, probe_java, jcmd_path.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{JavaProbe, JavaRuntime, JvmSource};

/// discoverJvm floor (the primary JVM is a 21+ runtime).
pub const MIN_MAJOR: u32 = 21;

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// `'25.0.4' -> 25 ; '1.8.0_392' -> 8 ; garbage -> 0` (JS parseMajor).
pub fn parse_major(version: &str) -> u32 {
    let digits: String = version.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 0;
    }
    let rest = &version[digits.len()..];
    if !(rest.is_empty() || rest.starts_with('.')) {
        return 0;
    }
    match digits.parse::<u64>() {
        Ok(1) => 8,
        Ok(n) => u32::try_from(n).unwrap_or(0),
        Err(_) => 0,
    }
}

/// Node `stat.mtimeMs` (sec * 1e3 + nsec / 1e6).
pub(crate) fn node_mtime_ms(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as f64 * 1000.0 + d.subsec_nanos() as f64 / 1e6)
        .unwrap_or(0.0)
}

/// JS `__probeMemoKey`: `${path}:${mtimeMs}:${size}`, None when stat fails.
pub fn probe_memo_key(java_path: &Path) -> Option<String> {
    let st = std::fs::metadata(java_path).ok()?;
    Some(format!("{}:{}:{}", java_path.display(), node_mtime_ms(&st), st.len()))
}

/// FastClient's bundled Temurin 25 (jvm.mjs flavour): Windows only —
/// `%APPDATA%\FastClient\runtimes\java-25\jdk-25.0.4+7\bin\java.exe`; None elsewhere.
pub fn fast_client_java(engine: &Engine) -> Option<PathBuf> {
    if let Some(o) = engine.game_resolve_state().hooks().fastclient_java {
        return o;
    }
    if !cfg!(windows) {
        return None;
    }
    fastclient_java_under_appdata("java.exe")
}

/// `<APPDATA | USERPROFILE\AppData\Roaming>\FastClient\runtimes\java-25\jdk-25.0.4+7\bin\<bin>`.
pub(crate) fn fastclient_java_under_appdata(bin: &str) -> Option<PathBuf> {
    let base = crate::paths::appdata_dir().or_else(|| {
        std::env::var("USERPROFILE")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|u| PathBuf::from(u).join("AppData").join("Roaming"))
    })?;
    Some(base.join("FastClient").join("runtimes").join("java-25").join("jdk-25.0.4+7").join("bin").join(bin))
}

/// First `java` on PATH (JS `which java` / `where java`), else None.
pub fn path_java(engine: &Engine) -> Option<PathBuf> {
    if let Some(o) = engine.game_resolve_state().hooks().path_java {
        return o;
    }
    find_on_path("java", std::env::var_os("PATH").as_deref())
}

/// Resolve `name` against a PATH value (executables only, like `which`).
pub(crate) fn find_on_path(name: &str, path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    let exts: Vec<String> = if cfg!(windows) {
        let pe = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let mut v: Vec<String> = pe.split(';').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
        v.insert(0, String::new());
        v
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for ext in &exts {
            let cand = dir.join(format!("{name}{ext}"));
            if is_executable_file(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// `tokio::process::Command` that never flashes a console window on Windows.
pub(crate) fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    {
        cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    cmd
}

/// Parse the merged stdout+stderr of `java -XshowSettings:properties -version`.
pub fn parse_probe_output(out: &str) -> JavaProbe {
    let mut result = JavaProbe::default();
    let prop_re = regex::Regex::new(r"^\s*([a-zA-Z0-9_.]+)\s*=\s*(.*)$").expect("regex");
    let mut props = std::collections::HashMap::<String, String>::new();
    for line in out.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(c) = prop_re.captures(line) {
            props.insert(c[1].trim().to_string(), c[2].trim().to_string());
        }
    }
    let version = props.get("java.version").cloned();
    let build = props.get("java.runtime.version").cloned().or_else(|| props.get("java.version").cloned());
    let vendor = props.get("java.vendor").cloned();
    match version {
        None => {
            let banner = regex::Regex::new(r#"version "([^"]+)""#).expect("regex");
            if let Some(b) = banner.captures(out) {
                result.version = Some(b[1].to_string());
                result.build = Some(build.unwrap_or_else(|| b[1].to_string()));
                let bm = regex::Regex::new(r"\(build ([^)]+)\)").expect("regex");
                if let Some(m) = bm.captures(out) {
                    if m[1].starts_with(|c: char| c.is_ascii_digit()) {
                        result.build = Some(m[1].to_string());
                    }
                }
            } else {
                let head: String = out.chars().take(200).collect();
                result.error = Some(format!("unparseable java output: {head}"));
                return result;
            }
        }
        Some(v) => {
            result.version = Some(v);
            result.build = build;
        }
    }
    result.vendor = vendor;
    result.major = parse_major(result.version.as_deref().or(result.build.as_deref()).unwrap_or(""));
    // Any parseable JDK is ok; the >= 21 floor is a discovery policy.
    result.ok = result.major > 0;
    if !result.ok {
        result.error = Some(format!(
            "unparseable java version '{}'",
            result.version.as_deref().or(result.build.as_deref()).unwrap_or("null")
        ));
    }
    result
}

async fn spawn_probe(java_path: &Path) -> JavaProbe {
    let mut cmd = hidden_command(java_path);
    cmd.args(["-XshowSettings:properties", "-version"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let out = match cmd.spawn() {
        Ok(child) => match tokio::time::timeout(PROBE_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return JavaProbe { error: Some(e.to_string()), ..Default::default() },
            Err(_) => {
                return JavaProbe {
                    error: Some(format!("spawnSync {} ETIMEDOUT", java_path.display())),
                    ..Default::default()
                }
            }
        },
        Err(e) => {
            return JavaProbe {
                error: Some(format!("spawn {} failed: {e}", java_path.display())),
                ..Default::default()
            }
        }
    };
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    parse_probe_output(&text)
}

/// Probe one java binary (memoized by path+mtime+size). Never fails: an
/// unusable binary yields `ok: false` + `error`. The test hook
/// (`ResolveHooks::probe`) replaces the spawn entirely.
pub async fn probe_java(engine: &Engine, java_path: &Path) -> EngineResult<JavaProbe> {
    let state = engine.game_resolve_state();
    if let Some(f) = state.hooks().probe {
        return Ok(f(java_path));
    }
    let key = probe_memo_key(java_path);
    if let Some(k) = &key {
        if let Some(hit) = state.probe_memo.lock().unwrap_or_else(|p| p.into_inner()).get(k) {
            return Ok(hit.clone());
        }
    }
    let result = spawn_probe(java_path).await;
    if let Some(k) = key {
        state.probe_memo.lock().unwrap_or_else(|p| p.into_inner()).insert(k, result.clone());
    }
    Ok(result)
}

/// probe + path -> cached JvmInfo shape (always carries `major`).
pub(crate) fn to_jvm_info(java_path: &Path, probe: &JavaProbe, source: JvmSource) -> JavaRuntime {
    let major = if probe.major > 0 {
        probe.major
    } else {
        parse_major(probe.version.as_deref().or(probe.build.as_deref()).unwrap_or(""))
    };
    JavaRuntime {
        path: java_path.to_string_lossy().into_owned(),
        version: probe.version.clone().unwrap_or_default(),
        build: probe.build.clone().unwrap_or_default(),
        vendor: probe.vendor.clone().unwrap_or_default(),
        source,
        major: Some(major),
        extra: Default::default(),
    }
}

fn cache_jvm(engine: &Engine, info: JavaRuntime) -> JavaRuntime {
    engine.config().set_jvm(Some(&info));
    info
}

/// `version || build` the way the JS `??` chain read a JvmInfo record.
pub(crate) fn version_or_build(j: &JavaRuntime) -> &str {
    if !j.version.is_empty() {
        &j.version
    } else {
        &j.build
    }
}

/// Cached `config.jvm` when its file still exists; `major` backfilled for
/// records written before JvmInfo carried it.
fn cached_jvm(engine: &Engine) -> Option<JavaRuntime> {
    let mut info = engine.config().jvm()?;
    if info.path.is_empty() || !Path::new(&info.path).exists() {
        return None;
    }
    if info.major.unwrap_or(0) == 0 {
        info.major = Some(parse_major(version_or_build(&info)));
    }
    Some(info)
}

/// Full discovery (see module docs). `NO_JDK: …` when nothing works.
pub async fn discover_jvm(engine: &Engine) -> EngineResult<JavaRuntime> {
    let override_path = engine.config().jdk_path_override();
    let mut warnings: Vec<String> = vec![];

    if let Some(p) = &override_path {
        let pp = Path::new(p);
        let probe = if pp.exists() {
            probe_java(engine, pp).await?
        } else {
            JavaProbe { error: Some(format!("not found: {p}")), ..Default::default() }
        };
        if probe.ok && probe.major >= MIN_MAJOR {
            return Ok(cache_jvm(engine, to_jvm_info(pp, &probe, JvmSource::Path)));
        }
        let w = format!(
            "jdk_path_override {p} unusable ({}); falling through to discovery",
            probe.error.as_deref().unwrap_or("undefined")
        );
        tracing::warn!("[jvm] {w}");
        warnings.push(w);
    }

    if override_path.is_none() {
        if let Some(c) = cached_jvm(engine) {
            return Ok(c);
        }
    }

    if let Some(fc) = fast_client_java(engine).filter(|p| p.exists()) {
        let probe = probe_java(engine, &fc).await?;
        if probe.ok && probe.major >= MIN_MAJOR {
            return Ok(cache_jvm(engine, to_jvm_info(&fc, &probe, JvmSource::Fastclient)));
        }
        warnings.push(format!("FastClient JDK unusable ({})", probe.error.as_deref().unwrap_or("undefined")));
    }

    if let Some(pj) = path_java(engine) {
        let probe = probe_java(engine, &pj).await?;
        if probe.ok && probe.major >= MIN_MAJOR {
            return Ok(cache_jvm(engine, to_jvm_info(&pj, &probe, JvmSource::Path)));
        }
        warnings.push(format!(
            "PATH java {} unusable ({})",
            pj.display(),
            probe.error.as_deref().unwrap_or("undefined")
        ));
    }

    match download_adoptium(engine).await {
        Ok(info) => return Ok(cache_jvm(engine, info)),
        Err(e) => warnings.push(format!("Adoptium download failed: {e}")),
    }

    Err(EngineError::other(format!(
        "NO_JDK: no JDK {MIN_MAJOR}+ found (override, FastClient, PATH, Adoptium). {}",
        warnings.join(" | ")
    )))
}

/// Cached-if-possible primary JVM; with `override_path` (per-instance JDK
/// override) probe exactly that java — any parseable JDK, never falls through.
pub async fn get_jvm_info(engine: &Engine, override_path: Option<&str>) -> EngineResult<JavaRuntime> {
    if let Some(o) = override_path.filter(|s| !s.is_empty()) {
        let p = Path::new(o);
        if !p.exists() {
            return Err(EngineError::other(format!("jdk_path_override not found: {o}")));
        }
        let probe = probe_java(engine, p).await?;
        if !probe.ok {
            return Err(EngineError::other(format!(
                "jdk_path_override unusable: {}",
                probe.error.as_deref().unwrap_or("undefined")
            )));
        }
        return Ok(cache_jvm(engine, to_jvm_info(p, &probe, JvmSource::Path)));
    }
    if let Some(c) = cached_jvm(engine) {
        return Ok(c);
    }
    discover_jvm(engine).await
}

/// Latest Temurin 25 via the shared Adoptium pipeline (source 'downloaded').
pub async fn download_adoptium(engine: &Engine) -> EngineResult<JavaRuntime> {
    crate::runtimes::download_temurin(engine, 25).await
}

/// `jcmd` next to a java binary (AOT training uses it).
pub fn jcmd_path(java_path: &Path) -> PathBuf {
    let name = if cfg!(windows) { "jcmd.exe" } else { "jcmd" };
    java_path.parent().map(|p| p.join(name)).unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    #[test]
    fn parse_major_handles_modern_and_legacy() {
        assert_eq!(parse_major("25.0.4"), 25);
        assert_eq!(parse_major("21.0.1"), 21);
        assert_eq!(parse_major("1.8.0_392"), 8);
        assert_eq!(parse_major("17.0.9"), 17);
        assert_eq!(parse_major("21"), 21);
        assert_eq!(parse_major("garbage"), 0);
        assert_eq!(parse_major(""), 0);
        assert_eq!(parse_major("21-ea"), 0);
    }

    #[test]
    fn parse_probe_output_reads_property_block() {
        let out = "\nProperty settings:\n    java.runtime.version = 25.0.4+7-LTS\r\n    java.vendor = Eclipse Adoptium\n    java.version = 25.0.4\n\nopenjdk version \"25.0.4\" 2025-07-15 LTS\n";
        let p = parse_probe_output(out);
        assert!(p.ok);
        assert_eq!(p.major, 25);
        assert_eq!(p.version.as_deref(), Some("25.0.4"));
        assert_eq!(p.build.as_deref(), Some("25.0.4+7-LTS"));
        assert_eq!(p.vendor.as_deref(), Some("Eclipse Adoptium"));
        assert_eq!(p.error, None);
    }

    #[test]
    fn parse_probe_output_banner_fallback_and_garbage() {
        let p =
            parse_probe_output("openjdk version \"1.8.0_392\"\nOpenJDK Runtime Environment (build 1.8.0_392-b08)\n");
        assert!(p.ok);
        assert_eq!(p.major, 8);
        assert_eq!(p.build.as_deref(), Some("1.8.0_392-b08"));
        let g = parse_probe_output("bash: nope");
        assert!(!g.ok);
        assert!(g.error.unwrap().starts_with("unparseable java output: bash: nope"));
    }

    #[test]
    fn jcmd_next_to_java() {
        let j = PathBuf::from("/jdk/bin").join(crate::paths::java_bin_name());
        assert_eq!(jcmd_path(&j), PathBuf::from("/jdk/bin").join(if cfg!(windows) { "jcmd.exe" } else { "jcmd" }));
    }

    // linux-compat.test.mjs: fastClientJava is Windows-only.
    #[cfg(not(windows))]
    #[test]
    fn fast_client_java_is_none_off_windows() {
        let (_d, e) = temp_engine();
        assert_eq!(fast_client_java(&e), None);
    }

    #[test]
    fn fastclient_java_layout_under_appdata() {
        // The %APPDATA% layout the Windows build probes (pure path shape).
        if let Some(p) = fastclient_java_under_appdata("java.exe") {
            assert!(p.ends_with(Path::new("FastClient/runtimes/java-25/jdk-25.0.4+7/bin/java.exe")));
        }
    }

    #[tokio::test]
    async fn missing_binary_probe_is_not_ok_and_not_memoized() {
        let (d, e) = temp_engine();
        let missing = d.path().join("__no_such__").join("java");
        assert_eq!(probe_memo_key(&missing), None);
        let r = probe_java(&e, &missing).await.unwrap();
        assert!(!r.ok);
        assert!(r.error.is_some());
        assert_eq!(e.game_resolve_state().probe_memo_size(), 0);
    }

    #[test]
    fn find_on_path_only_returns_existing_executables() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let pv = std::env::join_paths([bin.clone()]).unwrap();
        assert_eq!(find_on_path("java", Some(&pv)), None);
        let java = bin.join(crate::paths::java_bin_name());
        std::fs::write(&java, "x").unwrap();
        #[cfg(unix)]
        {
            assert_eq!(find_on_path("java", Some(&pv)), None, "no exec bit -> not found (like which)");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(find_on_path("java", Some(&pv)), Some(java));
    }

    /// A fake `java` that prints a property block (unix only: shell script).
    #[cfg(unix)]
    pub(crate) fn fake_java(dir: &Path, version: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let p = bin.join("java");
        let major = version.split('.').next().unwrap();
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\necho 'Property settings:' >&2\necho '    java.runtime.version = {version}+7' >&2\necho '    java.vendor = Eclipse Adoptium' >&2\necho '    java.version = {version}' >&2\necho 'openjdk version \"{version}\" {major}' >&2\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_memo_hits_for_same_identity() {
        let (d, e) = temp_engine();
        let java = fake_java(&d.path().join("jdk25"), "25.0.4");
        let st = std::fs::metadata(&java).unwrap();
        let key = probe_memo_key(&java).unwrap();
        assert_eq!(key, format!("{}:{}:{}", java.display(), node_mtime_ms(&st), st.len()));
        assert!(key.starts_with(&format!("{}:", java.display())));

        let a = probe_java(&e, &java).await.unwrap();
        assert!(a.ok, "{:?}", a.error);
        assert_eq!(a.major, 25);
        assert_eq!(a.build.as_deref(), Some("25.0.4+7"));
        assert_eq!(e.game_resolve_state().probe_memo_size(), 1);
        let b = probe_java(&e, &java).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(e.game_resolve_state().probe_memo_size(), 1);
        // A different (missing) path does not add an entry; the real one still hits.
        probe_java(&e, &d.path().join("nope/java")).await.unwrap();
        assert_eq!(e.game_resolve_state().probe_memo_size(), 1);
        e.game_resolve_state().clear_probe_memo();
        assert_eq!(e.game_resolve_state().probe_memo_size(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn override_jvm_info_carries_major_and_cached_record_is_backfilled() {
        let (d, e) = temp_engine();
        e.game_resolve_state().isolate();
        let java = fake_java(&d.path().join("jdk25"), "25.0.4");
        let info = get_jvm_info(&e, Some(&java.to_string_lossy())).await.unwrap();
        assert_eq!(info.path, java.to_string_lossy());
        assert_eq!(info.major, Some(25));
        assert_eq!(info.source, JvmSource::Path);
        // memo reused (override path probes once)
        assert_eq!(e.game_resolve_state().probe_memo_size(), 1);

        // Legacy config.jvm record without major -> backfilled from version.
        e.config().set(
            "jvm",
            serde_json::json!({ "path": info.path, "version": info.version, "build": info.build, "vendor": info.vendor, "source": "path" }),
        );
        assert!(e.config().get("jvm").unwrap().get("major").is_none());
        let again = get_jvm_info(&e, None).await.unwrap();
        assert_eq!(again.major, Some(25));
    }

    #[tokio::test]
    async fn override_errors() {
        let (d, e) = temp_engine();
        let missing = d.path().join("nope").join("java");
        let err = get_jvm_info(&e, Some(&missing.to_string_lossy())).await.unwrap_err();
        assert!(err.to_string().starts_with("jdk_path_override not found: "));
        let junk = d.path().join("junk");
        std::fs::write(&junk, "x").unwrap();
        e.game_resolve_state().set_probe_override(Some(std::sync::Arc::new(|_: &Path| JavaProbe {
            error: Some("boom".into()),
            ..Default::default()
        })));
        let err = get_jvm_info(&e, Some(&junk.to_string_lossy())).await.unwrap_err();
        assert_eq!(err.to_string(), "jdk_path_override unusable: boom");
    }

    #[tokio::test]
    async fn discovery_prefers_override_then_path_and_caches_winner() {
        let (d, e) = temp_engine();
        e.game_resolve_state().isolate();
        let p25 = d.path().join("jdk25").join("java");
        let p17 = d.path().join("jdk17").join("java");
        for p in [&p25, &p17] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let p25c = p25.clone();
        e.game_resolve_state().set_probe_override(Some(std::sync::Arc::new(move |p: &Path| {
            let major = if p == p25c { 25 } else { 17 };
            JavaProbe {
                ok: true,
                major,
                version: Some(format!("{major}.0.1")),
                build: Some(format!("{major}.0.1+1")),
                vendor: Some("T".into()),
                error: None,
            }
        })));
        // Override below the floor (17) -> falls through to PATH (25).
        e.config().set("jdk_path_override", serde_json::Value::String(p17.to_string_lossy().into()));
        e.game_resolve_state().update_hooks(|h| h.path_java = Some(Some(p25.clone())));
        let info = discover_jvm(&e).await.unwrap();
        assert_eq!(info.path, p25.to_string_lossy());
        assert_eq!(info.major, Some(25));
        assert_eq!(e.config().jvm().unwrap().path, p25.to_string_lossy());
        // Override >= 21 wins outright.
        e.config().set("jdk_path_override", serde_json::Value::String(p25.to_string_lossy().into()));
        e.game_resolve_state().update_hooks(|h| h.path_java = Some(None));
        assert_eq!(discover_jvm(&e).await.unwrap().source, JvmSource::Path);
    }
}
