//! Data-directory resolution + every on-disk location the JS engine uses.
//!
//! Resolution order (identical to what the Tauri shell handed the Node engine
//! through `ESPECTRAL_DATA_DIR`, so existing installs keep their data):
//!
//! 1. `ESPECTRAL_DATA_DIR` (non-empty) — resolved to an absolute path;
//! 2. Windows: `%LOCALAPPDATA%\espectral-client`;
//! 3. Linux/other: `$XDG_DATA_HOME/espectral-client`, else
//!    `$HOME/.local/share/espectral-client`;
//! 4. last resort (no LOCALAPPDATA / HOME): `<exe dir>/data` — the JS engine's
//!    own `<repo>/data` fallback.
//!
//! [`Paths`] is the single place path layout is spelled out. Porters MUST use
//! these helpers instead of re-joining strings so the layout stays in sync
//! with the JS engine (and so tests can point an engine at a tempdir).

use std::path::{Path, PathBuf};

/// Folder name under the platform data base.
pub const APP_DIR_NAME: &str = "espectral-client";

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn absolutize(p: PathBuf) -> PathBuf {
    if p.is_absolute() {
        return p;
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(p),
        Err(_) => p,
    }
}

/// Resolve the data dir from the environment (see module docs). Does not
/// create it — [`crate::Engine::new`] does.
pub fn resolve_data_dir() -> PathBuf {
    if let Some(d) = non_empty_env("ESPECTRAL_DATA_DIR") {
        return absolutize(PathBuf::from(d));
    }
    if let Some(base) = platform_data_base() {
        return base.join(APP_DIR_NAME);
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    absolutize(exe_dir.join("data"))
}

#[cfg(windows)]
fn platform_data_base() -> Option<PathBuf> {
    non_empty_env("LOCALAPPDATA").map(PathBuf::from)
}

#[cfg(not(windows))]
fn platform_data_base() -> Option<PathBuf> {
    non_empty_env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| non_empty_env("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
}

/// User home (`USERPROFILE` on Windows, `HOME` elsewhere), JS `USERPROFILE ?? HOME`.
pub fn home_dir() -> Option<PathBuf> {
    non_empty_env("USERPROFILE").or_else(|| non_empty_env("HOME")).map(PathBuf::from).or_else(dirs::home_dir)
}

/// `%APPDATA%` on Windows (Roaming), used for `.minecraft` / `.fastclient` /
/// Lunar detection. On Linux the JS falls back to `$HOME`.
pub fn appdata_dir() -> Option<PathBuf> {
    non_empty_env("APPDATA").map(PathBuf::from)
}

/// JS resolver.fastClientRoot(): `%APPDATA%/.fastclient`, else `<home>/.fastclient`.
pub fn fast_client_root() -> Option<PathBuf> {
    if let Some(a) = appdata_dir() {
        return Some(a.join(".fastclient"));
    }
    home_dir().map(|h| h.join(".fastclient"))
}

/// Every engine-owned location under the data dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Paths { root: root.into() }
    }

    /// The data dir itself (`AppConfig.data_dir`).
    pub fn data_dir(&self) -> &Path {
        &self.root
    }

    // ---- top-level documents -------------------------------------------
    /// `<data>/config.json` (config.mjs)
    pub fn config_json(&self) -> PathBuf {
        self.root.join("config.json")
    }
    /// `<data>/launch-stats.jsonl` (stats.mjs)
    pub fn launch_stats(&self) -> PathBuf {
        self.root.join("launch-stats.jsonl")
    }
    /// `<data>/running-games.json` (game-sessions.mjs)
    pub fn running_games(&self) -> PathBuf {
        self.root.join("running-games.json")
    }
    /// `<data>/mc-token-cache.json` (msauth.mjs)
    pub fn mc_token_cache(&self) -> PathBuf {
        self.root.join("mc-token-cache.json")
    }
    /// `<data>/identity.json` (identity.mjs)
    pub fn identity_json(&self) -> PathBuf {
        self.root.join("identity.json")
    }
    /// `<data>/cosmetics.json` (cosmetics.mjs)
    pub fn cosmetics_json(&self) -> PathBuf {
        self.root.join("cosmetics.json")
    }
    /// `<data>/discord-users.json` (discordAuth.mjs)
    pub fn discord_users_json(&self) -> PathBuf {
        self.root.join("discord-users.json")
    }
    /// `<data>/launcher_profiles.json` (neoforge.mjs: the official installer needs one)
    pub fn launcher_profiles_json(&self) -> PathBuf {
        self.root.join("launcher_profiles.json")
    }

    // ---- game files (resolver.mjs) --------------------------------------
    /// `<data>/versions`
    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }
    /// `<data>/versions/manifest.json` (6h cached version manifest)
    pub fn version_manifest_cache(&self) -> PathBuf {
        self.versions_dir().join("manifest.json")
    }
    /// `<data>/versions/<id>/<id>.json`
    pub fn version_json(&self, id: &str) -> PathBuf {
        self.versions_dir().join(id).join(format!("{id}.json"))
    }
    /// `<data>/versions/<id>/<id>.jar` (resolver.clientJarPath)
    pub fn client_jar(&self, version: &str) -> PathBuf {
        self.versions_dir().join(version).join(format!("{version}.jar"))
    }
    /// `<data>/libraries`
    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }
    /// `<data>/assets`
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }
    /// `<data>/assets/indexes/<id>.json`
    pub fn asset_index(&self, id: &str) -> PathBuf {
        self.assets_dir().join("indexes").join(format!("{id}.json"))
    }
    /// `<data>/runtimes`
    pub fn runtimes_dir(&self) -> PathBuf {
        self.root.join("runtimes")
    }
    /// `<data>/runtimes/jdk-<major>`
    pub fn runtime_dir(&self, major: u32) -> PathBuf {
        self.runtimes_dir().join(format!("jdk-{major}"))
    }
    /// `<data>/runtimes/jdk-<major>/info.json`
    pub fn runtime_info(&self, major: u32) -> PathBuf {
        self.runtime_dir(major).join("info.json")
    }
    /// `<data>/runtimes/jdk-<major>/bin/java(.exe)` (runtimes.runtimeJavaPath)
    pub fn runtime_java(&self, major: u32) -> PathBuf {
        self.runtime_dir(major).join("bin").join(java_bin_name())
    }

    // ---- caches ----------------------------------------------------------
    /// `<data>/cache`
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
    /// `<data>/cache/aot` (aot.aotRootDir)
    pub fn aot_root(&self) -> PathBuf {
        self.cache_dir().join("aot")
    }
    /// `<data>/cache/aot/<key>`
    pub fn aot_cache_dir(&self, key: &str) -> PathBuf {
        self.aot_root().join(key)
    }
    /// `<data>/cache/aot/<key>/game.aot`
    pub fn aot_cache_file(&self, key: &str) -> PathBuf {
        self.aot_cache_dir(key).join("game.aot")
    }
    /// `<data>/cache/aot/<key>/meta.json`
    pub fn aot_meta(&self, key: &str) -> PathBuf {
        self.aot_cache_dir(key).join("meta.json")
    }
    /// `<data>/cache/neoforge/neoforge-<v>-installer.jar`
    pub fn neoforge_installer(&self, version: &str) -> PathBuf {
        self.cache_dir().join("neoforge").join(format!("neoforge-{version}-installer.jar"))
    }

    // ---- per-user stores -------------------------------------------------
    /// `<data>/avatars` (account avatars `<uuid>.png`)
    pub fn avatars_dir(&self) -> PathBuf {
        self.root.join("avatars")
    }
    /// `<data>/skins` (skin texture cache `<uuid>.png`)
    pub fn skins_dir(&self) -> PathBuf {
        self.root.join("skins")
    }
    /// `<data>/skin-library` (gallery pngs)
    pub fn skin_library_dir(&self) -> PathBuf {
        self.root.join("skin-library")
    }
    /// `<data>/skin-library.json` (gallery index)
    pub fn skin_library_index(&self) -> PathBuf {
        self.root.join("skin-library.json")
    }
    /// `<data>/backgrounds` (menu background packs `<id>/pack.json`)
    pub fn backgrounds_dir(&self) -> PathBuf {
        self.root.join("backgrounds")
    }

    // ---- instances (instances.mjs / resolver.mjs) ------------------------
    /// `<data>/instances`
    pub fn instances_dir(&self) -> PathBuf {
        self.root.join("instances")
    }
    /// `<data>/instances/<name>` — instance.json / icon.png always live here,
    /// even when the instance has a custom `game_dir`.
    pub fn instance_dir(&self, name: &str) -> PathBuf {
        self.instances_dir().join(name)
    }
    /// `<data>/instances/<name>/instance.json`
    pub fn instance_json(&self, name: &str) -> PathBuf {
        self.instance_dir(name).join("instance.json")
    }
    /// `<data>/instances/<name>/icon.png`
    pub fn instance_icon(&self, name: &str) -> PathBuf {
        self.instance_dir(name).join("icon.png")
    }
    /// `<data>/instances/<name>/natives` (resolver.instanceNativesDir)
    pub fn instance_natives_dir(&self, name: &str) -> PathBuf {
        self.instance_dir(name).join("natives")
    }
    /// `<data>/instances/<name>/mods` (resolver.instanceModsDir — NOT the
    /// effective mods dir; see instances::effective_mods_dir for custom game_dir)
    pub fn instance_mods_dir(&self, name: &str) -> PathBuf {
        self.instance_dir(name).join("mods")
    }
    /// `<data>/instances/<name>/resolved.json` (fingerprint.mjs)
    pub fn instance_fingerprint(&self, name: &str) -> PathBuf {
        self.instance_dir(name).join("resolved.json")
    }
    /// `<data>/instances/<name>/fabric-profile.json` (resolver.seedInstance)
    pub fn instance_fabric_profile(&self, name: &str) -> PathBuf {
        self.instance_dir(name).join("fabric-profile.json")
    }
}

/// Per-game-dir file the in-game Espectral mod reads:
/// `<gameDir>/config/espectral-client.json` (client.mjs).
pub fn client_config_in(game_dir: &Path) -> PathBuf {
    game_dir.join("config").join("espectral-client.json")
}

/// `<gameDir>/config/espectral-cosmetics.json` (cosmetics.mjs).
pub fn cosmetics_config_in(game_dir: &Path) -> PathBuf {
    game_dir.join("config").join("espectral-cosmetics.json")
}

/// `<gameDir>/config/espectral-menu-bg` (backgrounds.mjs staging dir).
pub fn menu_bg_staging_in(game_dir: &Path) -> PathBuf {
    game_dir.join("config").join("espectral-menu-bg")
}

/// `java.exe` on Windows, `java` elsewhere.
pub fn java_bin_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// Atomic JSON write used by every JS module (`.tmp` + rename, 2-space indent).
pub fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let json = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_js_engine() {
        let p = Paths::new("/d");
        assert_eq!(p.config_json(), PathBuf::from("/d/config.json"));
        assert_eq!(p.client_jar("1.21.11"), PathBuf::from("/d/versions/1.21.11/1.21.11.jar"));
        assert_eq!(p.aot_cache_file("k"), PathBuf::from("/d/cache/aot/k/game.aot"));
        assert_eq!(p.instance_json("A b"), PathBuf::from("/d/instances/A b/instance.json"));
        assert_eq!(p.instance_fingerprint("x"), PathBuf::from("/d/instances/x/resolved.json"));
        assert_eq!(p.runtime_info(21), PathBuf::from("/d/runtimes/jdk-21/info.json"));
        assert_eq!(client_config_in(Path::new("/g")), PathBuf::from("/g/config/espectral-client.json"));
    }

    #[test]
    fn write_json_atomic_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sub").join("a.json");
        write_json_atomic(&f, &serde_json::json!({ "b": 1, "a": 2 })).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{\n  \"b\": 1,\n  \"a\": 2\n}");
        assert!(!dir.path().join("sub").join("a.json.tmp").exists());
    }
}
