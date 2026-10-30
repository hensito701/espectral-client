//! Core routes — implemented in the foundation.
//!
//! | JS route                  | method |
//! |---------------------------|--------|
//! | GET  /api/health          | [`Engine::health`] |
//! | POST /api/shutdown        | [`Engine::request_shutdown`] |
//! | GET  /api/events (SSE)    | [`Engine::subscribe`] (engine.rs) |
//! | POST /api/open-folder     | [`Engine::open_folder`] |
//! | GET/PUT /api/theme        | [`Engine::get_theme`] / [`Engine::set_theme`] |
//! | GET/PUT /api/accent       | [`Engine::get_accent`] / [`Engine::set_accent`] |
//! | GET/PATCH /api/config     | [`Engine::get_config`] / [`Engine::patch_config`] |
//!
//! NOT ported: POST /api/pick-file and /api/pick-folder (PowerShell pickers)
//! — the native UI uses `rfd` dialogs. 501 UNSUPPORTED_PLATFORM off-Windows
//! in the JS; nothing to call natively.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::{ACCENT_IDS, THEMES};
use crate::engine::{Engine, VERSION};
use crate::error::{EngineError, EngineResult};
use crate::types::{de, AppConfig, HealthInfo, InstanceMeta, OkReply};
use crate::util;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeReply {
    pub theme: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccentReply {
    pub accent: String,
}

/// PATCH /api/config body (whitelisted AppConfig fields). `Some(None)` = explicit null.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ConfigPatch {
    /// null = back to automatic; else 512..=65536.
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub default_memory_mb: Option<Option<i64>>,
    /// 1..=16
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_concurrency: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_boot: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aot_auto_train: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduced_motion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_on_game_start: Option<bool>,
    #[serde(default, deserialize_with = "de::double_option", skip_serializing_if = "Option::is_none")]
    pub jdk_path_override: Option<Option<String>>,
}

impl ConfigPatch {
    /// Accepts the UI envelope `{ patch: {...} }` or a bare patch (JS PATCH /api/config).
    /// Unknown keys are ignored; JS truthiness applies to the boolean flags.
    pub fn from_json(body: &Value) -> EngineResult<ConfigPatch> {
        let raw = match body.get("patch") {
            Some(p @ Value::Object(_)) => p,
            _ => body,
        };
        let get = |k: &str| raw.get(k);
        let truthy = |k: &str| get(k).map(|v| crate::config::json_truthy(Some(v)));
        let mut p = ConfigPatch {
            fast_boot: truthy("fast_boot"),
            aot_auto_train: truthy("aot_auto_train"),
            discord_enabled: truthy("discord_enabled"),
            reduced_motion: truthy("reduced_motion"),
            close_on_game_start: truthy("close_on_game_start"),
            ..Default::default()
        };
        match get("default_memory_mb") {
            None => {}
            Some(Value::Null) => p.default_memory_mb = Some(None),
            Some(v) => {
                let n = number_like(v).ok_or_else(bad_memory)?;
                p.default_memory_mb = Some(Some(n));
            }
        }
        if let Some(v) = get("download_concurrency") {
            p.download_concurrency = Some(number_like(v).ok_or_else(bad_concurrency)?);
        }
        match get("jdk_path_override") {
            None => {}
            Some(Value::Null) => p.jdk_path_override = Some(None),
            Some(Value::String(s)) => p.jdk_path_override = Some(Some(s.clone())),
            Some(_) => return Err(bad_jdk()),
        }
        Ok(p)
    }
}

/// JS `Number(x)` + `Number.isInteger` for numbers and numeric strings.
fn number_like(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.fract() == 0.0).map(|f| f as i64),
        other => crate::config::json_integer(Some(other)),
    }
}

fn bad_memory() -> EngineError {
    EngineError::bad_request("BAD_MEMORY", "default_memory_mb must be an integer between 512 and 65536")
}
fn bad_concurrency() -> EngineError {
    EngineError::bad_request("BAD_CONCURRENCY", "download_concurrency must be an integer between 1 and 16")
}
fn bad_jdk() -> EngineError {
    EngineError::bad_request("BAD_JDK", "jdk_path_override must be a path string or null")
}

/// Whitelist + validate (JS sanitizeConfigPatch) -> config.json patch.
fn sanitize_config_patch(p: &ConfigPatch) -> EngineResult<Map<String, Value>> {
    let mut out = Map::new();
    match p.default_memory_mb {
        None => {}
        Some(None) => {
            out.insert("default_memory_mb".into(), Value::Null);
        }
        Some(Some(mb)) => {
            if !(512..=65536).contains(&mb) {
                return Err(bad_memory());
            }
            out.insert("default_memory_mb".into(), Value::from(mb));
        }
    }
    if let Some(c) = p.download_concurrency {
        if !(1..=16).contains(&c) {
            return Err(bad_concurrency());
        }
        out.insert("download_concurrency".into(), Value::from(c));
    }
    for (k, v) in [
        ("fast_boot", p.fast_boot),
        ("aot_auto_train", p.aot_auto_train),
        ("discord_enabled", p.discord_enabled),
        ("reduced_motion", p.reduced_motion),
        ("close_on_game_start", p.close_on_game_start),
    ] {
        if let Some(b) = v {
            out.insert(k.into(), Value::Bool(b));
        }
    }
    if let Some(j) = &p.jdk_path_override {
        out.insert("jdk_path_override".into(), j.clone().map(Value::from).unwrap_or(Value::Null));
    }
    Ok(out)
}

impl Engine {
    /// GET /api/health -> `{ ok: true, version }`.
    pub async fn health(&self) -> EngineResult<HealthInfo> {
        Ok(HealthInfo { ok: true, version: VERSION.to_string() })
    }

    /// POST /api/shutdown -> `{ ok: true }`. Runs the shutdown hooks; the app
    /// then exits (the in-app updater calls this before the installer runs).
    /// Running games are NOT touched.
    pub async fn request_shutdown(&self) -> EngineResult<OkReply> {
        self.shutdown().await;
        Ok(OkReply::TRUE)
    }

    /// Every instance's custom `game_dir` (allowed open-folder roots).
    fn custom_game_dirs(&self) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(self.paths().instances_dir()) else { return vec![] };
        rd.filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !util::is_trash_dir_name(n))
            .filter_map(|n| {
                let s = std::fs::read_to_string(self.paths().instance_json(&n)).ok()?;
                let m: InstanceMeta = serde_json::from_str(&s).ok()?;
                m.game_dir.filter(|d| !d.is_empty()).map(|d| util::normalize_path(&absolute(Path::new(&d))))
            })
            .collect()
    }

    /// POST /api/open-folder `{ path }` -> `{ ok: true }`. The path must
    /// resolve inside the data dir or an instance's custom game dir
    /// (400 BAD_PATH), and exist (404 BAD_PATH); 500 OPEN_FAILED when no opener.
    pub async fn open_folder(&self, path: &str) -> EngineResult<OkReply> {
        let raw = path.trim();
        if raw.is_empty() {
            return Err(EngineError::bad_request("BAD_PATH", "path is required"));
        }
        let root = util::normalize_path(&absolute(self.paths().data_dir()));
        let p = Path::new(raw);
        let target = util::normalize_path(&if p.is_absolute() { p.to_path_buf() } else { root.join(p) });
        let mut allowed = vec![root];
        allowed.extend(self.custom_game_dirs());
        if !allowed.iter().any(|r| util::is_within_dir(&target, r)) {
            return Err(EngineError::bad_request(
                "BAD_PATH",
                "path must be inside the engine data directory or an instance folder",
            ));
        }
        if !target.exists() {
            return Err(EngineError::not_found("BAD_PATH", format!("path does not exist: {raw}")));
        }
        crate::platform::open_path(&target)
            .map_err(|e| EngineError::api(500, "OPEN_FAILED", format!("could not open folder: {e}")))?;
        Ok(OkReply::TRUE)
    }

    /// GET /api/theme -> theme id (default 'dark').
    pub async fn get_theme(&self) -> EngineResult<String> {
        Ok(self.config().theme())
    }

    /// PUT /api/theme `{ theme }` -> `{ theme }`; 400 BAD_THEME.
    pub async fn set_theme(&self, theme: &str) -> EngineResult<ThemeReply> {
        if !THEMES.contains(&theme) {
            return Err(EngineError::bad_request("BAD_THEME", "theme must be one of dark|light|system"));
        }
        self.config().set("theme", Value::from(theme));
        Ok(ThemeReply { theme: theme.to_string() })
    }

    /// GET /api/accent -> accent id (default 'oro').
    pub async fn get_accent(&self) -> EngineResult<String> {
        Ok(self.config().accent())
    }

    /// PUT /api/accent `{ accent }` -> `{ accent }`; 400 BAD_ACCENT.
    pub async fn set_accent(&self, accent: &str) -> EngineResult<AccentReply> {
        if !ACCENT_IDS.contains(&accent) {
            return Err(EngineError::bad_request(
                "BAD_ACCENT",
                format!("accent must be one of {}", ACCENT_IDS.join("|")),
            ));
        }
        self.config().set("accent", Value::from(accent));
        Ok(AccentReply { accent: accent.to_string() })
    }

    /// GET /api/config -> AppConfig.
    pub async fn get_config(&self) -> EngineResult<AppConfig> {
        Ok(self.config().app_config())
    }

    /// PATCH /api/config -> AppConfig (400 BAD_MEMORY / BAD_CONCURRENCY / BAD_JDK).
    pub async fn patch_config(&self, patch: ConfigPatch) -> EngineResult<AppConfig> {
        let p = sanitize_config_patch(&patch)?;
        self.config().save(p);
        Ok(self.config().app_config())
    }
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use serde_json::json;

    #[tokio::test]
    async fn health_reports_package_version() {
        let (_d, e) = temp_engine();
        let h = e.health().await.unwrap();
        assert!(h.ok);
        assert_eq!(h.version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn theme_and_accent_validation() {
        let (_d, e) = temp_engine();
        assert_eq!(e.get_theme().await.unwrap(), "dark");
        assert_eq!(e.set_theme("light").await.unwrap().theme, "light");
        assert_eq!(e.get_theme().await.unwrap(), "light");
        assert_eq!(e.set_theme("neon").await.unwrap_err().envelope().code, "BAD_THEME");
        assert_eq!(e.get_accent().await.unwrap(), "oro");
        assert_eq!(e.set_accent("glaciar").await.unwrap().accent, "glaciar");
        let err = e.set_accent("pink").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_ACCENT"));
        assert!(err.message.contains("oro|carmesi|amatista|esmeralda|glaciar"));
    }

    #[tokio::test]
    async fn patch_config_accepts_envelope_and_validates() {
        let (_d, e) = temp_engine();
        let p =
            ConfigPatch::from_json(&json!({ "patch": { "download_concurrency": 4, "fast_boot": 1, "theme": "x" } }))
                .unwrap();
        let c = e.patch_config(p).await.unwrap();
        assert_eq!(c.download_concurrency, 4);
        assert!(c.fast_boot);
        assert_eq!(e.config().theme(), "dark", "engine state is off-limits");
        let p = ConfigPatch::from_json(&json!({ "default_memory_mb": 5120 })).unwrap();
        assert_eq!(e.patch_config(p).await.unwrap().default_memory_mb, 5120);
        let p = ConfigPatch::from_json(&json!({ "default_memory_mb": null })).unwrap();
        assert!(e.patch_config(p).await.unwrap().default_memory_auto);
        for (body, code) in [
            (json!({ "default_memory_mb": 100 }), "BAD_MEMORY"),
            (json!({ "default_memory_mb": "abc" }), "BAD_MEMORY"),
            (json!({ "download_concurrency": 17 }), "BAD_CONCURRENCY"),
            (json!({ "jdk_path_override": 5 }), "BAD_JDK"),
        ] {
            let r = match ConfigPatch::from_json(&body) {
                Ok(p) => e.patch_config(p).await.map(|_| ()),
                Err(err) => Err(err),
            };
            assert_eq!(r.unwrap_err().envelope().code, code, "{body}");
        }
    }

    #[tokio::test]
    async fn open_folder_guards_paths() {
        let (d, e) = temp_engine();
        assert_eq!(e.open_folder("  ").await.unwrap_err().envelope().code, "BAD_PATH");
        let outside = e.open_folder("/definitely/not/inside").await.unwrap_err();
        assert_eq!((outside.status(), outside.envelope().code), (400, "BAD_PATH".into()));
        let traversal = e.open_folder("instances/../../../etc").await.unwrap_err();
        assert_eq!(traversal.status(), 400);
        let missing = e.open_folder("instances/nope").await.unwrap_err();
        assert_eq!(missing.status(), 404);
        // a custom game_dir outside the data dir is an allowed root
        let custom = tempfile::tempdir().unwrap();
        crate::paths::write_json_atomic(
            &e.paths().instance_json("x"),
            &json!({ "name": "x", "version": "1", "game_dir": custom.path().to_string_lossy() }),
        )
        .unwrap();
        assert!(e.custom_game_dirs().iter().any(|p| p == &util::normalize_path(custom.path())));
        drop(d);
    }
}
