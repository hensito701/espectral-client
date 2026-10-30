//! Port of `src/engine/resolver.mjs` — version manifest, version JSON,
//! Fabric profiles, library / natives / asset installation, classpath.
//!
//! **Owner: game-resolve.** Status: STUB (contract signatures only, except
//! the pure helpers already implemented below).
//!
//! JS exports still to port: MANIFEST_URL FABRIC_META_BASE FABRIC_MAVEN
//! ASSET_BASE MANIFEST_TTL_MS SUPPORTED_MAJORS getVersionManifest
//! getVersionJson resolveVersion readLocalVersionJson dedupeByName mergeArgs
//! resolveVersionChain installLibraries installAssets getFabricLoaders
//! fabricSupportedForVersion resolveFabric installFabricLibraries seedInstance
//! installInstance (+ natives extraction, FastClient seed fast-path, maven
//! .sha1 hydration). Tests: tests/resolver.test.mjs (rulesAllow cases are
//! already ported in crate::rules).
//!
//! ALREADY IN THE FOUNDATION — use, don't re-port:
//! - `osName` / `osArch`        -> crate::platform::{os_name, node_arch}
//! - `rulesAllow` `mavenPath` `nativesClassifier` `libraryDownload` -> crate::rules
//! - `versionsDir` `librariesDir` `assetsDir` `instanceDir` `instanceNativesDir`
//!   `clientJarPath` `fastClientRoot` -> crate::paths (Engine::paths())
//! - `fetchJson` / `downloadFile` / `downloadAll` + Fabric meta2/maven2 mirrors -> crate::download
//!
//! CONTRACT: the `pub` functions below are called by other clusters
//! (game-launch, content). Keep their signatures; add more freely.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{
    AssetsInstall, FabricLibrariesInstall, FabricProfile, InstanceMeta, LibrariesInstall, VersionInfo, VersionJson,
    VersionManifest,
};

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const FABRIC_META_BASE: &str = "https://meta.fabricmc.net/v2";
pub const FABRIC_MAVEN: &str = "https://maven.fabricmc.net/";
pub const ASSET_BASE: &str = "https://resources.download.minecraft.net";
/// Java tiers the launcher can provision.
pub const SUPPORTED_MAJORS: [u32; 5] = [8, 16, 17, 21, 25];

/// installLibraries() options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallLibrariesOptions {
    pub seed_from_fast_client: bool,
    pub force_extract: bool,
}

impl Default for InstallLibrariesOptions {
    fn default() -> Self {
        InstallLibrariesOptions { seed_from_fast_client: true, force_extract: false }
    }
}

/// seedInstance() result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedResult {
    pub version_json: VersionJson,
    pub libraries: LibrariesInstall,
    pub fabric: Option<FabricProfile>,
    pub neoforge: Option<Value>,
    pub assets: AssetsInstall,
}

/// loadInstanceMeta(name): instance.json, 409 `invalid_instance_name` /
/// 404 `instance_not_found` (lowercase codes, as in the JS).
pub fn load_instance_meta(engine: &Engine, name: &str) -> EngineResult<InstanceMeta> {
    if !crate::util::is_valid_instance_name(name) {
        return Err(EngineError::api(409, "invalid_instance_name", format!("invalid instance name: {name}")));
    }
    std::fs::read_to_string(engine.paths().instance_json(name))
        .ok()
        .and_then(|s| serde_json::from_str::<InstanceMeta>(&s).ok())
        .ok_or_else(|| EngineError::api(404, "instance_not_found", format!("instance not found: {name}")))
}

/// Classpath: loader libs first, then allowed non-natives vanilla libs, then
/// the client jar (resolver.resolveClasspath).
pub fn resolve_classpath(
    engine: &Engine,
    instance: &InstanceMeta,
    version_json: &VersionJson,
    fabric: Option<&FabricProfile>,
) -> Vec<PathBuf> {
    let libs_dir = engine.paths().libraries_dir();
    let mut out: Vec<PathBuf> =
        fabric.map(|f| f.libraries.iter().map(|l| libs_dir.join(&l.rel_path)).collect()).unwrap_or_default();
    if let Some(Value::Array(libs)) = version_json.get("libraries") {
        for lib in libs {
            let name = lib.get("name").and_then(Value::as_str).unwrap_or("");
            if crate::rules::rules_allow(lib) && crate::rules::natives_classifier(name).is_none() {
                out.push(libs_dir.join(crate::rules::maven_path(name)));
            }
        }
    }
    out.push(engine.paths().client_jar(&instance.version));
    out
}

/// GET /api/versions source: cached manifest (6h TTL, stale-while-revalidate).
pub async fn get_version_manifest(engine: &Engine, force: bool) -> EngineResult<VersionManifest> {
    let _ = (engine, force);
    crate::not_ported!("resolver::get_version_manifest")
}

/// resolveVersion(id): 404 `version_not_found` when absent from the manifest.
pub async fn resolve_version(engine: &Engine, id: &str) -> EngineResult<VersionInfo> {
    let _ = (engine, id);
    crate::not_ported!("resolver::resolve_version")
}

/// Mojang version JSON for `id` (cached under versions/<id>/<id>.json, sha1-checked).
pub async fn get_version_json(engine: &Engine, id: &str) -> EngineResult<VersionJson> {
    let _ = (engine, id);
    crate::not_ported!("resolver::get_version_json")
}

/// Locally persisted version JSON (no network), None when absent/corrupt.
pub fn read_local_version_json(engine: &Engine, id: &str) -> Option<VersionJson> {
    let s = std::fs::read_to_string(engine.paths().version_json(id)).ok()?;
    serde_json::from_str(&s).ok()
}

/// Merge an `inheritsFrom` chain (NeoForge profile over vanilla).
pub async fn resolve_version_chain(engine: &Engine, json: VersionJson) -> EngineResult<VersionJson> {
    let _ = (engine, json);
    crate::not_ported!("resolver::resolve_version_chain")
}

/// True when Fabric meta lists at least one loader for `mc_version`
/// (network failure -> optimistic true, like the JS).
pub async fn fabric_supported_for_version(engine: &Engine, mc_version: &str) -> EngineResult<bool> {
    let _ = (engine, mc_version);
    crate::not_ported!("resolver::fabric_supported_for_version")
}

/// Fabric loader profile (404 `fabric_loader_not_found`).
pub async fn resolve_fabric(
    engine: &Engine,
    mc_version: &str,
    loader_version: Option<&str>,
) -> EngineResult<FabricProfile> {
    let _ = (engine, mc_version, loader_version);
    crate::not_ported!("resolver::resolve_fabric")
}

/// Download (sha1-verified) libraries + natives + client jar; extract natives.
pub async fn install_libraries(
    engine: &Engine,
    instance: &InstanceMeta,
    version_json: &VersionJson,
    opts: InstallLibrariesOptions,
) -> EngineResult<LibrariesInstall> {
    let _ = (engine, instance, version_json, opts);
    crate::not_ported!("resolver::install_libraries")
}

/// Download the Fabric loader libraries.
pub async fn install_fabric_libraries(
    engine: &Engine,
    instance: &InstanceMeta,
    fabric: &FabricProfile,
    seed_from_fast_client: bool,
) -> EngineResult<FabricLibrariesInstall> {
    let _ = (engine, instance, fabric, seed_from_fast_client);
    crate::not_ported!("resolver::install_fabric_libraries")
}

/// Asset index + objects (asset-progress events).
pub async fn install_assets(
    engine: &Engine,
    instance: &InstanceMeta,
    version_json: &VersionJson,
) -> EngineResult<AssetsInstall> {
    let _ = (engine, instance, version_json);
    crate::not_ported!("resolver::install_assets")
}

/// Everything an instance needs to launch (background seeding at instance creation).
pub async fn seed_instance(
    engine: &Engine,
    instance: &InstanceMeta,
    seed_from_fast_client: bool,
) -> EngineResult<SeedResult> {
    let _ = (engine, instance, seed_from_fast_client);
    crate::not_ported!("resolver::seed_instance")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use crate::types::{LibraryDownload, Loader};
    use serde_json::json;

    #[test]
    fn classpath_order_and_filters() {
        let (_d, e) = temp_engine();
        let inst =
            InstanceMeta { name: "x".into(), version: "1.21.1".into(), loader: Loader::Fabric, ..Default::default() };
        let vj = json!({ "libraries": [
            { "name": "a:b:1" },
            { "name": "org.lwjgl:lwjgl:3:natives-linux" },
            { "name": "c:d:2", "rules": [{ "action": "allow", "os": { "name": "nonexistent" } }] }
        ] });
        let fabric = FabricProfile {
            mc_version: "1.21.1".into(),
            loader_version: "0.16".into(),
            loader_maven: None,
            intermediary: None,
            main_class: "K".into(),
            jvm_args: vec![],
            libraries: vec![LibraryDownload {
                name: "f:g:1".into(),
                rel_path: "f/g/1/g-1.jar".into(),
                url: "u".into(),
                sha1: None,
                size: None,
            }],
            min_java_version: None,
            library_count: None,
            seeded: None,
            downloaded: None,
        };
        let cp = resolve_classpath(&e, &inst, &vj, Some(&fabric));
        let libs = e.paths().libraries_dir();
        assert_eq!(cp, vec![libs.join("f/g/1/g-1.jar"), libs.join("a/b/1/b-1.jar"), e.paths().client_jar("1.21.1")]);
    }

    #[test]
    fn load_instance_meta_codes() {
        let (_d, e) = temp_engine();
        assert_eq!(load_instance_meta(&e, "bad/name").unwrap_err().envelope().code, "invalid_instance_name");
        assert_eq!(load_instance_meta(&e, "missing").unwrap_err().status(), 404);
        crate::paths::write_json_atomic(
            &e.paths().instance_json("ok"),
            &json!({ "name": "ok", "version": "1.21.1", "loader": "vanilla" }),
        )
        .unwrap();
        assert_eq!(load_instance_meta(&e, "ok").unwrap().version, "1.21.1");
    }
}
