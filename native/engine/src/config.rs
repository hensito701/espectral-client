//! Port of `src/engine/config.mjs` — `<data>/config.json` persistence.
//!
//! A single JSON document holding the AppConfig subset plus engine-owned
//! state (theme, accent, accounts, active account, cached JVM info, MSA client
//! id, onboarding markers). The document is kept as an ordered
//! `serde_json::Map` — NOT a typed struct — so that:
//!
//! - unknown keys written by the JS engine (or a newer native build) survive a
//!   load/save round-trip untouched, and
//! - a hand-edited / odd value (e.g. `"fast_boot": 1`) never makes the whole
//!   file fail to parse (the JS spread semantics: `{...DEFAULT_CONFIG, ...raw}`).
//!
//! Typed accessors apply the exact JS coercions (`=== true`, `!== false`,
//! `Number.isInteger(x) && x >= 512`, …).
//!
//! Writes are atomic (`config.json.tmp` + rename) and NEVER contain the
//! in-memory MC access token or XBL/XSTS chain material (H2): the keys
//! `access_token`, `xbl_token`, `xsts_token`, `user_hash` are stripped at any
//! depth before serializing — exactly like the JS `JSON.stringify` replacer.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::paths::Paths;
use crate::platform;
use crate::types::{Account, AppConfig, JavaRuntime};

/// Canonical espectral-id service URL (the espectral.es zone lapsed 2026-10-22).
pub const DEFAULT_IDENTITY_URL: &str = "https://id.uhce.top";

/// Identity hosts that predate the uhce.top move; migrated on load.
pub const LEGACY_IDENTITY_URLS: &[&str] = &["https://id.espectral.es"];

/// Old fixed defaults (engine 3072, wizard 4096): instances carrying one were
/// never chosen deliberately (instances.mjs migrates them to automatic).
pub const LEGACY_DEFAULT_MEMORY_MB: [u32; 2] = [3072, 4096];

/// Keys that must never reach disk (H2).
pub const SECRET_KEYS: &[&str] = &["access_token", "xbl_token", "xsts_token", "user_hash"];

/// Launcher theme ids (routes/misc.mjs THEMES).
pub const THEMES: &[&str] = &["dark", "light", "system"];

/// Accent palette ids (client.mjs THEME_IDS) — same set as the mod palettes.
pub const ACCENT_IDS: &[&str] = &["oro", "carmesi", "amatista", "esmeralda", "glaciar"];

/// JS `DEFAULT_CONFIG`, in the JS key order.
pub fn default_config() -> Map<String, Value> {
    let v = json!({
        "default_memory_mb": null,
        "memory_auto_migrated": false,
        "download_concurrency": 6,
        "fast_boot": false,
        "aot_auto_train": true,
        "fullbright_on_launch": false,
        "jdk_path_override": null,
        "discord_enabled": true,
        "close_on_game_start": false,
        "identity_url": DEFAULT_IDENTITY_URL,
        "theme": "dark",
        "accent": "oro",
        "reduced_motion": false,
        "active_username": null,
        "accounts": [],
        "jvm": null,
        "first_launch_setup_complete": false,
        "first_launch_import_source": null,
        // Agent access (MCP). Inert unless enabled (upstream c22767f).
        "agent": { "enabled": false, "allowed_servers": [] },
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// Heap ceiling (-Xmx) picked when the player has not chosen one, by system
/// RAM: 2 GB up to "8 GB" machines, 3 GB up to 16 GB, 4 GB above.
pub fn recommended_memory_mb_for(total_bytes: f64) -> u32 {
    let total_gb = total_bytes / 1024f64.powi(3);
    if !total_gb.is_finite() || total_gb <= 0.0 {
        return 2048;
    }
    if total_gb <= 8.5 {
        return 2048;
    }
    if total_gb <= 16.5 {
        return 3072;
    }
    4096
}

/// [`recommended_memory_mb_for`] with this machine's RAM.
pub fn recommended_memory_mb() -> u32 {
    recommended_memory_mb_for(platform::total_memory_bytes() as f64)
}

/// True when `url` is exactly a bare legacy identity host (trailing slashes
/// and surrounding whitespace tolerated, host case-insensitive).
pub fn is_legacy_identity_url(url: &str) -> bool {
    let norm = url.trim().trim_end_matches('/').to_lowercase();
    LEGACY_IDENTITY_URLS.contains(&norm.as_str())
}

/// JS `Number.isInteger(v)` for a JSON value.
pub fn json_integer(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                Some(i)
            } else {
                n.as_f64().filter(|f| f.fract() == 0.0 && f.is_finite()).map(|f| f as i64)
            }
        }
        _ => None,
    }
}

/// JS truthiness of a JSON value (`!!v`).
pub fn json_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// Clone of `v` with every [`SECRET_KEYS`] entry removed at any depth.
pub fn strip_secrets(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !SECRET_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), strip_secrets(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip_secrets).collect()),
        other => other.clone(),
    }
}

/// The live config document of one engine (JS module-level `cached`).
#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    data_dir: PathBuf,
    doc: Mutex<Option<Map<String, Value>>>,
}

impl ConfigStore {
    pub fn new(paths: &Paths) -> Self {
        ConfigStore { path: paths.config_json(), data_dir: paths.data_dir().to_path_buf(), doc: Mutex::new(None) }
    }

    /// Absolute path of the config document.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Map<String, Value>>> {
        self.doc.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Lazily load (and create / migrate) the document. Must be called with the lock held.
    fn ensure_loaded<'a>(&self, slot: &'a mut Option<Map<String, Value>>) -> &'a mut Map<String, Value> {
        if slot.is_none() {
            let file_existed = self.path.exists();
            let raw: Map<String, Value> = std::fs::read_to_string(&self.path)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| match v {
                    Value::Object(m) => Some(m),
                    _ => None,
                })
                .unwrap_or_default(); // missing or corrupt -> defaults
            let mut doc = default_config();
            for (k, v) in raw {
                doc.insert(k, v);
            }
            if !matches!(doc.get("accounts"), Some(Value::Array(_))) {
                doc.insert("accounts".into(), Value::Array(vec![]));
            }
            if !matches!(doc.get("agent"), Some(Value::Object(_))) {
                doc.insert("agent".into(), json!({ "enabled": false, "allowed_servers": [] }));
            }
            // Only a *legacy* identity_url is rewritten; a custom one is the
            // user's own self-hosted service.
            let migrated_identity =
                matches!(doc.get("identity_url"), Some(Value::String(u)) if is_legacy_identity_url(u));
            if migrated_identity {
                doc.insert("identity_url".into(), Value::from(DEFAULT_IDENTITY_URL));
            }
            // The persisted 3072 is the old default every install saved, not a choice.
            let migrated_memory = doc.get("memory_auto_migrated") != Some(&Value::Bool(true));
            if migrated_memory {
                if doc.get("default_memory_mb").and_then(Value::as_f64) == Some(3072.0) {
                    doc.insert("default_memory_mb".into(), Value::Null);
                }
                doc.insert("memory_auto_migrated".into(), Value::Bool(true));
            }
            if migrated_identity || migrated_memory || !file_existed {
                self.persist(&doc);
            }
            *slot = Some(doc);
        }
        slot.as_mut().expect("loaded")
    }

    fn persist(&self, doc: &Map<String, Value>) {
        let stripped = strip_secrets(&Value::Object(doc.clone()));
        if let Err(e) = crate::paths::write_json_atomic(&self.path, &stripped) {
            tracing::warn!("[config] could not persist {}: {e}", self.path.display());
        }
    }

    /// Snapshot of the whole document (JS `loadConfig()` — a clone, not the live object).
    pub fn load(&self) -> Map<String, Value> {
        let mut g = self.lock();
        self.ensure_loaded(&mut g).clone()
    }

    /// Read access to the live document without cloning it.
    pub fn with<R>(&self, f: impl FnOnce(&Map<String, Value>) -> R) -> R {
        let mut g = self.lock();
        f(self.ensure_loaded(&mut g))
    }

    /// Atomic read-modify-write of the live document; persists afterwards.
    /// Use this (not load + save) for list edits such as `accounts`.
    pub fn update<R>(&self, f: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
        let mut g = self.lock();
        let doc = self.ensure_loaded(&mut g);
        let r = f(doc);
        self.persist(doc);
        r
    }

    /// JS `saveConfig(patch)`: merge `patch` into the live document and persist.
    /// Returns the merged document. (JS skipped `undefined` values; in Rust
    /// simply leave the key out of the patch.)
    pub fn save(&self, patch: Map<String, Value>) -> Map<String, Value> {
        self.update(|doc| {
            for (k, v) in patch {
                doc.insert(k, v);
            }
            doc.clone()
        })
    }

    /// Set one key and persist.
    pub fn set(&self, key: &str, value: Value) {
        self.update(|doc| {
            doc.insert(key.to_string(), value);
        });
    }

    /// Set a key in memory only (NOT persisted until the next save). For
    /// same-session state the JS kept on the live object.
    pub fn set_in_memory(&self, key: &str, value: Value) {
        let mut g = self.lock();
        self.ensure_loaded(&mut g).insert(key.to_string(), value);
    }

    /// Raw value of one key.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.with(|d| d.get(key).cloned())
    }

    /// One key deserialized; `None` when absent, null, or of the wrong shape.
    pub fn get_as<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.with(|d| d.get(key).filter(|v| !v.is_null()).and_then(|v| serde_json::from_value(v.clone()).ok()))
    }

    /// String value of one key (non-strings -> None).
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.with(|d| d.get(key).and_then(Value::as_str).map(str::to_string))
    }

    /// `=== true`
    pub fn is_true(&self, key: &str) -> bool {
        self.with(|d| d.get(key) == Some(&Value::Bool(true)))
    }

    /// `!== false`
    pub fn is_not_false(&self, key: &str) -> bool {
        self.with(|d| d.get(key) != Some(&Value::Bool(false)))
    }

    // ---- typed accessors (exact JS coercions) ----------------------------

    /// GET /api/theme: `loadConfig().theme ?? 'dark'`.
    pub fn theme(&self) -> String {
        self.get_str("theme").unwrap_or_else(|| "dark".into())
    }

    /// GET /api/accent: `loadConfig().accent ?? 'oro'`.
    pub fn accent(&self) -> String {
        self.get_str("accent").unwrap_or_else(|| "oro".into())
    }

    /// Global `discord_enabled !== false`.
    pub fn discord_enabled(&self) -> bool {
        self.is_not_false("discord_enabled")
    }

    /// Global `aot_auto_train` (raw truthiness; default true).
    pub fn aot_auto_train(&self) -> bool {
        self.with(|d| json_truthy(d.get("aot_auto_train")))
    }

    /// `jdk_path_override ?? null` (strings only).
    pub fn jdk_path_override(&self) -> Option<String> {
        self.get_str("jdk_path_override").filter(|s| !s.is_empty())
    }

    /// Persisted espectral-id URL (identity.mjs applies the env override + default).
    pub fn identity_url(&self) -> Option<String> {
        self.get_str("identity_url")
    }

    /// download.mjs downloadConcurrency(): positive integer, else 6.
    pub fn download_concurrency(&self) -> usize {
        self.with(|d| json_integer(d.get("download_concurrency"))).filter(|n| *n > 0).map(|n| n as usize).unwrap_or(6)
    }

    /// Heap ceiling for instances without their own memory_mb (JS defaultMemoryMb).
    pub fn default_memory_mb(&self) -> u32 {
        self.with(|d| json_integer(d.get("default_memory_mb")))
            .filter(|mb| *mb >= 512)
            .map(|mb| mb as u32)
            .unwrap_or_else(recommended_memory_mb)
    }

    /// Cached primary JVM (`config.jvm`).
    pub fn jvm(&self) -> Option<JavaRuntime> {
        self.get_as("jvm")
    }

    /// Store the primary JVM cache (`saveConfig({ jvm })`).
    pub fn set_jvm(&self, jvm: Option<&JavaRuntime>) {
        let v = jvm.map(|j| serde_json::to_value(j).unwrap_or(Value::Null)).unwrap_or(Value::Null);
        self.set("jvm", v);
    }

    /// `config.accounts` as typed entries (non-object entries are skipped).
    pub fn accounts(&self) -> Vec<Account> {
        self.with(|d| match d.get("accounts") {
            Some(Value::Array(a)) => a.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect(),
            _ => vec![],
        })
    }

    /// Replace `config.accounts` (and optionally `active_username`) in one write.
    pub fn set_accounts(&self, accounts: &[Account], active_username: Option<Option<&str>>) {
        self.update(|d| {
            d.insert("accounts".into(), serde_json::to_value(accounts).unwrap_or(Value::Array(vec![])));
            if let Some(active) = active_username {
                d.insert("active_username".into(), active.map(Value::from).unwrap_or(Value::Null));
            }
        });
    }

    /// `config.active_username`.
    pub fn active_username(&self) -> Option<String> {
        self.get_str("active_username")
    }

    /// The AppConfig shape exposed by GET /api/config (no engine-internal state).
    pub fn app_config(&self) -> AppConfig {
        let data_dir = self.data_dir.to_string_lossy().into_owned();
        self.with(|c| {
            let pinned = json_integer(c.get("default_memory_mb")).filter(|mb| *mb >= 512);
            AppConfig {
                default_memory_mb: pinned.map(|m| m as u32).unwrap_or_else(recommended_memory_mb),
                recommended_memory_mb: recommended_memory_mb(),
                default_memory_auto: pinned.is_none(),
                accent: c.get("accent").and_then(Value::as_str).unwrap_or("oro").to_string(),
                reduced_motion: c.get("reduced_motion") == Some(&Value::Bool(true)),
                close_on_game_start: c.get("close_on_game_start") == Some(&Value::Bool(true)),
                download_concurrency: json_integer(c.get("download_concurrency"))
                    .filter(|n| *n > 0)
                    .map(|n| n as u32)
                    .unwrap_or(6),
                aot_auto_train: json_truthy(c.get("aot_auto_train")),
                fast_boot: c.get("fast_boot") == Some(&Value::Bool(true)),
                fullbright_on_launch: c.get("fullbright_on_launch") == Some(&Value::Bool(true)),
                jdk_path_override: c.get("jdk_path_override").and_then(Value::as_str).map(str::to_string),
                discord_enabled: c.get("discord_enabled") != Some(&Value::Bool(false)),
                data_dir,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = ConfigStore::new(&Paths::new(dir.path()));
        (dir, s)
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn load_creates_the_document_from_defaults_when_missing() {
        let (_d, s) = store();
        assert!(!s.path().exists());
        let cfg = s.load();
        assert_eq!(cfg["default_memory_mb"], Value::Null);
        assert_eq!(cfg["theme"], "dark");
        assert_eq!(cfg["accounts"], json!([]));
        assert_eq!(cfg["identity_url"], "https://id.uhce.top");
        assert!(s.path().exists());
    }

    #[test]
    fn save_merges_and_persists_atomically() {
        let (_d, s) = store();
        s.save(obj(json!({ "default_memory_mb": 4096, "download_concurrency": 4, "jdk_path_override": "/opt/jdk25" })));
        let mut tmp = s.path().as_os_str().to_owned();
        tmp.push(".tmp");
        assert!(!PathBuf::from(tmp).exists());
        let parsed: Value = serde_json::from_str(&std::fs::read_to_string(s.path()).unwrap()).unwrap();
        assert_eq!(parsed["default_memory_mb"], 4096);
        assert_eq!(parsed["download_concurrency"], 4);
        assert_eq!(parsed["jdk_path_override"], "/opt/jdk25");
        assert_eq!(parsed["theme"], "dark");
    }

    #[test]
    fn h2_secrets_are_never_written_to_disk() {
        let (_d, s) = store();
        let live = s.save(obj(json!({
            "access_token": "ACCESS_SECRET_VALUE",
            "xbl_token": "XBL_SECRET_VALUE",
            "xsts_token": "XSTS_SECRET_VALUE",
            "user_hash": "USER_HASH_SECRET_VALUE",
            "discord_enabled": true,
            "accounts": [{ "username": "a", "microsoft": { "refresh_token": "R", "access_token": "NESTED_SECRET" } }],
        })));
        assert_eq!(live["access_token"], "ACCESS_SECRET_VALUE"); // memory keeps it
        let raw = std::fs::read_to_string(s.path()).unwrap();
        for secret in
            ["ACCESS_SECRET_VALUE", "XBL_SECRET_VALUE", "XSTS_SECRET_VALUE", "USER_HASH_SECRET_VALUE", "NESTED_SECRET"]
        {
            assert!(!raw.contains(secret), "{secret} leaked to disk");
        }
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["discord_enabled"], true);
        assert_eq!(parsed["accounts"][0]["microsoft"]["refresh_token"], "R");
    }

    #[test]
    fn app_config_exposes_only_the_whitelisted_subset() {
        let (_d, s) = store();
        let v = serde_json::to_value(s.app_config()).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "accent",
                "aot_auto_train",
                "close_on_game_start",
                "data_dir",
                "default_memory_auto",
                "default_memory_mb",
                "discord_enabled",
                "download_concurrency",
                "fast_boot",
                "fullbright_on_launch",
                "jdk_path_override",
                "recommended_memory_mb",
                "reduced_motion",
            ]
        );
    }

    #[test]
    fn app_config_coerces_flags_to_strict_booleans() {
        let (_d, s) = store();
        s.save(obj(json!({ "fullbright_on_launch": 1, "fast_boot": 1 })));
        assert!(!s.app_config().fullbright_on_launch);
        assert!(!s.app_config().fast_boot);
        s.save(obj(json!({ "fullbright_on_launch": true, "fast_boot": true })));
        assert!(s.app_config().fullbright_on_launch);
        assert!(s.app_config().fast_boot);
    }

    #[test]
    fn recommended_memory_tiers() {
        let gb = 1024f64.powi(3);
        assert_eq!(recommended_memory_mb_for(4.0 * gb), 2048);
        assert_eq!(recommended_memory_mb_for(7.8 * gb), 2048);
        assert_eq!(recommended_memory_mb_for(8.0 * gb), 2048);
        assert_eq!(recommended_memory_mb_for(12.0 * gb), 3072);
        assert_eq!(recommended_memory_mb_for(15.9 * gb), 3072);
        assert_eq!(recommended_memory_mb_for(32.0 * gb), 4096);
        assert_eq!(recommended_memory_mb_for(f64::NAN), 2048);
    }

    #[test]
    fn default_memory_null_is_automatic_and_a_setting_wins() {
        let (_d, s) = store();
        s.save(obj(json!({ "default_memory_mb": null })));
        assert_eq!(s.default_memory_mb(), recommended_memory_mb());
        assert!(s.app_config().default_memory_auto);
        s.save(obj(json!({ "default_memory_mb": 5120 })));
        assert_eq!(s.default_memory_mb(), 5120);
        assert!(!s.app_config().default_memory_auto);
    }

    #[test]
    fn legacy_3072_default_migrates_to_automatic_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        std::fs::write(p.config_json(), r#"{"default_memory_mb":3072}"#).unwrap();
        let s = ConfigStore::new(&p);
        assert_eq!(s.load()["default_memory_mb"], Value::Null);
        assert_eq!(s.load()["memory_auto_migrated"], true);
        // a deliberate 3072 after migration is kept
        s.save(obj(json!({ "default_memory_mb": 3072 })));
        let s2 = ConfigStore::new(&p);
        assert_eq!(s2.load()["default_memory_mb"], 3072);
    }

    // config-identity-migration.test.mjs
    #[test]
    fn legacy_identity_url_is_migrated_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        std::fs::write(p.config_json(), r#"{"identity_url":"https://id.espectral.es","default_memory_mb":2048}"#)
            .unwrap();
        let s = ConfigStore::new(&p);
        let live = s.load();
        assert_eq!(live["identity_url"], "https://id.uhce.top");
        assert_eq!(live["default_memory_mb"], 2048);
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(p.config_json()).unwrap()).unwrap();
        assert_eq!(on_disk["identity_url"], "https://id.uhce.top");

        std::fs::write(p.config_json(), r#"{"identity_url":"https://id.espectral.es/"}"#).unwrap();
        assert_eq!(ConfigStore::new(&p).load()["identity_url"], "https://id.uhce.top");

        std::fs::write(p.config_json(), r#"{"identity_url":"https://id.example.test/v1"}"#).unwrap();
        assert_eq!(ConfigStore::new(&p).load()["identity_url"], "https://id.example.test/v1");
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(p.config_json()).unwrap()).unwrap();
        assert_eq!(on_disk["identity_url"], "https://id.example.test/v1");
    }

    #[test]
    fn is_legacy_identity_url_matches_only_the_bare_host() {
        for url in [
            "https://id.espectral.es",
            "https://id.espectral.es/",
            "https://id.espectral.es//",
            "  https://id.espectral.es  ",
            "https://ID.espectral.es",
        ] {
            assert!(is_legacy_identity_url(url), "{url}");
        }
        for url in [
            "https://id.espectral.es.evil.example",
            "http://id.espectral.es",
            "https://id.espectral.es/v1",
            "https://id.uhce.top",
            "https://espectral.es",
            "",
        ] {
            assert!(!is_legacy_identity_url(url), "{url}");
        }
    }

    #[test]
    fn unknown_keys_and_order_survive_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        std::fs::write(
            p.config_json(),
            r#"{"msa_client_id":"abc","memory_auto_migrated":true,"zzz":{"nested":[1,2]}}"#,
        )
        .unwrap();
        let s = ConfigStore::new(&p);
        s.set("theme", json!("light"));
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(p.config_json()).unwrap()).unwrap();
        assert_eq!(on_disk["msa_client_id"], "abc");
        assert_eq!(on_disk["zzz"], json!({ "nested": [1, 2] }));
        // defaults first (JS spread order), extras appended
        let keys: Vec<_> = on_disk.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys[0], "default_memory_mb");
        assert_eq!(keys.last().unwrap(), "zzz");
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        std::fs::write(p.config_json(), "{not json").unwrap();
        let s = ConfigStore::new(&p);
        assert_eq!(s.theme(), "dark");
        assert_eq!(s.download_concurrency(), 6);
        std::fs::write(p.config_json(), r#"{"accounts":"nope","download_concurrency":0}"#).unwrap();
        let s = ConfigStore::new(&p);
        assert_eq!(s.load()["accounts"], json!([]));
        assert_eq!(s.download_concurrency(), 6);
    }
}
