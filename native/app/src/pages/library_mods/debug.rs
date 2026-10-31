//! Screenshot-only states for the library pages. The headless screenshot tool
//! cannot click, so `ESPECTRAL_SHOT_LIB` (comma-separated) seeds UI state when
//! a page is entered. Never set in normal runs; ignored when absent.
//!
//! | token          | effect |
//! |----------------|--------|
//! | `mods-tab:N`   | ModCatalog tab N (0 installed, 1 presets, 2 Modrinth) |
//! | `mods-fake`    | sample installed mods + Modrinth results (no network) |
//! | `versions-fake`| sample manifest |
//! | `mrpack`       | open the MrpackModal (ImportCenter hosts it) |
//! | `import-done`  | lane A in its success state |

use espectral_engine::types::{ModEntry, ModrinthProject, VersionEntry, VersionManifest};
use slint::ComponentHandle;

use crate::{AppWindow, LibraryBridge};

pub fn tokens() -> Vec<String> {
    std::env::var("ESPECTRAL_SHOT_LIB")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

pub fn has(token: &str) -> bool {
    tokens().iter().any(|t| t == token)
}

pub fn mods_tab() -> Option<i32> {
    tokens().iter().find_map(|t| t.strip_prefix("mods-tab:").and_then(|n| n.parse().ok()))
}

pub fn open_mrpack(ui: &AppWindow) {
    if has("mrpack") {
        ui.global::<LibraryBridge>().set_mrpack_open(true);
    }
}

pub fn fake_manifest() -> VersionManifest {
    let v = |id: &str, kind: &str, t: &str| VersionEntry {
        id: id.into(),
        kind: kind.into(),
        release_time: t.into(),
        sha1: String::new(),
        url: String::new(),
    };
    VersionManifest {
        latest_release: "26.3".into(),
        latest_snapshot: "26w14a".into(),
        versions: vec![
            v("26w14a", "snapshot", "2026-04-02T10:00:00+00:00"),
            v("26.3", "release", "2026-03-24T09:00:00+00:00"),
            v("1.21.4", "release", "2024-12-03T10:12:57+00:00"),
            v("1.20.4", "release", "2023-12-07T12:56:20+00:00"),
            v("1.17.1", "release", "2021-07-06T12:01:34+00:00"),
            v("1.8.9", "release", "2015-12-03T09:24:39+00:00"),
            v("b1.7.3", "old_beta", "2011-07-07T22:00:00+00:00"),
            v("a1.2.6", "old_alpha", "2010-12-02T22:00:00+00:00"),
        ],
    }
}

pub fn fake_mods() -> Vec<ModEntry> {
    let m = |f: &str, slug: Option<&str>, ver: Option<&str>, enabled: bool| ModEntry {
        filename: f.into(),
        project_slug: slug.map(Into::into),
        version_number: ver.map(Into::into),
        version_id: None,
        sha1: Some("3f2a9c1d7be04411".into()),
        size: Some(1_245_184),
        enabled,
        installed: true,
    };
    vec![
        m("sodium-fabric-0.6.13+mc26.3.jar", Some("sodium"), Some("0.6.13"), true),
        m("iris-fabric-1.8.12+mc26.3.jar", Some("iris"), Some("1.8.12"), true),
        m("lithium-fabric-0.15.0.jar", Some("lithium"), Some("0.15.0"), false),
        m("my-local-mod.jar", None, None, true),
    ]
}

pub fn fake_projects() -> Vec<ModrinthProject> {
    let p = |id: &str, title: &str, desc: &str, dl: u64, installed: bool| ModrinthProject {
        project_id: id.into(),
        slug: id.into(),
        title: title.into(),
        description: desc.into(),
        icon_url: None,
        downloads: dl,
        game_versions: vec![],
        latest_version_number: "1.0".into(),
        installed: installed.then_some(true),
    };
    vec![
        p("AANobbMI", "Sodium", "The fastest and most compatible rendering optimization mod for Minecraft. Now available for both NeoForge and Fabric!", 98_441_210, true),
        p("YL57xq9U", "Iris Shaders", "A modern shader pack loader for Minecraft intended to be compatible with existing OptiFine shader packs", 61_200_455, false),
        p("P7dR8mSH", "Fabric API", "Lightweight and modular API providing common hooks and intercompatibility measures utilized by mods using the Fabric toolchain.", 120_903_002, false),
        p("gvQqBUqZ", "Lithium", "No-compromises game logic optimization mod. Well suited for clients and servers of all kinds.", 44_018_377, false),
        p("mOgUt4GM", "Mod Menu", "Adds a mod menu to view the list of mods you have installed.", 70_118_934, false),
        p("fQEb0iXm", "Krypton", "A mod to optimize the Minecraft networking stack", 9_871, false),
    ]
}
