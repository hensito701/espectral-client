//! Port of `src/engine/client.mjs` — the Espectral Client mod config
//! `<gameDir>/config/espectral-client.json` (schema 2): suite registry
//! (`src/engine/suite-registry.json`, compiled in), feature defaults, macros,
//! personalization (theme / hud_theme / background / reduced_motion /
//! tagline), seeding at launch, PATCH reconciliation.
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! Hard rules (Contract A):
//! - NEVER clobber unknown fields — top-level, per-feature, or inside a
//!   macro. Read-modify-write on the RAW file; GET returns a normalized view,
//!   PATCH merges into the raw object.
//! - Atomic writes: temp file + rename in the same directory (2-space JSON +
//!   trailing newline, like the JS).
//! - `features[id]` is an OBJECT `{ enabled, ...extra }`, never a bare
//!   boolean. PATCH shallow-merges per feature id.
//!
//! CONTRACT (called by game-launch resolveLaunch, accounts-skins
//! backgrounds, content routes/client facade): load_client_config,
//! seed_client_config, get_client_info, patch_client_config. The raw-JSON
//! variants ([`load_client_config_value`], [`patch_client_config_value`])
//! keep every unknown key and accept an arbitrary JSON body.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{ClientConfig, ClientInfo, ClientMacro, ClientPatch, ClientRegistryEntry, InstanceMeta};

pub use crate::config::ACCENT_IDS as THEME_IDS;
pub const HUD_THEME_IDS: [&str; 4] = ["cristal", "oro", "minimo", "pixel"];
pub const TAGLINE_MAX: usize = 40;

/// `src/engine/suite-registry.json` — shared verbatim with the mod jar.
pub const SUITE_REGISTRY_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/engine/suite-registry.json"));

const MAX_MACROS: usize = 32;
const MAX_MACRO_NAME: usize = 40;
const MAX_KEYBIND: usize = 32;
const MAX_ACTIONS: usize = 16;
const MAX_ACTION_TEXT: usize = 256;

/// Parsed registry document + derived tables.
#[derive(Debug)]
pub struct SuiteRegistry {
    pub doc: Value,
    pub schema: u64,
    pub suite_default_enabled: bool,
    pub registry: Vec<ClientRegistryEntry>,
    /// id -> `{ enabled, ...state }` (registry order).
    pub feature_defaults: Map<String, Value>,
}

/// JSON numbers like `1.0` become integers, as JS `JSON.stringify` writes them.
fn js_numbers(v: Value) -> Value {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 => Value::from(f as i64),
            _ => Value::Number(n),
        },
        Value::Array(a) => Value::Array(a.into_iter().map(js_numbers).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, js_numbers(v))).collect()),
        other => other,
    }
}

/// The compiled-in suite registry.
pub fn suite_registry() -> &'static SuiteRegistry {
    static R: OnceLock<SuiteRegistry> = OnceLock::new();
    R.get_or_init(|| {
        let doc: Value =
            js_numbers(serde_json::from_str(SUITE_REGISTRY_JSON).expect("suite-registry.json is valid JSON"));
        let features = doc.get("features").and_then(Value::as_array).cloned().unwrap_or_default();
        let s = |f: &Value, k: &str| f.get(k).and_then(Value::as_str).map(str::to_string);
        let registry = features
            .iter()
            .map(|f| ClientRegistryEntry {
                id: s(f, "id").unwrap_or_default(),
                name: s(f, "name_en").unwrap_or_default(),
                description: s(f, "description_en").unwrap_or_default(),
                kind: s(f, "kind").unwrap_or_default(),
                default_enabled: f.get("default_enabled").and_then(Value::as_bool).unwrap_or(false),
                keybind: s(f, "keybind"),
                category: s(f, "category"),
            })
            .collect();
        let mut feature_defaults = Map::new();
        for f in &features {
            let mut d = Map::new();
            d.insert("enabled".into(), f.get("default_enabled").cloned().unwrap_or(Value::Bool(false)));
            if let Some(Value::Object(state)) = f.get("state") {
                for (k, v) in state {
                    d.insert(k.clone(), v.clone());
                }
            }
            feature_defaults.insert(s(f, "id").unwrap_or_default(), Value::Object(d));
        }
        SuiteRegistry {
            schema: doc.get("schema").and_then(Value::as_u64).unwrap_or(2),
            suite_default_enabled: doc.pointer("/suite/default_enabled").and_then(Value::as_bool).unwrap_or(true),
            registry,
            feature_defaults,
            doc,
        }
    })
}

/// Schema version written by seed/PATCH.
pub fn client_config_schema() -> u64 {
    suite_registry().schema
}

/// `PERSONALIZATION_DEFAULTS` (key order as in the JS).
pub fn personalization_defaults() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("theme".into(), Value::from("oro"));
    m.insert("hud_theme".into(), Value::from("cristal"));
    m.insert("background".into(), Value::Null);
    m.insert("reduced_motion".into(), Value::Bool(false));
    m.insert("tagline".into(), Value::Null);
    m
}

/// `<effective game dir>/config/espectral-client.json`.
pub fn client_config_path(engine: &Engine, instance_name: &str) -> PathBuf {
    crate::paths::client_config_in(&crate::instances::effective_game_dir_by_name(engine, instance_name))
}

fn obj(v: Option<&Value>) -> Option<&Map<String, Value>> {
    v.and_then(Value::as_object)
}

/// JS deepEqual (numbers compared by value, object key order ignored).
fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| deep_equal(a, b)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => a == b,
    }
}

/// Read the RAW file; `{}` when missing or unparseable (never fails).
fn read_raw(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

fn merge(base: &Map<String, Value>, over: &Map<String, Value>) -> Map<String, Value> {
    let mut out = base.clone();
    for (k, v) in over {
        out.insert(k.clone(), v.clone());
    }
    out
}

fn coerce_feature_entry(def: &Map<String, Value>, entry: Option<&Value>) -> Map<String, Value> {
    match entry {
        Some(Value::Bool(b)) => {
            let mut m = def.clone();
            m.insert("enabled".into(), Value::Bool(*b));
            m
        }
        Some(Value::Object(e)) => merge(def, e),
        _ => def.clone(),
    }
}

fn normalize(raw: &Map<String, Value>) -> Map<String, Value> {
    let reg = suite_registry();
    let mut out = raw.clone();
    if raw.get("schema").map_or(true, Value::is_null) {
        out.insert("schema".into(), Value::from(reg.schema));
    }
    let mut suite = obj(raw.get("suite")).cloned().unwrap_or_default();
    let enabled = suite.get("enabled").and_then(Value::as_bool).unwrap_or(reg.suite_default_enabled);
    suite.insert("enabled".into(), Value::Bool(enabled));
    out.insert("suite".into(), Value::Object(suite));
    let raw_features = obj(raw.get("features"));
    let mut features = Map::new();
    for (id, def) in &reg.feature_defaults {
        let def = def.as_object().expect("default object");
        features.insert(id.clone(), Value::Object(coerce_feature_entry(def, raw_features.and_then(|f| f.get(id)))));
    }
    if let Some(rf) = raw_features {
        for (id, entry) in rf {
            if features.contains_key(id) {
                continue;
            }
            match entry {
                Value::Object(e) => {
                    let mut m = Map::new();
                    m.insert("enabled".into(), Value::Bool(false));
                    features.insert(id.clone(), Value::Object(merge(&m, e)));
                }
                Value::Bool(b) => {
                    let mut m = Map::new();
                    m.insert("enabled".into(), Value::Bool(*b));
                    features.insert(id.clone(), Value::Object(m));
                }
                _ => {}
            }
        }
    }
    out.insert("features".into(), Value::Object(features));
    let macros = match raw.get("macros") {
        Some(Value::Array(a)) => Value::Array(a.clone()),
        _ => Value::Array(vec![]),
    };
    out.insert("macros".into(), macros);
    let personal = merge(&personalization_defaults(), &obj(raw.get("personalization")).cloned().unwrap_or_default());
    out.insert("personalization".into(), Value::Object(personal));
    out
}

/// Normalized raw JSON view (every unknown key kept). Never fails.
pub fn load_client_config_value(engine: &Engine, instance_name: &str) -> Map<String, Value> {
    normalize(&read_raw(&client_config_path(engine, instance_name)))
}

/// Normalized JSON view -> the typed [`ClientConfig`]. Macros that do not
/// fit the typed shape are left out of the typed view (they stay on disk).
pub fn to_client_config(view: &Map<String, Value>) -> ClientConfig {
    let mut extra = view.clone();
    for k in ["schema", "personalization", "suite", "features", "macros"] {
        extra.remove(k);
    }
    ClientConfig {
        schema: view.get("schema").and_then(Value::as_u64).map(|s| s as u32).unwrap_or(client_config_schema() as u32),
        personalization: obj(view.get("personalization")).cloned(),
        suite: view.get("suite").cloned(),
        features: obj(view.get("features")).cloned().unwrap_or_default(),
        macros: view
            .get("macros")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|m| serde_json::from_value::<ClientMacro>(m.clone()).ok()).collect())
            .unwrap_or_default(),
        extra,
    }
}

/// Load + normalize. Never fails in practice; missing file -> pure defaults.
pub fn load_client_config(engine: &Engine, instance_name: &str) -> EngineResult<ClientConfig> {
    Ok(to_client_config(&load_client_config_value(engine, instance_name)))
}

fn write_atomic(path: &Path, obj: &Map<String, Value>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        "{}.{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("espectral-client.json"),
        std::process::id(),
        crate::platform::now_ms()
    ));
    let mut text = serde_json::to_string_pretty(obj).map_err(std::io::Error::other)?;
    text.push('\n');
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// `supported` = fabric loader AND version in the pinned QoL set.
pub fn is_supported(instance: &InstanceMeta) -> bool {
    instance.loader == crate::types::Loader::Fabric && crate::mods::qol_pins_by_version(&instance.version).is_some()
}

/// instances.getInstance (409 INVALID_NAME / 404 NOT_FOUND like the JS).
async fn load_instance(engine: &Engine, name: &str) -> EngineResult<InstanceMeta> {
    crate::instances::get_instance(engine, name).await
}

fn registry_entries() -> Vec<ClientRegistryEntry> {
    suite_registry().registry.clone()
}

/// GET /api/instances/:name/client -> ClientInfo.
pub async fn get_client_info(engine: &Engine, instance_name: &str) -> EngineResult<ClientInfo> {
    let instance = load_instance(engine, instance_name).await?;
    Ok(ClientInfo {
        config: load_client_config(engine, instance_name)?,
        registry: registry_entries(),
        supported: is_supported(&instance),
        errors: None,
    })
}

/// Seed defaults into `<gameDir>/config/espectral-client.json` (any dir).
pub fn seed_client_config_at(engine: &Engine, path: &Path) {
    let reg = suite_registry();
    let raw = read_raw(path);
    let mut merged = raw.clone();
    let schema = merged.get("schema");
    if schema.map_or(true, Value::is_null) || schema.and_then(Value::as_f64) == Some(1.0) {
        merged.insert("schema".into(), Value::from(reg.schema));
    }
    let mut suite = obj(merged.get("suite")).cloned().unwrap_or_default();
    if !suite.get("enabled").is_some_and(Value::is_boolean) {
        suite.insert("enabled".into(), Value::Bool(reg.suite_default_enabled));
    }
    merged.insert("suite".into(), Value::Object(suite));
    let mut features = obj(merged.get("features")).cloned().unwrap_or_default();
    let mut changed = !deep_equal(&Value::Object(merged.clone()), &Value::Object(raw.clone()));
    for (id, def) in &reg.feature_defaults {
        let def = def.as_object().expect("default object");
        let entry = features.get(id).cloned();
        let filled = Value::Object(coerce_feature_entry(def, entry.as_ref()));
        let base = match &entry {
            Some(Value::Bool(b)) => {
                let mut m = Map::new();
                m.insert("enabled".into(), Value::Bool(*b));
                Some(Value::Object(m))
            }
            other => other.clone(),
        };
        if !base.as_ref().is_some_and(|b| deep_equal(&filled, b)) || matches!(entry, Some(Value::Bool(_))) {
            features.insert(id.clone(), filled);
            changed = true;
        }
    }
    merged.insert("features".into(), Value::Object(features));
    if !merged.get("macros").is_some_and(Value::is_array) {
        merged.insert("macros".into(), Value::Array(vec![]));
        changed = true;
    }
    // Personalization: mirror the launcher accent into `theme` and the global
    // reduced-motion choice; hud_theme/background keep the instance's choice.
    let raw_personal = obj(merged.get("personalization")).cloned();
    let mut personal = raw_personal.clone().unwrap_or_default();
    let accent = engine.config().get_str("accent").unwrap_or_default();
    let theme = if THEME_IDS.contains(&accent.as_str()) { accent } else { "oro".to_string() };
    personal.insert("theme".into(), Value::from(theme));
    let hud_ok = personal.get("hud_theme").and_then(Value::as_str).is_some_and(|h| HUD_THEME_IDS.contains(&h));
    if !hud_ok {
        personal.insert("hud_theme".into(), Value::from("cristal"));
    }
    if !personal.contains_key("background") {
        personal.insert("background".into(), Value::Null);
    }
    personal.insert("reduced_motion".into(), Value::Bool(engine.config().is_true("reduced_motion")));
    let personal = Value::Object(personal);
    if !raw_personal.is_some_and(|rp| deep_equal(&Value::Object(rp), &personal)) {
        merged.insert("personalization".into(), personal);
        changed = true;
    }
    if changed {
        if let Err(e) = write_atomic(path, &merged) {
            tracing::warn!("[client] could not seed espectral-client.json: {e}");
        }
    }
}

/// Write/upgrade the canonical espectral-client.json before a launch.
/// Merges (file wins), writes only on change, never fails a launch.
pub fn seed_client_config(engine: &Engine, instance_name: &str) -> EngineResult<()> {
    seed_client_config_at(engine, &client_config_path(engine, instance_name));
    Ok(())
}

// --- PATCH validation -------------------------------------------------------

fn bad(code: &str, msg: impl Into<String>) -> EngineError {
    EngineError::bad_request(code, msg)
}

fn bad_macro(i: usize, msg: &str) -> EngineError {
    bad("BAD_MACRO", format!("macros[{i}]: {msg}"))
}

fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn validate_macro(m: &Value, i: usize, seen: &mut std::collections::HashSet<String>) -> EngineResult<()> {
    let Some(m) = m.as_object() else { return Err(bad_macro(i, "must be an object")) };
    let id = m.get("id").and_then(Value::as_str).unwrap_or("");
    let id_ok = (1..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !m.get("id").is_some_and(Value::is_string) || !id_ok {
        return Err(bad_macro(i, "id must match ^[A-Za-z0-9_-]{1,32}$"));
    }
    if !seen.insert(id.to_string()) {
        return Err(bad_macro(i, &format!("duplicate id '{id}'")));
    }
    match m.get("name").and_then(Value::as_str) {
        Some(n) if !n.is_empty() && js_len(n) <= MAX_MACRO_NAME => {}
        _ => return Err(bad_macro(i, "name must be a non-empty string of at most 40 chars")),
    }
    match m.get("keybind").and_then(Value::as_str) {
        Some(k) if !k.is_empty() && js_len(k) <= MAX_KEYBIND => {}
        _ => return Err(bad_macro(i, "keybind must be a non-empty string of at most 32 chars")),
    }
    let actions = match m.get("actions").and_then(Value::as_array) {
        Some(a) if !a.is_empty() && a.len() <= MAX_ACTIONS => a,
        _ => return Err(bad_macro(i, &format!("actions must be a non-empty array of at most {MAX_ACTIONS} entries"))),
    };
    for (j, a) in actions.iter().enumerate() {
        let t = a.get("type").and_then(Value::as_str);
        if !a.is_object() || !matches!(t, Some("chat") | Some("command")) {
            return Err(bad_macro(i, &format!("actions[{j}].type must be 'chat' or 'command'")));
        }
        match a.get("text").and_then(Value::as_str) {
            Some(x) if !x.is_empty() && js_len(x) <= MAX_ACTION_TEXT => {}
            _ => {
                return Err(bad_macro(i, &format!("actions[{j}].text must be a non-empty string of at most 256 chars")))
            }
        }
    }
    Ok(())
}

fn is_background_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn validate_patch(patch: &Value) -> EngineResult<()> {
    let Some(p) = patch.as_object() else { return Err(bad("BAD_PATCH", "PATCH body must be a JSON object")) };
    let unknown: Vec<&str> = p
        .keys()
        .map(String::as_str)
        .filter(|k| !matches!(*k, "features" | "macros" | "suite" | "personalization"))
        .collect();
    if !unknown.is_empty() {
        return Err(bad("BAD_PATCH", format!("unknown PATCH fields: {}", unknown.join(", "))));
    }
    if let Some(suite) = p.get("suite") {
        let Some(s) = suite.as_object() else {
            return Err(bad("BAD_PATCH", "suite must be an object ({ enabled, ... })"));
        };
        if s.get("enabled").is_some_and(|e| !e.is_boolean()) {
            return Err(bad("BAD_PATCH", "suite.enabled must be a boolean"));
        }
    }
    if let Some(features) = p.get("features") {
        let Some(f) = features.as_object() else {
            return Err(bad("BAD_FEATURES", "features must be an object keyed by feature id"));
        };
        for (id, entry) in f {
            if entry.is_boolean() {
                continue;
            }
            let Some(e) = entry.as_object() else {
                return Err(bad("BAD_FEATURES", format!("features.{id} must be an object ({{ enabled, ... }})")));
            };
            if e.get("enabled").is_some_and(|v| !v.is_boolean()) {
                return Err(bad("BAD_FEATURES", format!("features.{id}.enabled must be a boolean")));
            }
        }
    }
    if let Some(macros) = p.get("macros") {
        let Some(list) = macros.as_array() else { return Err(bad("BAD_MACROS", "macros must be an array")) };
        if list.len() > MAX_MACROS {
            return Err(bad("TOO_MANY_MACROS", format!("at most {MAX_MACROS} macros per instance")));
        }
        let mut seen = std::collections::HashSet::new();
        for (i, m) in list.iter().enumerate() {
            validate_macro(m, i, &mut seen)?;
        }
    }
    if let Some(personal) = p.get("personalization") {
        let Some(pm) = personal.as_object() else {
            return Err(bad("BAD_PERSONALIZATION", "personalization must be an object"));
        };
        let defaults = personalization_defaults();
        for k in pm.keys() {
            if !defaults.contains_key(k) {
                return Err(bad("BAD_PERSONALIZATION", format!("unknown personalization key: {k}")));
            }
        }
        if let Some(t) = pm.get("theme") {
            if !t.as_str().is_some_and(|t| THEME_IDS.contains(&t)) {
                return Err(bad("BAD_PERSONALIZATION", format!("theme must be one of {}", THEME_IDS.join("|"))));
            }
        }
        if let Some(h) = pm.get("hud_theme") {
            if !h.as_str().is_some_and(|h| HUD_THEME_IDS.contains(&h)) {
                return Err(bad(
                    "BAD_PERSONALIZATION",
                    format!("hud_theme must be one of {}", HUD_THEME_IDS.join("|")),
                ));
            }
        }
        if let Some(b) = pm.get("background") {
            if !b.is_null() && !b.as_str().is_some_and(is_background_id) {
                return Err(bad("BAD_PERSONALIZATION", "background must be a pack id or null"));
            }
        }
        if pm.get("reduced_motion").is_some_and(|r| !r.is_boolean()) {
            return Err(bad("BAD_PERSONALIZATION", "reduced_motion must be a boolean"));
        }
        if let Some(t) = pm.get("tagline").filter(|t| !t.is_null()) {
            let ok = t.as_str().is_some_and(|v| {
                !v.trim().is_empty()
                    && v.chars().count() <= TAGLINE_MAX
                    && !v.chars().any(|c| (c as u32) <= 0x1f || c == '\u{7f}' || c == '\u{a7}')
            });
            if !ok {
                return Err(bad(
                    "BAD_PERSONALIZATION",
                    format!("tagline must be null or 1-{TAGLINE_MAX} characters without control codes or §"),
                ));
            }
        }
    }
    Ok(())
}

/// PATCH /api/instances/:name/client with an arbitrary JSON body
/// (`{ features?, macros?, suite?, personalization? }`; unknown keys -> 400 BAD_PATCH).
/// Response: ClientInfo plus an always-present `errors` array (legacy
/// reconcile is a no-op, so it is always empty).
pub async fn patch_client_config_value(
    engine: &Engine,
    instance_name: &str,
    patch: &Value,
) -> EngineResult<ClientInfo> {
    validate_patch(patch)?;
    let instance = load_instance(engine, instance_name).await?;
    let p = patch.as_object().expect("validated object");
    let path = client_config_path(engine, instance_name);
    let mut raw = read_raw(&path);
    let schema = raw.get("schema");
    if schema.map_or(true, Value::is_null) || schema.and_then(Value::as_f64) == Some(1.0) {
        raw.insert("schema".into(), Value::from(client_config_schema()));
    }
    if let Some(Value::Object(s)) = p.get("suite") {
        let base = obj(raw.get("suite")).cloned().unwrap_or_default();
        raw.insert("suite".into(), Value::Object(merge(&base, s)));
    }
    if let Some(Value::Object(f)) = p.get("features") {
        let mut features = obj(raw.get("features")).cloned().unwrap_or_default();
        for (id, entry) in f {
            let base = match features.get(id) {
                Some(Value::Object(m)) => m.clone(),
                Some(Value::Bool(b)) => {
                    let mut m = Map::new();
                    m.insert("enabled".into(), Value::Bool(*b));
                    m
                }
                _ => Map::new(),
            };
            let over = match entry {
                Value::Bool(b) => {
                    let mut m = Map::new();
                    m.insert("enabled".into(), Value::Bool(*b));
                    m
                }
                Value::Object(m) => m.clone(),
                _ => Map::new(),
            };
            features.insert(id.clone(), Value::Object(merge(&base, &over)));
        }
        raw.insert("features".into(), Value::Object(features));
    }
    if let Some(m) = p.get("macros") {
        raw.insert("macros".into(), m.clone());
    }
    if let Some(Value::Object(pp)) = p.get("personalization") {
        let base = obj(raw.get("personalization")).cloned().unwrap_or_default();
        raw.insert("personalization".into(), Value::Object(merge(&base, pp)));
    }
    write_atomic(&path, &raw)?;
    Ok(ClientInfo {
        config: load_client_config(engine, instance_name)?,
        registry: registry_entries(),
        supported: is_supported(&instance),
        errors: Some(vec![]),
    })
}

/// PATCH /api/instances/:name/client (typed body).
pub async fn patch_client_config(engine: &Engine, instance_name: &str, patch: ClientPatch) -> EngineResult<ClientInfo> {
    let v = serde_json::to_value(&patch)?;
    patch_client_config_value(engine, instance_name, &v).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use serde_json::json;

    fn write_instance(e: &Engine, name: &str) {
        let dir = e.paths().instance_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            e.paths().instance_json(name),
            json!({ "name": name, "version": "1.21.11", "loader": "vanilla" }).to_string(),
        )
        .unwrap();
    }

    fn write_cfg(e: &Engine, name: &str, v: Value) -> PathBuf {
        let p = client_config_path(e, name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, v.to_string()).unwrap();
        p
    }

    fn read(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn defaults() -> &'static Map<String, Value> {
        &suite_registry().feature_defaults
    }

    #[test]
    fn registry_exports_contract_a_entries() {
        let reg = &suite_registry().registry;
        let ids: Vec<&str> = reg.iter().map(|r| r.id.as_str()).collect();
        for id in [
            "fullbright",
            "nofog",
            "zoom",
            "macros",
            "potionstatus",
            "coords",
            "healthstatus",
            "armorstatus",
            "fpsping",
            "lowfire",
            "clearwater",
            "chatheads",
            "skin3d",
            "tabheads",
        ] {
            assert!(ids.contains(&id), "missing registry entry: {id}");
        }
        for item in reg {
            assert_eq!(item.kind, "owned");
            assert!(item.category.is_some(), "{} has a category", item.id);
        }
        assert_eq!(reg.len(), 16);
        let mut cats: Vec<&str> = reg.iter().filter_map(|r| r.category.as_deref()).collect();
        cats.sort();
        cats.dedup();
        assert_eq!(cats, ["chat", "controls", "cosmetics", "hud", "visual"]);
        assert_eq!(reg.iter().find(|r| r.id == "zoom").unwrap().keybind.as_deref(), Some("key.keyboard.z"));
        // Wire shape (camelCase defaultEnabled, keybind only when present).
        let v = serde_json::to_value(&reg[0]).unwrap();
        assert!(v.get("defaultEnabled").is_some());
        assert!(v.get("keybind").is_none());
    }

    #[test]
    fn suite_registry_shape() {
        let doc = &suite_registry().doc;
        assert_eq!(doc["schema"], 2);
        assert_eq!(doc["suite"]["default_enabled"], true);
        assert_eq!(doc["features"].as_array().unwrap().len(), 16);
        assert_eq!(doc["categories"].as_array().unwrap().len(), 5);
        for f in doc["features"].as_array().unwrap() {
            for k in ["name_en", "name_es", "description_en", "description_es"] {
                assert!(f[k].is_string(), "{} {k}", f["id"]);
            }
        }
        let nofog = doc["features"].as_array().unwrap().iter().find(|f| f["id"] == "nofog").unwrap();
        assert_eq!(nofog["default_enabled"], false);
        let cos = doc["features"].as_array().unwrap().iter().find(|f| f["id"] == "cosmetics").unwrap();
        assert_eq!(
            cos["state"],
            json!({ "own": true, "others": true, "slot_espalda": true, "slot_cuello": true,
                    "slot_cabeza": true, "slot_elitros": true, "emotes": true, "physics": true })
        );
    }

    #[test]
    fn feature_defaults() {
        let d = defaults();
        assert_eq!(d["fullbright"]["enabled"], true);
        assert_eq!(d["fullbright"]["gamma"], 1);
        assert_eq!(d["fullbright"]["gamma"].to_string(), "1", "JS number formatting");
        assert_eq!(d["nofog"]["enabled"], false);
        assert_eq!(d["zoom"]["enabled"], true);
        assert_eq!(d["macros"]["enabled"], true);
        for id in [
            "potionstatus",
            "coords",
            "healthstatus",
            "armorstatus",
            "fpsping",
            "lowfire",
            "clearwater",
            "chatheads",
            "skin3d",
        ] {
            assert_eq!(d[id]["enabled"], false, "{id} defaults off");
        }
        for k in ["own", "others", "slot_espalda", "slot_cuello", "slot_cabeza", "slot_elitros", "emotes", "physics"] {
            assert_eq!(d["cosmetics"][k], true, "cosmetics.{k}");
        }
    }

    #[test]
    fn load_defaults_when_missing() {
        let (_d, e) = temp_engine();
        let cfg = load_client_config_value(&e, "missing-inst");
        assert_eq!(cfg["schema"], 2);
        assert_eq!(cfg["suite"]["enabled"], true);
        assert_eq!(cfg["features"], Value::Object(defaults().clone()));
        assert_eq!(cfg["macros"], json!([]));
        assert_eq!(
            cfg["personalization"],
            json!({ "theme": "oro", "hud_theme": "cristal", "background": null, "reduced_motion": false, "tagline": null })
        );
        let typed = load_client_config(&e, "missing-inst").unwrap();
        assert_eq!(typed.schema, 2);
        assert!(typed.macros.is_empty());
    }

    #[test]
    fn seed_writes_defaults_and_preserves_unknown_fields() {
        let (_d, e) = temp_engine();
        let p = write_cfg(
            &e,
            "test-inst",
            json!({
                "schema": 1, "custom_field": "keep-me",
                "features": { "fullbright": { "enabled": false }, "custom_mod": { "enabled": true } },
                "macros": [{ "id": "m1", "name": "Test", "keybind": "key.keyboard.h", "actions": [{ "type": "chat", "text": "hi" }], "future": 1 }],
            }),
        );
        seed_client_config(&e, "test-inst").unwrap();
        let s = read(&p);
        assert_eq!(s["schema"], 2);
        assert_eq!(s["suite"]["enabled"], true);
        assert_eq!(s["custom_field"], "keep-me");
        assert_eq!(s["features"]["fullbright"]["enabled"], false);
        assert_eq!(s["features"]["fullbright"]["gamma"], 1);
        assert_eq!(s["features"]["zoom"]["enabled"], true);
        assert_eq!(s["features"]["custom_mod"]["enabled"], true);
        assert_eq!(s["macros"].as_array().unwrap().len(), 1);
        assert_eq!(s["macros"][0]["future"], 1, "unknown macro keys survive");
        assert!(std::fs::read_to_string(&p).unwrap().ends_with("}\n"));
        // Idempotent: a second seed does not rewrite.
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        seed_client_config(&e, "test-inst").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn seed_fresh_and_v1_upgrade() {
        let (_d, e) = temp_engine();
        seed_client_config(&e, "fresh-suite-inst").unwrap();
        let s = read(&client_config_path(&e, "fresh-suite-inst"));
        assert_eq!(s["schema"], 2);
        assert_eq!(s["suite"]["enabled"], true);
        assert_eq!(s["features"]["nofog"]["enabled"], false);
        assert_eq!(s["features"]["fullbright"]["gamma"], 1);

        let p = write_cfg(
            &e,
            "v1",
            json!({ "schema": 1, "custom_field": "keep-me", "suite": { "enabled": false },
                    "features": { "fullbright": { "enabled": false, "gamma": 3.5 }, "custom_mod": { "enabled": true } },
                    "macros": [] }),
        );
        seed_client_config(&e, "v1").unwrap();
        let s = read(&p);
        assert_eq!(s["schema"], 2);
        assert_eq!(s["suite"]["enabled"], false);
        assert_eq!(s["custom_field"], "keep-me");
        assert_eq!(s["features"]["fullbright"]["enabled"], false);
        assert_eq!(s["features"]["fullbright"]["gamma"], 3.5);
        assert_eq!(s["features"]["custom_mod"]["enabled"], true);
    }

    #[test]
    fn bare_boolean_feature_entries_tolerated() {
        let (_d, e) = temp_engine();
        let p = write_cfg(
            &e,
            "bool",
            json!({ "schema": 2, "suite": { "enabled": true }, "features": { "coords": true }, "macros": [] }),
        );
        assert_eq!(load_client_config_value(&e, "bool")["features"]["coords"]["enabled"], true);
        seed_client_config(&e, "bool").unwrap();
        assert_eq!(read(&p)["features"]["coords"], json!({ "enabled": true }));
    }

    #[test]
    fn personalization_defaults_fill_and_unknown_keys_survive() {
        let (_d, e) = temp_engine();
        write_cfg(
            &e,
            "perso",
            json!({ "schema": 2, "features": {}, "personalization": { "theme": "glaciar", "future_key": 7 } }),
        );
        let c = load_client_config_value(&e, "perso");
        assert_eq!(c["personalization"]["theme"], "glaciar");
        assert_eq!(c["personalization"]["hud_theme"], "cristal");
        assert_eq!(c["personalization"]["background"], Value::Null);
        assert_eq!(c["personalization"]["reduced_motion"], false);
        assert_eq!(c["personalization"]["future_key"], 7);
    }

    #[test]
    fn seed_mirrors_accent_and_reduced_motion() {
        let (_d, e) = temp_engine();
        e.config().set("accent", json!("amatista"));
        e.config().set("reduced_motion", json!(true));
        let p = write_cfg(
            &e,
            "ps",
            json!({ "schema": 2, "features": {}, "personalization": { "hud_theme": "minimo", "background": "aurora" } }),
        );
        seed_client_config(&e, "ps").unwrap();
        let s = read(&p);
        assert_eq!(s["personalization"]["theme"], "amatista");
        assert_eq!(s["personalization"]["reduced_motion"], true);
        assert_eq!(s["personalization"]["hud_theme"], "minimo");
        assert_eq!(s["personalization"]["background"], "aurora");
    }

    #[tokio::test]
    async fn patch_updates_features_and_macros() {
        let (_d, e) = temp_engine();
        write_instance(&e, "test-inst");
        let res = patch_client_config_value(
            &e,
            "test-inst",
            &json!({
                "features": { "fullbright": { "enabled": false }, "zoom": { "enabled": false } },
                "macros": [{ "id": "macro-1", "name": "Say Hello", "keybind": "key.keyboard.h", "actions": [{ "type": "chat", "text": "hello world" }] }],
            }),
        )
        .await
        .unwrap();
        assert_eq!(res.config.features["fullbright"]["enabled"], false);
        assert_eq!(res.config.features["zoom"]["enabled"], false);
        assert_eq!(res.errors, Some(vec![]));
        assert_eq!(res.config.macros.len(), 1);
        assert_eq!(res.config.macros[0].id, "macro-1");
        assert!(!res.supported, "vanilla is not supported");
        let reloaded = load_client_config(&e, "test-inst").unwrap();
        assert_eq!(reloaded.features["zoom"]["enabled"], false);
        assert_eq!(reloaded.macros.len(), 1);
        // Typed entry point.
        let typed = patch_client_config(
            &e,
            "test-inst",
            ClientPatch { suite: Some(json!({ "enabled": false })), ..Default::default() },
        )
        .await
        .unwrap();
        assert_eq!(typed.config.suite.unwrap()["enabled"], false);
        let on = patch_client_config_value(&e, "test-inst", &json!({ "suite": { "enabled": true } })).await.unwrap();
        assert_eq!(on.config.suite.unwrap()["enabled"], true);
        assert_eq!(on.config.schema, 2);
        assert_eq!(read(&client_config_path(&e, "test-inst"))["suite"]["enabled"], true);
    }

    #[tokio::test]
    async fn patch_rejections() {
        let (_d, e) = temp_engine();
        write_instance(&e, "t");
        let code = |r: EngineResult<ClientInfo>| r.unwrap_err().raw_code().map(str::to_string).unwrap();
        assert_eq!(
            code(patch_client_config_value(&e, "t", &json!({ "macros": [{ "id": "bad1", "name": "Bad", "keybind": "h", "actions": [{ "type": "invalid", "text": "test" }] }] })).await),
            "BAD_MACRO"
        );
        assert_eq!(
            code(
                patch_client_config_value(
                    &e,
                    "t",
                    &json!({ "macros": [
                { "id": "dup", "name": "M1", "keybind": "h", "actions": [] },
                { "id": "dup", "name": "M2", "keybind": "j", "actions": [] }] })
                )
                .await
            ),
            "BAD_MACRO"
        );
        assert_eq!(code(patch_client_config_value(&e, "t", &json!({ "bogus_field": true })).await), "BAD_PATCH");
        assert_eq!(code(patch_client_config_value(&e, "t", &json!([])).await), "BAD_PATCH");
        assert_eq!(code(patch_client_config_value(&e, "t", &json!({ "features": { "x": 1 } })).await), "BAD_FEATURES");
        assert_eq!(code(patch_client_config_value(&e, "t", &json!({ "macros": {} })).await), "BAD_MACROS");
        let many: Vec<Value> = (0..33).map(|i| json!({ "id": format!("m{i}") })).collect();
        assert_eq!(code(patch_client_config_value(&e, "t", &json!({ "macros": many })).await), "TOO_MANY_MACROS");
        let missing = patch_client_config_value(&e, "nope", &json!({})).await.unwrap_err();
        assert_eq!((missing.status(), missing.raw_code()), (404, Some("NOT_FOUND")));
    }

    #[tokio::test]
    async fn personalization_validation() {
        let (_d, e) = temp_engine();
        write_instance(&e, "t");
        let ok = patch_client_config_value(
            &e,
            "t",
            &json!({ "personalization": { "theme": "carmesi", "hud_theme": "pixel", "background": "nebula-night", "reduced_motion": true } }),
        )
        .await
        .unwrap();
        let p = ok.config.personalization.unwrap();
        assert_eq!((p["theme"].clone(), p["hud_theme"].clone()), (json!("carmesi"), json!("pixel")));
        assert_eq!(p["background"], "nebula-night");
        assert_eq!(p["reduced_motion"], true);
        let cleared =
            patch_client_config_value(&e, "t", &json!({ "personalization": { "background": null } })).await.unwrap();
        assert_eq!(cleared.config.personalization.unwrap()["background"], Value::Null);
        let forty = format!("{}!", "á".repeat(39));
        let tagged =
            patch_client_config_value(&e, "t", &json!({ "personalization": { "tagline": forty } })).await.unwrap();
        assert_eq!(tagged.config.personalization.unwrap()["tagline"], json!(forty));
        let reset =
            patch_client_config_value(&e, "t", &json!({ "personalization": { "tagline": null } })).await.unwrap();
        assert_eq!(reset.config.personalization.unwrap()["tagline"], Value::Null);
        for (label, body) in [
            ("bad theme id", json!({ "personalization": { "theme": "obsidiana" } })),
            ("non-string theme", json!({ "personalization": { "theme": 4 } })),
            ("bad hud_theme id", json!({ "personalization": { "hud_theme": "neon" } })),
            ("bad background type", json!({ "personalization": { "background": 12 } })),
            ("bad background chars", json!({ "personalization": { "background": "../evil" } })),
            ("bad reduced_motion type", json!({ "personalization": { "reduced_motion": "yes" } })),
            ("unknown key", json!({ "personalization": { "sparkle": true } })),
            ("tagline over 40", json!({ "personalization": { "tagline": "x".repeat(41) } })),
            ("blank tagline", json!({ "personalization": { "tagline": "   " } })),
            ("tagline with §", json!({ "personalization": { "tagline": "§cRojo" } })),
            ("tagline with newline", json!({ "personalization": { "tagline": "a\nb" } })),
            ("non-string tagline", json!({ "personalization": { "tagline": 7 } })),
            ("non-object personalization", json!({ "personalization": "gold" })),
        ] {
            let err = patch_client_config_value(&e, "t", &body).await.unwrap_err();
            assert!(err.has_code("BAD_PERSONALIZATION"), "{label}: {err}");
        }
    }

    #[tokio::test]
    async fn cosmetics_state_keys_round_trip() {
        let (_d, e) = temp_engine();
        write_instance(&e, "c");
        let r = patch_client_config_value(
            &e,
            "c",
            &json!({ "features": { "cosmetics": { "physics": false, "slot_cabeza": false } } }),
        )
        .await
        .unwrap();
        assert_eq!(r.config.features["cosmetics"]["physics"], false);
        assert_eq!(r.config.features["cosmetics"]["own"], true);
        let raw = read(&client_config_path(&e, "c"));
        assert_eq!(raw["features"]["cosmetics"], json!({ "physics": false, "slot_cabeza": false }));
        let re = load_client_config_value(&e, "c");
        assert_eq!(re["features"]["cosmetics"]["emotes"], true);
    }

    #[tokio::test]
    async fn macros_in_the_in_game_editor_shape() {
        let (_d, e) = temp_engine();
        write_instance(&e, "m");
        let macros = json!([
            { "id": "macro-helpop", "name": "macro-helpop", "keybind": "key.keyboard.semicolon", "actions": [{ "type": "command", "text": "/helpop ayuda" }] },
            { "id": "saludo", "name": "saludo", "keybind": "key.keyboard.231", "actions": [{ "type": "chat", "text": "hola a todos" }] },
            { "id": "tl", "name": "tl", "keybind": "key.keyboard.t", "actions": [{ "type": "command", "text": "/tl" }] },
        ]);
        let r = patch_client_config_value(&e, "m", &json!({ "macros": macros })).await.unwrap();
        assert_eq!(r.errors, Some(vec![]));
        let got: Vec<(String, String)> =
            load_client_config(&e, "m").unwrap().macros.into_iter().map(|m| (m.id, m.keybind)).collect();
        assert_eq!(
            got,
            [("macro-helpop", "key.keyboard.semicolon"), ("saludo", "key.keyboard.231"), ("tl", "key.keyboard.t")]
                .map(|(a, b)| (a.to_string(), b.to_string()))
        );
    }

    #[tokio::test]
    async fn get_client_info_and_supported() {
        let (_d, e) = temp_engine();
        std::fs::create_dir_all(e.paths().instance_dir("fab")).unwrap();
        std::fs::write(
            e.paths().instance_json("fab"),
            json!({ "name": "fab", "version": "1.21.11", "loader": "fabric" }).to_string(),
        )
        .unwrap();
        let info = get_client_info(&e, "fab").await.unwrap();
        assert!(info.supported);
        assert_eq!(info.registry.len(), 16);
        assert_eq!(info.errors, None);
        // Custom game_dir: the config follows the effective game dir.
        let gd = e.paths().data_dir().join("elsewhere");
        std::fs::write(
            e.paths().instance_json("fab"),
            json!({ "name": "fab", "version": "1.21.11", "loader": "fabric", "game_dir": gd }).to_string(),
        )
        .unwrap();
        seed_client_config(&e, "fab").unwrap();
        assert!(gd.join("config").join("espectral-client.json").exists());
    }
}
