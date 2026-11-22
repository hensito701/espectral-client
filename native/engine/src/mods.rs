//! Port of `src/engine/mods.mjs` — mod listing / enable-disable, pinned
//! presets (performance / QoL / branding), Modrinth search + dependency
//! planning + install, version-change mod planning.
//!
//! **Owner: content.** Status: PORTED.
//!
//! Modrinth version metadata stays raw JSON (`serde_json::Value`) like the JS:
//! only a handful of fields are read and the schema is Modrinth's. Every
//! download is SHA-1 verified by `HttpClient::download_all`. The Modrinth base
//! URL comes from `ContentSettings::modrinth_api` (tests: a mock server).
//!
//! Bundled branding jars: `<exe dir>/branding` (Windows installer),
//! `<exe dir>/resources/branding`, `<exe dir>/../branding`,
//! `<exe dir>/../lib/Espectral Client/branding` (Linux .deb), then the repo's
//! `assets/branding` (dev) — or `ContentSettings::branding_dir`.
//!
//! Error codes from routes/mods.mjs are lowercase and VERBATIM (see
//! `api::content::send_error`).
//!
//! CONTRACT (called by onboarding, game-launch launch/version-change):
//! install_preset, ensure_branding_seeded, plan_mods_for_target, apply_mods_plan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::download::{DownloadAllOptions, DownloadItem, FetchOptions};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult, HttpError, HttpErrorKind};
use crate::events::{EngineEvent, ProgressChannel, ProgressEvent};
use crate::instances::effective_mods_dir;
use crate::types::{
    CompanionChange, InstanceMeta, Loader, ModDependencyRef, ModEntry, ModInstallPlan, ModInstallPreview,
    ModInstallUpgrade, ModPreset, ModRef, ModRename, ModVersionItem, ModVersionSwitchPlan, ModVersionSwitchResult,
    ModVersionsReply, ModrinthProject, ModrinthSearchResult, PresetInstallResult, VersionTarget,
};

pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";
pub const PERFORMANCE_PRESET: &str = "performance";
pub const BRANDING_PRESET: &str = "branding";
pub const QOL_PRESET: &str = "qol";
pub const SODIUM_PROJECT_ID: &str = "AANobbMI";
pub const IRIS_PROJECT_ID: &str = "YL57xq9U";

pub const PINNED_VERSIONS: [&str; 3] = ["1.21.11", "26.2", "26.3"];
pub const PINNED_VERSION: &str = "1.21.11";

/// One researched Modrinth pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub slug: String,
    pub project_id: String,
    pub version_id: String,
    pub version_number: String,
}

type PinRow = (&'static str, &'static str, &'static str, &'static str);

// Performance pin set — researched 2026-11-11 against Modrinth (releases only,
// `loaders=["fabric"]`). Every row is the newest RELEASE build of that project
// for the MC version; betas/alphas are never pinned (Sodium 0.9.3 is beta, so
// 26.3 stays on 0.9.2 — the exact build Iris 1.11.7 declares as its Sodium
// dependency). The sha1 install verifies is the one Modrinth publishes for the
// pinned version id (immutable per version), checked live for every row below:
//   1.21.11: sodium 4a8733fe…, iris acfbe6d0…, immediatelyfast 43e45146…,
//            lithium 203bdcb2…, ferrite eac76ff0…, krypton 94ddaeea…,
//            modmenu d5612d70…, fabric-api c98467cb…
//   26.2:    sodium 62a70961…, iris bd0b2473…, immediatelyfast 02862602…,
//            lithium 435ad028…, ferrite eac76ff0…, krypton c83ed92c…,
//            modmenu 9fc3816…,  fabric-api 332da34e…
//   26.3:    sodium 9acfe851…, iris fa03640b…, immediatelyfast f565bbeb…,
//            lithium 2d3afe9d…, ferrite eac76ff0…, krypton 21b21cb8…,
//            modmenu 66ec40f1…, fabric-api 7f16cfcd…
//
// Why these eight for a PvP/UHC client (Sodium+Iris+the rest of the bundle),
// and why the tempting alternatives are not here:
// - ImmediatelyFast (LGPL-3.0-or-later) batches immediate-mode GUI/text draw
//   calls. It is the one addition with clear, upstream-documented client FPS
//   value at every pinned version and it ships in Fabulously Optimized on both
//   the 1.21.11 and 26.3 lanes. Its mixins touch GuiRenderer / GuiGraphics,
//   FontTexture, FeatureRenderDispatcher, RenderTypes, SignText, MapRenderer
//   and Minecraft (§constructor / level change). We share only Minecraft and
//   RenderTypes with it, and never the same method: our MinecraftMixin injects
//   at the tail of tick(), our HUD/cosmetic path goes through Hud, Gui, Screen,
//   ChatComponent, LivingEntityRenderer and AvatarRenderer — none of them
//   mixed by ImmediatelyFast.
// - EntityCulling rejected: not open source (tr7zw Protective
//   License forbids commercial advantage) and it rewrites the entity renderer
//   path our cosmetic layers live on.
// - ModernFix rejected: the canonical project publishes no Fabric builds for
//   26.x (they moved to the ModernFix-mVUS fork), and it is a heap/boot mod.
// - Dynamic FPS rejected: it throttles fps precisely when nobody is looking,
//   which fights AgentThrottleMixin (agent launches must never be throttled).
// - More Culling rejected: needs Cloth Config and has release gaps for 1.21.6
//   through 1.21.10, which would break dynamic resolution for those versions.
// - BadOptimizations rejected: it cancels vanilla lightmap extraction, which is
//   the exact tail our LightmapFullbrightMixin writes the fullbright factor on.
// - ScalableLux (no 26.3 release), C2ME (betas only, server chunk-gen), Noisium
//   (no 26.x, client unsupported), Sodium Extra (a video-options expansion,
//   not an optimization) rejected.
// - Krypton stays: it is LGPL-3.0, tiny, has releases for all three pins, and
//   still consolidates the client's outbound Netty flush path. It targets
//   Connection#createFrameEncoder and the compression/encryption handlers —
//   never the Bootstrap.connect / channelActive hooks Ruta Espectral uses.
//
// fabric-api is pinned to the newest release for each MC version, which is one
// patch ahead of the dev runtime in branding-mod/mc26*/build.gradle
// (0.165.0 / 0.156.0 / 0.141.6). That direction is safe — the branding mod is
// compiled against the older API and Fabric API keeps its modules stable within
// a Minecraft version — but the dev runtime should be bumped to match so the
// compile-time and ship-time surfaces stop drifting.
const PINS_1_21_11: [PinRow; 8] = [
    ("sodium", "AANobbMI", "rkdTcxoT", "mc1.21.11-0.8.14-fabric"),
    ("iris", "YL57xq9U", "ka9PDlsN", "1.10.8+1.21.11-fabric"),
    ("immediatelyfast", "5ZwdcRci", "4EwhsTu7", "1.14.3+1.21.11-fabric"),
    ("lithium", "gvQqBUqZ", "Ow7wA0kG", "mc1.21.11-0.21.4-fabric"),
    ("ferrite-core", "uXXizFIs", "Ii0gP3D8", "8.2.0-fabric"),
    ("krypton", "fQEb0iXm", "O9LmWYR7", "0.2.10"),
    ("modmenu", "mOgUt4GM", "cLkYaVuL", "17.0.1"),
    ("fabric-api", "P7dR8mSH", "6qAuTtLR", "0.141.6+1.21.11"),
];
const PINS_26_2: [PinRow; 8] = [
    ("sodium", "AANobbMI", "xJZxADzI", "mc26.2-0.9.2-fabric"),
    ("iris", "YL57xq9U", "gxZWWnKH", "1.11.4+26.2-fabric"),
    ("immediatelyfast", "5ZwdcRci", "pMWERcSu", "1.16.5+26.2-fabric"),
    ("lithium", "gvQqBUqZ", "f7vZ0VWU", "mc26.2-0.25.3-fabric"),
    ("ferrite-core", "uXXizFIs", "d5ddUdiB", "9.0.0-fabric"),
    ("krypton", "fQEb0iXm", "5WeL0Nkz", "0.3.1"),
    // modmenu 20.0.x lists placeholder-api on Modrinth as a required dependency
    // but ships no copy of it and does not declare it in fabric.mod.json's
    // `depends` (17.0.1 and 21.0.0 bundle it). JAR_IN_JAR therefore skips it —
    // the install note is expected — and the loader graph stays satisfied.
    ("modmenu", "mOgUt4GM", "iRo7AZXf", "20.0.3"),
    ("fabric-api", "P7dR8mSH", "ewUK83HI", "0.161.0+26.2"),
];
// Iris 1.11.7 requires exactly Sodium 0.9.2 (bAZQdGpg); Sodium 0.9.3 is still beta.
const PINS_26_3: [PinRow; 8] = [
    ("sodium", "AANobbMI", "bAZQdGpg", "mc26.3-0.9.2-fabric"),
    ("iris", "YL57xq9U", "vTN4NRGW", "1.11.7+26.3-fabric"),
    ("immediatelyfast", "5ZwdcRci", "3MP9UR23", "1.17.1+26.3-fabric"),
    ("lithium", "gvQqBUqZ", "fWv5bnZk", "mc26.3-0.26.3-fabric"),
    ("ferrite-core", "uXXizFIs", "d5ddUdiB", "9.0.0-fabric"),
    ("krypton", "fQEb0iXm", "UugdIYJw", "0.3.2"),
    ("modmenu", "mOgUt4GM", "kyy7dbrZ", "21.0.0"),
    ("fabric-api", "P7dR8mSH", "998R2rNP", "0.166.0+26.3"),
];
/// Legacy-optional QoL pins (gamma-utils / clear-fog).
const QOL_1_21_11: [PinRow; 2] =
    [("gamma-utils", "wdLuzzEP", "aVJkWMQl", "2.5.10"), ("clear-fog", "46n24c6r", "CjWMitqz", "1.21.11_Fabric")];
const QOL_26_2: [PinRow; 2] =
    [("gamma-utils", "wdLuzzEP", "bR4tEPFq", "3.1.1+Fabric"), ("clear-fog", "46n24c6r", "GZ2Uiwgv", "26.2_Fabric")];
const QOL_26_3: [PinRow; 2] =
    [("gamma-utils", "wdLuzzEP", "k6ywOirE", "3.2.2+Fabric"), ("clear-fog", "46n24c6r", "C8eOUsdr", "26.3_Fabric")];

/// The eight canonical performance projects (dynamic resolution for versions
/// without an exact pin table — every one has a release build for 1.21 through
/// 26.x, which is what keeps this list safe to resolve at runtime).
const PERFORMANCE_PROJECTS: [(&str, &str); 8] = [
    ("sodium", "AANobbMI"),
    ("iris", "YL57xq9U"),
    ("immediatelyfast", "5ZwdcRci"),
    ("lithium", "gvQqBUqZ"),
    ("ferrite-core", "uXXizFIs"),
    ("krypton", "fQEb0iXm"),
    ("modmenu", "mOgUt4GM"),
    ("fabric-api", "P7dR8mSH"),
];
const QOL_PROJECTS: [(&str, &str); 2] = [("gamma-utils", "wdLuzzEP"), ("clear-fog", "46n24c6r")];

/// Jar-in-jar bundled dependencies (placeholder-api inside modmenu).
const JAR_IN_JAR: [&str; 1] = ["eXts2L7r"];

fn to_pins(rows: &[PinRow]) -> Vec<Pin> {
    rows.iter()
        .map(|(s, p, v, n)| Pin {
            slug: s.to_string(),
            project_id: p.to_string(),
            version_id: v.to_string(),
            version_number: n.to_string(),
        })
        .collect()
}

/// PINS_BY_VERSION[version] (exact researched performance pins).
pub fn pins_by_version(version: &str) -> Option<Vec<Pin>> {
    match version {
        "1.21.11" => Some(to_pins(&PINS_1_21_11)),
        "26.2" => Some(to_pins(&PINS_26_2)),
        "26.3" => Some(to_pins(&PINS_26_3)),
        _ => None,
    }
}

/// PINS_QOL_BY_VERSION[version].
pub fn qol_pins_by_version(version: &str) -> Option<Vec<Pin>> {
    match version {
        "1.21.11" => Some(to_pins(&QOL_1_21_11)),
        "26.2" => Some(to_pins(&QOL_26_2)),
        "26.3" => Some(to_pins(&QOL_26_3)),
        _ => None,
    }
}

fn mod_progress(engine: &Engine, p: ProgressEvent) {
    engine.emit(EngineEvent::ModProgress(p));
}

/// JS `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn api(engine: &Engine) -> String {
    engine.content_settings().modrinth_api.trim_end_matches('/').to_string()
}

fn code_err(status: u16, code: &str, message: impl Into<String>) -> EngineError {
    EngineError::api(status, code, message)
}

// ---------------------------------------------------------------------------
// Version support (pure)
// ---------------------------------------------------------------------------

/// Release ids in [1.21, 26.x]: `1.2X[.patch]` (minor 21..=26) or `NN[.minor]` (21..=26).
pub fn supports_performance_bundle(version: &str) -> bool {
    if version.is_empty() {
        return false;
    }
    let b = version.as_bytes();
    if b.len() >= 4 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b'w' && b[3].is_ascii_digit() {
        return false; // snapshots (25w11a)
    }
    let parts: Vec<&str> = version.split('.').collect();
    if parts.iter().any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit())) {
        return false;
    }
    let num = |s: &str| s.parse::<u64>().unwrap_or(u64::MAX);
    if parts[0] == "1" && parts.len() >= 2 {
        let minor = num(parts[1]);
        return (21..=26).contains(&minor);
    }
    (21..=26).contains(&num(parts[0]))
}

pub fn pin_note_for_version(version: &str) -> Option<String> {
    if supports_performance_bundle(version) {
        None
    } else {
        Some(format!("no performance mod bundle for {version}; the bundle covers release versions 1.21 through 26.x"))
    }
}

/// Pin set for a version: exact pins, or dynamically resolved (None when
/// unsupported or any project lacks a build; never fails).
pub async fn pins_for_version(engine: &Engine, version: &str) -> Option<Vec<Pin>> {
    if let Some(p) = pins_by_version(version) {
        return Some(p);
    }
    if !supports_performance_bundle(version) {
        return None;
    }
    resolve_dynamic_pins(engine, version, &PERFORMANCE_PROJECTS).await
}

/// QoL bundle pins (exact or dynamic).
pub async fn qol_pins_for_version(engine: &Engine, version: &str) -> Option<Vec<Pin>> {
    if let Some(p) = qol_pins_by_version(version) {
        return Some(p);
    }
    if !supports_performance_bundle(version) {
        return None;
    }
    resolve_dynamic_pins(engine, version, &QOL_PROJECTS).await
}

async fn resolve_dynamic_pins(engine: &Engine, version: &str, projects: &[(&str, &str)]) -> Option<Vec<Pin>> {
    let resolved = futures_util::future::join_all(projects.iter().map(|(slug, pid)| async move {
        let meta = best_version_for_cached(engine, pid, version, "fabric").await?;
        Some(Pin {
            slug: slug.to_string(),
            project_id: pid.to_string(),
            version_id: s(&meta, "id")?,
            version_number: s(&meta, "version_number").unwrap_or_default(),
        })
    }))
    .await;
    resolved.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Branding preset (bundled jars)
// ---------------------------------------------------------------------------

/// Pin-shaped branding entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandingPin {
    pub slug: String,
    pub filename: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub version_number: Option<String>,
}

/// The bundled branding jars dir (first existing candidate, else the last).
pub fn branding_dir(engine: &Engine) -> PathBuf {
    if let Some(d) = engine.content_settings().branding_dir {
        return d;
    }
    let mut candidates = vec![];
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        candidates.push(exe_dir.join("branding"));
        candidates.push(exe_dir.join("resources").join("branding"));
        if let Some(up) = exe_dir.parent() {
            candidates.push(up.join("branding"));
            // Linux .deb: /usr/bin/espectral-client + /usr/lib/Espectral Client/branding
            candidates.push(up.join("lib").join("Espectral Client").join("branding"));
        }
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    candidates.push(repo.join("resources").join("branding"));
    candidates.push(repo.join("assets").join("branding"));
    candidates.iter().find(|d| d.join("branding.json").exists()).cloned().unwrap_or_else(|| candidates.pop().unwrap())
}

fn branding_manifest(engine: &Engine) -> Option<Value> {
    let dir = branding_dir(engine);
    let state = engine.content_state();
    if let Some(m) = state.branding_manifest.lock().unwrap_or_else(|p| p.into_inner()).get(&dir) {
        return m.clone();
    }
    let m = std::fs::read(dir.join("branding.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    state.branding_manifest.lock().unwrap_or_else(|p| p.into_inner()).insert(dir, m.clone());
    m
}

/// Minecraft versions covered by the bundled brand jars (any loader).
pub fn branding_versions(engine: &Engine) -> Vec<String> {
    branding_manifest(engine)
        .and_then(|m| m.get("mods").and_then(Value::as_object).map(|o| o.keys().cloned().collect()))
        .unwrap_or_default()
}

/// Branding pin for version + loader (None when unsupported).
pub fn branding_pin_for_version(engine: &Engine, version: &str, loader: Loader) -> Option<BrandingPin> {
    let manifest = branding_manifest(engine)?;
    let entry = manifest.get("mods")?.get(version)?;
    let loaders: Vec<String> = match entry.get("loaders").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        None => vec!["fabric".into()],
    };
    if !loaders.iter().any(|l| l == loader.as_str()) {
        return None;
    }
    Some(BrandingPin {
        slug: s(entry, "mod_id").or_else(|| s(&manifest, "mod_id")).unwrap_or_default(),
        filename: s(entry, "filename")?,
        sha1: s(entry, "sha1"),
        size: entry.get("size").and_then(Value::as_u64),
        version_number: s(&manifest, "mod_version"),
    })
}

/// True when a bundled brand jar exists for this version + loader.
pub fn supports_branding(engine: &Engine, version: &str, loader: Loader) -> bool {
    branding_pin_for_version(engine, version, loader).is_some()
}

/// Mod id of the full Espectral mod: the Suite (Right Shift), zoom,
/// fullbright, no fog, HUD, cosmetics and the title menu. The NeoForge jar is
/// `espectral-brand`, a brand handshake with none of those features.
pub const SUITE_MOD_ID: &str = "espectral-menu";

/// True when this version + loader gets the full Espectral mod.
pub fn supports_suite(engine: &Engine, version: &str, loader: Loader) -> bool {
    branding_pin_for_version(engine, version, loader).is_some_and(|p| p.slug == SUITE_MOD_ID)
}

/// Every (loader, version) the bundled jars give the full Espectral mod,
/// grouped by loader (Fabric, Forge, NeoForge), newest version first.
pub fn suite_targets(engine: &Engine) -> Vec<(Loader, String)> {
    let Some(manifest) = branding_manifest(engine) else { return vec![] };
    let Some(mods) = manifest.get("mods").and_then(Value::as_object) else { return vec![] };
    let top_id = s(&manifest, "mod_id");
    let mut out: Vec<(Loader, String)> = vec![];
    for (version, entry) in mods {
        if s(entry, "mod_id").or_else(|| top_id.clone()).as_deref() != Some(SUITE_MOD_ID) {
            continue;
        }
        let loaders: Vec<Loader> = match entry.get("loaders").and_then(Value::as_array) {
            Some(a) => a.iter().filter_map(Value::as_str).filter_map(Loader::parse).collect(),
            None => vec![Loader::Fabric],
        };
        out.extend(loaders.into_iter().map(|l| (l, version.clone())));
    }
    let loader_rank = |l: Loader| match l {
        Loader::Fabric => 0,
        Loader::Forge => 1,
        Loader::Neoforge => 2,
        Loader::Vanilla => 3,
    };
    let numeric = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    out.sort_by(|a, b| loader_rank(a.0).cmp(&loader_rank(b.0)).then_with(|| numeric(&b.1).cmp(&numeric(&a.1))));
    out
}

fn branding_note(engine: &Engine, version: &str, loader: Loader) -> String {
    let versions = branding_versions(engine).join(", ");
    let covers = if versions.is_empty() { "nothing".to_string() } else { versions };
    format!("no bundled brand mod for {version} on {loader}; branding covers {covers}")
}

fn is_in_use(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    match e.raw_os_error() {
        #[cfg(windows)]
        Some(5 | 32 | 33) => true,
        #[cfg(unix)]
        Some(1 | 13 | 16) => true,
        _ => false,
    }
}

async fn install_branding_preset(
    engine: &Engine,
    instance: &InstanceMeta,
    mods_dir: &Path,
) -> EngineResult<PresetInstallResult> {
    let ev = |phase: &str| ProgressEvent {
        preset: Some(BRANDING_PRESET.into()),
        ..ProgressEvent::for_instance(&instance.name).phase(phase).total(1)
    };
    let Some(pin) = branding_pin_for_version(engine, &instance.version, instance.loader) else {
        let note = branding_note(engine, &instance.version, instance.loader);
        tracing::warn!("[mods] {note}");
        mod_progress(
            engine,
            ProgressEvent { note: Some(note.clone()), done: Some(1), ..ev("error").index(1).message(&note) },
        );
        return Ok(PresetInstallResult { installed: vec![], notes: vec![note] });
    };
    let src = branding_dir(engine).join(&pin.filename);
    let dest = mods_dir.join(&pin.filename);
    mod_progress(
        engine,
        ProgressEvent {
            done: Some(0),
            ..ev("start").filename(&pin.filename).index(0).message(format!("installing {}", pin.filename))
        },
    );
    let _ = std::fs::remove_file(format!("{}.disabled", dest.to_string_lossy()));
    let data = std::fs::read(&src)?;
    let src_sha1 = crate::download::sha1_hex(&data);
    if let Some(expected) = &pin.sha1 {
        if &src_sha1 != expected {
            return Err(code_err(
                500,
                "branding_sha1_mismatch",
                format!("branding source sha1 mismatch for {}: {src_sha1} != {expected}", pin.filename),
            ));
        }
    }
    // Write beside the jar and rename over it — never in place: a running
    // game holds the jar open and an in-place write swaps bytes under the JVM.
    let tmp = PathBuf::from(format!("{}.{}.tmp", dest.to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, &data)?;
    let tmp_sha1 = crate::download::sha1_file(&tmp).await?;
    if tmp_sha1 != src_sha1 {
        let _ = std::fs::remove_file(&tmp);
        return Err(code_err(
            500,
            "branding_sha1_mismatch",
            format!("branding install failed sha1 verification: {tmp_sha1}"),
        ));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        if is_in_use(&e) {
            return Err(code_err(
                500,
                "branding_in_use",
                format!("{} is in use by a running game; it will be updated on the next launch", pin.filename),
            ));
        }
        return Err(e.into());
    }
    mod_progress(
        engine,
        ProgressEvent {
            done: Some(1),
            ..ev("done").filename(&pin.filename).index(1).message(format!("installed {}", pin.filename))
        },
    );
    Ok(PresetInstallResult { installed: vec![pin.filename], notes: vec![] })
}

/// Retro-seed the branding jar for instances that predate the preset (best-effort, never fails).
pub async fn ensure_branding_seeded(engine: &Engine, instance: &InstanceMeta) -> EngineResult<()> {
    if let Err(e) = ensure_branding_seeded_inner(engine, instance).await {
        tracing::warn!("[mods] ensureBrandingSeeded failed for {}: {e}", instance.name);
    }
    Ok(())
}

fn is_brand_jar(f: &str) -> bool {
    let l = f.to_ascii_lowercase();
    (l.starts_with("espectral-menu-") || l.starts_with("espectral-brand-"))
        && (l.ends_with(".jar") || l.ends_with(".jar.disabled"))
}

async fn ensure_branding_seeded_inner(engine: &Engine, instance: &InstanceMeta) -> EngineResult<()> {
    if !matches!(instance.loader, Loader::Fabric | Loader::Neoforge | Loader::Forge) {
        return Ok(());
    }
    let Some(pin) = branding_pin_for_version(engine, &instance.version, instance.loader) else { return Ok(()) };
    let mods_dir = effective_mods_dir(engine, instance);
    let dest = mods_dir.join(&pin.filename);
    let disabled_twin = format!("{}.disabled", pin.filename);
    let stale: Vec<String> = read_names(&mods_dir)
        .into_iter()
        .filter(|f| *f != pin.filename && *f != disabled_twin && is_brand_jar(f))
        .collect();
    let fresh = if dest.exists() {
        match crate::download::sha1_file(&dest).await {
            Ok(h) => pin.sha1.as_ref().map(|p| *p == h).unwrap_or(true),
            Err(_) => false,
        }
    } else {
        false
    };
    if !fresh {
        install_branding_preset(engine, instance, &mods_dir).await?;
    }
    for f in stale {
        let _ = std::fs::remove_file(mods_dir.join(&f));
        tracing::info!("[mods] pruned stale branding jar {f} from {}", instance.name);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Modrinth fetch helpers
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// fetchJsonWithRetry: 2 retries with linear backoff on top of the engine's
/// own retrying fetch; the final error carries the last status (else 502).
async fn fetch_json_with_retry(engine: &Engine, url: &str, retries: u32) -> EngineResult<Value> {
    let backoff = engine.content_settings().modrinth_retry_backoff;
    let opts = FetchOptions { timeout: std::time::Duration::from_secs(30), ..Default::default() };
    let mut last: Option<EngineError> = None;
    for attempt in 0..=retries {
        match engine.http().fetch_json::<Value>(url, &opts).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                if attempt < retries {
                    tokio::time::sleep(backoff * (attempt + 1)).await;
                }
            }
        }
    }
    let last = last.expect("at least one attempt");
    let status = match &last {
        EngineError::Http(h) => h.status(),
        EngineError::Api { status, .. } => Some(*status),
        _ => None,
    }
    .unwrap_or(502);
    Err(EngineError::Http(HttpError {
        kind: HttpErrorKind::Status(status),
        url: url.to_string(),
        message: format!("fetch failed for {url}: {last}"),
    }))
}

/// Modrinth version metadata (cached per process).
async fn version_meta(engine: &Engine, version_id: &str) -> EngineResult<Value> {
    if let Some(v) = engine.content_state().version_meta.lock().unwrap_or_else(|p| p.into_inner()).get(version_id) {
        return Ok(v.clone());
    }
    let url = format!("{}/version/{}", api(engine), encode_uri_component(version_id));
    let meta = fetch_json_with_retry(engine, &url, 2).await?;
    engine
        .content_state()
        .version_meta
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(version_id.into(), meta.clone());
    Ok(meta)
}

/// Newest listed/approved version of a project for game version + loader.
async fn best_version_for(
    engine: &Engine,
    project_id: &str,
    game_version: &str,
    loader: &str,
) -> EngineResult<Option<Value>> {
    let q = format!(
        "game_versions={}&loaders={}",
        encode_uri_component(&json!([game_version]).to_string()),
        encode_uri_component(&json!([loader]).to_string())
    );
    let url = format!("{}/project/{}/version?{q}", api(engine), encode_uri_component(project_id));
    let list = fetch_json_with_retry(engine, &url, 2).await?;
    Ok(list.as_array().and_then(|a| {
        a.iter().find(|v| matches!(v.get("status").and_then(Value::as_str), Some("listed") | Some("approved"))).cloned()
    }))
}

/// Cached best_version_for; fetch failures are NOT cached (returns None).
async fn best_version_for_cached(engine: &Engine, project_id: &str, game_version: &str, loader: &str) -> Option<Value> {
    let key = format!("{project_id}\u{0}{game_version}\u{0}{loader}");
    if let Some(v) = engine.content_state().best_version.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return v.clone();
    }
    let meta = best_version_for(engine, project_id, game_version, loader).await.ok()?;
    engine.content_state().best_version.lock().unwrap_or_else(|p| p.into_inner()).insert(key, meta.clone());
    meta
}

/// The primary file of a version (else the first).
fn primary_file(meta: &Value) -> Option<&Value> {
    let files = meta.get("files")?.as_array()?;
    files.iter().find(|f| f.get("primary") == Some(&Value::Bool(true))).or_else(|| files.first())
}

fn file_name(f: &Value) -> String {
    s(f, "filename").unwrap_or_default()
}

fn file_sha1(f: &Value) -> Option<String> {
    f.get("hashes").and_then(|h| s(h, "sha1"))
}

fn no_file_error(meta: &Value) -> EngineError {
    EngineError::other(format!("version {} has no downloadable file", s(meta, "id").unwrap_or_default()))
}

fn read_names(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![],
    }
}

fn is_mod_file(f: &str) -> bool {
    f.ends_with(".jar") || f.ends_with(".jar.disabled")
}

// ---------------------------------------------------------------------------
// List / enable-disable
// ---------------------------------------------------------------------------

/// ModEntry[] for an instance: on-disk jars joined with pin metadata, then
/// not-installed pins, branding jar tagged. Uninstalled-pin rows need Modrinth.
pub async fn list_mods(engine: &Engine, instance_name: &str) -> EngineResult<Vec<ModEntry>> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;

    let pins = pins_for_version(engine, &instance.version).await.unwrap_or_default();
    if pins.is_empty() {
        if let Some(note) = pin_note_for_version(&instance.version) {
            tracing::warn!("[mods] {note}");
        }
    }
    let metas: Vec<Option<Value>> =
        futures_util::future::join_all(pins.iter().map(|p| async { version_meta(engine, &p.version_id).await.ok() }))
            .await;
    let mut file_by_version: IndexMap<String, Value> = IndexMap::new();
    let mut pin_filename_to_version: IndexMap<String, String> = IndexMap::new();
    for (p, meta) in pins.iter().zip(&metas) {
        if let Some(file) = meta.as_ref().and_then(primary_file) {
            file_by_version.insert(p.version_id.clone(), file.clone());
            pin_filename_to_version.insert(file_name(file), p.version_id.clone());
        }
    }
    let meta_of = |vid: &str| pins.iter().position(|p| p.version_id == vid).and_then(|i| metas[i].clone());

    let mut on_disk: Vec<String> = read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f)).collect();
    on_disk.sort();
    let mut entries = vec![];
    for file in on_disk.clone() {
        let disabled = file.ends_with(".jar.disabled");
        let base = if disabled { file[..file.len() - ".disabled".len()].to_string() } else { file.clone() };
        let abs = mods_dir.join(&file);
        let (mut sha1, mut size) = (None, None);
        if let Ok(st) = std::fs::metadata(&abs) {
            size = Some(st.len());
            sha1 = crate::download::sha1_file(&abs).await.ok();
        }
        let fabric_meta = crate::fabric_compat::read_fabric_jar(&abs, &base, false);
        let version_id = pin_filename_to_version.get(&base).cloned();
        let pin = version_id.as_ref().and_then(|v| pins.iter().find(|p| &p.version_id == v));
        entries.push(ModEntry {
            filename: base,
            project_slug: pin.map(|p| p.slug.clone()),
            version_number: pin
                .map(|p| {
                    meta_of(&p.version_id)
                        .and_then(|m| s(&m, "version_number"))
                        .unwrap_or_else(|| p.version_number.clone())
                })
                .or_else(|| {
                    fabric_meta
                        .as_ref()
                        .map(|f| crate::fabric_compat::clean_version_display(&f.root.version).to_string())
                        .filter(|v| !v.is_empty())
                }),
            version_id,
            sha1,
            size,
            enabled: !disabled,
            installed: true,
            project_id: pin.map(|p| p.project_id.clone()),
            mod_id: fabric_meta.as_ref().map(|f| f.root.id.clone()),
            display_name: fabric_meta.as_ref().map(|f| f.root.name.clone()),
        });
    }
    // Enrich on-disk non-branding jars with cached Modrinth project_id/version_id
    // (one batched POST /version_files for uncached hashes; gracefully skipped offline).
    let lookup_hashes: Vec<String> = entries
        .iter()
        .filter(|e| e.installed && !is_brand_jar(&e.filename) && e.project_id.is_none())
        .filter_map(|e| e.sha1.clone())
        .collect();
    if !lookup_hashes.is_empty() {
        let by_hash = cached_version_files(engine, &lookup_hashes).await;
        for e in &mut entries {
            if e.project_id.is_some() || is_brand_jar(&e.filename) {
                continue;
            }
            if let Some(Some(meta)) = e.sha1.as_deref().and_then(|h| by_hash.get(h)) {
                e.project_id = s(meta, "project_id");
                if e.version_id.is_none() {
                    e.version_id = s(meta, "id");
                }
                if let Some(vn) = s(meta, "version_number") {
                    e.version_number = Some(vn);
                }
            }
        }
    }
    let seen: HashSet<String> = entries.iter().filter_map(|e| e.version_id.clone()).collect();
    // A pin whose project is already on disk as another build (the set moved
    // from sodium 0.9.1 to 0.9.2) is not a missing mod: listing it as a
    // disabled row offered a switch for a file that doesn't exist.
    let missing: Vec<&Pin> = pins.iter().filter(|p| !seen.contains(&p.version_id)).collect();
    let present_projects = if missing.is_empty() {
        None
    } else {
        let jars: Vec<String> = on_disk.iter().filter(|f| !f.ends_with(".disabled")).cloned().collect();
        installed_project_ids(engine, &mods_dir, &jars).await
    };
    for p in missing {
        // By sha1 through Modrinth; the filename guess only when that lookup
        // is unavailable (offline).
        let on_disk_build = match &present_projects {
            Some(ids) => ids.contains(&p.project_id),
            None => on_disk.iter().any(|f| is_filename_for_slug(f, &p.slug)),
        };
        if on_disk_build {
            continue;
        }
        let meta = meta_of(&p.version_id);
        let file = file_by_version.get(&p.version_id);
        entries.push(ModEntry {
            filename: file.map(file_name).unwrap_or_else(|| format!("{}.jar", p.slug)),
            project_slug: Some(p.slug.clone()),
            version_number: Some(
                meta.and_then(|m| s(&m, "version_number")).unwrap_or_else(|| p.version_number.clone()),
            ),
            version_id: Some(p.version_id.clone()),
            sha1: file.and_then(file_sha1),
            size: file.and_then(|f| f.get("size").and_then(Value::as_u64)),
            enabled: false,
            installed: false,
            project_id: Some(p.project_id.clone()),
            mod_id: None,
            display_name: None,
        });
    }
    if let Some(bp) = branding_pin_for_version(engine, &instance.version, instance.loader) {
        if let Some(e) = entries.iter_mut().find(|e| e.filename == bp.filename) {
            e.project_slug = Some(bp.slug.clone());
            e.version_number = bp.version_number.clone();
        }
    }
    Ok(entries)
}

/// `<base>` of a safe mod filename (400 `invalid_filename`).
fn assert_safe_filename(filename: &str) -> EngineResult<String> {
    let base = filename.strip_suffix(".jar.disabled").or_else(|| filename.strip_suffix(".jar"));
    let ok = base.map(|b| {
        !b.is_empty()
            && b.encode_utf16().count() <= 120
            && !b.contains("..")
            && b.chars().all(|c| c.is_ascii_alphanumeric() || " _.+@-".contains(c))
    });
    match (base, ok) {
        (Some(b), Some(true)) => Ok(b.to_string()),
        _ => Err(code_err(400, "invalid_filename", format!("invalid mod filename: {filename}"))),
    }
}

/// Rename `<base>.jar` <-> `<base>.jar.disabled` -> `(filename, enabled)`.
pub async fn set_mod_enabled(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
    enabled: bool,
) -> EngineResult<String> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let base = assert_safe_filename(filename)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let jar = mods_dir.join(format!("{base}.jar"));
    let disabled = mods_dir.join(format!("{base}.jar.disabled"));
    let (src, dst) = if enabled { (&disabled, &jar) } else { (&jar, &disabled) };
    let (src_exists, dst_exists) = (src.exists(), dst.exists());
    if !src_exists && !dst_exists {
        // A pinned mod listed as not installed: turning it on downloads it.
        if enabled && install_missing_pin(engine, &instance, &mods_dir, &format!("{base}.jar")).await? {
            return Ok(format!("{base}.jar"));
        }
        return Err(code_err(404, "mod_not_found", format!("mod not found: {filename}")));
    }
    if src_exists {
        if dst_exists {
            let _ = std::fs::remove_file(dst);
        }
        std::fs::rename(src, dst)?;
    }
    Ok(format!("{base}.jar"))
}

/// Download the pinned build of this version whose file is `fname` into
/// `mods_dir`; false when no pin has that file.
async fn install_missing_pin(
    engine: &Engine,
    instance: &InstanceMeta,
    mods_dir: &Path,
    fname: &str,
) -> EngineResult<bool> {
    let pins = pins_for_version(engine, &instance.version).await.unwrap_or_default();
    // Only pins whose slug appears in the file name (ignoring separators:
    // `ferritecore-…jar` is `ferrite-core`) cost a version lookup; any other
    // name is a plain 404 without touching the network.
    let squash = |t: &str| t.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    let wanted = squash(fname);
    for p in pins.iter().filter(|p| wanted.contains(&squash(&p.slug))) {
        let Ok(meta) = version_meta(engine, &p.version_id).await else { continue };
        let Some(file) = primary_file(&meta) else { continue };
        if file_name(file) != fname {
            continue;
        }
        let name = safe_download_name(file)?;
        let item = DownloadItem::new(s(file, "url").unwrap_or_default(), mods_dir.join(&name))
            .sha1(file_sha1(file))
            .size(file.get("size").and_then(Value::as_u64))
            .event(ProgressChannel::Mod, ProgressEvent::for_instance(&instance.name).filename(&name).index(0).total(1));
        engine.http().download_all(vec![item], DownloadAllOptions::default()).await?;
        tracing::info!("[mods] installed pinned {name} in {}", instance.name);
        return Ok(true);
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// Install a preset into the instance's effective mods dir (mod-progress events).
pub async fn install_preset(
    engine: &Engine,
    instance_name: &str,
    preset: ModPreset,
) -> EngineResult<PresetInstallResult> {
    match install_preset_inner(engine, instance_name, preset).await {
        Ok(r) => Ok(r),
        Err(e) => {
            mod_progress(
                engine,
                ProgressEvent {
                    preset: Some(preset.as_str().into()),
                    error: Some(e.to_string()),
                    ..ProgressEvent::for_instance(instance_name).phase("error").message(e.to_string())
                },
            );
            Err(e)
        }
    }
}

async fn install_preset_inner(
    engine: &Engine,
    instance_name: &str,
    preset: ModPreset,
) -> EngineResult<PresetInstallResult> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let pname = preset.as_str();
    let note_event = |note: &str| ProgressEvent {
        preset: Some(pname.into()),
        note: Some(note.into()),
        ..ProgressEvent::for_instance(instance_name).phase("error").message(note)
    };
    if preset != ModPreset::Branding && instance.loader != Loader::Fabric {
        let note = format!(
            "preset '{pname}' needs a Fabric instance (this one is '{}'); integrated fullbright/no-fog are available under Settings → Client",
            instance.loader
        );
        mod_progress(engine, note_event(&note));
        return Ok(PresetInstallResult { installed: vec![], notes: vec![note] });
    }
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    if preset == ModPreset::Branding {
        return install_branding_preset(engine, &instance, &mods_dir).await;
    }

    let pins = if preset == ModPreset::Qol {
        qol_pins_for_version(engine, &instance.version).await
    } else {
        pins_for_version(engine, &instance.version).await
    };
    let Some(pins) = pins else {
        // Deliberate difference: the JS sent a null note for a supported
        // version whose dynamic pins could not be resolved.
        let note = pin_note_for_version(&instance.version).unwrap_or_else(|| {
            format!("could not resolve the {pname} mods for {} (Modrinth unreachable?)", instance.version)
        });
        tracing::warn!("[mods] {note}");
        mod_progress(engine, note_event(&note));
        return Ok(PresetInstallResult { installed: vec![], notes: vec![note] });
    };

    mod_progress(
        engine,
        ProgressEvent {
            preset: Some(pname.into()),
            ..ProgressEvent::for_instance(instance_name)
                .phase("start")
                .total(pins.len() as u64)
                .index(0)
                .message(format!("installing {} pinned mods", pins.len()))
        },
    );

    let metas = futures_util::future::try_join_all(pins.iter().map(|p| version_meta(engine, &p.version_id))).await?;
    let mut install_map: IndexMap<String, Value> = IndexMap::new();
    for (p, m) in pins.iter().zip(metas) {
        install_map.insert(p.version_id.clone(), m);
    }
    let mut notes = vec![];
    let mut qi = 0;
    while qi < install_map.len() {
        let meta = install_map[qi].clone();
        qi += 1;
        for dep in meta.get("dependencies").and_then(Value::as_array).cloned().unwrap_or_default() {
            if dep.get("dependency_type").and_then(Value::as_str) != Some("required") {
                continue;
            }
            // One build per project. Iris names the exact Sodium build it was
            // released against (1.9.6+1.21.8 -> 0.7.2) while the set pins the
            // newest one (0.7.3): adding the dependency's build as well put two
            // Sodium jars in mods/ and Fabric refuses to start with a duplicate
            // mod id. Iris itself only asks Fabric for `sodium 0.7.x`, so the
            // build already in the set satisfies it.
            let pid = s(&dep, "project_id").unwrap_or_default();
            if !pid.is_empty()
                && (pins.iter().any(|p| p.project_id == pid)
                    || install_map.values().any(|m| s(m, "project_id").as_deref() == Some(pid.as_str())))
            {
                continue;
            }
            if let Some(vid) = s(&dep, "version_id") {
                if !install_map.contains_key(&vid) {
                    let m = version_meta(engine, &vid).await?;
                    install_map.insert(vid, m);
                }
                continue;
            }
            if JAR_IN_JAR.contains(&pid.as_str()) {
                notes.push(format!("dependency {pid} is jar-in-jar bundled; skipped"));
                continue;
            }
            match best_version_for(engine, &pid, &instance.version, "fabric").await? {
                Some(best) => {
                    let id = s(&best, "id").unwrap_or_default();
                    install_map.entry(id).or_insert(best);
                }
                None => notes.push(format!("could not resolve dependency {pid} for {}", instance.version)),
            }
        }
    }

    let total = install_map.len();
    let mut items = vec![];
    let mut installed = vec![];
    for (i, meta) in install_map.values().enumerate() {
        let file = primary_file(meta).ok_or_else(|| no_file_error(meta))?;
        let fname = safe_download_name(file)?;
        installed.push(fname.clone());
        items.push(
            DownloadItem::new(s(file, "url").unwrap_or_default(), mods_dir.join(&fname))
                .sha1(file_sha1(file))
                .size(file.get("size").and_then(Value::as_u64))
                .event(
                    ProgressChannel::Mod,
                    ProgressEvent {
                        preset: Some(pname.into()),
                        ..ProgressEvent::for_instance(instance_name)
                            .filename(&fname)
                            .index(i as u64)
                            .total(total as u64)
                    },
                ),
        );
    }
    engine.http().download_all(items, DownloadAllOptions::default()).await?;
    // install_preset is authoritative for its own projects: a version bump
    // renames the jar (sodium-fabric-0.8.7… -> …-0.8.14…), and leaving the old
    // file enabled would load two Sodium jars (duplicate mod id -> hard crash).
    // Superseded jars are DISABLED, not deleted, so the player can re-enable a
    // build they installed themselves. They are identified by sha1 through
    // Modrinth, not by filename: `ferritecore-9.0.0-fabric.jar` does not
    // contain the `ferrite-core` slug, and a filename heuristic could touch a
    // jar that is not ours. When the lookup is unavailable nothing is touched —
    // a stale jar is annoying, a wrongly disabled mod is not.
    for name in &installed {
        let _ = std::fs::remove_file(mods_dir.join(format!("{name}.disabled")));
    }
    let mut candidates: Vec<(String, String)> = vec![];
    for file in read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f)) {
        if file.ends_with(".disabled") {
            continue;
        }
        if installed.iter().any(|n| n == &file) {
            continue;
        }
        if let Ok(hash) = crate::download::sha1_file(&mods_dir.join(&file)).await {
            candidates.push((file, hash));
        }
    }
    if !candidates.is_empty() {
        let hashes: Vec<String> = candidates.iter().map(|(_, h)| h.clone()).collect();
        if let Ok(by_hash) = post_version_files(engine, &hashes).await {
            for file in superseded_pinned_jars(&candidates, &by_hash, &pins) {
                if std::fs::rename(mods_dir.join(file), mods_dir.join(format!("{file}.disabled"))).is_ok() {
                    tracing::info!("[mods] disabled superseded pinned jar {file} in {}", instance.name);
                }
            }
        }
    }
    mod_progress(
        engine,
        ProgressEvent {
            preset: Some(pname.into()),
            ..ProgressEvent::for_instance(instance_name)
                .phase("done")
                .total(total as u64)
                .index(total as u64)
                .message(format!("installed {total} mods"))
        },
    );
    Ok(PresetInstallResult { installed, notes })
}

/// Filenames among `candidates` (name, sha1) that belong to a project the
/// preset already installed a newer build of, given Modrinth's hash ->
/// version map. Pure so the rule can be tested without a download.
fn superseded_pinned_jars<'a>(
    candidates: &'a [(String, String)],
    by_hash: &Map<String, Value>,
    pins: &[Pin],
) -> Vec<&'a str> {
    candidates
        .iter()
        .filter(|(_, hash)| {
            by_hash
                .get(hash)
                .and_then(|v| s(v, "project_id"))
                .is_some_and(|pid| pins.iter().any(|p| p.project_id == pid))
        })
        .map(|(file, _)| file.as_str())
        .collect()
}

/// Fabric refuses to boot when two enabled jars in `mods/` declare the same
/// mod id. Launchers up to 1.8.3 put two Sodium builds into the dynamically
/// pinned 1.21.2–1.21.8 instances (the preset's newest plus the one Iris
/// names), so a launch disables every older duplicate and keeps the newest.
/// Only duplicates whose versions both parse as semver are touched; anything
/// else is left for Fabric to report. Reads only each jar's central directory
/// and `fabric.mod.json`. Returns `(disabled, kept)` filename pairs.
pub fn disable_duplicate_mod_ids(mods_dir: &Path) -> Vec<(String, String)> {
    let mut by_id: IndexMap<String, Vec<(semver::Version, String)>> = IndexMap::new();
    let mut names: Vec<String> = read_names(mods_dir).into_iter().filter(|f| f.ends_with(".jar")).collect();
    names.sort();
    for name in names {
        let Some((id, version)) = fabric_mod_identity(&mods_dir.join(&name)) else { continue };
        if let Ok(v) = semver::Version::parse(&version) {
            by_id.entry(id).or_default().push((v, name));
        }
    }
    let mut out = vec![];
    for (_, mut jars) in by_id.into_iter().filter(|(_, j)| j.len() > 1) {
        jars.sort_by(|a, b| b.0.cmp_precedence(&a.0));
        let kept = jars[0].1.clone();
        for (_, stale) in &jars[1..] {
            if std::fs::rename(mods_dir.join(stale), mods_dir.join(format!("{stale}.disabled"))).is_ok() {
                out.push((stale.clone(), kept.clone()));
            }
        }
    }
    out
}

/// `(id, version)` from a jar's `fabric.mod.json` (None when absent/unreadable).
fn fabric_mod_identity(jar: &Path) -> Option<(String, String)> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar).ok()?).ok()?;
    let mut raw = String::new();
    zip.by_name("fabric.mod.json").ok()?.take(1 << 20).read_to_string(&mut raw).ok()?;
    let meta: Value = serde_json::from_str(&raw).ok()?;
    Some((s(&meta, "id")?, s(&meta, "version")?))
}

// ---------------------------------------------------------------------------
// Capture tools: Replay Mod + performance pins + default shader pack
// ---------------------------------------------------------------------------
//
// Replay Mod is GPL-3: it is downloaded from Modrinth at install time and
// NEVER bundled into the MIT espectral-menu jar or the app payload.

/// Modrinth project id of Replay Mod.
pub const REPLAY_MOD_PROJECT: &str = "Nv2fQJo5";
/// Preset name the capture-tools progress events carry.
pub const CAPTURE_PRESET: &str = "capture";

/// One pinned Replay Mod build (sha1-verified, 2.6.28 builds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayModPin {
    pub version_id: String,
    pub sha1: String,
    pub filename: String,
    pub version_number: String,
}

type ReplayPinRow = (&'static str, &'static str, &'static str, &'static str);

/// version -> (version_id, sha1, filename, version_number).
const REPLAY_PIN_ROWS: [(&str, ReplayPinRow); 3] = [
    ("1.21.11", ("KJkQjrpd", "745cc0e46c36f11d377fd4a38ec268791462398a", "replaymod-1.21.11-2.6.28.jar", "2.6.28")),
    ("26.2", ("eHbYuA7P", "4c0a9b4fa4852af576c20083c4f07e4fea1a32f0", "replaymod-26.2-2.6.28.jar", "2.6.28")),
    ("26.3", ("7c5mnPyr", "6e6cc19cb5b5a88c2c37dfe567c3d36685ba591b", "replaymod-26.3-2.6.28.jar", "2.6.28")),
];

/// Versions that have a Replay Mod pin (`Object.keys(REPLAY_MOD_PINS_BY_VERSION)`).
pub const REPLAY_MOD_PINNED_VERSIONS: [&str; 3] = ["1.21.11", "26.2", "26.3"];

/// Replay Mod pin for a version, or None when the bundle has no build for it.
pub fn replay_mod_pin(version: &str) -> Option<ReplayModPin> {
    let (_, (version_id, sha1, filename, version_number)) = REPLAY_PIN_ROWS.iter().find(|(v, _)| *v == version)?;
    Some(ReplayModPin {
        version_id: (*version_id).to_string(),
        sha1: (*sha1).to_string(),
        filename: (*filename).to_string(),
        version_number: (*version_number).to_string(),
    })
}

/// Replay Mod pin honouring the engine's `capture_pins` override (tests point
/// the pinned build at a mock origin).
fn replay_pin_for(engine: &Engine, version: &str) -> Option<ReplayModPin> {
    match engine.content_settings().capture_pins {
        Some(pins) => pins.into_iter().find(|(v, _)| v == version).map(|(_, p)| p),
        None => replay_mod_pin(version),
    }
}

/// True when capture tools can be enabled (Fabric + a Replay Mod pin).
pub fn supports_capture_tools(engine: &Engine, instance: &InstanceMeta) -> bool {
    instance.loader == Loader::Fabric && replay_pin_for(engine, &instance.version).is_some()
}

/// A shader pack shipped into `<gameDir>/shaderpacks/` (licences vary per
/// pack — never bundled).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderPack {
    pub slug: String,
    pub project_id: String,
    pub version_id: String,
    pub version_number: String,
    pub filename: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

/// Complementary Reimagined r5.9.3: supports Iris 1.11.x and stays playable
/// on the low-end iGPU test box. `CAPTURE_SHADER_PACKS` is prettiest-first.
pub fn default_shader_pack() -> ShaderPack {
    ShaderPack {
        slug: "complementary-reimagined".into(),
        project_id: "HVnmMxH1".into(),
        version_id: "Bqen1mJX".into(),
        version_number: "r5.9.3".into(),
        filename: "ComplementaryReimagined_r5.9.3.zip".into(),
        sha1: "838139b54cddb56b2e83cd260d8efd960ac536d6".into(),
        size: 553_397,
        url: "https://cdn.modrinth.com/data/HVnmMxH1/versions/Bqen1mJX/ComplementaryReimagined_r5.9.3.zip".into(),
    }
}

/// MakeUp Ultra Fast 9.6: the fast pack used by live play and `record_*`
/// (cinematic `clip_render` output is frame-locked, so it keeps the pretty one).
pub fn fast_shader_pack() -> ShaderPack {
    ShaderPack {
        slug: "makeup-ultra-fast".into(),
        project_id: "izsIPI7a".into(),
        version_id: "t4YRxQug".into(),
        version_number: "9.6".into(),
        filename: "MakeUp-UltraFast-9.6.zip".into(),
        sha1: "e2a930f1ffb7ca6a56cf466c13c9e36193425a38".into(),
        size: 406_830,
        url: "https://cdn.modrinth.com/data/izsIPI7a/versions/t4YRxQug/MakeUp-UltraFast-9.6.zip".into(),
    }
}

/// Every pack capture tools installs, prettiest first (`CAPTURE_SHADER_PACKS`).
pub fn capture_shader_packs() -> Vec<ShaderPack> {
    vec![default_shader_pack(), fast_shader_pack()]
}

/// `CAPTURE_SHADER_PACKS.length`.
pub const CAPTURE_SHADER_PACKS_LEN: usize = 2;

/// Result of [`install_capture_tools`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureToolsResult {
    pub replay_mod: String,
    pub performance: Vec<String>,
    pub shaderpacks: Vec<String>,
    pub iris_pack: String,
    pub notes: Vec<String>,
}

/// `{applied, pack}` from [`select_default_shader_pack`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderSelection {
    pub applied: bool,
    pub pack: String,
}

/// Test/future-pin seam (JS `installCaptureTools(instance, opts)`).
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct CaptureToolsOptions {
    /// Packs to download; default [`capture_shader_packs`].
    pub packs: Option<Vec<ShaderPack>>,
    /// Replay build to install; default [`replay_mod_pin`] of the instance version.
    pub replay_pin: Option<ReplayModPin>,
}

/// A pinned artifact is a plain basename — never a path that could leave the
/// target dir (no separators, `..`, drive/stream colon, control characters,
/// a trailing dot/space Windows would strip, or a Windows device name such
/// as `con.jar`).
fn assert_safe_pinned_file_name<'a>(name: &'a str, label: &str) -> EngineResult<&'a str> {
    let bad = name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.starts_with('.')
        || !crate::mrpack::is_portable_segment(name)
        || Path::new(name).is_absolute();
    if bad {
        return Err(code_err(400, "invalid_filename", format!("{label} pins an unsafe filename: {name}")));
    }
    Ok(name)
}

/// The download destination name a Modrinth version file names (400
/// `invalid_filename` unless it is a plain basename): the API response is
/// remote data and becomes a path under the mods dir.
fn safe_download_name(f: &Value) -> EngineResult<String> {
    let name = file_name(f);
    assert_safe_pinned_file_name(&name, "Modrinth file")?;
    Ok(name)
}

/// Resolve the pinned Replay Mod build, cross-checked against the pin. The pin
/// — not the Modrinth response — is the trust anchor: verifying a download
/// against a sha1 the server itself supplied would accept any substituted jar,
/// so a renamed artifact or a changed hash is refused outright.
async fn pinned_replay_file(engine: &Engine, pin: &ReplayModPin) -> EngineResult<Value> {
    let meta = version_meta(engine, &pin.version_id).await?;
    if let Some(pid) = s(&meta, "project_id") {
        if pid != REPLAY_MOD_PROJECT {
            return Err(EngineError::other(format!(
                "Modrinth version {} belongs to project {pid}, not Replay Mod ({REPLAY_MOD_PROJECT})",
                pin.version_id
            )));
        }
    }
    let file = primary_file(&meta)
        .ok_or_else(|| EngineError::other(format!("Replay Mod build {} has no downloadable file", pin.version_id)))?;
    let fname = file_name(file);
    if fname != pin.filename {
        return Err(EngineError::other(format!(
            "Replay Mod build {} is served as {fname} but the pin says {}; \
             re-verify the build and update the pin",
            pin.version_id, pin.filename
        )));
    }
    if let Some(served) = file_sha1(file) {
        if served != pin.sha1 {
            return Err(EngineError::other(format!(
                "Replay Mod build {} sha1 is {served} but the pin says {}; \
                 re-verify the build and update the pin",
                pin.version_id, pin.sha1
            )));
        }
    }
    Ok(file.clone())
}

/// Iris' own config file (written by the game; keys are Java properties).
pub fn iris_properties_path(game_dir: &Path) -> PathBuf {
    game_dir.join("config").join("iris.properties")
}

/// File lines, or empty when it does not exist yet (corrupt is read as-is).
fn read_properties_lines(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .map(|s| s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect())
        .unwrap_or_default()
}

/// `{key, value}` for a data line; None for blanks/comments/other.
fn parse_property_line(line: &str) -> Option<(&str, &str)> {
    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
        return None;
    }
    let i = line.find('=')?;
    if i == 0 {
        return None;
    }
    Some((line[..i].trim(), &line[i + 1..]))
}

/// `key=value` value from the lines, last occurrence winning (Java semantics).
fn property_value<'a>(lines: &'a [String], key: &str) -> Option<&'a str> {
    lines.iter().rev().find_map(|l| parse_property_line(l).filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// Set `key=value`, replacing the last existing occurrence in place (so the
/// player's ordering and comments survive) and appending when absent. Any
/// other occurrence is dropped: Java properties read last-wins, so a stale
/// earlier duplicate would otherwise win over the value we just wrote.
fn set_property(lines: &mut Vec<String>, key: &str, value: &str) {
    let mut last = None;
    for (i, line) in lines.iter().enumerate() {
        if parse_property_line(line).is_some_and(|(k, _)| k == key) {
            last = Some(i);
        }
    }
    match last {
        None => lines.push(format!("{key}={value}")),
        Some(idx) => {
            lines[idx] = format!("{key}={value}");
            for i in (0..lines.len()).rev() {
                if i != idx && parse_property_line(&lines[i]).is_some_and(|(k, _)| k == key) {
                    lines.remove(i);
                }
            }
        }
    }
}

/// Pick the default shader pack for Iris, but only when the player has not
/// chosen one: a non-empty `pack` value other than `off` means the user (or a
/// previous capture-tools install) already decided, and overwriting it would
/// silently change their graphics.
///
/// The game rewrites iris.properties on exit, so this is a seed, not state.
pub fn select_default_shader_pack(game_dir: &Path, pack: &ShaderPack) -> EngineResult<ShaderSelection> {
    let file = iris_properties_path(game_dir);
    let mut lines = read_properties_lines(&file);
    let current = property_value(&lines, "shaderPack").unwrap_or_default().trim().to_string();
    if !current.is_empty() && !current.eq_ignore_ascii_case("off") {
        return Ok(ShaderSelection { applied: false, pack: current });
    }
    set_property(&mut lines, "shaderPack", &pack.filename);
    set_property(&mut lines, "enableShaders", "true");
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write beside the file and rename over it: Iris (or the game) rewrites
    // this config on exit, and an in-place write interrupted mid-way would
    // leave the player's other graphics settings truncated.
    let tmp = {
        let mut s = file.as_os_str().to_owned();
        s.push(format!(".{}.tmp", std::process::id()));
        PathBuf::from(s)
    };
    std::fs::write(&tmp, format!("{}\n", lines.join("\n")))?;
    std::fs::rename(&tmp, &file)?;
    Ok(ShaderSelection { applied: true, pack: pack.filename.clone() })
}

/// Enable capture tools on an instance: install the pinned Replay Mod build
/// (sha1-verified), make sure the performance pins (Iris/Sodium and friends)
/// are present so shaders can actually run, and download the shipped shader
/// packs into `<gameDir>/shaderpacks/`.
///
/// Nothing is bundled: every artifact comes from Modrinth at install time.
/// A partial failure rolls the Replay Mod jar back and throws.
pub async fn install_capture_tools(engine: &Engine, instance: &InstanceMeta) -> EngineResult<CaptureToolsResult> {
    install_capture_tools_with(engine, instance, &CaptureToolsOptions { packs: None, replay_pin: None }).await
}

/// [`install_capture_tools`] with explicit pins (tests, future per-instance pins).
#[doc(hidden)]
pub async fn install_capture_tools_with(
    engine: &Engine,
    instance: &InstanceMeta,
    opts: &CaptureToolsOptions,
) -> EngineResult<CaptureToolsResult> {
    let packs = opts
        .packs
        .clone()
        .or_else(|| engine.content_settings().capture_shader_packs)
        .unwrap_or_else(capture_shader_packs);
    let pin = opts.replay_pin.clone().or_else(|| replay_pin_for(engine, &instance.version));
    let Some(pin) = pin.filter(|_| instance.loader == Loader::Fabric) else {
        return Err(code_err(
            400,
            "CAPTURE_TOOLS_UNSUPPORTED",
            format!(
                "capture tools need a Fabric instance on a version with a Replay Mod pin ({})",
                REPLAY_MOD_PINNED_VERSIONS.join(", ")
            ),
        ));
    };
    let game_dir = crate::instances::effective_game_dir(engine, instance);
    let mods_dir = game_dir.join("mods");
    let shader_dir = game_dir.join("shaderpacks");
    std::fs::create_dir_all(&mods_dir)?;

    mod_progress(
        engine,
        ProgressEvent {
            preset: Some(CAPTURE_PRESET.into()),
            ..ProgressEvent::for_instance(&instance.name)
                .phase("start")
                .total(3)
                .index(0)
                .message("installing capture tools (Replay Mod + shaders)")
        },
    );

    let mut installed_replay: Option<PathBuf> = None; // set once THIS call installed the jar
    let outcome = async {
        // 1. Replay Mod pin — exact build, sha1 verified against the PIN (not
        //    against the hash the same server handed us alongside the URL).
        let file = pinned_replay_file(engine, &pin).await?;
        let fname = assert_safe_pinned_file_name(&file_name(&file), "Replay Mod pin")?.to_string();
        let replay_dest = mods_dir.join(&fname);
        let already_pinned = replay_dest.exists()
            && crate::download::sha1_file(&replay_dest).await.ok().as_deref() == Some(pin.sha1.as_str());
        // A leftover `.disabled` twin would show as a second row in the mod
        // list and could be re-enabled over the verified jar.
        let disabled_twin = {
            let mut s = replay_dest.as_os_str().to_owned();
            s.push(".disabled");
            PathBuf::from(s)
        };
        let _ = std::fs::remove_file(disabled_twin);
        // A single artifact: download_file directly, so a sha1/size failure
        // reaches the PATCH response as "sha1 mismatch for <url>".
        let dl = crate::download::DownloadOptions {
            sha1: Some(pin.sha1.clone()),
            size: file.get("size").and_then(Value::as_u64),
            ..Default::default()
        };
        engine.http().download_file(&s(&file, "url").unwrap_or_default(), &replay_dest, &dl).await?;
        // download_file unlinks a mismatching .part and throws, so a bad hash
        // never reaches `dest`; assert it once more so a pre-existing jar with
        // the right name and the wrong bytes cannot be mistaken for the pin.
        if crate::download::sha1_file(&replay_dest).await.ok().as_deref() != Some(pin.sha1.as_str()) {
            let _ = std::fs::remove_file(&replay_dest);
            return Err(EngineError::other(format!(
                "Replay Mod {} failed sha1 verification after download",
                pin.version_number
            )));
        }
        if !already_pinned {
            installed_replay = Some(replay_dest);
        }
        let mut notes = vec![format!("replaymod {} installed", pin.version_number)];

        // 2. Performance pins (Iris + Sodium + friends). install_preset skips
        //    jars that already verify, so a second enable is a cheap no-op.
        let preset = install_preset(engine, &instance.name, ModPreset::Performance).await?;
        if !preset.installed.is_empty() {
            notes.push(format!("performance pins installed: {}", preset.installed.len()));
        }
        notes.extend(preset.notes.iter().cloned());

        // 3. Both shipped packs — plain zips in shaderpacks/, sha1-verified.
        std::fs::create_dir_all(&shader_dir)?;
        for pack in &packs {
            assert_safe_pinned_file_name(&pack.filename, "shader pack")?;
        }
        let items = packs
            .iter()
            .map(|pack| {
                DownloadItem::new(pack.url.clone(), shader_dir.join(&pack.filename))
                    .sha1(Some(pack.sha1.clone()))
                    .size(Some(pack.size))
                    .event(
                        ProgressChannel::Mod,
                        ProgressEvent {
                            preset: Some(CAPTURE_PRESET.into()),
                            ..ProgressEvent::for_instance(&instance.name).filename(&pack.filename)
                        },
                    )
            })
            .collect();
        engine.http().download_all(items, DownloadAllOptions { concurrency: Some(2), on_item: None }).await?;

        // 4. Iris selection: only when the player has no pack chosen yet.
        //    Prettiest-first, so [0] is the cinematic one.
        let preferred = packs.first().cloned().unwrap_or_else(default_shader_pack);
        let selection = select_default_shader_pack(&game_dir, &preferred)?;
        if selection.applied {
            notes.push(format!("iris shader pack set to {}", preferred.filename));
        }

        mod_progress(
            engine,
            ProgressEvent {
                preset: Some(CAPTURE_PRESET.into()),
                ..ProgressEvent::for_instance(&instance.name)
                    .phase("done")
                    .total(3)
                    .index(3)
                    .message("capture tools ready")
            },
        );
        Ok(CaptureToolsResult {
            replay_mod: fname,
            performance: preset.installed,
            shaderpacks: packs.iter().map(|p| p.filename.clone()).collect(),
            iris_pack: selection.pack,
            notes,
        })
    }
    .await;

    match outcome {
        Ok(r) => Ok(r),
        Err(err) => {
            // A jar this call installed but could not finish provisioning for
            // must not outlive the failure: the PATCH leaves `capture_tools`
            // false, so a leftover Replay Mod would record with the flag
            // reported off.
            if let Some(path) = installed_replay {
                let _ = std::fs::remove_file(path);
            }
            let msg = err.to_string();
            mod_progress(
                engine,
                ProgressEvent {
                    preset: Some(CAPTURE_PRESET.into()),
                    error: Some(msg.clone()),
                    ..ProgressEvent::for_instance(&instance.name).phase("error").message(msg)
                },
            );
            Err(err)
        }
    }
}

/// `replaymod*.jar[.disabled]`, case-insensitive.
static REPLAY_JAR_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^replaymod([-.].*)?\.jar(\.disabled)?$").expect("valid regex"));

/// Disable capture tools: remove the Replay Mod jar only. Shader packs and the
/// Iris/Sodium performance pins stay — disabling captures should not strip the
/// user's graphics setup.
///
/// Matches the mod's own filename prefix rather than the single pinned
/// filename: an instance upgraded from an older Replay Mod build still holds
/// `replaymod-<ver>-<older>.jar`, and leaving it behind would keep recording
/// active after the user switched capture tools off.
pub async fn remove_capture_tools(engine: &Engine, instance: &InstanceMeta) -> EngineResult<Vec<String>> {
    let mods_dir = crate::instances::effective_mods_dir(engine, instance);
    let entries = match std::fs::read_dir(&mods_dir) {
        Ok(rd) => rd,
        // No mods dir yet is the normal "nothing installed" case. Any other
        // failure must surface: reporting success while the jar stays in place
        // would leave Replay Mod recording after the user turned captures off.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut removed = vec![];
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !REPLAY_JAR_RE.is_match(&name) {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => removed.push(name),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Installed-mod detection (pure)
// ---------------------------------------------------------------------------

/// Filename plausibly belongs to the project slug (whole token / prefix, case-insensitive).
pub fn is_filename_for_slug(filename: &str, slug: &str) -> bool {
    let mut base = filename.to_lowercase();
    if let Some(b) = base.strip_suffix(".jar.disabled") {
        base = b.to_string();
    } else if let Some(b) = base.strip_suffix(".jar") {
        base = b.to_string();
    }
    let target = slug.to_lowercase();
    if base == target || base.starts_with(&format!("{target}-")) || base.starts_with(&format!("{target}_")) {
        return true;
    }
    base.split(['.', '_', '+', '-']).any(|t| t == target)
}

/// 'Ferrite Core' -> 'ferrite-core'.
pub fn slugify_title(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out
}

/// *.jar / *.jar.disabled basenames ('.disabled' stripped); no network.
pub fn installed_mod_filenames(engine: &Engine, instance_name: &str) -> EngineResult<Vec<String>> {
    if !crate::util::is_valid_instance_name(instance_name) {
        return Err(code_err(409, "invalid_instance_name", format!("invalid instance name: {instance_name}")));
    }
    let dir = crate::instances::effective_game_dir_by_name(engine, instance_name).join("mods");
    Ok(read_names(&dir)
        .into_iter()
        .filter_map(|e| {
            if e.ends_with(".jar") {
                Some(e)
            } else {
                e.strip_suffix(".disabled").filter(|b| b.ends_with(".jar")).map(str::to_string)
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Modrinth search
// ---------------------------------------------------------------------------

/// Browse Modrinth (12 per page, sorted by downloads). Empty query = explore.
pub async fn search_modrinth(
    engine: &Engine,
    query: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
    instance_name: Option<&str>,
    offset: u64,
) -> EngineResult<ModrinthSearchResult> {
    let mut facets = vec![json!(["project_type:mod"])];
    if let Some(l) = loader.filter(|l| !l.is_empty()) {
        facets.push(json!([format!("loaders:{l}")]));
    }
    if let Some(v) = game_version.filter(|v| !v.is_empty()) {
        facets.push(json!([format!("versions:{v}")]));
    }
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("query", query)
        .append_pair("facets", &Value::Array(facets).to_string())
        .append_pair("limit", "12")
        .append_pair("offset", &offset.to_string())
        .append_pair("index", "downloads")
        .finish();
    let url = format!("{}/search?{params}", api(engine));
    let data = match fetch_json_with_retry(engine, &url, 2).await {
        Ok(d) => d,
        Err(err) => {
            let status = if let EngineError::Http(h) = &err { h.status().unwrap_or(502) } else { 502 };
            return Err(EngineError::api(status, "MODRINTH_SEARCH_FAILED", format!("Modrinth search failed: {err}")));
        }
    };
    let mut results: Vec<ModrinthProject> = data
        .get("hits")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .map(|hit| ModrinthProject {
                    project_id: s(hit, "project_id").unwrap_or_default(),
                    slug: s(hit, "slug").unwrap_or_default(),
                    title: s(hit, "title").unwrap_or_default(),
                    description: s(hit, "description").unwrap_or_default(),
                    icon_url: s(hit, "icon_url"),
                    downloads: hit.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                    game_versions: hit
                        .get("versions")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                        .unwrap_or_default(),
                    latest_version_number: s(hit, "latest_version").unwrap_or_default(),
                    installed: None,
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(name) = instance_name.filter(|n| !n.is_empty()) {
        let installed = installed_mod_filenames(engine, name)?;
        for hit in &mut results {
            let slug_t = slugify_title(&hit.title);
            hit.installed =
                Some(installed.iter().any(|f| is_filename_for_slug(f, &hit.slug) || is_filename_for_slug(f, &slug_t)));
        }
    }
    let total = data.get("total_hits").and_then(Value::as_u64).unwrap_or(results.len() as u64);
    Ok(ModrinthSearchResult { results, total, offset, limit: 12 })
}

// ---------------------------------------------------------------------------
// Single-mod install with dependencies
// ---------------------------------------------------------------------------

async fn post_version_files(engine: &Engine, hashes: &[String]) -> Result<Map<String, Value>, Option<u16>> {
    let url = format!("{}/version_files", api(engine));
    let res = engine
        .http()
        .client()
        .post(&url)
        .header("Content-Type", "application/json")
        .body(json!({ "hashes": hashes, "algorithm": "sha1" }).to_string())
        .send()
        .await
        .map_err(|_| None)?;
    if !res.status().is_success() {
        return Err(Some(res.status().as_u16()));
    }
    match res.json::<Value>().await {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Ok(Map::new()),
        Err(_) => Err(None),
    }
}

/// Project ids of the jars already in the mods dir (by sha1); None when the lookup fails.
async fn installed_project_ids(engine: &Engine, mods_dir: &Path, files: &[String]) -> Option<HashSet<String>> {
    let mut hashes = vec![];
    for f in files {
        if let Ok(h) = crate::download::sha1_file(&mods_dir.join(f)).await {
            hashes.push(h);
        }
    }
    if hashes.is_empty() {
        return Some(HashSet::new());
    }
    let by_hash = post_version_files(engine, &hashes).await.ok()?;
    Some(by_hash.values().filter_map(|v| s(v, "project_id")).collect())
}

/// project id -> title (cosmetic; never fails).
async fn project_titles(engine: &Engine, ids: &[String]) -> IndexMap<String, String> {
    let mut unique: IndexSet<String> = IndexSet::new();
    for id in ids.iter().filter(|i| !i.is_empty()) {
        unique.insert(id.clone());
    }
    let mut out = IndexMap::new();
    if unique.is_empty() {
        return out;
    }
    let list: Vec<&String> = unique.iter().collect();
    let url = format!("{}/projects?ids={}", api(engine), encode_uri_component(&json!(list).to_string()));
    if let Ok(Value::Array(projects)) = fetch_json_with_retry(engine, &url, 1).await {
        for p in projects {
            if let (Some(id), Some(title)) = (s(&p, "id"), s(&p, "title")) {
                out.insert(id, title);
            }
        }
    }
    out
}

/// planModInstall() result.
#[derive(Debug, Clone)]
pub struct ModPlan {
    pub root: Value,
    /// (project id, version meta), root first.
    pub installs: Vec<(String, Value)>,
    pub project: ModRef,
    pub dependencies: Vec<ModDependencyRef>,
    pub missing: Vec<ModRef>,
    pub optional: Vec<ModRef>,
    pub conflicts: Vec<ModRef>,
}

/// Work out what installing `project_id` means (installs / missing / optional / conflicts).
pub async fn plan_mod_install(
    engine: &Engine,
    instance: &InstanceMeta,
    project_id: &str,
    loader: &str,
) -> EngineResult<ModPlan> {
    let not_found = |msg: String| code_err(404, "MOD_NOT_FOUND_FOR_VERSION", msg);
    let root = best_version_for_cached(engine, project_id, &instance.version, loader)
        .await
        .ok_or_else(|| not_found(format!("no {loader} build of project {project_id} for {}", instance.version)))?;
    if primary_file(&root).is_none() {
        return Err(not_found(format!("project {project_id} has no downloadable file")));
    }
    let mods_dir = effective_mods_dir(engine, instance);
    let files: Vec<String> = read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f)).collect();
    let installed_ids = installed_project_ids(engine, &mods_dir, &files).await;
    let on_disk: HashSet<String> = files.iter().map(|f| f.strip_suffix(".disabled").unwrap_or(f).to_string()).collect();
    let installed_has = |pid: &str| installed_ids.as_ref().map(|s| s.contains(pid)).unwrap_or(false);
    let already_installed = |pid: &str, meta: &Value| match &installed_ids {
        Some(ids) => ids.contains(pid),
        None => primary_file(meta).map(|f| on_disk.contains(&file_name(f))).unwrap_or(false),
    };

    let root_id = s(&root, "project_id").unwrap_or_else(|| project_id.to_string());
    let mut installs: IndexMap<String, Value> = IndexMap::new();
    installs.insert(root_id.clone(), root.clone());
    let mut missing: IndexSet<String> = IndexSet::new();
    let mut optional: IndexSet<String> = IndexSet::new();
    let mut conflicts: IndexSet<String> = IndexSet::new();
    let mut qi = 0;
    while qi < installs.len() {
        let meta = installs[qi].clone();
        let is_root = qi == 0;
        qi += 1;
        for dep in meta.get("dependencies").and_then(Value::as_array).cloned().unwrap_or_default() {
            let pid = s(&dep, "project_id");
            match dep.get("dependency_type").and_then(Value::as_str) {
                Some("incompatible") => {
                    if let Some(p) = pid.filter(|p| installed_has(p)) {
                        conflicts.insert(p);
                    }
                    continue;
                }
                Some("optional") => {
                    if let Some(p) = pid.filter(|p| is_root && !installs.contains_key(p) && !installed_has(p)) {
                        optional.insert(p);
                    }
                    continue;
                }
                Some("required") => {}
                _ => continue, // embedded ships inside the parent
            }
            if let Some(p) = &pid {
                if installs.contains_key(p) || JAR_IN_JAR.contains(&p.as_str()) || installed_has(p) {
                    continue;
                }
            }
            let mut dep_meta = None;
            if let Some(vid) = s(&dep, "version_id") {
                dep_meta = version_meta(engine, &vid).await.ok();
            }
            if dep_meta.is_none() {
                if let Some(p) = &pid {
                    dep_meta = best_version_for_cached(engine, p, &instance.version, loader).await;
                }
            }
            let Some(dep_meta) = dep_meta.filter(|m| primary_file(m).is_some()) else {
                if let Some(id) = pid.clone().or_else(|| s(&dep, "version_id")) {
                    missing.insert(id);
                }
                continue;
            };
            let dep_id = s(&dep_meta, "project_id").or(pid).unwrap_or_default();
            if installs.contains_key(&dep_id) || already_installed(&dep_id, &dep_meta) {
                continue;
            }
            installs.insert(dep_id, dep_meta);
        }
    }
    for m in &missing {
        optional.shift_remove(m);
    }

    let mut all_ids: Vec<String> = installs.keys().cloned().collect();
    all_ids.extend(missing.iter().cloned());
    all_ids.extend(optional.iter().cloned());
    all_ids.extend(conflicts.iter().cloned());
    let titles = project_titles(engine, &all_ids).await;
    let label = |pid: &str| ModRef {
        project_id: pid.to_string(),
        title: titles.get(pid).cloned().unwrap_or_else(|| pid.to_string()),
    };
    Ok(ModPlan {
        project: label(&root_id),
        dependencies: installs
            .iter()
            .filter(|(pid, _)| **pid != root_id)
            .map(|(pid, meta)| {
                let l = label(pid);
                ModDependencyRef {
                    project_id: l.project_id,
                    title: l.title,
                    filename: primary_file(meta).map(file_name).unwrap_or_default(),
                }
            })
            .collect(),
        missing: missing.iter().map(|p| label(p)).collect(),
        optional: optional.iter().map(|p| label(p)).collect(),
        conflicts: conflicts.iter().map(|p| label(p)).collect(),
        installs: installs.into_iter().collect(),
        root,
    })
}

/// installModrinthMod() result: installed filenames (+ the plan summary).
#[derive(Debug, Clone)]
pub struct ModrinthInstallResult {
    pub installed: Vec<String>,
    pub plan: ModPlan,
}

/// Install a Modrinth project plus its missing required dependencies.
pub async fn install_modrinth_mod(
    engine: &Engine,
    instance_name: &str,
    project_id: &str,
    loader: Option<&str>,
    plan: Option<ModPlan>,
) -> EngineResult<ModrinthInstallResult> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let loader = loader.unwrap_or_else(|| instance.loader.content_loader());
    let plan = match plan {
        Some(p) => p,
        None => plan_mod_install(engine, &instance, project_id, loader).await?,
    };
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    let root_file = primary_file(&plan.root).map(file_name).unwrap_or_default();
    let total = plan.installs.len();
    let deps = if total > 1 { format!(" + {} dependencies", total - 1) } else { String::new() };
    mod_progress(
        engine,
        ProgressEvent::for_instance(instance_name)
            .phase("start")
            .filename(&root_file)
            .index(0)
            .total(total as u64)
            .message(format!("installing {root_file}{deps}")),
    );
    let mut items = vec![];
    let mut installed = vec![];
    for (i, (_, meta)) in plan.installs.iter().enumerate() {
        let file = primary_file(meta).ok_or_else(|| no_file_error(meta))?;
        let fname = safe_download_name(file)?;
        installed.push(fname.clone());
        items.push(
            DownloadItem::new(s(file, "url").unwrap_or_default(), mods_dir.join(&fname))
                .sha1(file_sha1(file))
                .size(file.get("size").and_then(Value::as_u64))
                .event(
                    ProgressChannel::Mod,
                    ProgressEvent::for_instance(instance_name).filename(&fname).index(i as u64).total(total as u64),
                ),
        );
    }
    if let Err(err) = engine.http().download_all(items, DownloadAllOptions::default()).await {
        let e: EngineError = err.into();
        mod_progress(
            engine,
            ProgressEvent {
                error: Some(e.to_string()),
                ..ProgressEvent::for_instance(instance_name).phase("error").filename(&root_file)
            },
        );
        return Err(e);
    }
    mod_progress(
        engine,
        ProgressEvent::for_instance(instance_name)
            .phase("done")
            .filename(&root_file)
            .index(total as u64)
            .total(total as u64)
            .message(format!("installed {root_file}")),
    );
    Ok(ModrinthInstallResult { installed, plan })
}

// ---------------------------------------------------------------------------
// Delete mod / Version switcher (with companions) / Pre-install compat check
// ---------------------------------------------------------------------------

/// Delete an installed mod jar (enabled and/or disabled twin). Refuses bundled
/// Espectral branding jars (`400 branding_protected`).
pub async fn delete_mod(engine: &Engine, instance_name: &str, filename: &str) -> EngineResult<String> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let base = assert_safe_filename(filename)?;
    let jar_name = format!("{base}.jar");
    if is_brand_jar(&jar_name) {
        return Err(code_err(400, "branding_protected", "cannot delete the bundled Espectral mod"));
    }
    let mods_dir = effective_mods_dir(engine, &instance);
    let jar = mods_dir.join(&jar_name);
    let disabled = mods_dir.join(format!("{base}.jar.disabled"));
    let mut removed = false;
    for path in [&jar, &disabled] {
        if path.exists() {
            std::fs::remove_file(path)?;
            removed = true;
        }
    }
    if !removed {
        return Err(code_err(404, "mod_not_found", format!("mod not found: {filename}")));
    }
    tracing::info!("[mods] deleted {jar_name} from {}", instance.name);
    Ok(jar_name)
}

/// Batched `POST /version_files` backed by `ContentState::sha1_versions`.
/// Network failures leave uncached hashes absent (never poisoned as None).
async fn cached_version_files(engine: &Engine, hashes: &[String]) -> IndexMap<String, Option<Value>> {
    let mut out = IndexMap::new();
    let mut missing = Vec::new();
    {
        let cache = engine.content_state().sha1_versions.lock().unwrap_or_else(|p| p.into_inner());
        for h in hashes {
            if let Some(entry) = cache.get(h) {
                out.insert(h.clone(), entry.clone());
            } else if !missing.contains(h) {
                missing.push(h.clone());
            }
        }
    }
    if !missing.is_empty() {
        if let Ok(by_hash) = post_version_files(engine, &missing).await {
            let mut cache = engine.content_state().sha1_versions.lock().unwrap_or_else(|p| p.into_inner());
            for h in missing {
                let val = by_hash.get(&h).cloned();
                cache.insert(h.clone(), val.clone());
                out.insert(h, val);
            }
        }
    }
    out
}

/// All listed/approved Modrinth versions of `project_id` for `(game_version, loader)`.
async fn list_project_versions(
    engine: &Engine,
    project_id: &str,
    game_version: &str,
    loader: &str,
) -> EngineResult<Vec<Value>> {
    let q = format!(
        "game_versions={}&loaders={}",
        encode_uri_component(&json!([game_version]).to_string()),
        encode_uri_component(&json!([loader]).to_string())
    );
    let url = format!("{}/project/{}/version?{q}", api(engine), encode_uri_component(project_id));
    let list = fetch_json_with_retry(engine, &url, 2).await?;
    let versions: Vec<Value> = list
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|v| {
                    matches!(v.get("status").and_then(Value::as_str), Some("listed") | Some("approved") | None)
                        && primary_file(v).is_some()
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    // Populate version_meta cache so subsequent lookups are instant.
    {
        let mut cache = engine.content_state().version_meta.lock().unwrap_or_else(|p| p.into_inner());
        for v in &versions {
            if let Some(id) = s(v, "id") {
                cache.insert(id, v.clone());
            }
        }
    }
    Ok(versions)
}

struct ResolvedInstalledMod {
    base_filename: String,
    sha1: Option<String>,
    project_id: Option<String>,
    version_id: Option<String>,
    version_number: Option<String>,
    mod_id: Option<String>,
    display_name: Option<String>,
}

async fn resolve_installed_mod(
    engine: &Engine,
    instance: &InstanceMeta,
    mods_dir: &Path,
    filename: &str,
) -> EngineResult<ResolvedInstalledMod> {
    let base = assert_safe_filename(filename)?;
    let jar_name = format!("{base}.jar");
    let enabled_path = mods_dir.join(&jar_name);
    let disabled_path = mods_dir.join(format!("{base}.jar.disabled"));
    let actual_path = if enabled_path.exists() {
        enabled_path
    } else if disabled_path.exists() {
        disabled_path
    } else {
        return Err(code_err(404, "mod_not_found", format!("mod not found: {filename}")));
    };

    let sha1 = crate::download::sha1_file(&actual_path).await.ok();
    let fabric = crate::fabric_compat::read_fabric_jar(&actual_path, &jar_name, false);
    let mod_id = fabric.as_ref().map(|f| f.root.id.clone());
    let display_name = fabric.as_ref().map(|f| f.root.name.clone());
    let mut version_number = fabric
        .as_ref()
        .map(|f| crate::fabric_compat::clean_version_display(&f.root.version).to_string())
        .filter(|v| !v.is_empty());
    let mut project_id: Option<String> = None;
    let mut version_id: Option<String> = None;

    if let Some(ref h) = sha1 {
        let map = cached_version_files(engine, std::slice::from_ref(h)).await;
        if let Some(Some(meta)) = map.get(h) {
            project_id = s(meta, "project_id");
            version_id = s(meta, "id");
            if let Some(vn) = s(meta, "version_number") {
                version_number = Some(vn);
            }
        }
    }
    if project_id.is_none() {
        if let Some(ref mid) = mod_id {
            if let Some((pid, _)) = crate::fabric_compat::known_modrinth_for_id(mid) {
                project_id = Some(pid.to_string());
            }
        }
    }
    if project_id.is_none() {
        let pins = pins_for_version(engine, &instance.version).await.unwrap_or_default();
        if let Some(pin) = pins.iter().find(|p| is_filename_for_slug(&jar_name, &p.slug)) {
            project_id = Some(pin.project_id.clone());
        }
    }
    Ok(ResolvedInstalledMod {
        base_filename: jar_name,
        sha1,
        project_id,
        version_id,
        version_number,
        mod_id,
        display_name,
    })
}

/// List all Modrinth versions (release / beta / alpha) of an installed mod for
/// this instance's Minecraft version + loader.
pub async fn list_mod_versions(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
) -> EngineResult<ModVersionsReply> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let info = resolve_installed_mod(engine, &instance, &mods_dir, filename).await?;
    if is_brand_jar(&info.base_filename) {
        return Err(code_err(400, "branding_protected", "cannot switch version of bundled Espectral mod"));
    }
    let Some(project_id) = info.project_id else {
        return Err(code_err(404, "mod_not_on_modrinth", format!("{} is not tracked on Modrinth", info.base_filename)));
    };
    let loader = instance.loader.content_loader();
    let raw_versions = list_project_versions(engine, &project_id, &instance.version, loader).await?;
    let titles = project_titles(engine, std::slice::from_ref(&project_id)).await;
    let title = titles
        .get(&project_id)
        .cloned()
        .or(info.display_name)
        .unwrap_or_else(|| info.base_filename.clone());

    let versions: Vec<ModVersionItem> = raw_versions
        .into_iter()
        .filter_map(|v| {
            let vid = s(&v, "id")?;
            let f = primary_file(&v)?;
            let fname = file_name(f);
            let fsha = file_sha1(f);
            let is_current = info.version_id.as_deref() == Some(vid.as_str())
                || (info.sha1.is_some() && info.sha1 == fsha)
                || fname == info.base_filename;
            Some(ModVersionItem {
                version_id: vid,
                version_number: s(&v, "version_number").unwrap_or_default(),
                name: s(&v, "name").unwrap_or_default(),
                version_type: s(&v, "version_type").unwrap_or_else(|| "release".into()),
                date_published: s(&v, "date_published").unwrap_or_default(),
                filename: fname,
                installed: is_current,
            })
        })
        .collect();

    Ok(ModVersionsReply {
        project_id,
        title,
        current_version_id: info.version_id,
        current_version_number: info.version_number,
        versions,
    })
}

/// Find the best Modrinth version for `filename` that satisfies a Fabric version
/// predicate (e.g. `>=0.9.3`), preferring release > beta > alpha, newest first.
pub async fn find_mod_version_for_predicate(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
    predicate: &str,
) -> EngineResult<Option<ModVersionItem>> {
    let reply = list_mod_versions(engine, instance_name, filename).await?;
    let pred = crate::fabric_compat::VersionPredicate::parse_str(predicate);
    for tier in ["release", "beta", "alpha"] {
        for item in &reply.versions {
            if item.version_type != tier {
                continue;
            }
            if let Some(sv) = crate::fabric_compat::extract_mod_semver(&item.version_number) {
                if pred.matches(&sv) {
                    return Ok(Some(item.clone()));
                }
            }
        }
    }
    Ok(None)
}

fn version_tier_rank(version_type: &str) -> u8 {
    match version_type {
        "release" => 0,
        "beta" => 1,
        _ => 2,
    }
}

/// True if a Modrinth version JSON's `dependencies` array lists `version_id`.
fn modrinth_version_pins(meta: &Value, target_version_id: &str) -> bool {
    meta.get("dependencies")
        .and_then(Value::as_array)
        .is_some_and(|deps| {
            deps.iter().any(|d| {
                d.get("dependency_type").and_then(Value::as_str) == Some("required")
                    && d.get("version_id").and_then(Value::as_str) == Some(target_version_id)
            })
        })
}

/// Plan switching an installed mod to `target_version_id`, bringing along
/// companion mods (especially Iris <-> Sodium, plus any other installed mod
/// whose required dependency or `fabric.mod.json` moves with it).
pub async fn plan_mod_version_switch(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
    target_version_id: &str,
) -> EngineResult<ModVersionSwitchPlan> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    let primary = resolve_installed_mod(engine, &instance, &mods_dir, filename).await?;
    let Some(primary_pid) = primary.project_id.clone() else {
        return Err(code_err(404, "mod_not_on_modrinth", format!("{} is not tracked on Modrinth", primary.base_filename)));
    };
    let target_meta = version_meta(engine, target_version_id).await?;
    let target_file = primary_file(&target_meta).ok_or_else(|| no_file_error(&target_meta))?;
    let to_filename = safe_download_name(target_file)?;
    let to_version_number = s(&target_meta, "version_number").unwrap_or_default();
    let target_semver = crate::fabric_compat::extract_mod_semver(&to_version_number);

    // Scan all other installed jars + resolve their Modrinth project/version IDs in one batch.
    let scanned = crate::fabric_compat::scan_mods_dir(&mods_dir);
    let mut disk_files: Vec<(String, bool, Option<String>)> = Vec::new();
    let mut hashes = Vec::new();
    for f in read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f) && !is_brand_jar(f)) {
        let enabled = f.ends_with(".jar");
        let base = if enabled { f.clone() } else { f[..f.len() - ".disabled".len()].to_string() };
        if base == primary.base_filename {
            continue;
        }
        let h = crate::download::sha1_file(&mods_dir.join(&f)).await.ok();
        if let Some(ref hs) = h {
            hashes.push(hs.clone());
        }
        disk_files.push((base, enabled, h));
    }
    let by_hash = cached_version_files(engine, &hashes).await;
    let loader = instance.loader.content_loader();

    // Build lookup of other installed mods by project_id.
    struct OtherMod {
        filename: String,
        enabled: bool,
        project_id: String,
        version_id: Option<String>,
        version_number: String,
        scanned_idx: Option<usize>,
        modrinth_meta: Option<Value>,
    }
    let mut others: Vec<OtherMod> = Vec::new();
    for (base, enabled, h) in disk_files {
        let meta = h.as_deref().and_then(|hs| by_hash.get(hs)).and_then(|o| o.clone());
        let scanned_idx = scanned.iter().position(|j| j.filename == base);
        let pid = meta
            .as_ref()
            .and_then(|m| s(m, "project_id"))
            .or_else(|| {
                scanned_idx
                    .and_then(|i| crate::fabric_compat::known_modrinth_for_id(&scanned[i].root.id))
                    .map(|(p, _)| p.to_string())
            });
        let Some(pid) = pid else { continue };
        let vid = meta.as_ref().and_then(|m| s(m, "id"));
        let vnum = meta
            .as_ref()
            .and_then(|m| s(m, "version_number"))
            .or_else(|| scanned_idx.map(|i| scanned[i].root.version.clone()))
            .unwrap_or_default();
        others.push(OtherMod {
            filename: base,
            enabled,
            project_id: pid,
            version_id: vid,
            version_number: vnum,
            scanned_idx,
            modrinth_meta: meta,
        });
    }

    let mut companions: Vec<CompanionChange> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut handled_pids: HashSet<String> = HashSet::new();
    handled_pids.insert(primary_pid.clone());

    // Direction 1: Outgoing required deps of `target_meta` that pin a specific
    // `version_id` for an already-installed mod (e.g. Iris -> Sodium).
    if let Some(deps) = target_meta.get("dependencies").and_then(Value::as_array) {
        for dep in deps {
            if dep.get("dependency_type").and_then(Value::as_str) != Some("required") {
                continue;
            }
            let Some(dep_vid) = s(dep, "version_id") else { continue };
            let dep_pid = s(dep, "project_id");
            let dep_meta = match version_meta(engine, &dep_vid).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            let resolved_pid = s(&dep_meta, "project_id").or(dep_pid).unwrap_or_default();
            if resolved_pid.is_empty() || handled_pids.contains(&resolved_pid) {
                continue;
            }
            let Some(other) = others.iter().find(|o| o.project_id == resolved_pid) else { continue };
            if other.version_id.as_deref() == Some(dep_vid.as_str()) {
                continue;
            }
            let Some(f) = primary_file(&dep_meta) else { continue };
            let Ok(to_f) = safe_download_name(f) else { continue };
            handled_pids.insert(resolved_pid.clone());
            companions.push(CompanionChange {
                filename: other.filename.clone(),
                project_id: resolved_pid,
                title: other
                    .scanned_idx
                    .map(|i| scanned[i].root.name.clone())
                    .unwrap_or_else(|| other.filename.clone()),
                from_version: other.version_number.clone(),
                to_version_id: dep_vid,
                to_version_number: s(&dep_meta, "version_number").unwrap_or_default(),
                to_filename: to_f,
                reason: "required".into(),
            });
        }
    }

    // Direction 2: Incoming companions & dependents (Sodium -> Iris, plus any
    // other enabled mod that depends on `primary`).
    let mut primary_ids: HashSet<String> = HashSet::new();
    if let Some(ref mid) = primary.mod_id {
        primary_ids.insert(mid.clone());
    }
    if let Some(j) = scanned.iter().find(|j| j.filename == primary.base_filename) {
        primary_ids.insert(j.root.id.clone());
        for p in &j.root.provides {
            primary_ids.insert(p.clone());
        }
    }

    for other in &others {
        if !other.enabled || handled_pids.contains(&other.project_id) {
            continue;
        }
        let is_iris_for_sodium = primary_pid == SODIUM_PROJECT_ID && other.project_id == IRIS_PROJECT_ID;
        let fabric_dep_clauses: Option<Vec<String>> = other.scanned_idx.and_then(|idx| {
            let j = &scanned[idx];
            primary_ids.iter().find_map(|pid| j.root.depends.get(pid).cloned())
        });
        let modrinth_depends_on_primary = other
            .modrinth_meta
            .as_ref()
            .and_then(|m| m.get("dependencies"))
            .and_then(Value::as_array)
            .is_some_and(|deps| {
                deps.iter().any(|d| {
                    d.get("dependency_type").and_then(Value::as_str) == Some("required")
                        && d.get("project_id").and_then(Value::as_str) == Some(primary_pid.as_str())
                })
            });

        if !is_iris_for_sodium && fabric_dep_clauses.is_none() && !modrinth_depends_on_primary {
            continue;
        }

        let current_satisfies = match (&fabric_dep_clauses, &target_semver) {
            (Some(clauses), Some(sv)) => crate::fabric_compat::VersionPredicate::parse_list(clauses).matches(sv),
            _ => true,
        };

        let other_name = other
            .scanned_idx
            .map(|i| scanned[i].root.name.clone())
            .unwrap_or_else(|| other.filename.clone());

        let other_versions = list_project_versions(engine, &other.project_id, &instance.version, loader)
            .await
            .unwrap_or_default();

        // First preference: a build of `other` that explicitly pins `target_version_id`
        // in Modrinth dependencies (exact Iris <-> Sodium pairing!).
        let mut exact_matches: Vec<&Value> =
            other_versions.iter().filter(|v| modrinth_version_pins(v, target_version_id)).collect();
        exact_matches.sort_by_key(|v| version_tier_rank(s(v, "version_type").as_deref().unwrap_or("release")));

        if let Some(best) = exact_matches.first() {
            let best_id = s(best, "id").unwrap_or_default();
            if other.version_id.as_deref() != Some(best_id.as_str()) {
                if let Some(f) = primary_file(best) {
                    if let Ok(to_f) = safe_download_name(f) {
                        handled_pids.insert(other.project_id.clone());
                        companions.push(CompanionChange {
                            filename: other.filename.clone(),
                            project_id: other.project_id.clone(),
                            title: other_name,
                            from_version: other.version_number.clone(),
                            to_version_id: best_id,
                            to_version_number: s(best, "version_number").unwrap_or_default(),
                            to_filename: to_f,
                            reason: "companion".into(),
                        });
                        continue;
                    }
                }
            } else {
                continue;
            }
        }

        // If the current installed version of `other` will break against `target_semver`
        // (or is Iris with a different Sodium build that no Iris pins yet):
        if !current_satisfies {
            // Try newest release/beta of `other` that doesn't pin a DIFFERENT version of `primary_pid`.
            let fallback = other_versions
                .iter()
                .filter(|v| {
                    let pins_other = v
                        .get("dependencies")
                        .and_then(Value::as_array)
                        .is_some_and(|deps| {
                            deps.iter().any(|d| {
                                d.get("dependency_type").and_then(Value::as_str) == Some("required")
                                    && d.get("project_id").and_then(Value::as_str) == Some(primary_pid.as_str())
                                    && d.get("version_id")
                                        .and_then(Value::as_str)
                                        .is_some_and(|vid| vid != target_version_id)
                            })
                        });
                    !pins_other && s(v, "id").as_deref() != other.version_id.as_deref()
                })
                .min_by_key(|v| version_tier_rank(s(v, "version_type").as_deref().unwrap_or("release")));

            if let Some(best) = fallback {
                if let (Some(best_id), Some(f)) = (s(best, "id"), primary_file(best)) {
                    if let Ok(to_f) = safe_download_name(f) {
                        handled_pids.insert(other.project_id.clone());
                        companions.push(CompanionChange {
                            filename: other.filename.clone(),
                            project_id: other.project_id.clone(),
                            title: other_name,
                            from_version: other.version_number.clone(),
                            to_version_id: best_id,
                            to_version_number: s(best, "version_number").unwrap_or_default(),
                            to_filename: to_f,
                            reason: "compat".into(),
                        });
                        continue;
                    }
                }
            }
            warnings.push(other_name);
        } else if is_iris_for_sodium
            && exact_matches.is_empty()
            && primary.version_id.as_deref() != Some(target_version_id)
        {
            // Check if current Iris explicitly pins the OLD Sodium version (even if fabric.mod.json says 0.9.x).
            let pins_different = other
                .modrinth_meta
                .as_ref()
                .and_then(|m| m.get("dependencies"))
                .and_then(Value::as_array)
                .is_some_and(|deps| {
                    deps.iter().any(|d| {
                        d.get("project_id").and_then(Value::as_str) == Some(SODIUM_PROJECT_ID)
                            && d.get("version_id")
                                .and_then(Value::as_str)
                                .is_some_and(|vid| vid != target_version_id)
                    })
                });
            if pins_different {
                warnings.push(other_name);
            }
        }
    }

    // Resolve friendly project titles for primary + companions.
    let mut title_ids = vec![primary_pid.clone()];
    title_ids.extend(companions.iter().map(|c| c.project_id.clone()));
    let titles = project_titles(engine, &title_ids).await;
    let primary_title = titles
        .get(&primary_pid)
        .cloned()
        .or(primary.display_name)
        .unwrap_or_else(|| primary.base_filename.clone());
    for c in &mut companions {
        if let Some(t) = titles.get(&c.project_id) {
            c.title = t.clone();
        }
    }

    Ok(ModVersionSwitchPlan {
        filename: primary.base_filename,
        project_id: primary_pid,
        title: primary_title,
        from_version: primary.version_number.unwrap_or_default(),
        to_version_id: target_version_id.to_string(),
        to_version_number,
        to_filename,
        companions,
        warnings,
    })
}

/// Apply a version switch (and optionally its planned companion updates).
/// Downloads all target files with SHA-1 verification BEFORE deleting old jars.
pub async fn apply_mod_version_switch(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
    target_version_id: &str,
    include_companions: bool,
) -> EngineResult<ModVersionSwitchResult> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;

    let plan = plan_mod_version_switch(engine, instance_name, filename, target_version_id).await?;
    struct Job {
        old_filename: String,
        was_enabled: bool,
        new_filename: String,
        title: String,
        is_companion: bool,
        meta: Value,
    }
    let mut jobs = Vec::new();
    let primary_enabled = mods_dir.join(&plan.filename).exists();
    jobs.push(Job {
        old_filename: plan.filename.clone(),
        was_enabled: primary_enabled,
        new_filename: plan.to_filename.clone(),
        title: plan.title.clone(),
        is_companion: false,
        meta: version_meta(engine, &plan.to_version_id).await?,
    });
    if include_companions {
        for c in &plan.companions {
            let was_enabled = mods_dir.join(&c.filename).exists();
            jobs.push(Job {
                old_filename: c.filename.clone(),
                was_enabled,
                new_filename: c.to_filename.clone(),
                title: c.title.clone(),
                is_companion: true,
                meta: version_meta(engine, &c.to_version_id).await?,
            });
        }
    }

    let total = jobs.len() as u64;
    let mut downloads = Vec::new();
    for (i, job) in jobs.iter().enumerate() {
        let file = primary_file(&job.meta).ok_or_else(|| no_file_error(&job.meta))?;
        let fname = safe_download_name(file)?;
        let dest = mods_dir.join(&fname);
        let expected_sha1 = file_sha1(file);
        let already_valid = dest.exists()
            && expected_sha1
                .as_deref()
                .is_some_and(|exp| std::fs::read(&dest).ok().map(|b| crate::download::sha1_hex(&b)).as_deref() == Some(exp));
        if !already_valid {
            downloads.push(
                DownloadItem::new(s(file, "url").unwrap_or_default(), dest)
                    .sha1(expected_sha1)
                    .size(file.get("size").and_then(Value::as_u64))
                    .event(
                        ProgressChannel::Mod,
                        ProgressEvent::for_instance(instance_name)
                            .filename(&fname)
                            .index(i as u64)
                            .total(total),
                    ),
            );
        }
    }
    if !downloads.is_empty() {
        engine.http().download_all(downloads, DownloadAllOptions::default()).await?;
    }

    // All downloads succeeded — now swap old files out and preserve enabled state.
    let mut changed = Vec::new();
    let mut companions_changed = Vec::new();
    for job in jobs {
        if job.old_filename != job.new_filename {
            let _ = std::fs::remove_file(mods_dir.join(&job.old_filename));
            let _ = std::fs::remove_file(mods_dir.join(format!("{}.disabled", job.old_filename)));
        }
        if job.was_enabled {
            let _ = std::fs::remove_file(mods_dir.join(format!("{}.disabled", job.new_filename)));
        } else {
            let src = mods_dir.join(&job.new_filename);
            let dst = mods_dir.join(format!("{}.disabled", job.new_filename));
            if src.exists() {
                let _ = std::fs::rename(src, dst);
            }
        }
        changed.push(ModRename { from: job.old_filename, to: job.new_filename });
        if job.is_companion {
            companions_changed.push(job.title);
        }
    }
    Ok(ModVersionSwitchResult { changed, companions_changed })
}

const MAX_STAGE_BYTES: u64 = 15 * 1024 * 1024;

fn staging_path(engine: &Engine, sha1: &str) -> PathBuf {
    engine.paths().data_dir().join("cache").join("mod-staging").join(format!("{sha1}.jar"))
}

/// Pre-install compatibility & upgrade preview for installing a Modrinth mod.
/// For Fabric instances, downloads the root jar (if <= 15 MiB) into a SHA-1
/// staging cache and runs the full `fabric.mod.json` compatibility check
/// BEFORE touching `mods/`, detecting outdated dependencies (like Sodium) and
/// planning companion updates (like Iris).
pub async fn preview_modrinth_install(
    engine: &Engine,
    instance_name: &str,
    project_id: &str,
) -> EngineResult<ModInstallPreview> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let loader = instance.loader.content_loader();
    let plan = plan_mod_install(engine, &instance, project_id, loader).await?;
    let root_file = primary_file(&plan.root).ok_or_else(|| no_file_error(&plan.root))?;
    let root_fname = safe_download_name(root_file)?;
    let root_vid = s(&plan.root, "id").unwrap_or_default();
    let root_vnum = s(&plan.root, "version_number").unwrap_or_default();

    let mods_dir = effective_mods_dir(engine, &instance);
    let mut upgrades: Vec<ModInstallUpgrade> = Vec::new();
    let mut upgraded_files: HashSet<String> = HashSet::new();
    let mut enable_disabled: Vec<String> = Vec::new();
    let mut compat_issues: Vec<crate::fabric_compat::ModCompatIssue> = Vec::new();

    if instance.loader == Loader::Fabric {
        let env = crate::fabric_compat::discover_fabric_env(engine, &instance);
        let existing_jars = crate::fabric_compat::scan_mods_dir(&mods_dir);

        // Stage the root jar to inspect its real `fabric.mod.json`.
        let file_size = root_file.get("size").and_then(Value::as_u64).unwrap_or(0);
        let root_sha1 = file_sha1(root_file);
        let staged_jar = if file_size <= MAX_STAGE_BYTES {
            if let Some(ref sha) = root_sha1 {
                let st = staging_path(engine, sha);
                let ok = if st.exists()
                    && crate::download::sha1_file(&st).await.ok().as_deref() == Some(sha.as_str())
                {
                    true
                } else {
                    if let Some(parent) = st.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let dl = crate::download::DownloadOptions {
                        sha1: Some(sha.clone()),
                        size: root_file.get("size").and_then(Value::as_u64),
                        ..Default::default()
                    };
                    engine.http().download_file(&s(root_file, "url").unwrap_or_default(), &st, &dl).await.is_ok()
                };
                if ok {
                    crate::fabric_compat::read_fabric_jar(&st, &root_fname, true)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(candidate) = staged_jar {
            // Simulate adding candidate to the existing jars.
            let mut sim_jars = existing_jars.clone();
            sim_jars.push(candidate.clone());
            let sim_report = crate::fabric_compat::evaluate_fabric_compat(&env, &sim_jars);

            // Collect project ids of newly planned deps so we don't report them as missing.
            let planned_dep_files: HashSet<String> =
                plan.dependencies.iter().map(|d| d.filename.to_ascii_lowercase()).collect();
            let planned_dep_pids: HashSet<String> = plan.dependencies.iter().map(|d| d.project_id.clone()).collect();

            for issue in sim_report.issues {
                // Only surface issues caused by or targeting the new candidate mod.
                let touches_candidate = issue.filename == root_fname
                    || issue.target_filename.as_deref() == Some(root_fname.as_str());
                if !touches_candidate {
                    continue;
                }
                match issue.kind {
                    crate::fabric_compat::CompatIssueKind::MissingDep => {
                        // Skip if plan.dependencies is already going to install this mod!
                        let covered = crate::fabric_compat::known_modrinth_for_id(&issue.target_id)
                            .is_some_and(|(pid, _)| planned_dep_pids.contains(pid))
                            || planned_dep_files
                                .iter()
                                .any(|f| is_filename_for_slug(f, &issue.target_id));
                        if !covered {
                            compat_issues.push(issue);
                        }
                    }
                    crate::fabric_compat::CompatIssueKind::DisabledDep => {
                        if let Some(crate::fabric_compat::CompatFix::Enable { ref filename, .. }) = issue.fix {
                            if !enable_disabled.contains(filename) {
                                enable_disabled.push(filename.clone());
                            }
                        } else if let Some(crate::fabric_compat::CompatFix::UpdateMod {
                            ref filename,
                            ref required,
                            ..
                        }) = issue.fix
                        {
                            if !enable_disabled.contains(filename) {
                                enable_disabled.push(filename.clone());
                            }
                            if !try_plan_upgrade(
                                engine,
                                instance_name,
                                filename,
                                required,
                                &mut upgrades,
                                &mut upgraded_files,
                            )
                            .await
                            {
                                compat_issues.push(issue);
                            }
                        } else {
                            compat_issues.push(issue);
                        }
                    }
                    crate::fabric_compat::CompatIssueKind::IncompatibleDep => {
                        if let Some(crate::fabric_compat::CompatFix::UpdateMod {
                            ref filename,
                            ref required,
                            ..
                        }) = issue.fix
                        {
                            if !try_plan_upgrade(
                                engine,
                                instance_name,
                                filename,
                                required,
                                &mut upgrades,
                                &mut upgraded_files,
                            )
                            .await
                            {
                                compat_issues.push(issue);
                            }
                        } else {
                            compat_issues.push(issue);
                        }
                    }
                    _ => compat_issues.push(issue),
                }
            }
        }
    }

    let has_blocking_compat =
        compat_issues.iter().any(|i| i.severity == crate::fabric_compat::CompatSeverity::Error);
    let needs_confirmation = !upgrades.is_empty()
        || !enable_disabled.is_empty()
        || !plan.conflicts.is_empty()
        || !plan.missing.is_empty()
        || has_blocking_compat;

    Ok(ModInstallPreview {
        project_id: plan.project.project_id,
        title: plan.project.title,
        version_id: root_vid,
        version_number: root_vnum,
        filename: root_fname,
        dependencies: plan.dependencies,
        upgrades,
        enable_disabled,
        missing: plan.missing,
        optional: plan.optional,
        conflicts: plan.conflicts,
        compat_issues,
        needs_confirmation,
    })
}

async fn try_plan_upgrade(
    engine: &Engine,
    instance_name: &str,
    filename: &str,
    required: &str,
    upgrades: &mut Vec<ModInstallUpgrade>,
    upgraded_files: &mut HashSet<String>,
) -> bool {
    if upgraded_files.contains(filename) {
        return true;
    }
    let Ok(Some(best)) = find_mod_version_for_predicate(engine, instance_name, filename, required).await else {
        return false;
    };
    let Ok(switch_plan) = plan_mod_version_switch(engine, instance_name, filename, &best.version_id).await else {
        return false;
    };
    upgraded_files.insert(switch_plan.filename.clone());
    upgrades.push(ModInstallUpgrade {
        filename: switch_plan.filename,
        project_id: switch_plan.project_id,
        title: switch_plan.title,
        from_version: switch_plan.from_version,
        to_version_id: switch_plan.to_version_id,
        to_version_number: switch_plan.to_version_number,
        to_filename: switch_plan.to_filename,
    });
    for c in switch_plan.companions {
        if upgraded_files.insert(c.filename.clone()) {
            upgrades.push(ModInstallUpgrade {
                filename: c.filename,
                project_id: c.project_id,
                title: c.title,
                from_version: c.from_version,
                to_version_id: c.to_version_id,
                to_version_number: c.to_version_number,
                to_filename: c.to_filename,
            });
        }
    }
    true
}

/// Commit a Modrinth install: optionally applies planned dependency/companion
/// upgrades and re-enables disabled deps first, reuses any SHA-1 verified staged
/// jar, then downloads and installs the mod + required dependencies.
pub async fn commit_modrinth_install(
    engine: &Engine,
    instance_name: &str,
    project_id: &str,
    apply_upgrades: bool,
    force: bool,
) -> EngineResult<ModInstallPlan> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let loader = instance.loader.content_loader();
    let plan = plan_mod_install(engine, &instance, project_id, loader).await?;
    if !plan.conflicts.is_empty() && !force {
        let titles: Vec<&str> = plan.conflicts.iter().map(|c| c.title.as_str()).collect();
        return Err(EngineError::api_verbatim(409, "mod_conflicts", titles.join(", ")));
    }

    if apply_upgrades {
        if let Ok(preview) = preview_modrinth_install(engine, instance_name, project_id).await {
            for fname in &preview.enable_disabled {
                let _ = set_mod_enabled(engine, instance_name, fname, true).await;
            }
            for up in &preview.upgrades {
                apply_mod_version_switch(engine, instance_name, &up.filename, &up.to_version_id, false).await?;
            }
        }
    }

    // Reuse the staged root jar if present & hash-valid so we don't download it twice.
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    if let Some(rf) = primary_file(&plan.root) {
        if let (Ok(fname), Some(sha)) = (safe_download_name(rf), file_sha1(rf)) {
            let st = staging_path(engine, &sha);
            let dest = mods_dir.join(&fname);
            if st.exists()
                && !dest.exists()
                && crate::download::sha1_file(&st).await.ok().as_deref() == Some(sha.as_str())
            {
                let _ = std::fs::copy(&st, &dest);
            }
        }
    }

    let reply = ModInstallPlan {
        queued: true,
        project: plan.project.clone(),
        dependencies: plan.dependencies.clone(),
        missing: plan.missing.clone(),
        optional: plan.optional.clone(),
        conflicts: plan.conflicts.clone(),
    };
    let bg = engine.clone();
    let n = instance_name.to_string();
    let pid = project_id.to_string();
    let l = loader.to_string();
    engine.spawn_background("mods", async move {
        if let Err(e) = install_modrinth_mod(&bg, &n, &pid, Some(&l), Some(plan)).await {
            tracing::error!("[mods] commit-modrinth failed for {n}: {e}");
        }
        Ok(())
    });
    Ok(reply)
}

/// Resolve a Fabric mod id (e.g. `fabric-api`, `cloth-config`) to a Modrinth
/// project that has a build for this instance's MC version + loader.
pub async fn resolve_modrinth_for_mod_id(
    engine: &Engine,
    instance_name: &str,
    mod_id: &str,
) -> EngineResult<Option<ModRef>> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let loader = instance.loader.content_loader();
    if let Some((pid, name)) = crate::fabric_compat::known_modrinth_for_id(mod_id) {
        if best_version_for_cached(engine, pid, &instance.version, loader).await.is_some() {
            return Ok(Some(ModRef { project_id: pid.to_string(), title: name.to_string() }));
        }
    }
    // Fallback: try Modrinth slug lookup (`GET /v2/project/{mod_id}`).
    let url = format!("{}/project/{}", api(engine), encode_uri_component(mod_id));
    if let Ok(proj) = fetch_json_with_retry(engine, &url, 1).await {
        if s(&proj, "project_type").as_deref() == Some("mod") {
            if let Some(pid) = s(&proj, "id") {
                if best_version_for_cached(engine, &pid, &instance.version, loader).await.is_some() {
                    let title = s(&proj, "title").unwrap_or_else(|| mod_id.to_string());
                    return Ok(Some(ModRef { project_id: pid, title }));
                }
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Version / platform change
// ---------------------------------------------------------------------------

fn obj(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Per-jar plan for a version/loader change (JS planModsForTarget). Rows:
/// keep / update / disable / idle / add (see the JS doc); brand jars excluded.
pub async fn plan_mods_for_target(
    engine: &Engine,
    instance: &InstanceMeta,
    target: &VersionTarget,
) -> EngineResult<Value> {
    let target_version = target.version.clone().unwrap_or_else(|| instance.version.clone());
    let target_loader = target.loader.unwrap_or(instance.loader);
    let from_loader = instance.loader;
    let mods_dir = effective_mods_dir(engine, instance);
    let mut files: Vec<String> =
        read_names(&mods_dir).into_iter().filter(|f| is_mod_file(f) && !is_brand_jar(f)).collect();
    files.sort();
    struct Row {
        filename: String,
        enabled: bool,
        sha1: String,
    }
    let mut rows = vec![];
    for file in files {
        let enabled = file.ends_with(".jar");
        let filename = if enabled { file.clone() } else { file[..file.len() - ".disabled".len()].to_string() };
        let Ok(sha1) = crate::download::sha1_file(&mods_dir.join(&file)).await else { continue };
        rows.push(Row { filename, enabled, sha1 });
    }
    if target_loader == Loader::Vanilla {
        return Ok(Value::Array(
            rows.into_iter()
                .map(|r| {
                    Value::Object(obj(vec![
                        ("filename", r.filename.clone().into()),
                        ("enabled", r.enabled.into()),
                        ("title", r.filename.into()),
                        ("action", "idle".into()),
                    ]))
                })
                .collect(),
        ));
    }
    let hashes: Vec<String> = rows.iter().map(|r| r.sha1.clone()).collect();
    let by_hash = if hashes.is_empty() {
        Map::new()
    } else {
        post_version_files(engine, &hashes).await.map_err(|status| match status {
            Some(st) => code_err(502, "MODRINTH_UNREACHABLE", format!("Modrinth version lookup failed (HTTP {st})")),
            None => {
                code_err(502, "MODRINTH_UNREACHABLE", "Modrinth is unreachable; cannot check mods for the new version")
            }
        })?
    };
    let pins = if target_loader == Loader::Fabric {
        pins_for_version(engine, &target_version).await.unwrap_or_default()
    } else {
        vec![]
    };
    let mut present = HashSet::new();
    let mut out: Vec<Map<String, Value>> = vec![];
    for row in &rows {
        let base = |extra: Vec<(&str, Value)>| {
            let mut m = obj(vec![
                ("filename", row.filename.clone().into()),
                ("enabled", row.enabled.into()),
                ("title", row.filename.clone().into()),
            ]);
            for (k, v) in extra {
                m.insert(k.into(), v);
            }
            m
        };
        let Some(pid) = by_hash.get(&row.sha1).and_then(|c| s(c, "project_id")) else {
            out.push(base(vec![("action", "disable".into()), ("reason", "unknown".into())]));
            continue;
        };
        present.insert(pid.clone());
        let pin = pins.iter().find(|p| p.project_id == pid);
        let mut next = None;
        if let Some(p) = pin {
            next = version_meta(engine, &p.version_id).await.ok();
        }
        if next.is_none() {
            next = best_version_for_cached(engine, &pid, &target_version, target_loader.as_str()).await;
        }
        let file = next.as_ref().and_then(primary_file);
        match (file, &next) {
            (Some(f), Some(n)) if file_sha1(f).as_deref() == Some(row.sha1.as_str()) => {
                let _ = n;
                out.push(base(vec![("project_id", pid.clone().into()), ("action", "keep".into())]));
            }
            (Some(f), Some(n)) => out.push(base(vec![
                ("project_id", pid.clone().into()),
                ("action", "update".into()),
                ("bundle", pin.is_some().into()),
                ("version_id", s(n, "id").map(Value::String).unwrap_or(Value::Null)),
                ("to", json!({ "filename": file_name(f), "version_number": n.get("version_number").cloned().unwrap_or(Value::Null) })),
            ])),
            _ => out.push(base(vec![
                ("project_id", pid.clone().into()),
                ("action", "disable".into()),
                ("reason", "no_build".into()),
            ])),
        }
    }
    if target_loader == Loader::Fabric && from_loader != Loader::Fabric {
        for pin in &pins {
            if present.contains(&pin.project_id) {
                continue;
            }
            let Ok(meta) = version_meta(engine, &pin.version_id).await else { continue };
            let Some(f) = primary_file(&meta) else { continue };
            out.push(obj(vec![
                ("filename", file_name(f).into()),
                ("enabled", true.into()),
                ("title", pin.slug.clone().into()),
                ("project_id", pin.project_id.clone().into()),
                ("action", "add".into()),
                ("bundle", true.into()),
                ("version_id", pin.version_id.clone().into()),
                ("to", json!({ "filename": file_name(f), "version_number": meta.get("version_number").cloned().unwrap_or(Value::Null) })),
            ]));
        }
    }
    let ids: Vec<String> =
        out.iter().filter_map(|e| e.get("project_id").and_then(Value::as_str).map(str::to_string)).collect();
    let titles = project_titles(engine, &ids).await;
    for e in &mut out {
        if let Some(t) = e.get("project_id").and_then(Value::as_str).and_then(|p| titles.get(p)).cloned() {
            e.insert("title".into(), Value::String(t));
        }
    }
    Ok(Value::Array(out.into_iter().map(Value::Object).collect()))
}

/// Apply a plan from plan_mods_for_target -> `{ updated, added, disabled, errors }`.
pub async fn apply_mods_plan(engine: &Engine, instance_name: &str, plan: &Value) -> EngineResult<Value> {
    let instance = crate::resolver::load_instance_meta(engine, instance_name)?;
    let mods_dir = effective_mods_dir(engine, &instance);
    std::fs::create_dir_all(&mods_dir)?;
    let mut updated = vec![];
    let mut added = vec![];
    let disabled = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut errors = vec![];
    let disable = |filename: String| {
        let disabled = disabled.clone();
        let mods_dir = mods_dir.clone();
        async move {
            if mods_dir.join(&filename).exists() {
                set_mod_enabled(engine, instance_name, &filename, false).await?;
            }
            disabled.lock().unwrap_or_else(|p| p.into_inner()).push(filename);
            Ok::<_, EngineError>(())
        }
    };
    for entry in plan.as_array().cloned().unwrap_or_default() {
        let filename = s(&entry, "filename").unwrap_or_default();
        let action = s(&entry, "action").unwrap_or_default();
        let result: EngineResult<()> = async {
            match action.as_str() {
                "disable" => disable(filename.clone()).await?,
                "update" | "add" => {
                    let new_file = if entry.get("bundle") == Some(&Value::Bool(true)) {
                        let vid = s(&entry, "version_id").unwrap_or_default();
                        let meta = version_meta(engine, &vid).await?;
                        let file = primary_file(&meta).ok_or_else(|| no_file_error(&meta))?;
                        let fname = safe_download_name(file)?;
                        engine
                            .http()
                            .download_all(
                                vec![DownloadItem::new(s(file, "url").unwrap_or_default(), mods_dir.join(&fname))
                                    .sha1(file_sha1(file))
                                    .size(file.get("size").and_then(Value::as_u64))],
                                DownloadAllOptions { concurrency: Some(1), on_item: None },
                            )
                            .await?;
                        fname
                    } else {
                        let pid = s(&entry, "project_id").unwrap_or_default();
                        let res =
                            install_modrinth_mod(engine, instance_name, &pid, Some(instance.loader.as_str()), None)
                                .await?;
                        res.installed.first().cloned().unwrap_or_default()
                    };
                    if action == "update" {
                        if filename != new_file {
                            let _ = std::fs::remove_file(mods_dir.join(&filename));
                            let _ = std::fs::remove_file(mods_dir.join(format!("{filename}.disabled")));
                        }
                        if entry.get("enabled") == Some(&Value::Bool(false)) {
                            set_mod_enabled(engine, instance_name, &new_file, false).await?;
                        }
                        updated.push(json!({ "from": filename, "to": new_file }));
                    } else {
                        added.push(Value::String(new_file));
                    }
                }
                _ => {}
            }
            Ok(())
        }
        .await;
        if let Err(err) = result {
            errors.push(json!({ "filename": filename, "error": err.to_string() }));
            if action == "update" {
                let _ = disable(filename.clone()).await;
            }
        }
    }
    let disabled: Vec<Value> =
        disabled.lock().unwrap_or_else(|p| p.into_inner()).drain(..).map(Value::String).collect();
    Ok(json!({ "updated": updated, "added": added, "disabled": disabled, "errors": errors }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockRequest, MockResponse, MockServer};

    fn branding_assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("branding")
    }

    fn engine_with(server: Option<&MockServer>) -> (tempfile::TempDir, Engine) {
        let (d, e) = temp_engine();
        let api = server.map(|s| s.url("/v2")).unwrap_or_else(|| "http://127.0.0.1:9/v2".into());
        e.update_content_settings(|s| {
            s.modrinth_api = api;
            s.modrinth_retry_backoff = std::time::Duration::from_millis(1);
            s.branding_dir = Some(branding_assets());
            s.background_on_create = false;
        });
        (d, e)
    }

    fn write_instance(e: &Engine, name: &str, version: &str, loader: &str) -> PathBuf {
        let dir = e.paths().instance_dir(name);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(
            dir.join("instance.json"),
            json!({ "name": name, "version": version, "loader": loader }).to_string(),
        )
        .unwrap();
        dir.join("mods")
    }

    #[test]
    fn performance_bundle_support() {
        for v in ["1.21.11", "26.2", "1.22.4", "21.0", "1.21", "1.26", "21", "26"] {
            assert!(supports_performance_bundle(v), "{v}");
        }
        for v in ["1.20.4", "25w11a", "1.15.2", "27.0", "1.27.0", "27", "1.21.11-pre1", "", "not-a-version"] {
            assert!(!supports_performance_bundle(v), "{v}");
        }
        assert!(pin_note_for_version("1.20.4").unwrap().contains("1.21 through 26.x"));
    }

    #[tokio::test]
    async fn pins_exact_and_unsupported_offline() {
        let (_d, e) = engine_with(None);
        for v in PINNED_VERSIONS {
            let pins = pins_for_version(&e, v).await.unwrap();
            assert_eq!(pins.len(), 8, "{v}");
            assert_eq!(pins, pins_by_version(v).unwrap());
            // Every pinned project is in the dynamic list too, so a version
            // without an exact table resolves the same set.
            for p in &pins {
                assert!(PERFORMANCE_PROJECTS.iter().any(|(_, pid)| *pid == p.project_id), "{v} {}", p.slug);
            }
        }
        for v in ["1.20.4", "1.15.2", "25w11a", "27.0"] {
            assert!(pins_for_version(&e, v).await.is_none(), "{v}");
        }
        assert_eq!(pins_by_version("26.3").unwrap()[0].version_id, "bAZQdGpg");
    }

    /// Each table pins one build per project (a duplicated slug would download
    /// two jars of the same mod) and the same set of slugs on every version, so
    /// the preset means the same thing whichever lane a player is on.
    #[test]
    fn pin_tables_are_unique_and_aligned() {
        let mut expected: Option<Vec<String>> = None;
        for v in PINNED_VERSIONS {
            let pins = pins_by_version(v).unwrap();
            let mut slugs: Vec<String> = pins.iter().map(|p| p.slug.clone()).collect();
            slugs.sort();
            let before = slugs.len();
            slugs.dedup();
            assert_eq!(slugs.len(), before, "{v} pins a project twice");
            match &expected {
                None => expected = Some(slugs),
                Some(e) => assert_eq!(&slugs, e, "{v} pins a different project set"),
            }
            assert!(pins.iter().any(|p| p.slug == "immediatelyfast"), "{v}");
        }
    }

    /// A jar left over from a previous pin build is disabled (never deleted) so
    /// two builds of one mod cannot be loaded at once, and a jar that Modrinth
    /// does not attribute to a pinned project is left alone.
    #[test]
    fn superseded_pinned_jars_only_disables_known_old_pins() {
        let pins = pins_by_version("26.3").unwrap();
        let candidates = vec![
            ("sodium-fabric-0.9.1+mc26.3.jar".to_string(), "old-sodium".to_string()),
            ("my-own-pack.jar".to_string(), "unknown".to_string()),
        ];
        let mut by_hash = Map::new();
        by_hash.insert("old-sodium".into(), json!({ "project_id": "AANobbMI" }));
        let stale = superseded_pinned_jars(&candidates, &by_hash, &pins);
        assert_eq!(stale, vec!["sodium-fabric-0.9.1+mc26.3.jar"]);
    }

    #[test]
    fn branding_support_and_pins_match_bundled_jars() {
        let (_d, e) = engine_with(None);
        assert!(supports_branding(&e, "1.21.11", Loader::Fabric));
        assert!(supports_branding(&e, "26.2", Loader::Fabric));
        assert!(supports_branding(&e, "1.21.1", Loader::Neoforge));
        assert!(supports_branding(&e, "1.8.9", Loader::Forge));
        assert!(!supports_branding(&e, "1.8.9", Loader::Vanilla));
        assert!(!supports_branding(&e, "1.12.2", Loader::Forge));
        assert!(!supports_branding(&e, "1.21.1", Loader::Fabric));
        assert!(!supports_branding(&e, "1.21.11", Loader::Neoforge));
        assert!(!supports_branding(&e, "1.21.11", Loader::Vanilla));
        assert!(!supports_branding(&e, "1.22.4", Loader::Fabric));
        assert!(!supports_branding(&e, "", Loader::Fabric));
        let versions = branding_versions(&e);
        for v in ["1.21.11", "26.2", "1.21.1", "1.8.9"] {
            assert!(versions.iter().any(|x| x == v), "{v}");
        }
        for (v, loader, prefix) in [
            ("1.21.11", Loader::Fabric, "espectral-menu"),
            ("26.2", Loader::Fabric, "espectral-menu"),
            ("1.21.1", Loader::Neoforge, "espectral-brand"),
            ("1.8.9", Loader::Forge, "espectral-menu"),
        ] {
            let pin = branding_pin_for_version(&e, v, loader).unwrap();
            assert_eq!(pin.slug, prefix);
            assert_eq!(pin.filename, format!("{prefix}-{v}-{}.jar", pin.version_number.clone().unwrap()));
            let data = std::fs::read(branding_assets().join(&pin.filename)).unwrap();
            assert_eq!(pin.sha1.as_deref(), Some(crate::download::sha1_hex(&data).as_str()));
            assert_eq!(pin.size, Some(data.len() as u64));
        }
        assert!(branding_pin_for_version(&e, "1.20.4", Loader::Fabric).is_none());
    }

    /// The NeoForge jar only carries the brand handshake: the wizard must not
    /// promise zoom/fullbright there, and the list it shows is newest-first.
    #[test]
    fn suite_coverage_excludes_brand_only_jars() {
        let (_d, e) = engine_with(None);
        assert!(supports_suite(&e, "1.21.11", Loader::Fabric));
        assert!(supports_suite(&e, "1.8.9", Loader::Forge));
        assert!(supports_branding(&e, "1.21.1", Loader::Neoforge));
        assert!(!supports_suite(&e, "1.21.1", Loader::Neoforge));
        assert!(!supports_suite(&e, "1.21.8", Loader::Fabric));
        let targets = suite_targets(&e);
        assert!(targets.contains(&(Loader::Fabric, "1.21.11".into())));
        assert!(targets.contains(&(Loader::Forge, "1.8.9".into())));
        assert!(!targets.iter().any(|(l, _)| *l == Loader::Neoforge));
        let fabric: Vec<&str> = targets.iter().filter(|(l, _)| *l == Loader::Fabric).map(|(_, v)| v.as_str()).collect();
        let pos = |v: &str| fabric.iter().position(|x| *x == v).unwrap();
        assert!(pos("26.2") < pos("1.21.11"), "newest first: {fabric:?}");
        assert_eq!(targets.last().unwrap().0, Loader::Forge, "{targets:?}");
    }

    #[tokio::test]
    async fn list_mods_tags_branding_even_offline() {
        let server = MockServer::start(|_| MockResponse::status(404)).await;
        let (_d, e) = engine_with(Some(&server));
        for (name, version, loader, slug) in [
            ("brand-fabric-test", "1.21.11", Loader::Fabric, "espectral-menu"),
            ("brand-neoforge-test", "1.21.1", Loader::Neoforge, "espectral-brand"),
        ] {
            let mods = write_instance(&e, name, version, loader.as_str());
            let pin = branding_pin_for_version(&e, version, loader).unwrap();
            std::fs::copy(branding_assets().join(&pin.filename), mods.join(&pin.filename)).unwrap();
            std::fs::write(mods.join("other.jar.disabled"), "x").unwrap();
            let rows = list_mods(&e, name).await.unwrap();
            let row = rows.iter().find(|r| r.filename == pin.filename).expect("branding jar listed");
            assert_eq!(row.project_slug.as_deref(), Some(slug));
            assert_eq!(row.version_number, pin.version_number);
            assert!(row.installed && row.enabled);
            assert_eq!(row.sha1, pin.sha1);
            let other = rows.iter().find(|r| r.filename == "other.jar").unwrap();
            assert!(!other.enabled && other.project_slug.is_none());
            if version == "1.21.11" {
                // the 8 pins are listed as not-installed rows even when Modrinth is down
                assert_eq!(rows.iter().filter(|r| !r.installed).count(), 8);
                assert!(rows
                    .iter()
                    .any(|r| r.filename == "sodium.jar"
                        && r.version_number.as_deref() == Some("mc1.21.11-0.8.14-fabric")));
            }
        }
        let err = list_mods(&e, "missing").await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("instance_not_found")));
    }

    #[tokio::test]
    async fn ensure_branding_seeded_never_rewrites_under_open_handle() {
        let (_d, e) = engine_with(None);
        let pin = branding_pin_for_version(&e, "26.2", Loader::Fabric).unwrap();
        let mods = e.paths().instance_dir("reseed-test").join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let jar = mods.join(&pin.filename);
        let stale = b"stale brand jar bytes from the previous launcher".to_vec();
        std::fs::write(&jar, &stale).unwrap();
        std::fs::write(mods.join("espectral-menu-26.2-0.0.1.jar"), "old").unwrap();
        let inst = InstanceMeta {
            name: "reseed-test".into(),
            version: "26.2".into(),
            loader: Loader::Fabric,
            ..Default::default()
        };
        {
            use std::io::Read;
            let mut fh = std::fs::File::open(&jar).unwrap();
            ensure_branding_seeded(&e, &inst).await.unwrap();
            if cfg!(unix) {
                let mut seen = vec![];
                fh.read_to_end(&mut seen).unwrap();
                assert_eq!(seen, stale, "the open handle still reads the bytes it opened");
            }
        }
        let names = read_names(&mods);
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "no temp file left: {names:?}");
        assert!(!mods.join("espectral-menu-26.2-0.0.1.jar").exists(), "stale brand jar pruned");
        ensure_branding_seeded(&e, &inst).await.unwrap();
        assert_eq!(crate::download::sha1_file(&jar).await.unwrap(), pin.sha1.unwrap());
    }

    #[tokio::test]
    async fn install_preset_codes_and_notes() {
        let (_d, e) = engine_with(None);
        assert_eq!(install_preset(&e, "Invalid!", ModPreset::Branding).await.unwrap_err().status(), 409);
        assert_eq!(install_preset(&e, "no-such-instance", ModPreset::Branding).await.unwrap_err().status(), 404);
        write_instance(&e, "vanilla", "1.21.11", "vanilla");
        let r = install_preset(&e, "vanilla", ModPreset::Performance).await.unwrap();
        assert!(r.installed.is_empty() && r.notes[0].contains("needs a Fabric instance"));
        write_instance(&e, "old", "1.20.4", "fabric");
        let r = install_preset(&e, "old", ModPreset::Performance).await.unwrap();
        assert!(r.notes[0].contains("no performance mod bundle"));
        let mods = write_instance(&e, "brand", "26.3", "fabric");
        let r = install_preset(&e, "brand", ModPreset::Branding).await.unwrap();
        // the jar name follows the bundled assets/branding/branding.json (bumped every release)
        let jar = branding_pin_for_version(&e, "26.3", Loader::Fabric).unwrap().filename;
        assert!(jar.starts_with("espectral-menu-26.3-"), "{jar}");
        assert_eq!(r.installed, vec![jar.clone()]);
        assert!(mods.join(&jar).exists());
    }

    /// Modrinth's `files[].filename` becomes `<mods>/<filename>`: a crafted or
    /// compromised response must not reach `../instance.json` or another drive.
    #[test]
    fn download_names_must_be_plain_basenames() {
        for bad in [
            "../instance.json",
            "a/b.jar",
            "a\\b.jar",
            "C:x.jar",
            "..",
            ".hidden.jar",
            "x.jar.",
            "",
            "x\n.jar",
            "con.jar",
            "COM1.jar",
        ]
        {
            assert!(safe_download_name(&json!({ "filename": bad })).is_err(), "{bad:?}");
        }
        assert_eq!(
            safe_download_name(&json!({ "filename": "sodium-fabric-0.8.7+mc1.21.jar" })).unwrap(),
            "sodium-fabric-0.8.7+mc1.21.jar"
        );
    }

    #[test]
    fn filename_slug_helpers() {
        assert!(is_filename_for_slug("sodium-fabric-0.8.7.jar", "sodium"));
        assert!(is_filename_for_slug("ferrite-core-8.2.0.jar", "ferrite-core"));
        assert!(is_filename_for_slug("iris-mc1.21.1.jar", "iris"));
        assert!(is_filename_for_slug("modmenu_1.0.jar", "modmenu"));
        assert!(is_filename_for_slug("lithium.jar", "lithium"));
        assert!(is_filename_for_slug("a.jar", "a"));
        assert!(!is_filename_for_slug("lithium.jar", "sodium"));
        assert!(!is_filename_for_slug("ab.jar", "a"));
        assert!(is_filename_for_slug("SODIUM-FABRIC-0.8.7.JAR", "sodium"));
        assert!(is_filename_for_slug("sodium-fabric-0.8.7.jar.disabled", "sodium"));
        assert!(is_filename_for_slug("ferrite-core-8.2.0.jar", "Ferrite-Core"));
        assert_eq!(slugify_title("Ferrite Core"), "ferrite-core");
        assert_eq!(slugify_title("Sodium Extra"), "sodium-extra");
        assert_eq!(slugify_title("  X Y Z  "), "x-y-z");
        assert_eq!(slugify_title(""), "");
        assert_eq!(encode_uri_component("[\"26.2\"]"), "%5B%2226.2%22%5D");
        assert!(assert_safe_filename("a b+c@1.jar").is_ok());
        for bad in ["../x.jar", "x.zip", ".jar", "a/b.jar"] {
            assert_eq!(assert_safe_filename(bad).unwrap_err().raw_code(), Some("invalid_filename"), "{bad}");
        }
    }

    #[tokio::test]
    async fn set_mod_enabled_renames() {
        let (_d, e) = engine_with(None);
        let mods = write_instance(&e, "toggle", "1.21.11", "fabric");
        std::fs::write(mods.join("x.jar"), "x").unwrap();
        assert_eq!(set_mod_enabled(&e, "toggle", "x.jar", false).await.unwrap(), "x.jar");
        assert!(mods.join("x.jar.disabled").exists() && !mods.join("x.jar").exists());
        set_mod_enabled(&e, "toggle", "x.jar.disabled", true).await.unwrap();
        assert!(mods.join("x.jar").exists());
        let err = set_mod_enabled(&e, "toggle", "nope.jar", true).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("mod_not_found")));
    }

    /// The set moved to a newer build of a mod the instance already has: that
    /// pin is not listed as a switched-off row (its switch had no file and
    /// failed with "mod not found"). A pin that really is missing still is,
    /// and turning it on downloads it.
    #[tokio::test]
    async fn superseded_pins_are_hidden_and_missing_pins_install_on_enable() {
        let pins = pins_by_version("1.21.11").unwrap();
        let old = pins.iter().find(|p| p.slug == "sodium").unwrap();
        let missing = pins.iter().find(|p| p.slug == "lithium").unwrap();
        let scratch = tempfile::tempdir().unwrap();
        std::fs::write(scratch.path().join("old.jar"), "old sodium build").unwrap();
        let old_hash = crate::download::sha1_file(&scratch.path().join("old.jar")).await.unwrap();

        let srv = MockServer::start(fake_modrinth(json!({}), json!({}), json!({}))).await;
        let cdn = srv.url("/cdn");
        let pinned = json!({
            missing.version_id.clone(): modrinth_version("lithium", &missing.version_id, json!([]), &format!("{cdn}/lithium.jar")),
        });
        let server = MockServer::start(fake_modrinth(json!({}), pinned, json!({ old_hash: old.project_id.clone() }))).await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "superseded", "1.21.11", "fabric");
        std::fs::copy(scratch.path().join("old.jar"), mods.join("sodium-fabric-0.0.1.jar")).unwrap();

        let rows = list_mods(&e, "superseded").await.unwrap();
        assert!(
            !rows.iter().any(|r| !r.installed && r.project_slug.as_deref() == Some("sodium")),
            "superseded sodium pin listed: {rows:?}"
        );
        let row = rows.iter().find(|r| !r.installed && r.project_slug.as_deref() == Some("lithium")).unwrap();
        assert_eq!(row.filename, "lithium-1.0.0.jar");

        assert_eq!(set_mod_enabled(&e, "superseded", &row.filename, true).await.unwrap(), "lithium-1.0.0.jar");
        assert_eq!(std::fs::read_to_string(mods.join("lithium-1.0.0.jar")).unwrap(), "jar:lithium.jar");
    }

    fn modrinth_version(project: &str, version: &str, deps: Value, file_url: &str) -> Value {
        json!({
            "id": version, "project_id": project, "version_number": "1.0.0", "status": "listed",
            "dependencies": deps,
            "files": [{ "primary": true, "filename": format!("{project}-1.0.0.jar"), "url": file_url, "hashes": {}, "size": null }]
        })
    }

    /// Fake Modrinth: versions per project, pinned versions, installed-by-hash, titles, jar bytes.
    fn fake_modrinth(
        versions: Value,
        pinned: Value,
        installed_by_hash: Value,
    ) -> impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static {
        move |req: &MockRequest| {
            let path = req.path.clone();
            if let Some(rest) = path.strip_prefix("/cdn/") {
                return MockResponse::ok(&format!("jar:{rest}"));
            }
            if path.ends_with("/version_files") && req.method == "POST" {
                let body = req.body_json().unwrap();
                let mut out = Map::new();
                for h in body["hashes"].as_array().unwrap() {
                    if let Some(p) = installed_by_hash.get(h.as_str().unwrap()) {
                        out.insert(h.as_str().unwrap().into(), json!({ "project_id": p }));
                    }
                }
                return MockResponse::json(&Value::Object(out));
            }
            if let Some(rest) = path.strip_prefix("/v2/project/") {
                let id = rest.split('/').next().unwrap();
                return MockResponse::json(&versions.get(id).cloned().unwrap_or(json!([])));
            }
            if let Some(id) = path.strip_prefix("/v2/version/") {
                if let Some(v) = pinned.get(id) {
                    return MockResponse::json(v);
                }
            }
            if let Some(q) = path.strip_prefix("/v2/projects?ids=") {
                let decoded = url::form_urlencoded::parse(format!("x={q}").as_bytes()).next().unwrap().1.into_owned();
                let ids: Vec<String> = serde_json::from_str(&decoded).unwrap();
                return MockResponse::json(&Value::Array(
                    ids.iter()
                        .map(|id| json!({ "id": id, "title": format!("Title {id}"), "slug": id.to_lowercase() }))
                        .collect(),
                ));
            }
            MockResponse::status(404)
        }
    }

    #[tokio::test]
    async fn plan_mod_install_classifies_dependencies() {
        let sha = crate::download::sha1_hex;
        let server = MockServer::start(fake_modrinth(
            json!({
                "PROOT1": [modrinth_version("PROOT1", "root-v", json!([
                    { "project_id": "PDEPB1", "dependency_type": "required" },
                    { "project_id": "PAPI01", "dependency_type": "required" },
                    { "project_id": "PEMB01", "dependency_type": "embedded" },
                    { "project_id": "PMISS1", "dependency_type": "required" },
                    { "project_id": "POPT01", "dependency_type": "optional" },
                    { "project_id": "PENEMY", "dependency_type": "incompatible" }
                ]), "http://x/cdn/r")],
                "PDEPB1": [modrinth_version("PDEPB1", "b-v", json!([{ "project_id": "PDEPC1", "version_id": "c-pinned", "dependency_type": "required" }]), "http://x/cdn/b")],
                "PMISS1": []
            }),
            json!({ "c-pinned": modrinth_version("PDEPC1", "c-pinned", json!([]), "http://x/cdn/c") }),
            json!({ sha(b"installed-api"): "PAPI01", sha(b"installed-enemy"): "PENEMY" }),
        ))
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "deps-plan", "26.2", "fabric");
        std::fs::write(mods.join("whatever-api.jar"), "installed-api").unwrap();
        std::fs::write(mods.join("old.jar"), "installed-enemy").unwrap();
        let inst = crate::resolver::load_instance_meta(&e, "deps-plan").unwrap();
        let plan = plan_mod_install(&e, &inst, "PROOT1", "fabric").await.unwrap();
        let ids = |v: &[ModRef]| v.iter().map(|r| r.project_id.clone()).collect::<Vec<_>>();
        assert_eq!(
            plan.installs.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            vec!["PROOT1", "PDEPB1", "PDEPC1"]
        );
        assert_eq!(
            plan.dependencies.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            vec!["Title PDEPB1", "Title PDEPC1"]
        );
        assert_eq!(ids(&plan.missing), vec!["PMISS1"]);
        assert_eq!(ids(&plan.optional), vec!["POPT01"]);
        assert_eq!(ids(&plan.conflicts), vec!["PENEMY"]);
        assert_eq!(plan.project.title, "Title PROOT1");
        let err = plan_mod_install(&e, &inst, "NOPE", "fabric").await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("MOD_NOT_FOUND_FOR_VERSION")));
    }

    #[tokio::test]
    async fn install_modrinth_mod_downloads_mod_and_missing_deps() {
        let srv = MockServer::start(fake_modrinth(json!({}), json!({}), json!({}))).await;
        let cdn = srv.url("/cdn");
        let server = MockServer::start(fake_modrinth(
            json!({
                "PROOT2": [modrinth_version("PROOT2", "root2-v", json!([{ "project_id": "PDEPB2", "dependency_type": "required" }]), &format!("{cdn}/PROOT2.jar"))],
                "PDEPB2": [modrinth_version("PDEPB2", "b2-v", json!([]), &format!("{cdn}/PDEPB2.jar"))]
            }),
            json!({}),
            json!({}),
        ))
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "deps-install", "26.2", "fabric");
        let mut rx = e.subscribe();
        let res = install_modrinth_mod(&e, "deps-install", "PROOT2", Some("fabric"), None).await.unwrap();
        let mut installed = res.installed.clone();
        installed.sort();
        assert_eq!(installed, vec!["PDEPB2-1.0.0.jar", "PROOT2-1.0.0.jar"]);
        let mut on_disk = read_names(&mods);
        on_disk.sort();
        assert_eq!(on_disk, installed);
        assert_eq!(std::fs::read_to_string(mods.join("PROOT2-1.0.0.jar")).unwrap(), "jar:PROOT2.jar");
        let mut phases = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let EngineEvent::ModProgress(p) = ev {
                phases.push(p.phase.unwrap_or_default());
            }
        }
        assert_eq!(phases.first().map(String::as_str), Some("start"));
        assert_eq!(phases.last().map(String::as_str), Some("done"));
        assert_eq!(phases.iter().filter(|p| *p == "done").count(), 3, "{phases:?}");
    }

    #[test]
    fn duplicate_mod_ids_keep_the_newest_build() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let jar = |file: &str, fmj: &str| {
            let mut w = zip::ZipWriter::new(std::fs::File::create(d.path().join(file)).unwrap());
            w.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(fmj.as_bytes()).unwrap();
            w.finish().unwrap();
        };
        // Name order puts the newer Sodium first; versions decide, not names.
        jar("sodium-fabric-0.7.10+mc1.21.8.jar", r#"{"id":"sodium","version":"0.7.10+mc1.21.8"}"#);
        jar("sodium-fabric-0.7.2+mc1.21.8.jar", r#"{"id":"sodium","version":"0.7.2+mc1.21.8"}"#);
        jar("iris.jar", r#"{"id":"iris","version":"1.9.6+mc1.21.8"}"#);
        // Unparseable versions are Fabric's to report, never guessed at.
        jar("odd-a.jar", r#"{"id":"odd","version":"r12"}"#);
        jar("odd-b.jar", r#"{"id":"odd","version":"r13"}"#);
        let healed = disable_duplicate_mod_ids(d.path());
        assert_eq!(
            healed,
            vec![("sodium-fabric-0.7.2+mc1.21.8.jar".to_string(), "sodium-fabric-0.7.10+mc1.21.8.jar".to_string())]
        );
        assert!(d.path().join("sodium-fabric-0.7.2+mc1.21.8.jar.disabled").exists());
        assert!(d.path().join("odd-a.jar").exists() && d.path().join("odd-b.jar").exists());
        assert!(disable_duplicate_mod_ids(d.path()).is_empty(), "second launch finds nothing to do");
    }

    /// Dynamic pins (1.21.8): Iris names the exact Sodium build it shipped
    /// with (an older one than the set's newest). The set must keep ONE
    /// Sodium — two jars with mod id `sodium` stop Fabric at boot.
    #[tokio::test]
    async fn install_preset_keeps_one_build_per_pinned_project() {
        let srv = MockServer::start(fake_modrinth(json!({}), json!({}), json!({}))).await;
        let cdn = srv.url("/cdn");
        let mut versions = Map::new();
        let mut pinned = Map::new();
        for (slug, pid) in PERFORMANCE_PROJECTS {
            let deps = if slug == "iris" {
                json!([{ "project_id": "AANobbMI", "version_id": "sodium-old", "dependency_type": "required" }])
            } else {
                json!([])
            };
            let v = modrinth_version(pid, &format!("{slug}-new"), deps, &format!("{cdn}/{slug}.jar"));
            versions.insert(pid.into(), json!([v.clone()]));
            pinned.insert(format!("{slug}-new"), v);
        }
        let mut old = modrinth_version("AANobbMI", "sodium-old", json!([]), &format!("{cdn}/sodium-old.jar"));
        old["files"][0]["filename"] = json!("sodium-old.jar");
        pinned.insert("sodium-old".into(), old);
        let server = MockServer::start(fake_modrinth(Value::Object(versions), Value::Object(pinned), json!({}))).await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "dyn", "1.21.8", "fabric");
        let r = install_preset(&e, "dyn", ModPreset::Performance).await.unwrap();
        assert_eq!(r.installed.len(), 8, "{:?}", r.installed);
        assert!(r.installed.contains(&"AANobbMI-1.0.0.jar".to_string()));
        assert!(!mods.join("sodium-old.jar").exists(), "{:?}", read_names(&mods));
    }

    #[tokio::test]
    async fn search_maps_hits_and_installed_flags() {
        let server = MockServer::start(|req| {
            assert!(req.path.starts_with("/v2/search?query="), "{}", req.path);
            MockResponse::json(&json!({
                "hits": [
                    { "project_id": "uXXizFIs", "slug": "ferrite-core", "title": "FerriteCore", "description": "d",
                      "icon_url": null, "downloads": 5, "versions": ["26.2"], "latest_version": "9.0.0" },
                    { "project_id": "x", "slug": "other", "title": "Ferrite Core", "description": "", "downloads": 1, "versions": [] }
                ],
                "total_hits": 40
            }))
        })
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "s", "26.2", "fabric");
        std::fs::write(mods.join("ferrite-core-9.0.0.jar.disabled"), "x").unwrap();
        let r = search_modrinth(&e, "fer core", Some("26.2"), Some("fabric"), Some("s"), 12).await.unwrap();
        assert_eq!((r.total, r.offset, r.limit), (40, 12, 12));
        assert_eq!(r.results[0].installed, Some(true));
        assert_eq!(r.results[1].installed, Some(true), "slugified title matches");
        let facets = server.requests()[0].path.clone();
        assert!(facets.starts_with("/v2/search?query=fer+core&facets="), "{facets}");
        assert!(facets.contains("limit=12&offset=12&index=downloads"));
        let q: Vec<(String, String)> =
            url::form_urlencoded::parse(facets.split_once('?').unwrap().1.as_bytes()).into_owned().collect();
        assert_eq!(q[1].1, r#"[["project_type:mod"],["loaders:fabric"],["versions:26.2"]]"#);
        let err = search_modrinth(&e, "", None, None, Some("bad/name"), 0).await.unwrap_err();
        assert_eq!(err.status(), 409);
    }

    #[tokio::test]
    async fn search_failure_is_modrinth_search_failed() {
        let server = MockServer::start(|_| MockResponse::status(404)).await;
        let (_d, e) = engine_with(Some(&server));
        let err = search_modrinth(&e, "", None, None, None, 0).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (404, Some("MODRINTH_SEARCH_FAILED")));
        assert_eq!(server.hits(), 3, "2 retries");
    }

    #[tokio::test]
    async fn version_change_plan_and_apply() {
        let sha = crate::download::sha1_hex;
        let srv = MockServer::start(|_| MockResponse::status(404)).await;
        let cdn = srv.url("/cdn");
        drop(srv);
        let server = MockServer::start(fake_modrinth(
            json!({
                "PKEEP": [{ "id": "k2", "project_id": "PKEEP", "version_number": "2", "status": "listed",
                            "files": [{ "primary": true, "filename": "keep.jar", "url": "u", "hashes": { "sha1": sha(b"keep") } }] }],
                "PUPD": [{ "id": "u2", "project_id": "PUPD", "version_number": "2.0", "status": "listed",
                           "files": [{ "primary": true, "filename": "upd-2.jar", "url": format!("{cdn}/upd"), "hashes": {} }] }]
            }),
            json!({}),
            json!({ sha(b"keep"): "PKEEP", sha(b"upd"): "PUPD", sha(b"gone"): "PGONE" }),
        ))
        .await;
        let (_d, e) = engine_with(Some(&server));
        let mods = write_instance(&e, "vc", "26.2", "fabric");
        std::fs::write(mods.join("keep.jar"), "keep").unwrap();
        std::fs::write(mods.join("upd-1.jar.disabled"), "upd").unwrap();
        std::fs::write(mods.join("gone.jar"), "gone").unwrap();
        std::fs::write(mods.join("random.jar"), "random").unwrap();
        std::fs::write(mods.join("espectral-menu-26.2-1.6.2.jar"), "brand").unwrap();
        let inst = crate::resolver::load_instance_meta(&e, "vc").unwrap();

        let idle = plan_mods_for_target(&e, &inst, &VersionTarget { version: None, loader: Some(Loader::Vanilla) })
            .await
            .unwrap();
        assert_eq!(idle.as_array().unwrap().len(), 4);
        assert!(idle.as_array().unwrap().iter().all(|r| r["action"] == "idle"));

        let target = VersionTarget { version: Some("1.21.5".into()), loader: Some(Loader::Neoforge) };
        let plan = plan_mods_for_target(&e, &inst, &target).await.unwrap();
        let by: std::collections::HashMap<String, Value> =
            plan.as_array().unwrap().iter().map(|r| (r["filename"].as_str().unwrap().to_string(), r.clone())).collect();
        assert_eq!(by["keep.jar"]["action"], "keep");
        assert_eq!(by["keep.jar"]["title"], "Title PKEEP");
        assert_eq!(by["upd-1.jar"]["action"], "update");
        assert_eq!(by["upd-1.jar"]["to"]["filename"], "upd-2.jar");
        assert_eq!(by["upd-1.jar"]["enabled"], false);
        assert_eq!(by["gone.jar"]["reason"], "no_build");
        assert_eq!(by["random.jar"]["reason"], "unknown");
        let keys: Vec<&String> = by["random.jar"].as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["filename", "enabled", "title", "action", "reason"]);

        // apply: the cdn is gone (dropped server), so the update fails and the old jar is disabled
        let res = apply_mods_plan(&e, "vc", &plan).await.unwrap();
        assert_eq!(res["errors"].as_array().unwrap().len(), 1);
        assert_eq!(res["errors"][0]["filename"], "upd-1.jar");
        let disabled: Vec<&str> = res["disabled"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(disabled.contains(&"gone.jar") && disabled.contains(&"random.jar") && disabled.contains(&"upd-1.jar"));
        assert!(mods.join("gone.jar.disabled").exists() && mods.join("random.jar.disabled").exists());
        assert!(mods.join("keep.jar").exists());
    }

    fn make_fabric_jar(meta: Value) -> Vec<u8> {
        use std::io::Write;
        let buf = std::io::Cursor::new(Vec::new());
        let mut zw = zip::ZipWriter::new(buf);
        zw.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(meta.to_string().as_bytes()).unwrap();
        zw.finish().unwrap().into_inner()
    }

    #[tokio::test]
    async fn delete_mod_removes_enabled_disabled_and_protects_branding() {
        let (_d, e) = engine_with(None);
        let mods = write_instance(&e, "del", "26.3", "fabric");
        std::fs::write(mods.join("foo.jar"), b"abc").unwrap();
        std::fs::write(mods.join("bar.jar.disabled"), b"def").unwrap();
        std::fs::write(mods.join("espectral-menu-26.3-1.9.4.jar"), b"brand").unwrap();

        // Running instance refuses delete (409 ALREADY_RUNNING)
        e.active_instances().insert("del");
        let err = e.delete_mod("del", "foo.jar").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (409, "ALREADY_RUNNING"));
        e.active_instances().remove("del");

        // Branding jar is protected (400 branding_protected)
        let err = e.delete_mod("del", "espectral-menu-26.3-1.9.4.jar").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (400, "branding_protected"));

        // Enabled and disabled jars delete cleanly
        let r1 = e.delete_mod("del", "foo.jar").await.unwrap();
        assert_eq!(r1.deleted, "foo.jar");
        assert!(!mods.join("foo.jar").exists());

        let r2 = e.delete_mod("del", "bar.jar").await.unwrap();
        assert_eq!(r2.deleted, "bar.jar");
        assert!(!mods.join("bar.jar.disabled").exists());

        // Missing -> 404 mod_not_found
        let err = e.delete_mod("del", "missing.jar").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (404, "mod_not_found"));
    }

    #[tokio::test]
    async fn version_switch_and_modrinth_preview_bring_iris_along_with_sodium() {
        let sha = crate::download::sha1_hex;
        let sodium_092 = make_fabric_jar(json!({
            "id": "sodium", "name": "Sodium", "version": "0.9.2+mc26.3",
            "depends": { "minecraft": "26.3.x" }
        }));
        let sodium_093 = make_fabric_jar(json!({
            "id": "sodium", "name": "Sodium", "version": "0.9.3+mc26.3",
            "depends": { "minecraft": "26.3.x" }
        }));
        let iris_117 = make_fabric_jar(json!({
            "id": "iris", "name": "Iris", "version": "1.11.7+mc26.3",
            "depends": { "sodium": ["0.9.x"] }
        }));
        let iris_118 = make_fabric_jar(json!({
            "id": "iris", "name": "Iris", "version": "1.11.8+mc26.3",
            "depends": { "sodium": [">=0.9.3"] }
        }));
        let addon_jar = make_fabric_jar(json!({
            "id": "cool-addon", "name": "Cool Addon", "version": "1.0.0",
            "depends": { "minecraft": "26.3.x", "sodium": ">=0.9.3" }
        }));

        let s92_sha = sha(&sodium_092);
        let s93_sha = sha(&sodium_093);
        let i17_sha = sha(&iris_117);
        let i18_sha = sha(&iris_118);
        let add_sha = sha(&addon_jar);

        let cdn = MockServer::start({
            let s93 = sodium_093.clone();
            let i18 = iris_118.clone();
            let add = addon_jar.clone();
            move |req| match req.path.as_str() {
                "/sodium-093.jar" => MockResponse::bytes(s93.clone()),
                "/iris-118.jar" => MockResponse::bytes(i18.clone()),
                "/cool-addon.jar" => MockResponse::bytes(add.clone()),
                _ => MockResponse::status(404),
            }
        })
        .await;
        let cdn_url = cdn.url("");

        let (s92_len, s93_len, i17_len, i18_len, add_len) =
            (sodium_092.len(), sodium_093.len(), iris_117.len(), iris_118.len(), addon_jar.len());

        let api = MockServer::start(move |req| {
            let p = req.path.as_str();
            let s92_v = json!({
                "id": "vS92", "project_id": SODIUM_PROJECT_ID, "version_number": "mc26.3-0.9.2-fabric",
                "name": "Sodium 0.9.2", "version_type": "release", "status": "listed",
                "date_published": "2026-11-01T00:00:00Z", "dependencies": [],
                "files": [{ "primary": true, "filename": "sodium-fabric-0.9.2+mc26.3.jar",
                            "url": format!("{cdn_url}/sodium-092.jar"), "size": s92_len,
                            "hashes": { "sha1": s92_sha } }]
            });
            let s93_v = json!({
                "id": "vS93", "project_id": SODIUM_PROJECT_ID, "version_number": "mc26.3-0.9.3-fabric",
                "name": "Sodium 0.9.3", "version_type": "beta", "status": "listed",
                "date_published": "2026-11-15T00:00:00Z", "dependencies": [],
                "files": [{ "primary": true, "filename": "sodium-fabric-0.9.3+mc26.3.jar",
                            "url": format!("{cdn_url}/sodium-093.jar"), "size": s93_len,
                            "hashes": { "sha1": s93_sha } }]
            });
            let i17_v = json!({
                "id": "vI17", "project_id": IRIS_PROJECT_ID, "version_number": "1.11.7+26.3-fabric",
                "name": "Iris 1.11.7", "version_type": "release", "status": "listed",
                "date_published": "2026-11-01T00:00:00Z",
                "dependencies": [{ "dependency_type": "required", "project_id": SODIUM_PROJECT_ID, "version_id": "vS92" }],
                "files": [{ "primary": true, "filename": "iris-fabric-1.11.7+mc26.3.jar",
                            "url": format!("{cdn_url}/iris-117.jar"), "size": i17_len,
                            "hashes": { "sha1": i17_sha } }]
            });
            let i18_v = json!({
                "id": "vI18", "project_id": IRIS_PROJECT_ID, "version_number": "1.11.8+26.3-fabric",
                "name": "Iris 1.11.8", "version_type": "beta", "status": "listed",
                "date_published": "2026-11-15T00:00:00Z",
                "dependencies": [{ "dependency_type": "required", "project_id": SODIUM_PROJECT_ID, "version_id": "vS93" }],
                "files": [{ "primary": true, "filename": "iris-fabric-1.11.8+mc26.3.jar",
                            "url": format!("{cdn_url}/iris-118.jar"), "size": i18_len,
                            "hashes": { "sha1": i18_sha } }]
            });
            let add_v = json!({
                "id": "vADD", "project_id": "PADDON", "version_number": "1.0.0",
                "name": "Cool Addon 1.0", "version_type": "release", "status": "listed",
                "date_published": "2026-11-16T00:00:00Z",
                "dependencies": [{ "dependency_type": "required", "project_id": SODIUM_PROJECT_ID }],
                "files": [{ "primary": true, "filename": "cool-addon-1.0.0.jar",
                            "url": format!("{cdn_url}/cool-addon.jar"), "size": add_len,
                            "hashes": { "sha1": add_sha } }]
            });

            if p == "/v2/version_files" {
                let mut m = Map::new();
                m.insert(s92_sha.clone(), s92_v);
                m.insert(i17_sha.clone(), i17_v);
                return MockResponse::json(&Value::Object(m));
            }
            if p == "/v2/version/vS93" {
                return MockResponse::json(&s93_v);
            }
            if p == "/v2/version/vI18" {
                return MockResponse::json(&i18_v);
            }
            if p == "/v2/version/vADD" {
                return MockResponse::json(&add_v);
            }
            if p.starts_with(&format!("/v2/project/{SODIUM_PROJECT_ID}/version")) {
                return MockResponse::json(&json!([s93_v, s92_v]));
            }
            if p.starts_with(&format!("/v2/project/{IRIS_PROJECT_ID}/version")) {
                return MockResponse::json(&json!([i18_v, i17_v]));
            }
            if p.starts_with("/v2/project/PADDON/version") {
                return MockResponse::json(&json!([add_v]));
            }
            if p.starts_with("/v2/projects?ids=") {
                return MockResponse::json(&json!([
                    { "id": SODIUM_PROJECT_ID, "title": "Sodium" },
                    { "id": IRIS_PROJECT_ID, "title": "Iris Shaders" },
                    { "id": "PADDON", "title": "Cool Addon" }
                ]));
            }
            MockResponse::status(404)
        })
        .await;

        let (_d, e) = engine_with(Some(&api));
        let mods = write_instance(&e, "guaka", "26.3", "fabric");
        std::fs::write(mods.join("sodium-fabric-0.9.2+mc26.3.jar"), &sodium_092).unwrap();
        std::fs::write(mods.join("iris-fabric-1.11.7+mc26.3.jar"), &iris_117).unwrap();

        // 1. Version listing shows both 0.9.3 (beta) and 0.9.2 (release, installed)
        let vers = e.list_mod_versions("guaka", "sodium-fabric-0.9.2+mc26.3.jar").await.unwrap();
        assert_eq!(vers.versions.len(), 2);
        assert!(vers.versions.iter().any(|v| v.version_id == "vS92" && v.installed));
        assert!(vers.versions.iter().any(|v| v.version_id == "vS93" && v.version_type == "beta" && !v.installed));

        // 2. Switching Sodium -> 0.9.3 automatically plans Iris -> 1.11.8 as companion!
        let plan = e
            .plan_mod_version_switch("guaka", "sodium-fabric-0.9.2+mc26.3.jar", "vS93")
            .await
            .unwrap();
        assert_eq!(plan.companions.len(), 1);
        assert_eq!(plan.companions[0].project_id, IRIS_PROJECT_ID);
        assert_eq!(plan.companions[0].to_version_id, "vI18");

        // 3. Previewing Cool Addon (requires sodium >=0.9.3 in fabric.mod.json)
        //    automatically detects Sodium needs upgrade AND brings Iris along!
        let preview = e.preview_modrinth_install("guaka", "PADDON").await.unwrap();
        assert!(preview.needs_confirmation);
        assert_eq!(preview.upgrades.len(), 2, "upgrades: {:?}", preview.upgrades);
        assert!(preview.upgrades.iter().any(|u| u.project_id == SODIUM_PROJECT_ID && u.to_version_id == "vS93"));
        assert!(preview.upgrades.iter().any(|u| u.project_id == IRIS_PROJECT_ID && u.to_version_id == "vI18"));
        assert!(preview.compat_issues.is_empty(), "all issues resolved by upgrades: {:?}", preview.compat_issues);

        // 4. Applying Sodium switch (+ Iris companion) swaps both jars atomically!
        let applied = e
            .apply_mod_version_switch("guaka", "sodium-fabric-0.9.2+mc26.3.jar", "vS93", true)
            .await
            .unwrap();
        assert_eq!(applied.changed.len(), 2);
        assert!(mods.join("sodium-fabric-0.9.3+mc26.3.jar").exists());
        assert!(!mods.join("sodium-fabric-0.9.2+mc26.3.jar").exists());
        assert!(mods.join("iris-fabric-1.11.8+mc26.3.jar").exists());
        assert!(!mods.join("iris-fabric-1.11.7+mc26.3.jar").exists());

        // 5. After upgrading, compatibility report is 100% clean!
        let compat = e.check_mods_compat("guaka").await.unwrap();
        assert_eq!(compat.error_count, 0, "issues: {:?}", compat.issues);
    }
}
