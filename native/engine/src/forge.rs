//! Forge loader (MinecraftForge): build selection, installer run, launch profile.
//!
//! Two installer generations ship a Forge build:
//!
//! - **Legacy (1.8.9)**: `install_profile.json` carries `install` +
//!   `versionInfo`; the universal jar sits at the installer's root and
//!   `versionInfo` IS the launcher profile (launchwrapper,
//!   `--tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker`).
//! - **Processor-based (1.12.2 and 1.13+)**: `install_profile.json` describes
//!   `processors` and the installer body ships `version.json`; the official
//!   installer (`java -jar <installer> --installClient <data>`) generates the
//!   patched client artifacts (`…-client.jar`, `-srg`, `-extra`) inside
//!   `<data>/libraries`. 1.12.2 keeps the legacy profile shape
//!   (`minecraftArguments` + launchwrapper), 1.21+ uses the `bootstrap`
//!   launcher (`net.minecraftforge.bootstrap.ForgeBootstrap`) + module path.
//!
//! The installed profile is persisted under our own id
//! (`forge-<mc>-<build>`, see [`forge_version_id`]) in
//! `<data>/versions/forge-<mc>-<build>/forge-<mc>-<build>.json`, which is the
//! shape `resolver::read_local_version_json` and the launch resolver read.
//!
//! Which Minecraft versions Forge is offered for is a product decision, not a
//! maven question: [`forge_supported`] (1.8.9, 1.12.2, 1.21/1.21.x, 26.x).
//! The build to install comes from Forge's promotions feed
//! ([`default_forge_build`]: `<mc>-recommended`, else `<mc>-latest`) and, when
//! a UI needs to list them, from the maven metadata ([`forge_builds`]).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::resolver::join_rel;
use crate::types::VersionJson;

/// Forge maven (`net/minecraftforge/forge/<mc>-<build>/…`).
pub const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
/// Forge promotions feed: `{ "promos": { "<mc>-recommended": "<build>", … } }`.
pub const FORGE_PROMOS_URL: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
/// Where the legacy profiles' url-less, download-less libraries live
/// (launchwrapper, lzma, vecmath, trove4j…): NOT Maven Central.
const MOJANG_LIBRARIES: &str = "https://libraries.minecraft.net/";

/// The Minecraft versions Forge is offered for, for error messages.
pub const SUPPORTED_MC_SUMMARY: &str = "1.8.9, 1.12.2, 1.21.x and 26.x";

/// Version id under `<data>/versions`: `forge-<mc>-<build>`.
pub fn forge_version_id(mc: &str, build: &str) -> String {
    format!("forge-{mc}-{build}")
}

/// Forge support rule: exactly 1.8.9, 1.12.2, 1.21 / 1.21.x and 26.x.
///
/// Everything else is refused with 409 `FORGE_UNAVAILABLE` (the launcher does
/// not offer a Forge build there, even when maven has one).
pub fn forge_supported(mc: &str) -> bool {
    let mc = mc.trim();
    if mc == "1.8.9" || mc == "1.12.2" {
        return true;
    }
    let classic = regex::Regex::new(r"^1\.21(?:\.\d+)?$").expect("regex");
    if classic.is_match(mc) {
        return true;
    }
    let modern = regex::Regex::new(r"^26\.\d+(?:\.\d+)?$").expect("regex");
    modern.is_match(mc)
}

/// True when this Forge version uses the pre-1.13 launchwrapper launch
/// (`net.minecraft.launchwrapper.Launch` + `--tweakClass`): Forge for 1.8.9 and
/// 1.12.2. Those always run on Java 8.
pub fn is_legacy_launcher(mc: &str) -> bool {
    matches!(mc.trim(), "1.8.9" | "1.12.2")
}

/// Java tier a Forge launch needs. Legacy Forge (1.8.9/1.12.2) runs on 8;
/// modern Forge's bootstrap launcher needs at least 21, so the version's own
/// `javaVersion.majorVersion` (21 for 1.21.x, 25 for 26.x) is floored at 21.
pub fn java_tier(mc: &str, version_java_major: u32) -> u32 {
    if is_legacy_launcher(mc) {
        8
    } else {
        version_java_major.max(21)
    }
}

/// Java tier the installer JVM needs. The legacy (1.8.9) and 1.12.2
/// installers are simple Java-8 programs; the 1.21+ processor-based installers
/// run ASM/fmlloader patches, so they get the release's own tier floored at 21.
pub fn installer_java_tier(mc: &str, version_java_major: u32) -> u32 {
    if is_legacy_launcher(mc) {
        return 8;
    }
    version_java_major.max(21)
}

fn endpoints_maven(engine: &Engine) -> String {
    let base = engine.game_resolve_state().endpoints().forge_maven;
    base.trim_end_matches('/').to_string()
}

fn promos_url(engine: &Engine) -> String {
    engine.game_resolve_state().endpoints().forge_promos
}

/// Maven version candidates for a build: 1.7.10–1.9.4 builds carry a
/// `-<mc>` branch suffix (`1.8.9-11.15.1.2318-1.8.9`), most others do not.
fn maven_candidates(mc: &str, build: &str) -> [String; 2] {
    [format!("{mc}-{build}-{mc}"), format!("{mc}-{build}")]
}

/// `<build>` from a maven coordinate: strip the `<mc>-` prefix and the legacy
/// `-<mc>` branch suffix (`1.8.9-11.15.1.2318-1.8.9` -> `11.15.1.2318`).
pub fn build_from_coord(mc: &str, coord: &str) -> Option<String> {
    let rest = coord.strip_prefix(&format!("{mc}-"))?;
    let rest = rest.strip_suffix(&format!("-{mc}")).unwrap_or(rest);
    (!rest.is_empty()).then(|| rest.to_string())
}

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

/// Default (recommended) Forge build for `mc`, from the promotions feed:
/// `<mc>-recommended`, else `<mc>-latest`. `None` when Forge is not offered for
/// `mc` or the feed has no entry for it.
pub async fn default_forge_build(engine: &Engine, mc: &str) -> EngineResult<Option<String>> {
    if !forge_supported(mc) {
        return Ok(None);
    }
    let doc: Value = engine.http().fetch_json(&promos_url(engine), &Default::default()).await?;
    let promos = doc.get("promos").and_then(Value::as_object);
    let get =
        |k: &str| promos.and_then(|m| m.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    Ok(get(&format!("{mc}-recommended")).or_else(|| get(&format!("{mc}-latest"))))
}

/// The build to install: the requested one, else the promotions default.
/// 409 `FORGE_UNAVAILABLE` for an unsupported Minecraft version or when the
/// feed has no Forge build for it.
pub async fn resolve_forge_version(engine: &Engine, mc: &str, requested: Option<&str>) -> EngineResult<String> {
    if !forge_supported(mc) {
        return Err(EngineError::conflict(
            "FORGE_UNAVAILABLE",
            format!("Forge is not available for Minecraft {mc} (supported: {SUPPORTED_MC_SUMMARY})"),
        ));
    }
    if let Some(b) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(b.to_string());
    }
    default_forge_build(engine, mc).await?.ok_or_else(|| {
        EngineError::conflict("FORGE_UNAVAILABLE", format!("no Forge build is published for Minecraft {mc}"))
    })
}

/// Every Forge build published for `mc`, oldest first (maven metadata).
/// Empty for an unsupported Minecraft version (no request is made).
pub async fn forge_builds(engine: &Engine, mc: &str) -> EngineResult<Vec<String>> {
    if !forge_supported(mc) {
        return Ok(vec![]);
    }
    let url = format!("{}/net/minecraftforge/forge/maven-metadata.xml", endpoints_maven(engine));
    let xml = engine.http().fetch_text(&url, &Default::default()).await?;
    let re = regex::Regex::new(r"<version>([^<]+)</version>").expect("regex");
    let mut builds: Vec<String> = re.captures_iter(&xml).filter_map(|c| build_from_coord(mc, c[1].trim())).collect();
    builds.sort_by(|a, b| cmp_versions(a, b));
    builds.dedup();
    Ok(builds)
}

fn installer_url(engine: &Engine, coord: &str) -> String {
    format!("{}/net/minecraftforge/forge/{coord}/forge-{coord}-installer.jar", endpoints_maven(engine))
}

fn installer_path(engine: &Engine, coord: &str) -> PathBuf {
    engine.paths().cache_dir().join("forge").join(format!("forge-{coord}-installer.jar"))
}

/// `<data>/versions/<mc>/<mc>.json` (the nested layout the official installers
/// expect; the launcher caches Mojang jsons flat one level up).
pub fn nested_version_json(engine: &Engine, mc: &str) -> PathBuf {
    engine.paths().version_json(mc)
}

/// Every installer-generated library of `profile` is on disk (url-less
/// artifacts are produced locally, never downloaded).
fn generated_artifacts_ready(engine: &Engine, profile: &Value) -> bool {
    let libs_dir = engine.paths().libraries_dir();
    for lib in profile.get("libraries").and_then(Value::as_array).cloned().unwrap_or_default() {
        let Some(a) = lib.pointer("/downloads/artifact") else { continue };
        let path = a.get("path").and_then(Value::as_str).unwrap_or_default();
        let has_url = a.get("url").and_then(Value::as_str).is_some_and(|u| !u.is_empty());
        if !has_url && !join_rel(&libs_dir, path).is_file() {
            return false;
        }
    }
    true
}

/// The local profile when present, its Forge jar downloaded and every
/// installer-generated artifact on disk.
pub fn read_forge_json(engine: &Engine, mc: &str, build: &str) -> Option<VersionJson> {
    let j = crate::resolver::read_local_version_json(engine, &forge_version_id(mc, build))?;
    let forge_lib =
        j.get("libraries")?.as_array()?.iter().find(|l| {
            l.get("name").and_then(Value::as_str).is_some_and(|n| n.starts_with("net.minecraftforge:forge:"))
        })?;
    let rel = forge_lib.pointer("/downloads/artifact/path").and_then(Value::as_str)?;
    (join_rel(&engine.paths().libraries_dir(), rel).is_file() && generated_artifacts_ready(engine, &j)).then_some(j)
}

/// Install (once) Forge `build` for `mc`; returns the launch profile.
/// Serialized per `mc-build` so a seeding run and a launch never install twice.
pub async fn ensure_forge_loader(engine: &Engine, mc: &str, build: &str) -> EngineResult<VersionJson> {
    if !forge_supported(mc) {
        return Err(EngineError::conflict(
            "FORGE_UNAVAILABLE",
            format!("Forge is not available for Minecraft {mc} (supported: {SUPPORTED_MC_SUMMARY})"),
        ));
    }
    if let Some(j) = read_forge_json(engine, mc, build) {
        return Ok(j);
    }
    let _guard = engine.game_resolve_state().forge_locks.lock(&forge_version_id(mc, build)).await;
    if let Some(j) = read_forge_json(engine, mc, build) {
        return Ok(j);
    }
    let Some((coord, jar)) = download_installer(engine, mc, build).await? else {
        return Err(EngineError::api(404, "forge_version_not_found", format!("Forge {mc}-{build} no encontrado")));
    };

    let jp = jar.clone();
    let entry = tokio::task::spawn_blocking(move || {
        let profile = crate::ziputil::read_zip_entry(&jp, "install_profile.json")?;
        let version_json = crate::ziputil::read_zip_entry(&jp, "version.json")?;
        Ok::<_, crate::ziputil::ZipError>((profile, version_json))
    })
    .await
    .map_err(|e| EngineError::other(e.to_string()))?
    .map_err(|e| EngineError::other(e.to_string()))?;
    let (profile, version_json) = entry;
    let profile: Value = profile
        .as_deref()
        .map(serde_json::from_slice)
        .transpose()
        .map_err(|e| EngineError::api(502, "FORGE_PROFILE_MISSING", format!("install_profile.json: {e}")))?
        .ok_or_else(|| {
            EngineError::api(502, "FORGE_PROFILE_MISSING", format!("install_profile.json missing from {coord}"))
        })?;

    // 1.8.9 and older: install_profile carries the profile inline.
    if profile.get("install").is_some() && profile.get("versionInfo").is_some() {
        return install_legacy(engine, mc, build, &coord, &jar, &profile).await;
    }
    // 1.12.2 + 1.13+: the installer body ships `version.json` and generates the
    // patched artifacts when run.
    match version_json {
        Some(buf) => install_official(engine, mc, build, &jar, &buf).await,
        None => Err(EngineError::api(
            502,
            "FORGE_PROFILE_MISSING",
            format!("the Forge {coord} installer carries neither versionInfo nor version.json"),
        )),
    }
}

/// Download the installer jar (legacy/build-suffixed coordinate first).
async fn download_installer(engine: &Engine, mc: &str, build: &str) -> EngineResult<Option<(String, PathBuf)>> {
    for coord in maven_candidates(mc, build) {
        let dest = installer_path(engine, &coord);
        match engine.http().download_file(&installer_url(engine, &coord), &dest, &Default::default()).await {
            Ok(_) => return Ok(Some((coord, dest))),
            Err(e) if e.status() == 404 => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Legacy installer (1.8.9): extract the universal jar named by
/// `install.filePath`, point its library entry at the maven download and
/// persist `versionInfo` as our profile.
async fn install_legacy(
    engine: &Engine,
    mc: &str,
    build: &str,
    coord: &str,
    jar: &Path,
    profile: &Value,
) -> EngineResult<VersionJson> {
    let install = profile.get("install").cloned().unwrap_or_default();
    let version_info = profile.get("versionInfo").cloned().unwrap_or_default();
    let lib_name = install.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
    let entry = install.get("filePath").and_then(Value::as_str).unwrap_or_default().to_string();
    if lib_name.is_empty() || entry.is_empty() {
        return Err(EngineError::api(
            502,
            "FORGE_PROFILE_MISSING",
            format!("the Forge {coord} installer has no install.path/filePath"),
        ));
    }
    let rel = crate::rules::maven_path(&lib_name);
    let dest = join_rel(&engine.paths().libraries_dir(), &rel);
    let (jp, d) = (jar.to_path_buf(), dest.clone());
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
    let sha1 = crate::download::sha1_hex(&bytes);
    let universal_url =
        format!("{}/net/minecraftforge/forge/{coord}/forge-{coord}-universal.jar", endpoints_maven(engine));

    let id = forge_version_id(mc, build);
    let mut vi = version_info;
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

/// Processor-based installer (1.12.2, 1.21+): persist the shipped
/// `version.json` under our id, run the official installer (generates the
/// patched client jars), then fetch every profile library that has a URL.
async fn install_official(engine: &Engine, mc: &str, build: &str, jar: &Path, buf: &[u8]) -> EngineResult<VersionJson> {
    let mut profile: Value = serde_json::from_slice(buf)
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
                    format!("the Forge {mc}-{build} installer did not generate {path}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use std::io::Write;

    fn endpoints(e: &Engine, srv: &MockServer) {
        e.game_resolve_state().set_endpoints(crate::api::game_resolve::ResolveEndpoints::all_on(&srv.url("")));
        e.game_resolve_state().isolate();
    }

    #[test]
    fn support_rule_is_the_product_decision() {
        for v in ["1.8.9", "1.12.2", "1.21", "1.21.1", "1.21.11", "26.1", "26.2", "26.3", "26.1.2"] {
            assert!(forge_supported(v), "{v}");
        }
        for v in [
            "1.7.10", "1.8.8", "1.9.4", "1.10.2", "1.11.2", "1.12.1", "1.13.2", "1.15.2", "1.16.5", "1.20.1", "1.22.4",
            "1.210", "25.1", "27.0", "24w14a", "", " 1.8.9x",
        ] {
            assert!(!forge_supported(v), "{v}");
        }
        assert!(forge_supported(" 1.12.2 "), "surrounding whitespace is tolerated");
    }

    #[test]
    fn build_from_coord_strips_both_suffixes() {
        assert_eq!(build_from_coord("1.8.9", "1.8.9-11.15.1.2318-1.8.9").as_deref(), Some("11.15.1.2318"));
        assert_eq!(build_from_coord("1.12.2", "1.12.2-14.23.5.2859").as_deref(), Some("14.23.5.2859"));
        assert_eq!(build_from_coord("1.21.11", "1.21.11-61.2.4").as_deref(), Some("61.2.4"));
        assert_eq!(build_from_coord("1.21.1", "1.21.11-61.2.4"), None, "1.21.1 is not a prefix match of 1.21.11");
        assert_eq!(build_from_coord("1.8.9", "1.8.9-"), None);
    }

    #[tokio::test]
    async fn default_build_prefers_recommended_then_latest() {
        let (_d, e) = temp_engine();
        let srv = MockServer::start(|r| {
            assert_eq!(r.path, "/forge-promotions.json");
            MockResponse::ok(
                r#"{"promos":{"1.12.2-latest":"14.23.5.2864","1.12.2-recommended":"14.23.5.2859","26.3-latest":"66.1.2","1.20.1-latest":"47.4.28"}}"#,
            )
        })
        .await;
        endpoints(&e, &srv);
        assert_eq!(default_forge_build(&e, "1.12.2").await.unwrap().as_deref(), Some("14.23.5.2859"));
        assert_eq!(
            default_forge_build(&e, "26.3").await.unwrap().as_deref(),
            Some("66.1.2"),
            "no recommended -> latest"
        );
        assert_eq!(default_forge_build(&e, "1.20.1").await.unwrap(), None, "unsupported: never requested");
        assert_eq!(default_forge_build(&e, "1.8.9").await.unwrap(), None, "feed has no 1.8.9 entry");
        assert_eq!(srv.hits(), 3, "one request per supported version (1.20.1 is never asked)");
    }

    #[tokio::test]
    async fn resolve_forge_version_errors_are_409() {
        let (_d, e) = temp_engine();
        let srv = MockServer::start(|_| MockResponse::ok(r#"{"promos":{"1.8.9-recommended":"11.15.1.2318"}}"#)).await;
        endpoints(&e, &srv);
        assert_eq!(resolve_forge_version(&e, "1.8.9", Some("11.15.1.1902")).await.unwrap(), "11.15.1.1902");
        assert_eq!(resolve_forge_version(&e, "1.8.9", Some("  ")).await.unwrap(), "11.15.1.2318");
        let err = resolve_forge_version(&e, "1.20.1", None).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (409, Some("FORGE_UNAVAILABLE")));
        assert!(err.to_string().contains("1.8.9, 1.12.2, 1.21.x and 26.x"), "{err}");
        let err = resolve_forge_version(&e, "1.21.11", None).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (409, Some("FORGE_UNAVAILABLE")));
    }

    #[tokio::test]
    async fn forge_builds_reads_maven_metadata() {
        let (_d, e) = temp_engine();
        let srv = MockServer::start(|r| {
            assert_eq!(r.path, "/forge-maven/net/minecraftforge/forge/maven-metadata.xml");
            MockResponse::ok(
                "<metadata><versioning><versions>\
                 <version>1.8.9-11.15.1.2318-1.8.9</version>\
                 <version>1.8.9-11.15.1.1902-1.8.9</version>\
                 <version>1.12.2-14.23.5.2864</version>\
                 <version>1.21.11-61.2.4</version>\
                 <version>1.21.11-61.2.10</version>\
                 </versions></versioning></metadata>",
            )
        })
        .await;
        endpoints(&e, &srv);
        assert_eq!(forge_builds(&e, "1.8.9").await.unwrap(), ["11.15.1.1902", "11.15.1.2318"]);
        assert_eq!(forge_builds(&e, "1.21.11").await.unwrap(), ["61.2.4", "61.2.10"]);
        assert!(forge_builds(&e, "1.20.1").await.unwrap().is_empty(), "unsupported: never requested");
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// 1.8.9 installer: `install` + `versionInfo`, the universal jar at the root.
    fn legacy_installer(universal: &[u8]) -> Vec<u8> {
        let profile = json!({
            "install": {
                "profileName": "forge",
                "target": "1.8.9-forge1.8.9-11.15.1.2318-1.8.9",
                "path": "net.minecraftforge:forge:1.8.9-11.15.1.2318-1.8.9",
                "filePath": "forge-1.8.9-11.15.1.2318-1.8.9-universal.jar",
                "minecraft": "1.8.9"
            },
            "versionInfo": {
                "id": "1.8.9-forge1.8.9-11.15.1.2318-1.8.9",
                "inheritsFrom": "1.8.9",
                "mainClass": "net.minecraft.launchwrapper.Launch",
                "minecraftArguments": "--username ${auth_player_name} --tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker",
                "libraries": [
                    { "name": "net.minecraftforge:forge:1.8.9-11.15.1.2318-1.8.9" },
                    { "name": "net.minecraft:launchwrapper:1.12", "url": "https://libraries.minecraft.net/" },
                    { "name": "no:url:1" }
                ]
            }
        });
        zip(&[
            ("install_profile.json", profile.to_string().as_bytes()),
            ("forge-1.8.9-11.15.1.2318-1.8.9-universal.jar", universal),
        ])
    }

    #[tokio::test]
    async fn legacy_1_8_9_install_extracts_the_universal_jar() {
        let (_d, e) = temp_engine();
        let universal = b"forge-universal-1.8.9".to_vec();
        let jar = legacy_installer(&universal);
        let slot = std::sync::Arc::new(std::sync::Mutex::new(jar));
        let s = slot.clone();
        let srv = MockServer::start(move |r| match r.path.as_str() {
            "/forge-maven/net/minecraftforge/forge/1.8.9-11.15.1.2318-1.8.9/forge-1.8.9-11.15.1.2318-1.8.9-installer.jar" => {
                MockResponse::bytes(s.lock().unwrap().clone())
            }
            _ => MockResponse::status(404),
        })
        .await;
        endpoints(&e, &srv);
        let p = ensure_forge_loader(&e, "1.8.9", "11.15.1.2318").await.unwrap();
        assert_eq!(p["id"], "forge-1.8.9-11.15.1.2318");
        assert_eq!(p["mainClass"], "net.minecraft.launchwrapper.Launch");
        assert!(p["minecraftArguments"].as_str().unwrap().contains("--tweakClass"));
        // The profile entry's path is the plain `.jar`; its URL points at the
        // `-universal.jar` download (exactly what the real installer profile says).
        let rel = "net/minecraftforge/forge/1.8.9-11.15.1.2318-1.8.9/forge-1.8.9-11.15.1.2318-1.8.9.jar";
        let dest = e.paths().libraries_dir().join(rel);
        assert_eq!(std::fs::read(&dest).unwrap(), universal);
        let lib = &p["libraries"][0];
        assert_eq!(lib["downloads"]["artifact"]["path"], rel);
        assert_eq!(lib["downloads"]["artifact"]["sha1"], crate::download::sha1_hex(&universal));
        assert!(lib["downloads"]["artifact"]["url"].as_str().unwrap().ends_with("-universal.jar"));
        assert_eq!(p["libraries"][2]["url"], MOJANG_LIBRARIES, "url-less libs fall back to Mojang's library host");
        // cached: a second call never hits the network
        let hits = srv.hits();
        ensure_forge_loader(&e, "1.8.9", "11.15.1.2318").await.unwrap();
        assert_eq!(srv.hits(), hits);
        // the classic coordinate (with the -1.8.9 branch suffix) was tried first
        assert!(srv.requests()[0].path.contains("1.8.9-11.15.1.2318-1.8.9"));
    }

    #[test]
    fn read_forge_json_requires_generated_artifacts() {
        let (_d, e) = temp_engine();
        let id = forge_version_id("1.12.2", "14.23.5.2859");
        let path = e.paths().version_json(&id);
        let doc = json!({
            "id": id,
            "mainClass": "net.minecraft.launchwrapper.Launch",
            "minecraftArguments": "--tweakClass x",
            "libraries": [
                { "name": "net.minecraftforge:forge:1.12.2-14.23.5.2859", "downloads": { "artifact": {
                    "path": "net/minecraftforge/forge/1.12.2-14.23.5.2859/forge-1.12.2-14.23.5.2859.jar",
                    "url": "" } } }
            ]
        });
        crate::paths::write_json_atomic(&path, &doc).unwrap();
        assert!(read_forge_json(&e, "1.12.2", "14.23.5.2859").is_none(), "no jar on disk yet");
        let jar = e
            .paths()
            .libraries_dir()
            .join("net/minecraftforge/forge/1.12.2-14.23.5.2859/forge-1.12.2-14.23.5.2859.jar");
        std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
        std::fs::write(&jar, "forge").unwrap();
        assert!(read_forge_json(&e, "1.12.2", "14.23.5.2859").is_some());
        assert!(read_forge_json(&e, "1.20.1", "47.4.28").is_none(), "no profile written");
    }

    #[tokio::test]
    async fn ensure_rejects_unsupported_versions() {
        let (_d, e) = temp_engine();
        let err = ensure_forge_loader(&e, "1.20.1", "47.4.28").await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (409, Some("FORGE_UNAVAILABLE")));
    }
}
