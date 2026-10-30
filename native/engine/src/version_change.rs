//! Port of `src/engine/version-change.mjs` — change an existing instance's
//! Minecraft version and/or loader IN PLACE (worlds, options, servers and
//! the player's mods stay; nothing is recreated).
//!
//! **Owner: game-launch.** Status: PORTED.
//!
//!   plan_version_change(name, { version, loader })  — read-only preview
//!   apply_version_change(name, { version, loader }) — performs it
//!
//! The plan is recomputed on apply (the client's copy is never trusted).
//! Going to an OLDER version can corrupt worlds last opened in a newer one,
//! so a downgrade first copies saves/ to `<instanceDir>/backups/`. The
//! running-game guard lives in the facade (activeInstances), like rename.
//!
//! Cross-cluster calls: resolver (resolve_version, get_version_manifest,
//! fabric_supported_for_version), neoforge::latest_neoforge_for,
//! mods::{plan_mods_for_target, apply_mods_plan, ensure_branding_seeded},
//! instances::{get_instance, list_instances}. `supportsBranding` is not in the
//! mods contract yet, so a read-only copy reading the bundled
//! `branding/branding.json` lives here ([`supports_branding`]).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{
    InstanceMeta, Loader, VersionChangeMod, VersionChangeModsResult, VersionChangePlan, VersionChangeResult,
    VersionLoaderRef, VersionManifest, VersionTarget,
};

/// Validated target plus the loader build it needs (NeoForge only).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTarget {
    pub version: String,
    pub loader: Loader,
    pub loader_version: Option<String>,
}

/// Target version/loader with the instance's current values as defaults;
/// 400 NO_CHANGE when nothing would change (checked before any lookup).
pub fn requested_target(inst: &InstanceMeta, req: &VersionTarget) -> EngineResult<(String, Loader)> {
    let version = req
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| inst.version.clone());
    let loader = req.loader.unwrap_or(inst.loader);
    if version == inst.version && loader == inst.loader {
        return Err(EngineError::bad_request("NO_CHANGE", "the instance already uses that version and platform"));
    }
    Ok((version, loader))
}

async fn resolve_target(engine: &Engine, inst: &InstanceMeta, req: &VersionTarget) -> EngineResult<ResolvedTarget> {
    let (version, loader) = requested_target(inst, req)?;
    let info = match crate::resolver::resolve_version(engine, &version).await {
        Ok(i) => i,
        Err(e) if e.status() == 404 => {
            return Err(EngineError::not_found("VERSION_NOT_FOUND", format!("Minecraft {version} does not exist")))
        }
        Err(e) => return Err(e),
    };
    if !info.supported {
        return Err(EngineError::conflict("VERSION_UNSUPPORTED", info.reason));
    }
    let mut loader_version = None;
    if loader == Loader::Fabric && !crate::resolver::fabric_supported_for_version(engine, &version).await? {
        return Err(EngineError::conflict("FABRIC_UNAVAILABLE", format!("no Fabric loader for Minecraft {version}")));
    }
    if loader == Loader::Neoforge {
        loader_version = crate::neoforge::latest_neoforge_for(engine, &version).await?;
        if loader_version.is_none() {
            return Err(EngineError::conflict(
                "NEOFORGE_UNAVAILABLE",
                format!("no NeoForge build for Minecraft {version}"),
            ));
        }
    }
    Ok(ResolvedTarget { version, loader, loader_version })
}

/// True when `to` was released before `from` (manifest release times).
pub fn is_downgrade_in(manifest: &VersionManifest, from: &str, to: &str) -> bool {
    if from == to {
        return false;
    }
    let time =
        |id: &str| manifest.versions.iter().find(|v| v.id == id).map(|v| v.release_time.clone()).unwrap_or_default();
    let (a, b) = (time(from), time(to));
    !a.is_empty() && !b.is_empty() && b < a
}

async fn is_downgrade(engine: &Engine, from: &str, to: &str) -> EngineResult<bool> {
    if from == to {
        return Ok(false);
    }
    let manifest = crate::resolver::get_version_manifest(engine, false).await?;
    Ok(is_downgrade_in(&manifest, from, to))
}

fn has_worlds(engine: &Engine, inst: &InstanceMeta) -> bool {
    std::fs::read_dir(crate::instances::effective_game_dir(engine, inst).join("saves"))
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false)
}

/// Directories that may hold the bundled branding manifest (packaged
/// resources next to the executable, then the dev checkout).
fn branding_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = std::env::var("ESPECTRAL_RESOURCES_DIR") {
        out.push(PathBuf::from(dir).join("branding"));
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        out.push(exe_dir.join("branding"));
        out.push(exe_dir.join("resources").join("branding"));
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    out.push(repo.join("resources").join("branding"));
    out.push(repo.join("branding"));
    out.push(repo.join("assets").join("branding"));
    out
}

/// mods.mjs supportsBranding: a bundled Espectral jar exists for this
/// version + loader (`branding.json` entry, `loaders` absent = fabric only).
pub fn supports_branding(version: &str, loader: Loader) -> bool {
    let Some(dir) = branding_dir_candidates().into_iter().find(|d| d.exists()) else { return false };
    let Some(manifest) =
        std::fs::read_to_string(dir.join("branding.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return false;
    };
    let Some(entry) = manifest.get("mods").and_then(|m| m.get(version)) else { return false };
    match entry.get("loaders").and_then(Value::as_array) {
        Some(ls) => ls.iter().any(|l| l.as_str() == Some(loader.as_str())),
        None => loader == Loader::Fabric,
    }
}

/// Read-only preview of a version/loader change.
pub async fn plan_version_change(
    engine: &Engine,
    name: &str,
    target: &VersionTarget,
) -> EngineResult<VersionChangePlan> {
    let inst = crate::launch::load_instance(engine, name).await?;
    let t = resolve_target(engine, &inst, target).await?;
    let downgrade = is_downgrade(engine, &inst.version, &t.version).await?;
    let mods_target = VersionTarget { version: Some(t.version.clone()), loader: Some(t.loader) };
    let mods_raw = crate::mods::plan_mods_for_target(engine, &inst, &mods_target).await?;
    let mods: Vec<VersionChangeMod> = serde_json::from_value(mods_raw)?;
    Ok(VersionChangePlan {
        from: VersionLoaderRef { version: inst.version.clone(), loader: inst.loader, loader_version: None },
        to: VersionLoaderRef { version: t.version.clone(), loader: t.loader, loader_version: Some(t.loader_version) },
        downgrade,
        backup_worlds: downgrade && has_worlds(engine, &inst),
        espectral_menu: supports_branding(&t.version, t.loader),
        mods,
    })
}

/// Local-time `YYYYMMDD-HHMMSS` (backup folder suffix).
pub fn stamp(at: chrono::DateTime<chrono::Local>) -> String {
    at.format("%Y%m%d-%H%M%S").to_string()
}

/// Recursive copy that refuses to overwrite (fs.cp errorOnExist).
pub fn copy_dir_no_overwrite(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_no_overwrite(&entry.path(), &to)?;
        } else {
            if to.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("target already exists: {}", to.display()),
                ));
            }
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Park bundled Espectral jars that have no build for the new loader/version
/// (`espectral-(menu|brand)-*.jar` -> `.disabled`).
pub fn park_espectral_jars(mods_dir: &Path) -> std::io::Result<Vec<String>> {
    let mut parked = Vec::new();
    let Ok(rd) = std::fs::read_dir(mods_dir) else { return Ok(parked) };
    for entry in rd.flatten() {
        let Some(f) = entry.file_name().to_str().map(str::to_string) else { continue };
        let lower = f.to_ascii_lowercase();
        if (lower.starts_with("espectral-menu-") || lower.starts_with("espectral-brand-")) && lower.ends_with(".jar") {
            std::fs::rename(mods_dir.join(&f), mods_dir.join(format!("{f}.disabled")))?;
            parked.push(f);
        }
    }
    Ok(parked)
}

/// Perform a version/loader change (plan recomputed; downgrade backs up
/// worlds first).
pub async fn apply_version_change(
    engine: &Engine,
    name: &str,
    target: &VersionTarget,
) -> EngineResult<VersionChangeResult> {
    let plan = plan_version_change(engine, name, target).await?;
    let mut inst = crate::launch::load_instance(engine, name).await?;

    let mut backup = None;
    if plan.backup_worlds {
        let dir = engine.paths().instance_dir(name).join("backups").join(format!(
            "saves-{}-{}",
            inst.version,
            stamp(chrono::Local::now())
        ));
        let src = crate::instances::effective_game_dir(engine, &inst).join("saves");
        let d2 = dir.clone();
        tokio::task::spawn_blocking(move || copy_dir_no_overwrite(&src, &d2))
            .await
            .map_err(|e| EngineError::other(e.to_string()))??;
        backup = Some(dir.to_string_lossy().into_owned());
    }

    inst.version = plan.to.version.clone();
    inst.loader = plan.to.loader;
    // Fabric resolves the newest stable loader at launch; NeoForge needs its build.
    inst.loader_version = plan.to.loader_version.clone().flatten();
    inst.updated_at = Some(crate::platform::now_iso());
    crate::paths::write_json_atomic(&engine.paths().instance_json(name), &inst)?;

    let mods_raw = crate::mods::apply_mods_plan(engine, name, &serde_json::to_value(&plan.mods)?).await?;
    let mods: VersionChangeModsResult = serde_json::from_value(mods_raw)?;
    if plan.espectral_menu {
        // Swaps the bundled Espectral jar for the new version and prunes the old one.
        crate::mods::ensure_branding_seeded(engine, &inst).await?;
    } else if inst.loader != Loader::Vanilla {
        // No Espectral build for this version/loader: park the old one.
        park_espectral_jars(&crate::instances::effective_mods_dir(engine, &inst))?;
    }

    let summary = crate::instances::list_instances(engine)
        .await?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("instance not found: {name}")))?;
    Ok(VersionChangeResult { summary, from: Some(plan.from), to: Some(plan.to), backup, mods })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use crate::types::VersionEntry;
    use serde_json::json;

    fn inst(version: &str, loader: Loader) -> InstanceMeta {
        InstanceMeta { name: "vc".into(), version: version.into(), loader, ..Default::default() }
    }

    #[test]
    fn same_target_is_rejected_before_any_lookup() {
        let i = inst("26.2", Loader::Fabric);
        let err = requested_target(&i, &VersionTarget { version: Some("26.2".into()), loader: Some(Loader::Fabric) })
            .unwrap_err();
        assert!(err.has_code("NO_CHANGE"));
        assert_eq!(err.status(), 400);
        assert!(requested_target(&i, &VersionTarget::default()).unwrap_err().has_code("NO_CHANGE"));
        assert!(requested_target(&i, &VersionTarget { version: Some("  ".into()), loader: None }).is_err());
        assert_eq!(
            requested_target(&i, &VersionTarget { version: None, loader: Some(Loader::Vanilla) }).unwrap(),
            ("26.2".to_string(), Loader::Vanilla)
        );
        assert_eq!(
            requested_target(&i, &VersionTarget { version: Some(" 26.3 ".into()), loader: None }).unwrap(),
            ("26.3".to_string(), Loader::Fabric)
        );
    }

    #[tokio::test]
    async fn plan_no_change_via_engine() {
        let (_d, e) = temp_engine();
        crate::paths::write_json_atomic(
            &e.paths().instance_json("vc-up"),
            &json!({ "name": "vc-up", "version": "26.2", "loader": "fabric" }),
        )
        .unwrap();
        let err = plan_version_change(
            &e,
            "vc-up",
            &VersionTarget { version: Some("26.2".into()), loader: Some(Loader::Fabric) },
        )
        .await
        .unwrap_err();
        assert!(err.has_code("NO_CHANGE"));
    }

    #[test]
    fn downgrade_uses_release_times() {
        let entry = |id: &str, t: &str| VersionEntry {
            id: id.into(),
            kind: "release".into(),
            release_time: t.into(),
            sha1: String::new(),
            url: String::new(),
        };
        let m = VersionManifest {
            latest_release: "26.3".into(),
            latest_snapshot: "26.3".into(),
            versions: vec![entry("26.3", "2026-09-15T00:00:00+00:00"), entry("26.2", "2026-06-16T00:00:00+00:00")],
        };
        assert!(is_downgrade_in(&m, "26.3", "26.2"));
        assert!(!is_downgrade_in(&m, "26.2", "26.3"));
        assert!(!is_downgrade_in(&m, "26.2", "26.2"));
        assert!(!is_downgrade_in(&m, "26.2", "9.9"), "unknown version is never a downgrade");
    }

    #[test]
    fn backup_copy_and_stamp() {
        let d = tempfile::tempdir().unwrap();
        let saves = d.path().join("saves").join("World");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::write(saves.join("level.dat"), "world-bytes").unwrap();
        let backup = d.path().join("backups").join("saves-26.3-x");
        copy_dir_no_overwrite(&d.path().join("saves"), &backup).unwrap();
        assert_eq!(std::fs::read_to_string(backup.join("World").join("level.dat")).unwrap(), "world-bytes");
        assert!(copy_dir_no_overwrite(&d.path().join("saves"), &backup).is_err(), "errorOnExist");
        let s = stamp(chrono::Local::now());
        assert_eq!(s.len(), "20260930-120000".len());
        assert_eq!(&s[8..9], "-");
    }

    #[test]
    fn parks_espectral_jars_only() {
        let d = tempfile::tempdir().unwrap();
        for f in [
            "espectral-menu-26.2-1.6.2.jar",
            "Espectral-Brand-1.21.1-1.6.2.jar",
            "sodium.jar",
            "espectral-menu-x.jar.disabled",
        ] {
            std::fs::write(d.path().join(f), "x").unwrap();
        }
        let mut parked = park_espectral_jars(d.path()).unwrap();
        parked.sort();
        assert_eq!(parked, vec!["Espectral-Brand-1.21.1-1.6.2.jar", "espectral-menu-26.2-1.6.2.jar"]);
        assert!(d.path().join("espectral-menu-26.2-1.6.2.jar.disabled").exists());
        assert!(d.path().join("sodium.jar").exists());
        assert!(park_espectral_jars(&d.path().join("missing")).unwrap().is_empty());
    }

    #[test]
    fn branding_support_from_bundled_manifest() {
        // The dev checkout ships assets/branding/branding.json.
        assert!(supports_branding("1.21.11", Loader::Fabric));
        assert!(!supports_branding("1.21.11", Loader::Neoforge));
        assert!(supports_branding("1.21.1", Loader::Neoforge));
        assert!(!supports_branding("0.0.1", Loader::Fabric));
    }
}
