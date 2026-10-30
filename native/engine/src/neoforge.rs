//! Port of `src/engine/neoforge.mjs` — NeoForge loader: installer download,
//! profile extraction (crate::ziputil), official installer run
//! (`java -jar <installer> --installClient <data>`), library downloads.
//!
//! **Owner: game-resolve.**
//!
//! The installer jar's `version.json` IS the launcher profile; it is persisted
//! as `<data>/versions/neoforge-<v>/neoforge-<v>.json` (the shape
//! `resolver::read_local_version_json` reads). FML 4's production launch also
//! needs artifacts the official installer GENERATES locally (srg / extra /
//! client jars) — the installer is run once per version to produce them.
//!
//! Progress: `mod-progress` events, kind `neoforge-loader` (installer) and
//! `neoforge-library` (per library).
//!
//! CONTRACT (called by game-launch / content): neoforge_version_id,
//! ensure_neoforge_loader, launch_artifacts_ready, latest_neoforge_for.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::download::DownloadItem;
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::events::{EngineEvent, ProgressChannel, ProgressEvent};
use crate::resolver::join_rel;
use crate::types::{InstanceMeta, VersionJson};

pub const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";

/// Version id used under `<data>/versions`: `neoforge-<v>`.
pub fn neoforge_version_id(v: &str) -> String {
    format!("neoforge-{v}")
}

/// Installer jar URL for a NeoForge version (e.g. '21.1.242').
pub fn installer_url(engine: &Engine, version: &str) -> String {
    let base = engine.game_resolve_state().endpoints().neoforge_maven;
    format!("{}/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar", base.trim_end_matches('/'))
}

/// NeoForge version prefix for a Minecraft version: `1.A[.B]` -> `A.B.`
/// (1.21.1 -> 21.1., 1.21 -> 21.0.), modern `X.Y[.Z]` -> `X.Y.Z.`
/// (26.2 -> 26.2.0.). None for anything else (snapshots).
pub fn neoforge_prefix_for(mc_version: &str) -> Option<String> {
    let classic = regex::Regex::new(r"^1\.(\d+)(?:\.(\d+))?$").expect("regex");
    if let Some(c) = classic.captures(mc_version) {
        return Some(format!("{}.{}.", &c[1], c.get(2).map(|m| m.as_str()).unwrap_or("0")));
    }
    let modern = regex::Regex::new(r"^(\d{2})\.(\d+)(?:\.(\d+))?$").expect("regex");
    if let Some(c) = modern.captures(mc_version) {
        return Some(format!("{}.{}.{}.", &c[1], &c[2], c.get(3).map(|m| m.as_str()).unwrap_or("0")));
    }
    None
}

/// Numeric sort key of a NeoForge version (`split(/[.-]/)`, `Number(p) || 0`).
fn version_key(v: &str) -> Vec<f64> {
    v.split(['.', '-']).map(|p| p.trim().parse::<f64>().ok().filter(|n| n.is_finite()).unwrap_or(0.0)).collect()
}

fn cmp_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let (x, y) = (version_key(a), version_key(b));
    for i in 0..x.len().max(y.len()) {
        let (xa, ya) = (x.get(i).copied().unwrap_or(0.0), y.get(i).copied().unwrap_or(0.0));
        if xa != ya {
            return xa.partial_cmp(&ya).unwrap_or(std::cmp::Ordering::Equal);
        }
    }
    std::cmp::Ordering::Equal
}

/// Newest stable version in a maven-metadata.xml for `prefix`.
pub fn latest_from_metadata(xml: &str, prefix: &str) -> Option<String> {
    let re = regex::Regex::new(r"<version>([^<]+)</version>").expect("regex");
    let unstable = regex::Regex::new(r"(?i)beta|alpha|rc").expect("regex");
    let mut versions: Vec<String> = re
        .captures_iter(xml)
        .map(|c| c[1].to_string())
        .filter(|v| v.starts_with(prefix) && !unstable.is_match(v))
        .collect();
    versions.sort_by(|a, b| cmp_versions(a, b));
    versions.pop()
}

/// Newest stable NeoForge build for a Minecraft version, None when there is none.
pub async fn latest_neoforge_for(engine: &Engine, mc_version: &str) -> EngineResult<Option<String>> {
    let Some(prefix) = neoforge_prefix_for(mc_version) else { return Ok(None) };
    let base = engine.game_resolve_state().endpoints().neoforge_maven;
    let url = format!("{}/net/neoforged/neoforge/maven-metadata.xml", base.trim_end_matches('/'));
    let xml = engine.http().fetch_text(&url, &Default::default()).await?;
    Ok(latest_from_metadata(&xml, &prefix))
}

/// `<data>/versions/neoforge-<v>/neoforge-<v>.json`.
pub fn neoforge_json_path(engine: &Engine, v: &str) -> PathBuf {
    engine.paths().version_json(&neoforge_version_id(v))
}

/// `'1.21.1-20240808.144430'` from the profile's inheritsFrom + `--fml.neoFormVersion`.
pub fn mc_and_neoform_version(profile: &Value) -> Option<String> {
    let mc = match profile.get("inheritsFrom") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    if mc.is_empty() {
        return None;
    }
    let mut flat: Vec<&str> = vec![];
    if let Some(Value::Array(game)) = profile.pointer("/arguments/game") {
        for entry in game {
            match entry {
                Value::String(s) => flat.push(s),
                Value::Object(o) => {
                    if let Some(Value::Array(vals)) = o.get("value") {
                        flat.extend(vals.iter().filter_map(Value::as_str));
                    }
                }
                _ => {}
            }
        }
    }
    flat.windows(2).find(|w| w[0] == "--fml.neoFormVersion").map(|w| format!("{mc}-{}", w[1]))
}

/// `<data>/libraries/net/minecraft/client/<mcAndNeoForm>/`.
pub fn client_artifacts_dir(engine: &Engine, mc_and_neoform: &str) -> PathBuf {
    engine.paths().libraries_dir().join("net").join("minecraft").join("client").join(mc_and_neoform)
}

/// `<data>/libraries/net/neoforged/neoforge/<v>/neoforge-<v>[-classifier].jar`.
pub fn neoforge_jar_path(engine: &Engine, v: &str, classifier: &str) -> PathBuf {
    let file =
        if classifier.is_empty() { format!("neoforge-{v}.jar") } else { format!("neoforge-{v}-{classifier}.jar") };
    engine.paths().libraries_dir().join("net").join("neoforged").join("neoforge").join(v).join(file)
}

/// True when every installer-generated artifact the production launch needs exists.
pub fn launch_artifacts_ready(engine: &Engine, v: &str, profile: &VersionJson) -> EngineResult<bool> {
    let Some(mc_and) = mc_and_neoform_version(profile) else { return Ok(false) };
    let dir = client_artifacts_dir(engine, &mc_and);
    let required = [
        dir.join(format!("client-{mc_and}-srg.jar")),
        dir.join(format!("client-{mc_and}-extra.jar")),
        neoforge_jar_path(engine, v, "universal"),
        neoforge_jar_path(engine, v, "client"),
    ];
    Ok(required.iter().all(|p| p.exists()))
}

/// Persisted launcher profile for `v` (no network), None when not installed.
pub fn read_neoforge_json(engine: &Engine, v: &str) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(neoforge_json_path(engine, v)).ok()?).ok()
}

/// Whether the loader profile for `v` is installed.
pub fn is_neoforge_installed(engine: &Engine, v: &str) -> bool {
    neoforge_json_path(engine, v).exists()
}

/// A java 21+ executable for the installer (launcher runtime, exact-21
/// provisioning, else PATH `java`).
async fn installer_java(engine: &Engine) -> EngineResult<PathBuf> {
    if let Some(p) = engine.game_resolve_state().hooks().installer_java {
        return Ok(p);
    }
    let rt = crate::runtimes::runtime_java_path(engine, 21);
    if rt.exists() {
        return Ok(rt);
    }
    if let Ok(info) = crate::runtimes::java_for_major_exact(engine, 21).await {
        if !info.path.is_empty() && Path::new(&info.path).exists() {
            return Ok(PathBuf::from(info.path));
        }
    }
    let ok = crate::jvm::hidden_command("java")
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        return Ok(PathBuf::from("java"));
    }
    Err(EngineError::api(
        502,
        "NEOFORGE_JDK_REQUIRED",
        "NeoForge necesita un JDK 21 para generar sus artefactos de lanzamiento",
    ))
}

/// Run the official installer so it generates the srg/extra/client artifacts
/// into `<data>/libraries`. NEOFORGE_INSTALL_FAILED on a non-zero exit.
async fn run_installer_for(engine: &Engine, v: &str, profile: &Value) -> EngineResult<()> {
    let mc = profile.get("inheritsFrom").and_then(Value::as_str).unwrap_or("").to_string();
    // The installer wants versions/<mc>/<mc>.json (the launcher caches it flat
    // at versions/<mc>.json) and a launcher_profiles.json stub at the root.
    let nested = engine.paths().version_json(&mc);
    if !nested.exists() {
        let flat = engine.paths().versions_dir().join(format!("{mc}.json"));
        if let Some(parent) = nested.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if flat.exists() {
            tokio::fs::copy(&flat, &nested).await?;
        } else {
            let json = crate::resolver::get_version_json(engine, &mc).await?;
            tokio::fs::write(&nested, serde_json::to_string(&json)?).await?;
        }
    }
    let profiles = engine.paths().launcher_profiles_json();
    if !profiles.exists() {
        let stub = json!({ "profiles": { mc.clone(): { "lastVersionId": mc } }, "selectedProfile": mc });
        tokio::fs::write(&profiles, serde_json::to_string(&stub)?).await?;
    }

    let installer = engine.paths().neoforge_installer(v);
    if !installer.exists() {
        engine.http().download_file(&installer_url(engine, v), &installer, &Default::default()).await?;
    }
    let java = installer_java(engine).await?;
    let mut cmd = crate::jvm::hidden_command(&java);
    cmd.arg("-jar")
        .arg(&installer)
        .arg("--installClient")
        .arg(engine.paths().data_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let (code, out) = match cmd.spawn() {
        Err(e) => (-1, e.to_string()),
        Ok(mut child) => {
            use tokio::io::AsyncBufReadExt;
            let collected = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
            let mut tasks = vec![];
            if let Some(s) = child.stdout.take() {
                tasks.push(tokio::spawn(pump(tokio::io::BufReader::new(s).lines(), collected.clone())));
            }
            if let Some(s) = child.stderr.take() {
                tasks.push(tokio::spawn(pump(tokio::io::BufReader::new(s).lines(), collected.clone())));
            }
            let status = child.wait().await;
            for t in tasks {
                let _ = t.await;
            }
            let out = collected.lock().unwrap_or_else(|p| p.into_inner()).clone();
            match status {
                Ok(s) => (s.code().unwrap_or(-1), out),
                Err(e) => (-1, e.to_string()),
            }
        }
    };
    if code != 0 {
        let tail: String = {
            let chars: Vec<char> = out.chars().collect();
            chars[chars.len().saturating_sub(400)..].iter().collect()
        };
        return Err(EngineError::api(
            502,
            "NEOFORGE_INSTALL_FAILED",
            format!("El instalador de NeoForge falló (exit {code}): {tail}"),
        ));
    }
    Ok(())
}

async fn pump<R: tokio::io::AsyncBufRead + Unpin>(
    mut lines: tokio::io::Lines<R>,
    out: std::sync::Arc<std::sync::Mutex<String>>,
) {
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::info!("[neoforge-installer] {}", line.trim_end());
        let mut o = out.lock().unwrap_or_else(|p| p.into_inner());
        o.push_str(&line);
        o.push('\n');
    }
}

fn loader_event(instance: Option<&str>, phase: &str, filename: &str) -> ProgressEvent {
    ProgressEvent {
        instance: instance.map(str::to_string),
        phase: Some(phase.into()),
        index: Some(Value::from(0)),
        total: Some(1),
        kind: Some("neoforge-loader".into()),
        filename: Some(filename.into()),
        ..Default::default()
    }
}

/// Install (once) the NeoForge loader `v`: profile json, the
/// installer-generated launch artifacts, and every profile library.
/// Returns the JS `{ installed, cached, json }` object. Serialized per version.
///
/// Errors: 404 `neoforge_version_not_found`, 502 `NEOFORGE_PROFILE_MISSING`,
/// `NEOFORGE_INSTALL_FAILED`, `NEOFORGE_ARTIFACTS_MISSING`, `NEOFORGE_JDK_REQUIRED`.
pub async fn ensure_neoforge_loader(engine: &Engine, v: &str, instance: Option<&InstanceMeta>) -> EngineResult<Value> {
    let _guard = engine.game_resolve_state().neoforge_locks.lock(&v.to_string()).await;
    ensure_inner(engine, v, instance).await
}

async fn ensure_inner(engine: &Engine, v: &str, instance: Option<&InstanceMeta>) -> EngineResult<Value> {
    let existing = read_neoforge_json(engine, v);
    if let Some(ex) = &existing {
        if launch_artifacts_ready(engine, v, ex)? {
            return Ok(json!({ "installed": true, "cached": true, "json": ex }));
        }
        tracing::warn!("[neoforge] {v}: profile present but launch artifacts missing — regenerating");
    }

    let instance_name = instance.map(|i| i.name.as_str());
    let url = installer_url(engine, v);
    let jar_path = engine.paths().neoforge_installer(v);
    let jar_name = jar_path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();

    // 1. Installer jar.
    engine.emit(EngineEvent::ModProgress(loader_event(instance_name, "start", &jar_name)));
    if let Err(err) = engine.http().download_file(&url, &jar_path, &Default::default()).await {
        let msg = err.to_string();
        let mut ev = loader_event(instance_name, "error", &jar_name);
        ev.error = Some(msg.clone());
        ev.message = Some(msg);
        engine.emit(EngineEvent::ModProgress(ev));
        if err.status() == 404 {
            return Err(EngineError::api(404, "neoforge_version_not_found", format!("NeoForge {v} no encontrado")));
        }
        return Err(err);
    }
    let mut done = loader_event(instance_name, "done", &jar_name);
    done.dest = Some(jar_path.to_string_lossy().into_owned());
    done.sha1 = Some(Value::Null);
    engine.emit(EngineEvent::ModProgress(done));

    // 2. Launcher profile from the installer.
    let jp = jar_path.clone();
    let buf = tokio::task::spawn_blocking(move || crate::ziputil::read_zip_entry(&jp, "version.json"))
        .await
        .map_err(|e| EngineError::other(e.to_string()))?
        .map_err(|e| EngineError::other(e.to_string()))?;
    let Some(buf) = buf else {
        return Err(EngineError::api(
            502,
            "NEOFORGE_PROFILE_MISSING",
            "El instalador de NeoForge no contiene version.json",
        ));
    };
    let profile: Value = serde_json::from_slice(&buf).map_err(|e| {
        EngineError::api(
            502,
            "NEOFORGE_PROFILE_MISSING",
            format!("version.json del instalador de NeoForge inválido: {e}"),
        )
    })?;

    // 3. Persist, then generate the launch artifacts with the official installer.
    let json_path = neoforge_json_path(engine, v);
    crate::paths::write_json_atomic(&json_path, &profile)?;
    engine.emit(EngineEvent::ModProgress(ProgressEvent {
        instance: instance_name.map(str::to_string),
        phase: Some("neoforge-artifacts".into()),
        index: Some(Value::from(0)),
        total: Some(1),
        kind: Some("neoforge-loader".into()),
        filename: Some(format!("neoforge-{v} (instalador oficial)")),
        ..Default::default()
    }));
    if let Err(e) = run_installer_for(engine, v, &profile).await {
        let _ = std::fs::remove_file(&json_path);
        return Err(e);
    }
    if !launch_artifacts_ready(engine, v, &profile)? {
        let _ = std::fs::remove_file(&json_path);
        return Err(EngineError::api(
            502,
            "NEOFORGE_ARTIFACTS_MISSING",
            format!("El instalador de NeoForge no generó los artefactos de {v}"),
        ));
    }
    let _ = std::fs::remove_file(&jar_path);

    // 4. Every profile library, sha1-verified; roll the profile back on failure.
    let r: EngineResult<()> = async {
        let mut items = vec![];
        for lib in profile.get("libraries").and_then(Value::as_array).cloned().unwrap_or_default() {
            let artifact = lib.pointer("/downloads/artifact");
            let aurl = artifact.and_then(|a| a.get("url")).and_then(Value::as_str).filter(|s| !s.is_empty());
            let apath = artifact.and_then(|a| a.get("path")).and_then(Value::as_str).filter(|s| !s.is_empty());
            let (Some(aurl), Some(apath)) = (aurl, apath) else {
                let name = lib.get("name").and_then(Value::as_str).unwrap_or("undefined");
                return Err(EngineError::api(
                    502,
                    "NEOFORGE_PROFILE_MISSING",
                    format!("La librería {name} no tiene descarga en el perfil NeoForge"),
                ));
            };
            let dest = join_rel(&engine.paths().libraries_dir(), apath);
            let filename = dest.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let a = artifact.expect("artifact");
            items.push(
                DownloadItem::new(aurl, dest)
                    .sha1(a.get("sha1").and_then(Value::as_str).map(str::to_string))
                    .size(a.get("size").and_then(Value::as_u64))
                    .event(
                        ProgressChannel::Mod,
                        ProgressEvent {
                            instance: instance_name.map(str::to_string),
                            kind: Some("neoforge-library".into()),
                            filename: Some(filename),
                            ..Default::default()
                        },
                    ),
            );
        }
        engine.http().download_all(items, Default::default()).await?;
        Ok(())
    }
    .await;
    if let Err(e) = r {
        let _ = std::fs::remove_file(&json_path);
        return Err(e);
    }

    Ok(json!({ "installed": true, "cached": false, "json": profile }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fixture, temp_engine};
    #[cfg(unix)]
    use std::path::Path;

    #[test]
    fn version_id_and_prefixes() {
        assert_eq!(neoforge_version_id("21.1.242"), "neoforge-21.1.242");
        assert_eq!(neoforge_prefix_for("1.21.1").as_deref(), Some("21.1."));
        assert_eq!(neoforge_prefix_for("1.21.11").as_deref(), Some("21.11."));
        assert_eq!(neoforge_prefix_for("1.21").as_deref(), Some("21.0."));
        assert_eq!(neoforge_prefix_for("26.2").as_deref(), Some("26.2.0."));
        assert_eq!(neoforge_prefix_for("26.1.2").as_deref(), Some("26.1.2."));
        assert_eq!(neoforge_prefix_for("24w14a"), None);
        assert_eq!(neoforge_prefix_for(""), None);
    }

    #[test]
    fn latest_from_metadata_sorts_numerically_and_skips_unstable() {
        let xml = "<metadata><versioning><versions>\
            <version>21.1.9</version><version>21.1.242</version><version>21.1.100</version>\
            <version>21.1.300-beta</version><version>21.10.1</version><version>21.1.250-RC1</version>\
            <version>20.4.1</version></versions></versioning></metadata>";
        assert_eq!(latest_from_metadata(xml, "21.1.").as_deref(), Some("21.1.242"));
        assert_eq!(latest_from_metadata(xml, "21.10.").as_deref(), Some("21.10.1"));
        assert_eq!(latest_from_metadata(xml, "19.").as_deref(), None);
    }

    #[test]
    fn read_neoforge_json_from_data_dir() {
        let (_d, e) = temp_engine();
        assert!(read_neoforge_json(&e, "21.1.242").is_none(), "not installed yet");
        assert!(!is_neoforge_installed(&e, "21.1.242"));
        let p = neoforge_json_path(&e, "21.1.242");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::copy(fixture("neoforge-21.1.242-version.json"), &p).unwrap();
        let json = read_neoforge_json(&e, "21.1.242").expect("installed profile must parse");
        assert_eq!(json["id"], "neoforge-21.1.242");
        assert_eq!(json["libraries"].as_array().unwrap().len(), 47);
        assert!(is_neoforge_installed(&e, "21.1.242"));
    }

    #[test]
    fn launch_artifacts_follow_mc_and_neoform() {
        let (_d, e) = temp_engine();
        let profile: Value =
            serde_json::from_str(&std::fs::read_to_string(fixture("neoforge-21.1.242-version.json")).unwrap()).unwrap();
        assert_eq!(mc_and_neoform_version(&profile).as_deref(), Some("1.21.1-20240808.144430"));
        assert!(!launch_artifacts_ready(&e, "21.1.242", &profile).unwrap());
        let dir = client_artifacts_dir(&e, "1.21.1-20240808.144430");
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["client-1.21.1-20240808.144430-srg.jar", "client-1.21.1-20240808.144430-extra.jar"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        for c in ["universal", "client"] {
            let p = neoforge_jar_path(&e, "21.1.242", c);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        assert!(launch_artifacts_ready(&e, "21.1.242", &profile).unwrap());
        assert!(!launch_artifacts_ready(&e, "21.1.242", &json!({ "id": "x" })).unwrap());
        // rule-object values are flattened too
        let p2 = json!({ "inheritsFrom": "1.21.1", "arguments": { "game": [
            { "rules": [], "value": ["--fml.neoFormVersion", "X"] }
        ] } });
        assert_eq!(mc_and_neoform_version(&p2).as_deref(), Some("1.21.1-X"));
    }

    // ---- ensure_neoforge_loader end to end (fake installer JVM: unix shell) ----

    #[cfg(unix)]
    fn installer_jar(profile: &Value) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("version.json", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(profile.to_string().as_bytes()).unwrap();
        w.finish().unwrap().into_inner()
    }

    /// Stands in for `java -jar installer --installClient <dataDir>`: creates
    /// the four generated artifacts and exits with `exit`.
    #[cfg(unix)]
    fn fake_installer_java(dir: &Path, exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("fake-installer-java");
        let script = [
            "#!/bin/sh".to_string(),
            "set -e".into(),
            "D=\"$4\"".into(),
            "echo \"installing into $D\"".into(),
            "[ -f \"$2\" ] || exit 7".into(),
            "C=\"$D/libraries/net/minecraft/client/1.21.1-X\"".into(),
            "N=\"$D/libraries/net/neoforged/neoforge/9.9.9\"".into(),
            "mkdir -p \"$C\" \"$N\"".into(),
            ": > \"$C/client-1.21.1-X-srg.jar\"".into(),
            ": > \"$C/client-1.21.1-X-extra.jar\"".into(),
            ": > \"$N/neoforge-9.9.9-universal.jar\"".into(),
            ": > \"$N/neoforge-9.9.9-client.jar\"".into(),
            format!("exit {exit}"),
        ]
        .join("\n");
        std::fs::write(&p, script + "\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    async fn neoforge_env(exit: i32) -> (tempfile::TempDir, Engine, crate::test_support::MockServer, Value) {
        use crate::test_support::{MockResponse, MockServer};
        let (d, e) = temp_engine();
        let lib = b"neo-lib".to_vec();
        let lib_sha = crate::download::sha1_hex(&lib);
        let slot: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>> = Default::default();
        let jar_slot = slot.clone();
        let srv = MockServer::start(move |r| match r.path.as_str() {
            "/neoforge/net/neoforged/neoforge/9.9.9/neoforge-9.9.9-installer.jar" => {
                MockResponse::bytes(jar_slot.lock().unwrap().clone().unwrap())
            }
            "/lib/a.jar" => MockResponse::bytes(lib.clone()),
            _ => MockResponse::status(404),
        })
        .await;
        let profile = json!({
            "id": "neoforge-9.9.9",
            "inheritsFrom": "1.21.1",
            "arguments": { "game": ["--fml.neoFormVersion", "X"] },
            "libraries": [{ "name": "a:a:1", "downloads": { "artifact": {
                "url": srv.url("/lib/a.jar"), "path": "a/a/1/a-1.jar", "sha1": lib_sha, "size": 7 } } }],
        });
        *slot.lock().unwrap() = Some(installer_jar(&profile));
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on(&srv.url("")));
        e.game_resolve_state().isolate();
        let java = fake_installer_java(d.path(), exit);
        e.game_resolve_state().update_hooks(|h| h.installer_java = Some(java));
        std::fs::create_dir_all(e.paths().versions_dir()).unwrap();
        std::fs::write(e.paths().versions_dir().join("1.21.1.json"), "{\"id\":\"1.21.1\"}").unwrap();
        (d, e, srv, profile)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_installs_profile_artifacts_and_libraries_then_caches() {
        let (_d, e, _srv, profile) = neoforge_env(0).await;
        let inst = InstanceMeta { name: "Neo".into(), version: "1.21.1".into(), ..Default::default() };
        let mut rx = e.subscribe();
        let r = ensure_neoforge_loader(&e, "9.9.9", Some(&inst)).await.unwrap();
        assert_eq!((r["installed"].as_bool(), r["cached"].as_bool()), (Some(true), Some(false)));
        assert_eq!(r["json"], profile);
        assert_eq!(read_neoforge_json(&e, "9.9.9").unwrap(), profile);
        assert!(!e.paths().neoforge_installer("9.9.9").exists(), "installer jar deleted");
        assert!(e.paths().libraries_dir().join("a/a/1/a-1.jar").exists());
        assert!(e.paths().version_json("1.21.1").exists(), "vanilla json staged for the installer");
        let lp: Value =
            serde_json::from_str(&std::fs::read_to_string(e.paths().launcher_profiles_json()).unwrap()).unwrap();
        assert_eq!(lp, json!({ "profiles": { "1.21.1": { "lastVersionId": "1.21.1" } }, "selectedProfile": "1.21.1" }));
        let mut phases = vec![];
        while let Ok(EngineEvent::ModProgress(p)) = rx.try_recv() {
            assert_eq!(p.instance.as_deref(), Some("Neo"));
            phases.push(format!("{}:{}", p.kind.unwrap_or_default(), p.phase.unwrap_or_default()));
        }
        assert_eq!(
            phases,
            vec![
                "neoforge-loader:start",
                "neoforge-loader:done",
                "neoforge-loader:neoforge-artifacts",
                "neoforge-library:start",
                "neoforge-library:done"
            ]
        );
        let again = ensure_neoforge_loader(&e, "9.9.9", None).await.unwrap();
        assert_eq!(again["cached"], true);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_rolls_back_profile_when_installer_fails() {
        let (_d, e, _srv, _p) = neoforge_env(3).await;
        let err = ensure_neoforge_loader(&e, "9.9.9", None).await.unwrap_err();
        assert!(err.has_code("NEOFORGE_INSTALL_FAILED"), "{err:?}");
        assert!(err.to_string().contains("(exit 3)"), "{err}");
        assert!(err.to_string().contains("installing into"), "{err}");
        assert!(!is_neoforge_installed(&e, "9.9.9"));
    }

    #[tokio::test]
    async fn ensure_unknown_version_is_404() {
        use crate::test_support::{MockResponse, MockServer};
        let (_d, e) = temp_engine();
        let srv = MockServer::start(|_| MockResponse::status(404)).await;
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on(&srv.url("")));
        let err = ensure_neoforge_loader(&e, "0.0.1", None).await.unwrap_err();
        assert!(err.has_code("neoforge_version_not_found"), "{err:?}");
        assert_eq!(err.to_string(), "NeoForge 0.0.1 no encontrado");
    }

    #[tokio::test]
    async fn latest_neoforge_for_reads_maven_metadata() {
        use crate::test_support::{MockResponse, MockServer};
        let (_d, e) = temp_engine();
        let srv = MockServer::start(|r| {
            assert_eq!(r.path, "/neoforge/net/neoforged/neoforge/maven-metadata.xml");
            MockResponse::ok(
                "<versions><version>21.1.1</version><version>21.1.20</version><version>21.1.3-beta</version></versions>",
            )
        })
        .await;
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on(&srv.url("")));
        assert_eq!(latest_neoforge_for(&e, "1.21.1").await.unwrap().as_deref(), Some("21.1.20"));
        assert_eq!(latest_neoforge_for(&e, "1.20.4").await.unwrap(), None);
        assert_eq!(latest_neoforge_for(&e, "24w14a").await.unwrap(), None, "snapshot: no request");
        assert_eq!(srv.hits(), 2);
    }
}
