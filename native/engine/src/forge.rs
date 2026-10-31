//! Forge loader (MinecraftForge). EXPLORATION SPIKE.
//!
//! - Legacy installer format (Forge <= 1.12.2: `install_profile.json` with
//!   `install` + `versionInfo`; launchwrapper + `--tweakClass`): see below.
//! - Modern installer (1.13+): the same processor-based installer NeoForge
//!   ships — its `version.json` is the profile and the official installer
//!   generates the patched client artifacts (`neoforge::run_official_installer`).
//!
//! Legacy install = the installer jar's `versionInfo` IS the launcher profile
//! (persisted as `<data>/versions/forge-<mc>-<build>/forge-<mc>-<build>.json`)
//! plus the universal jar extracted from the installer into `libraries/`.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::resolver::join_rel;
use crate::types::VersionJson;

pub const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
/// Where the legacy profiles' url-less, download-less libraries live
/// (launchwrapper, lzma, vecmath, trove4j…): NOT Maven Central.
const MOJANG_LIBRARIES: &str = "https://libraries.minecraft.net/";

/// Version id under `<data>/versions`: `forge-<mc>-<build>`.
pub fn forge_version_id(mc: &str, build: &str) -> String {
    format!("forge-{mc}-{build}")
}

/// Maven version candidates for a Forge build: 1.7.10–1.9.4 builds carry a
/// `-<mc>` branch suffix (`1.8.9-11.15.1.2318-1.8.9`), most others do not.
fn maven_candidates(mc: &str, build: &str) -> [String; 2] {
    [format!("{mc}-{build}-{mc}"), format!("{mc}-{build}")]
}

fn installer_url(coord: &str) -> String {
    format!("{FORGE_MAVEN}/net/minecraftforge/forge/{coord}/forge-{coord}-installer.jar")
}

fn installer_path(engine: &Engine, coord: &str) -> PathBuf {
    engine.paths().cache_dir().join("forge").join(format!("forge-{coord}-installer.jar"))
}

/// The local profile when present (and its universal jar is on disk).
pub fn read_forge_json(engine: &Engine, mc: &str, build: &str) -> Option<VersionJson> {
    let j = crate::resolver::read_local_version_json(engine, &forge_version_id(mc, build))?;
    let forge_lib =
        j.get("libraries")?.as_array()?.iter().find(|l| {
            l.get("name").and_then(Value::as_str).is_some_and(|n| n.starts_with("net.minecraftforge:forge:"))
        })?;
    let rel = forge_lib.pointer("/downloads/artifact/path").and_then(Value::as_str)?;
    join_rel(&engine.paths().libraries_dir(), rel).is_file().then_some(j)
}

/// Install (once) Forge `build` for `mc`; returns the profile JSON.
pub async fn ensure_forge_loader(engine: &Engine, mc: &str, build: &str) -> EngineResult<VersionJson> {
    if let Some(j) = read_forge_json(engine, mc, build) {
        return Ok(j);
    }
    let mut jar = None;
    for coord in maven_candidates(mc, build) {
        let dest = installer_path(engine, &coord);
        match engine.http().download_file(&installer_url(&coord), &dest, &Default::default()).await {
            Ok(_) => {
                jar = Some((coord, dest));
                break;
            }
            Err(e) if e.status() == 404 => continue,
            Err(e) => return Err(e),
        }
    }
    let Some((coord, jar)) = jar else {
        return Err(EngineError::api(404, "forge_version_not_found", format!("Forge {mc}-{build} no encontrado")));
    };

    let jp = jar.clone();
    let profile = tokio::task::spawn_blocking(move || crate::ziputil::read_zip_entry(&jp, "install_profile.json"))
        .await
        .map_err(|e| EngineError::other(e.to_string()))?
        .map_err(|e| EngineError::other(e.to_string()))?
        .ok_or_else(|| EngineError::api(502, "FORGE_PROFILE_MISSING", "install_profile.json missing"))?;
    let profile: Value = serde_json::from_slice(&profile)
        .map_err(|e| EngineError::api(502, "FORGE_PROFILE_MISSING", format!("install_profile.json: {e}")))?;
    let (Some(install), Some(version_info)) = (profile.get("install"), profile.get("versionInfo")) else {
        return ensure_modern(engine, mc, build, &jar).await;
    };

    // Universal jar: extract from the installer to the library path of `install.path`.
    let lib_name = install.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
    let entry = install.get("filePath").and_then(Value::as_str).unwrap_or_default().to_string();
    let rel = crate::rules::maven_path(&lib_name);
    let dest = join_rel(&engine.paths().libraries_dir(), &rel);
    let (jp, d) = (jar.clone(), dest.clone());
    let bytes = tokio::task::spawn_blocking(move || -> EngineResult<Vec<u8>> {
        let b = crate::ziputil::read_zip_entry(&jp, &entry)
            .map_err(|e| EngineError::other(e.to_string()))?
            .ok_or_else(|| EngineError::api(502, "FORGE_PROFILE_MISSING", format!("{entry} missing")))?;
        if let Some(p) = d.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&d, &b)?;
        Ok(b)
    })
    .await
    .map_err(|e| EngineError::other(e.to_string()))??;
    let sha1 = {
        use sha1::Digest;
        hex::encode(sha1::Sha1::digest(&bytes))
    };
    let universal_url = format!("{FORGE_MAVEN}/net/minecraftforge/forge/{coord}/forge-{coord}-universal.jar");

    // Profile: our id, explicit download info for every library.
    let id = forge_version_id(mc, build);
    let mut vi = version_info.clone();
    vi["id"] = Value::String(id.clone());
    if let Some(libs) = vi.get_mut("libraries").and_then(Value::as_array_mut) {
        for lib in libs.iter_mut() {
            let name = lib.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            if name == lib_name {
                lib["downloads"] = json!({ "artifact": {
                    "path": rel, "url": universal_url, "sha1": sha1, "size": bytes.len()
                }});
            } else if lib.get("url").is_none() && lib.get("downloads").is_none() {
                lib["url"] = Value::String(MOJANG_LIBRARIES.into());
            }
        }
    }
    crate::paths::write_json_atomic(&engine.paths().version_json(&id), &vi)?;
    Ok(vi)
}

/// Modern Forge: persist the installer's `version.json` under our id, run the
/// official installer (generates srg/extra/client jars), then fetch every
/// profile library that has a URL (the locally generated ones have none).
async fn ensure_modern(engine: &Engine, mc: &str, build: &str, jar: &std::path::Path) -> EngineResult<VersionJson> {
    let jp = jar.to_path_buf();
    let buf = tokio::task::spawn_blocking(move || crate::ziputil::read_zip_entry(&jp, "version.json"))
        .await
        .map_err(|e| EngineError::other(e.to_string()))?
        .map_err(|e| EngineError::other(e.to_string()))?
        .ok_or_else(|| EngineError::api(502, "FORGE_PROFILE_MISSING", "version.json missing"))?;
    let mut profile: Value = serde_json::from_slice(&buf)
        .map_err(|e| EngineError::api(502, "FORGE_PROFILE_MISSING", format!("version.json: {e}")))?;
    crate::neoforge::run_official_installer(engine, jar, mc).await?;

    let libs_dir = engine.paths().libraries_dir();
    let mut items = vec![];
    for lib in profile.get("libraries").and_then(Value::as_array).cloned().unwrap_or_default() {
        let Some(a) = lib.pointer("/downloads/artifact") else { continue };
        let path = a.get("path").and_then(Value::as_str).unwrap_or_default();
        let dest = join_rel(&libs_dir, path);
        match a.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) {
            Some(url) => items.push(
                crate::download::DownloadItem::new(url, dest)
                    .sha1(a.get("sha1").and_then(Value::as_str).map(str::to_string))
                    .size(a.get("size").and_then(Value::as_u64)),
            ),
            None if dest.is_file() => {}
            None => {
                return Err(EngineError::api(
                    502,
                    "FORGE_ARTIFACTS_MISSING",
                    format!("the Forge installer did not generate {path}"),
                ))
            }
        }
    }
    engine.http().download_all(items, Default::default()).await?;

    let id = forge_version_id(mc, build);
    profile["id"] = Value::String(id.clone());
    crate::paths::write_json_atomic(&engine.paths().version_json(&id), &profile)?;
    Ok(profile)
}
