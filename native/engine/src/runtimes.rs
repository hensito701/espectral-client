//! Port of `src/engine/runtimes.mjs` — Java tier selection + Temurin
//! provisioning (`<data>/runtimes/jdk-<major>/`, `info.json`).
//!
//! **Owner: game-resolve.**
//!
//! A candidate JDK fits a tier when its major is `>=` the required major AND
//! within the tier's ceiling ([`max_major_for`]: 8→8, 16→16, 17→≤21,
//! 21/25→any), so a JDK 25 is never used for the 8/16/17 tiers.
//!
//! Resolution order per tier ([`java_for_version`]), all verified by probe:
//!   1. `config.jdk_path_override` (must fit the tier; never cached)
//!   2. a same-tier JDK already cached in `config.jvm`
//!   3. FastClient bundled Temurin 25 (never cached)
//!   4. system PATH `java` (never cached)
//!   5. already-downloaded runtime under `<data>/runtimes/jdk-<major>/`
//!   6. Adoptium download (Temurin LTS 8/16/17/21/25)
//!
//! Archives: zip (Windows/macOS) via the `zip` crate, tar.gz (Linux) via
//! `tar` + `flate2` (the JS shelled out to `tar -xzf`). Downloads go through
//! the engine HTTP client (retries, `.part` resume).
//!
//! CONTRACT (called by game-launch): java_for_version, java_for_major_exact.

use std::path::{Path, PathBuf};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult, HttpErrorKind};
use crate::jvm::{parse_major, path_java, probe_java, version_or_build};
use crate::types::{JavaProbe, JavaRuntime, JvmSource};

/// Tiers, newest first (JS runtimes.SUPPORTED_MAJORS).
pub const SUPPORTED_MAJORS: [u32; 5] = [25, 21, 17, 16, 8];

/// Adoptium API base (`/binary/latest/<major>/ga/<os>/<arch>/jdk/hotspot/normal/eclipse`).
pub const ADOPTIUM_API: &str = "https://api.adoptium.net/v3";

/// Minecraft version id -> Java major (the same cuts the UI uses).
pub fn java_major_for_version_id(id: &str) -> u32 {
    // Snapshots: 24w14a, 26w01a …
    let b = id.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b'w' {
        let yy: u32 = id[..2].parse().unwrap_or(0);
        if yy >= 26 {
            return 25;
        }
        return if yy >= 24 { 21 } else { 17 };
    }
    // Pre-releases / RCs: '1.20.5-pre1' -> '1.20.5'.
    let pre = regex::Regex::new(r"^(\d+\.\d+(?:\.\d+)?)").expect("regex");
    if let Some(c) = pre.captures(id) {
        if &c[1] != id {
            return java_major_for_version_id(&c[1]);
        }
    }
    let rel = regex::Regex::new(r"^1\.(\d+)(?:\.(\d+))?").expect("regex");
    if let Some(c) = rel.captures(id) {
        let minor: u32 = c[1].parse().unwrap_or(0);
        let patch: u32 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        return match minor {
            m if m >= 21 => 21,
            20 => {
                if patch >= 5 {
                    21
                } else {
                    17
                }
            }
            18 | 19 => 17,
            17 => 16,
            _ => 8,
        };
    }
    let modern = regex::Regex::new(r"^(\d+)\.").expect("regex");
    if let Some(c) = modern.captures(id) {
        let major: u32 = c[1].parse().unwrap_or(0);
        return if major >= 25 { 25 } else { 21 };
    }
    17
}

/// `<data>/runtimes/jdk-<major>/bin/java(.exe)`.
pub fn runtime_java_path(engine: &Engine, major: u32) -> PathBuf {
    engine.paths().runtime_java(major)
}

/// FastClient's bundled Temurin 25 (runtimes.mjs flavour — probed on every
/// OS: `<APPDATA | USERPROFILE\AppData\Roaming>\FastClient\…\bin\java(.exe)`).
/// None when neither variable is set (the JS built a cwd-relative path then).
pub fn fast_client_java(engine: &Engine) -> Option<PathBuf> {
    if let Some(o) = engine.game_resolve_state().hooks().fastclient_java {
        return o;
    }
    crate::jvm::fastclient_java_under_appdata(crate::paths::java_bin_name())
}

/// config.jvm when its file exists (no major backfill — runtimes.mjs flavour).
fn cached_jvm(engine: &Engine) -> Option<JavaRuntime> {
    let info = engine.config().jvm()?;
    (!info.path.is_empty() && Path::new(&info.path).exists()).then_some(info)
}

/// `<data>/runtimes/jdk-<major>/info.json` when its java still exists.
pub fn cached_major(engine: &Engine, major: u32) -> Option<JavaRuntime> {
    let f = engine.paths().runtime_info(major);
    let info: JavaRuntime = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
    (!info.path.is_empty() && Path::new(&info.path).exists()).then_some(info)
}

/// Persist `info.json` for `major`; skips the write when the file already
/// deep-equals `info` (keeps its mtime). Best-effort; returns `info`.
pub fn cache_major(engine: &Engine, major: u32, info: JavaRuntime) -> JavaRuntime {
    let f = engine.paths().runtime_info(major);
    let value = match serde_json::to_value(&info) {
        Ok(v) => v,
        Err(_) => return info,
    };
    if let Ok(existing) = std::fs::read_to_string(&f) {
        if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&existing) {
            if ev == value {
                return info;
            }
        }
    }
    if let Err(e) = crate::paths::write_json_atomic(&f, &value) {
        tracing::debug!("[runtimes] could not cache {}: {e}", f.display());
    }
    info
}

/// Adoptium os/arch tokens for the current platform.
fn adoptium_os_arch() -> (&'static str, &'static str) {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    };
    let arch = match crate::platform::node_arch() {
        "arm64" => "aarch64",
        "x64" => "x64",
        other => other,
    };
    (os, arch)
}

/// Download + extract a Temurin JDK for `major` into
/// `<data>/runtimes/jdk-<major>/`, probe it and persist info.json.
/// Serialized per major (JS `withMajorLock`).
pub async fn download_temurin(engine: &Engine, major: u32) -> EngineResult<JavaRuntime> {
    let _guard = engine.game_resolve_state().major_locks.lock(&major).await;
    download_temurin_inner(engine, major).await
}

async fn download_temurin_inner(engine: &Engine, major: u32) -> EngineResult<JavaRuntime> {
    let (os, arch) = adoptium_os_arch();
    let api = engine.game_resolve_state().endpoints().adoptium_api;
    let url = format!("{}/binary/latest/{major}/ga/{os}/{arch}/jdk/hotspot/normal/eclipse", api.trim_end_matches('/'));

    let dir = engine.paths().runtime_dir(major);
    tokio::fs::create_dir_all(&dir).await?;
    let is_zip = !cfg!(target_os = "linux");
    let archive = dir.join(format!("jdk-{major}.{}", if is_zip { "zip" } else { "tar.gz" }));
    let _ = tokio::fs::remove_file(&archive).await;

    if let Err(e) = engine.http().download_file(&url, &archive, &Default::default()).await {
        return Err(match &e {
            EngineError::Http(h) => match h.kind {
                HttpErrorKind::Status(s) => {
                    let reason = reqwest::StatusCode::from_u16(s).ok().and_then(|c| c.canonical_reason()).unwrap_or("");
                    EngineError::other(format!("Adoptium {s} {reason}").trim_end().to_string())
                }
                _ => e,
            },
            _ => e,
        });
    }

    let (a, d) = (archive.clone(), dir.clone());
    tokio::task::spawn_blocking(move || if is_zip { extract_zip(&a, &d) } else { extract_tar_gz(&a, &d) })
        .await
        .map_err(|e| EngineError::other(format!("JDK extraction task failed: {e}")))??;
    let _ = tokio::fs::remove_file(&archive).await;

    // Adoptium archives nest the JDK under a top-level folder (jdk8u412-b08,
    // jdk-21.0.7+6, and for major 8 exactly "jdk-8"): hoist it so the
    // canonical <dir>/bin/java(.exe) holds.
    let java_path = runtime_java_path(engine, major);
    if !java_path.exists() {
        let d = dir.clone();
        tokio::task::spawn_blocking(move || hoist_nested_runtime(&d))
            .await
            .map_err(|e| EngineError::other(format!("JDK hoist task failed: {e}")))??;
    }
    if !java_path.exists() {
        return Err(EngineError::other(format!(
            "Adoptium {major} extracted but no bin/{} found under {}",
            crate::paths::java_bin_name(),
            dir.display()
        )));
    }
    let probe = probe_java(engine, &java_path).await?;
    if !probe.ok {
        return Err(EngineError::other(format!(
            "downloaded JDK {major} failed probe: {}",
            probe.error.as_deref().unwrap_or("undefined")
        )));
    }
    Ok(cache_major(engine, major, runtime_from_probe(&java_path, &probe, JvmSource::Downloaded, major)))
}

fn runtime_from_probe(path: &Path, probe: &JavaProbe, source: JvmSource, major: u32) -> JavaRuntime {
    JavaRuntime {
        path: path.to_string_lossy().into_owned(),
        version: probe.version.clone().unwrap_or_default(),
        build: probe.build.clone().unwrap_or_default(),
        vendor: probe.vendor.clone().unwrap_or_default(),
        source,
        major: Some(major),
        extra: Default::default(),
    }
}

fn hoist_nested_runtime(dir: &Path) -> std::io::Result<()> {
    let bin = crate::paths::java_bin_name();
    let nested = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .find(|p| p.join("bin").join(bin).exists());
    if let Some(inner) = nested {
        for entry in std::fs::read_dir(&inner)? {
            let entry = entry?;
            let target = dir.join(entry.file_name());
            if target.is_dir() {
                let _ = std::fs::remove_dir_all(&target);
            }
            std::fs::rename(entry.path(), target)?;
        }
        let _ = std::fs::remove_dir_all(&inner);
    }
    Ok(())
}

/// Extract a zip archive into `dir` (entries outside `dir` are skipped).
pub(crate) fn extract_zip(archive: &Path, dir: &Path) -> EngineResult<()> {
    let f = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| EngineError::other(format!("zip extract failed: {e}")))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| EngineError::other(format!("zip extract failed: {e}")))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut w)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777));
        }
    }
    Ok(())
}

/// Extract a .tar.gz into `dir` (JS `tar -xzf <archive> -C <dir>`).
pub(crate) fn extract_tar_gz(archive: &Path, dir: &Path) -> EngineResult<()> {
    let f = std::fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(f)));
    ar.set_preserve_permissions(true);
    ar.set_overwrite(true);
    ar.unpack(dir).map_err(|e| EngineError::other(format!("tar extract failed: {e}")))
}

/// Highest Java major a tier accepts (u32::MAX = unbounded).
pub fn max_major_for(need: u32) -> u32 {
    if need <= 8 {
        8
    } else if need == 16 {
        16
    } else if need == 17 {
        21
    } else {
        u32::MAX
    }
}

/// JDK whose major is in `[need, max_major_for(need)]` (see module docs).
pub async fn java_for_version(engine: &Engine, major: u32) -> EngineResult<JavaRuntime> {
    let need = if major == 0 { 17 } else { major };
    let max_major = max_major_for(need);
    let fits = |p: &JavaProbe| p.ok && p.major >= need && p.major <= max_major;

    // 1. Explicit override — must fit; never persisted into jdk-<need>/info.json.
    if let Some(p) = engine.config().jdk_path_override() {
        let pp = Path::new(&p);
        let probe = if pp.exists() {
            probe_java(engine, pp).await?
        } else {
            JavaProbe { error: Some(format!("not found: {p}")), ..Default::default() }
        };
        if fits(&probe) {
            return Ok(runtime_from_probe(pp, &probe, JvmSource::Path, probe.major));
        }
        let why = probe.error.clone().unwrap_or_else(|| format!("probed major {} outside tier", probe.major));
        tracing::warn!("[runtimes] jdk_path_override {p} unusable for JDK {need}+ ({why}); falling through");
    }

    // 2. Already-discovered same-tier JDK (config.jvm).
    if let Some(cached) = cached_jvm(engine) {
        let cm = parse_major(if !cached.build.is_empty() { &cached.build } else { version_or_build(&cached) });
        if cm >= need && cm <= max_major && cm > 0 {
            return Ok(cache_major(engine, need, JavaRuntime { major: Some(cm), ..cached }));
        }
    }

    // 3. FastClient bundled Temurin 25 — returned directly (no poisoning).
    if let Some(fc) = fast_client_java(engine).filter(|p| p.exists()) {
        let probe = probe_java(engine, &fc).await?;
        if fits(&probe) {
            return Ok(runtime_from_probe(&fc, &probe, JvmSource::Fastclient, probe.major));
        }
    }

    // 4. PATH java — same no-poison rule.
    if let Some(pj) = path_java(engine) {
        let probe = probe_java(engine, &pj).await?;
        if fits(&probe) {
            return Ok(runtime_from_probe(&pj, &probe, JvmSource::Path, probe.major));
        }
    }

    // 5. Already-downloaded runtime.
    if let Some(existing) = cached_major(engine, need) {
        return Ok(existing);
    }

    // 6. Adoptium download. (The JS then called `saveConfig({ jvm })`, but
    // that name was never imported in runtimes.mjs, so the throw was
    // swallowed and config.jvm was effectively never written here — parity.)
    download_temurin(engine, need).await
}

/// JDK with EXACTLY `major` (NeoForge needs 21; never upgrades).
/// Order: cached exact-major runtime -> PATH java (exact) -> Adoptium.
pub async fn java_for_major_exact(engine: &Engine, major: u32) -> EngineResult<JavaRuntime> {
    let need = if major == 0 { 21 } else { major };

    if let Some(existing) = cached_major(engine, need) {
        if existing.major == Some(need) {
            return Ok(existing);
        }
    }

    if let Some(pj) = path_java(engine) {
        let probe = probe_java(engine, &pj).await?;
        if probe.ok && probe.major == need {
            return Ok(cache_major(engine, need, runtime_from_probe(&pj, &probe, JvmSource::Path, probe.major)));
        }
    }

    // Deliberately NOT written to config.jvm (the primary JVM cache must not
    // be downgraded to 21 by a NeoForge launch).
    download_temurin(engine, need).await.map_err(|cause| {
        EngineError::other(format!(
            "NO_JDK_{need}: no JDK {need} available (PATH probe mismatch and Adoptium download failed: {cause})"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use std::sync::Arc;

    fn fake_probe(major: u32) -> crate::api::game_resolve::ProbeFn {
        Arc::new(move |_: &Path| JavaProbe {
            ok: true,
            major,
            version: Some(format!("{major}.0.4")),
            build: Some(format!("{major}.0.4+7")),
            vendor: Some("Eclipse Temurin".into()),
            error: None,
        })
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "fake").unwrap();
    }

    fn engine() -> (tempfile::TempDir, Engine) {
        let (d, e) = temp_engine();
        e.game_resolve_state().isolate();
        // Any accidental Adoptium request fails fast on a dead loopback port.
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on("http://127.0.0.1:9"));
        (d, e)
    }

    #[test]
    fn version_id_to_java_major() {
        for (id, want) in [
            ("1.8.9", 8),
            ("1.15.2", 8),
            ("1.16.5", 8),
            ("1.17.1", 16),
            ("1.18.2", 17),
            ("1.19.4", 17),
            ("1.20.4", 17),
            ("1.20.5", 21),
            ("1.20.6", 21),
            ("1.21.1", 21),
            ("1.21.11", 21),
            ("1.20.5-pre1", 21),
            ("1.18-rc1", 17),
            ("23w51b", 17),
            ("24w14a", 21),
            ("26w01a", 25),
            ("26.1", 25),
            ("26.2", 25),
            ("24.1", 21),
            ("weird", 17),
        ] {
            assert_eq!(java_major_for_version_id(id), want, "{id}");
        }
    }

    #[test]
    fn tier_ceilings() {
        assert_eq!(max_major_for(8), 8);
        assert_eq!(max_major_for(16), 16);
        assert_eq!(max_major_for(17), 21);
        assert_eq!(max_major_for(21), u32::MAX);
        assert_eq!(max_major_for(25), u32::MAX);
    }

    // P3.1 regression: override probing 25 for need 21 must not create jdk-21/info.json.
    #[tokio::test]
    async fn override_mismatch_does_not_poison_tier_cache() {
        let (d, e) = engine();
        let fake = d.path().join("fake-jdk25").join("bin").join("java.exe");
        touch(&fake);
        e.config().set("jdk_path_override", fake.to_string_lossy().into());
        e.game_resolve_state().set_probe_override(Some(fake_probe(25)));
        let info = java_for_version(&e, 21).await.unwrap();
        assert_eq!(info.major, Some(25));
        assert_eq!(info.path, fake.to_string_lossy());
        assert_eq!(info.source, JvmSource::Path);
        assert!(!e.paths().runtime_info(21).exists(), "must not create jdk-21/info.json with major 25");
        assert!(!e.paths().runtime_info(25).exists(), "override path must not cache under any major");
    }

    #[tokio::test]
    async fn override_does_not_overwrite_existing_tier_cache() {
        let (d, e) = engine();
        let java21 = d.path().join("real-jdk21").join("bin").join("java.exe");
        touch(&java21);
        let existing = JavaRuntime {
            path: java21.to_string_lossy().into(),
            version: "21.0.1".into(),
            build: "21.0.1+12".into(),
            vendor: "Eclipse Temurin".into(),
            source: JvmSource::Downloaded,
            major: Some(21),
            extra: Default::default(),
        };
        cache_major(&e, 21, existing);
        let before = std::fs::read_to_string(e.paths().runtime_info(21)).unwrap();
        let fake25 = d.path().join("fake-jdk25-2").join("bin").join("java.exe");
        touch(&fake25);
        e.config().set("jdk_path_override", fake25.to_string_lossy().into());
        e.game_resolve_state().set_probe_override(Some(fake_probe(25)));
        let r = java_for_version(&e, 21).await.unwrap();
        assert_eq!(r.major, Some(25));
        assert_eq!(r.path, fake25.to_string_lossy());
        assert_eq!(std::fs::read_to_string(e.paths().runtime_info(21)).unwrap(), before);
    }

    #[tokio::test]
    async fn cache_major_is_idempotent_for_deep_equal_info() {
        let (d, e) = engine();
        let dummy = d.path().join("dummy-jdk21").join("bin").join("java.exe");
        touch(&dummy);
        let info = JavaRuntime {
            path: dummy.to_string_lossy().into(),
            version: "21.0.7".into(),
            build: "21.0.7+6".into(),
            vendor: "Eclipse Temurin".into(),
            source: JvmSource::Downloaded,
            major: Some(21),
            extra: Default::default(),
        };
        let f = e.paths().runtime_info(21);
        cache_major(&e, 21, info.clone());
        let m1 = std::fs::metadata(&f).unwrap().modified().unwrap();
        let c1 = std::fs::read_to_string(&f).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        // Same content with a different key order on disk still deep-equals.
        std::fs::write(
            &f,
            serde_json::to_string(&serde_json::json!({
                "major": 21, "source": "downloaded", "vendor": "Eclipse Temurin",
                "build": "21.0.7+6", "version": "21.0.7", "path": info.path,
            }))
            .unwrap(),
        )
        .unwrap();
        let m_reordered = std::fs::metadata(&f).unwrap().modified().unwrap();
        let c_reordered = std::fs::read_to_string(&f).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        cache_major(&e, 21, info.clone());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), c_reordered, "deep-equal -> write skipped");
        assert_eq!(std::fs::metadata(&f).unwrap().modified().unwrap(), m_reordered);
        assert!(m_reordered >= m1);
        let _ = c1;
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        cache_major(&e, 21, JavaRuntime { version: "21.0.8".into(), build: "21.0.8+9".into(), ..info });
        let c3 = std::fs::read_to_string(&f).unwrap();
        assert_ne!(c3, c_reordered);
        assert!(std::fs::metadata(&f).unwrap().modified().unwrap() > m_reordered);
        assert_eq!(cached_major(&e, 21).unwrap().version, "21.0.8");
    }

    #[tokio::test]
    async fn fastclient_and_path_mismatch_do_not_poison() {
        let (d, e) = engine();
        let fc = d.path().join("appdata-fake").join("FastClient").join("bin").join("java");
        touch(&fc);
        e.game_resolve_state().update_hooks(|h| h.fastclient_java = Some(Some(fc.clone())));
        e.game_resolve_state().set_probe_override(Some(fake_probe(25)));
        let info = java_for_version(&e, 21).await.unwrap();
        assert_eq!(info.source, JvmSource::Fastclient);
        assert_eq!(info.major, Some(25));
        assert_eq!(info.path, fc.to_string_lossy());
        assert!(!e.paths().runtime_info(21).exists());

        std::fs::remove_file(&fc).unwrap();
        let pj = d.path().join("fake-path-bin").join("java");
        touch(&pj);
        e.game_resolve_state().update_hooks(|h| h.path_java = Some(Some(pj.clone())));
        let info2 = java_for_version(&e, 21).await.unwrap();
        assert_eq!(info2.source, JvmSource::Path);
        assert_eq!(info2.major, Some(25));
        assert!(!e.paths().runtime_info(21).exists());
    }

    #[tokio::test]
    async fn modern_jdk_never_serves_legacy_tiers() {
        let (d, e) = engine();
        let pj = d.path().join("bin").join("java");
        touch(&pj);
        e.game_resolve_state().update_hooks(|h| h.path_java = Some(Some(pj.clone())));
        e.game_resolve_state().set_probe_override(Some(fake_probe(25)));
        // Tier 17 refuses 25, falls to the (empty) runtime cache, then Adoptium (dead port).
        assert!(java_for_version(&e, 17).await.is_err());
        // A downloaded jdk-17 is used.
        let j17 = e.paths().runtime_java(17);
        touch(&j17);
        cache_major(
            &e,
            17,
            JavaRuntime {
                path: j17.to_string_lossy().into(),
                version: "17.0.9".into(),
                source: JvmSource::Downloaded,
                major: Some(17),
                ..Default::default()
            },
        );
        assert_eq!(java_for_version(&e, 17).await.unwrap().major, Some(17));
        // config.jvm of the same tier is reused and cached for the tier.
        let j21 = d.path().join("j21").join("java");
        touch(&j21);
        e.config().set_jvm(Some(&JavaRuntime {
            path: j21.to_string_lossy().into(),
            version: "21.0.2".into(),
            build: "21.0.2+13".into(),
            ..Default::default()
        }));
        let r = java_for_version(&e, 17).await.unwrap();
        assert_eq!((r.path.as_str(), r.major), (&*j21.to_string_lossy(), Some(21)));
    }

    #[tokio::test]
    async fn exact_major_rejects_stale_cache_and_wraps_download_failure() {
        let (d, e) = engine();
        let j = e.paths().runtime_java(21);
        touch(&j);
        // Stale tier cache pointing at a 25 must not satisfy an exact-21 request.
        cache_major(&e, 21, JavaRuntime { path: j.to_string_lossy().into(), major: Some(25), ..Default::default() });
        let pj = d.path().join("bin").join("java");
        touch(&pj);
        e.game_resolve_state().update_hooks(|h| h.path_java = Some(Some(pj.clone())));
        e.game_resolve_state().set_probe_override(Some(fake_probe(25)));
        let err = java_for_major_exact(&e, 21).await.unwrap_err();
        assert!(err.to_string().starts_with("NO_JDK_21: no JDK 21 available"), "{err}");
        // PATH java with exactly 21 is accepted and cached.
        e.game_resolve_state().set_probe_override(Some(fake_probe(21)));
        let r = java_for_major_exact(&e, 21).await.unwrap();
        assert_eq!((r.source, r.major), (JvmSource::Path, Some(21)));
        assert_eq!(cached_major(&e, 21).unwrap().path, pj.to_string_lossy());
    }

    fn tar_gz_jdk(top: &str) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        let mut add = |path: &str, data: &[u8], mode: u32| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            b.append_data(&mut h, path, data).unwrap();
        };
        add(&format!("{top}/bin/{}", crate::paths::java_bin_name()), b"#!/bin/sh\n", 0o755);
        add(&format!("{top}/release"), b"JAVA_VERSION=\"17.0.9\"\n", 0o644);
        b.into_inner().unwrap().finish().unwrap()
    }

    fn zip_jdk(top: &str) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        w.start_file(format!("{top}/bin/{}", crate::paths::java_bin_name()), o).unwrap();
        w.write_all(b"MZ").unwrap();
        w.start_file(format!("{top}/release"), o).unwrap();
        w.write_all(b"JAVA_VERSION=\"17.0.9\"\n").unwrap();
        w.finish().unwrap().into_inner()
    }

    #[tokio::test]
    async fn download_temurin_extracts_hoists_probes_and_caches() {
        let (_d, e) = engine();
        // Major 8's top-level folder is literally "jdk-8" — it must still be hoisted.
        let body = if cfg!(target_os = "linux") { tar_gz_jdk("jdk-8") } else { zip_jdk("jdk-8") };
        let srv = MockServer::start(move |_| MockResponse::bytes(body.clone())).await;
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on(&srv.url("")));
        e.game_resolve_state().set_probe_override(Some(fake_probe(8)));
        let info = download_temurin(&e, 8).await.unwrap();
        assert_eq!(info.source, JvmSource::Downloaded);
        assert_eq!(info.major, Some(8));
        assert_eq!(PathBuf::from(&info.path), e.paths().runtime_java(8));
        assert!(e.paths().runtime_java(8).exists());
        assert!(e.paths().runtime_dir(8).join("release").exists());
        assert!(!e.paths().runtime_dir(8).join("jdk-8").exists());
        assert_eq!(cached_major(&e, 8).unwrap(), info);
        let (os, arch) = adoptium_os_arch();
        assert_eq!(
            srv.requests()[0].path,
            format!("/adoptium/v3/binary/latest/8/ga/{os}/{arch}/jdk/hotspot/normal/eclipse")
        );
        // No archive left behind.
        let leftovers: Vec<_> = std::fs::read_dir(e.paths().runtime_dir(8))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|x| x.file_name().to_string_lossy().starts_with("jdk-8."))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[tokio::test]
    async fn download_temurin_http_error_message() {
        let (_d, e) = engine();
        let srv = MockServer::start(|_| MockResponse::status(404)).await;
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on(&srv.url("")));
        let err = download_temurin(&e, 21).await.unwrap_err();
        assert_eq!(err.to_string(), "Adoptium 404 Not Found");
    }
}
