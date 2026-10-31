//! UI preferences — the native replacement for the Svelte UI's localStorage.
//! FOUNDATION-OWNED API; page modules may use any key prefixed with their
//! group (e.g. "home.selected", "mods.view").
//!
//! Stored as `<data_dir>/ui-prefs.json` (a flat JSON object), written
//! synchronously on every `set` (tiny file).
//!
//! Keys in use: "lang", "discord_session", "recent_commands", "hide_hotkeys",
//! "pip_pinned".

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value};

struct Store {
    path: PathBuf,
    map: Map<String, Value>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

/// Load (or create empty) prefs for a data dir. Call once at startup.
pub fn init(data_dir: &std::path::Path) {
    let path = data_dir.join("ui-prefs.json");
    let map = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let _ = STORE.set(Mutex::new(Store { path, map }));
}

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> Option<R> {
    STORE.get().map(|m| f(&mut m.lock().unwrap_or_else(|p| p.into_inner())))
}

pub fn get(key: &str) -> Option<Value> {
    with(|s| s.map.get(key).cloned()).flatten()
}

pub fn get_str(key: &str) -> Option<String> {
    get(key).and_then(|v| v.as_str().map(str::to_string))
}

pub fn get_bool(key: &str) -> bool {
    get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Set (Value::Null removes) and persist.
pub fn set(key: &str, value: Value) {
    with(|s| {
        if value.is_null() {
            s.map.remove(key);
        } else {
            s.map.insert(key.to_string(), value);
        }
        let text = serde_json::to_string_pretty(&Value::Object(s.map.clone())).unwrap_or_default();
        let tmp = s.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &s.path);
        }
    });
}
