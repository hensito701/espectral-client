//! Port of `src/engine/cosmetics.mjs` — supporter cosmetics: catalog
//! (`src/engine/cosmetics-catalog.json`), per-uuid state
//! (`<data>/cosmetics.json`), loadout validation vs verified flags +
//! exclusivity, the per-gameDir `config/espectral-cosmetics.json` the mod reads.
//!
//! **Owner: accounts-skins.** Status: PORTED.
//!
//! State file: `{ schema: 1, accounts: { <flat uuid>: { share, emotes,
//! loadout: {slot: itemId|null}, hidden_players: [names] } } }` — sharing and
//! emotes default ON.
//!
//! The mod file (`{ schema:1, uuid, grant|null, share, loadout,
//! hidden_players, emotes }`) is written on every launch and on every PUT;
//! its loadout is SANITIZED against the currently valid grant — a dead or
//! forged grant writes `grant: null` and an empty loadout. The stored loadout
//! (user intent) is never mutated by sanitization.
//!
//! Cross-cluster seams (identity-discord): the grant + validity come from
//! [`grant_view`] (see its doc — needs `identity` to expose the full JS
//! `flagsFor` shape), presence republish goes through
//! [`crate::presence::sync_share`], and "running launches of a uuid" are read
//! from `running-games.json` ([`crate::game_sessions::read_sessions`], live
//! pids only) instead of the bridge registry.
//!
//! Tests: tests/cosmetics.test.mjs.
//!
//! CONTRACT (called by game-launch resolveLaunch, identity-discord presence):
//! write_cosmetics_file, share_enabled.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use indexmap::{IndexMap, IndexSet};
use serde_json::{json, Map, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::identity::normalize_uuid;
use crate::types::{CosmeticItem, CosmeticsAccountState, CosmeticsCatalog};

pub const STATE_SCHEMA: u32 = 1;
pub const MAX_HIDDEN: usize = 200;

const CATALOG_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/engine/cosmetics-catalog.json"));

/// The parsed catalog + lookup tables (JS `catalog()`).
#[derive(Debug)]
pub struct Catalog {
    pub doc: CosmeticsCatalog,
    pub slots: IndexSet<String>,
    pub items: IndexMap<String, CosmeticItem>,
}

/// The catalog (single source: `src/engine/cosmetics-catalog.json`).
pub fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| {
        let doc: CosmeticsCatalog = serde_json::from_str(CATALOG_JSON).expect("cosmetics-catalog.json is valid");
        let slots = doc.slots.iter().cloned().collect();
        let items = doc.items.iter().map(|i| (i.id.clone(), i.clone())).collect();
        Catalog { doc, slots, items }
    })
}

pub fn cosmetics_path(engine: &Engine) -> PathBuf {
    engine.paths().cosmetics_json()
}

/// `accounts` map of cosmetics.json (missing/corrupt -> {}).
fn load_accounts(engine: &Engine) -> Map<String, Value> {
    std::fs::read_to_string(cosmetics_path(engine))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| match v.get("accounts") {
            Some(Value::Object(m)) => Some(m.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn persist(engine: &Engine, accounts: &Map<String, Value>) {
    let doc = json!({ "schema": STATE_SCHEMA, "accounts": accounts });
    if let Err(e) = crate::paths::write_json_atomic(&cosmetics_path(engine), &doc) {
        tracing::warn!("[cosmetics] could not persist cosmetics.json: {e}");
    }
}

/// One uuid's normalized cosmetics state (JS `getCosmeticsState`).
#[derive(Debug, Clone, PartialEq)]
pub struct AccountCosmetics {
    pub share: bool,
    pub emotes: bool,
    /// Only equipped slots (catalog order); stored nulls are omitted.
    pub loadout: IndexMap<String, Option<String>>,
    pub hidden_players: Vec<String>,
}

impl Default for AccountCosmetics {
    fn default() -> Self {
        // Sharing defaults ON (product decision 2026-10-24).
        AccountCosmetics { share: true, emotes: true, loadout: IndexMap::new(), hidden_players: vec![] }
    }
}

impl AccountCosmetics {
    fn to_stored(&self) -> Value {
        json!({ "share": self.share, "emotes": self.emotes, "loadout": self.loadout, "hidden_players": self.hidden_players })
    }
}

/// Normalize a stored record into the public account state (unknown keys tolerated, never returned).
pub fn get_cosmetics_state(engine: &Engine, uuid: &str) -> AccountCosmetics {
    let mut out = AccountCosmetics::default();
    let Some(flat) = normalize_uuid(uuid) else { return out };
    let accounts = load_accounts(engine);
    let Some(Value::Object(raw)) = accounts.get(&flat) else { return out };
    if let Some(Value::Bool(b)) = raw.get("share") {
        out.share = *b;
    }
    if let Some(Value::Bool(b)) = raw.get("emotes") {
        out.emotes = *b;
    }
    if let Some(Value::Object(l)) = raw.get("loadout") {
        for slot in &catalog().slots {
            if let Some(Value::String(v)) = l.get(slot) {
                if !v.is_empty() {
                    out.loadout.insert(slot.clone(), Some(v.clone()));
                }
            }
        }
    }
    if let Some(Value::Array(h)) = raw.get("hidden_players") {
        out.hidden_players =
            h.iter().filter_map(Value::as_str).filter(|s| !s.is_empty()).take(MAX_HIDDEN).map(str::to_string).collect();
    }
    out
}

fn item_allowed(item: &CosmeticItem, flags: &[String]) -> bool {
    item.requires.is_empty() || flags.contains(&item.requires)
}

fn bad_loadout(msg: impl Into<String>) -> EngineError {
    EngineError::bad_request("BAD_LOADOUT", msg)
}

/// Validate a candidate loadout (`{slot: itemId|null}`) against the catalog
/// and a flag set. 400 on the first problem (BAD_LOADOUT, UNKNOWN_COSMETIC,
/// WRONG_SLOT, FLAG_REQUIRED, EXCLUSIVE_CONFLICT). Returns the normalized
/// loadout (input order, then the missing slots as null).
pub fn validate_loadout(loadout: &Value, flags: &[String]) -> EngineResult<IndexMap<String, Option<String>>> {
    let cat = catalog();
    let Value::Object(input) = loadout else {
        return Err(bad_loadout("loadout must be an object {slot: itemId|null}"));
    };
    let mut normalized: IndexMap<String, Option<String>> = IndexMap::new();
    for (slot, value) in input {
        if !cat.slots.contains(slot) {
            return Err(bad_loadout(format!("unknown slot '{slot}'")));
        }
        let id = match value {
            Value::Null => {
                normalized.insert(slot.clone(), None);
                continue;
            }
            Value::String(s) if !s.is_empty() => s,
            _ => return Err(bad_loadout(format!("loadout.{slot} must be an item id or null"))),
        };
        let Some(item) = cat.items.get(id) else {
            return Err(EngineError::bad_request("UNKNOWN_COSMETIC", format!("unknown cosmetic id '{id}'")));
        };
        if item.slot != *slot {
            return Err(EngineError::bad_request(
                "WRONG_SLOT",
                format!("'{id}' belongs in slot '{}', not '{slot}'", item.slot),
            ));
        }
        if !item_allowed(item, flags) {
            return Err(EngineError::bad_request("FLAG_REQUIRED", format!("'{id}' requires '{}'", item.requires)));
        }
        normalized.insert(slot.clone(), Some(id.clone()));
    }
    for slot in &cat.slots {
        normalized.entry(slot.clone()).or_insert(None);
    }
    for (slot_name, blocked) in &cat.doc.exclusive {
        let Some(Some(equipped)) = normalized.get(slot_name) else { continue };
        for (other, item) in &normalized {
            let Some(item) = item else { continue };
            if other != slot_name && blocked.contains(item) {
                return Err(EngineError::bad_request(
                    "EXCLUSIVE_CONFLICT",
                    format!("'{item}' cannot be equipped together with '{equipped}'"),
                ));
            }
        }
    }
    Ok(normalized)
}

/// Drop everything `flags` does not allow (and anything unknown) — for the
/// file the mod reads. Never fails. Every catalog slot is present.
pub fn sanitize_loadout(
    loadout: &IndexMap<String, Option<String>>,
    flags: &[String],
) -> IndexMap<String, Option<String>> {
    let cat = catalog();
    let mut granted: IndexMap<String, Option<String>> = cat.slots.iter().map(|s| (s.clone(), None)).collect();
    for (slot, id) in loadout {
        let Some(id) = id else { continue };
        if !cat.slots.contains(slot) {
            continue;
        }
        let Some(item) = cat.items.get(id) else { continue };
        if item.slot != *slot || !item_allowed(item, flags) {
            continue;
        }
        granted.insert(slot.clone(), Some(id.clone()));
    }
    for (slot_name, blocked) in &cat.doc.exclusive {
        if !matches!(granted.get(slot_name), Some(Some(_))) {
            continue;
        }
        for (other, item) in granted.iter_mut() {
            if other != slot_name && item.as_ref().map(|i| blocked.contains(i)).unwrap_or(false) {
                *item = None;
            }
        }
    }
    granted
}

/// JS `identity.flagsFor(uuid)` full shape.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GrantView {
    pub flags: Vec<String>,
    pub grant: Option<String>,
    pub grant_valid: bool,
}

/// The grant view of a uuid.
///
/// INTEGRATION (identity-discord): the JS read `{ flags, grant, grantValid }`
/// from `identity.flagsFor`; the Rust contract `identity::flags_for` only
/// returns the flags. Until identity exposes the grant + its validity, this
/// conservatively reports `grant_valid = false` (the mod file then carries
/// `grant: null` and an empty loadout — the documented dead-grant behaviour,
/// never a forged ★). Replace the body with the identity call at merge.
pub fn grant_view(engine: &Engine, flat_uuid: &str) -> GrantView {
    GrantView { flags: verified_flags(engine, flat_uuid), grant: None, grant_valid: false }
}

/// `identity::flags_for` (empty while identity is not ported / on error).
pub fn verified_flags(engine: &Engine, flat_uuid: &str) -> Vec<String> {
    match crate::identity::flags_for(engine, flat_uuid) {
        Ok(f) => f,
        Err(EngineError::NotImplemented(_)) => vec![],
        Err(e) => {
            tracing::debug!("[cosmetics] flags_for({flat_uuid}) failed: {e}");
            vec![]
        }
    }
}

/// Build the document the mod reads for a uuid. The grant is included only
/// when it still verifies against THIS uuid.
pub fn cosmetics_doc(engine: &Engine, uuid: &str, view: &GrantView) -> Value {
    let flat = normalize_uuid(uuid);
    let state = get_cosmetics_state(engine, flat.as_deref().unwrap_or(""));
    let valid = flat.is_some() && view.grant_valid;
    let empty: IndexMap<String, Option<String>> = catalog().slots.iter().map(|s| (s.clone(), None)).collect();
    let loadout = if valid { sanitize_loadout(&state.loadout, &view.flags) } else { empty };
    json!({
        "schema": 1,
        "uuid": flat.clone().unwrap_or_else(|| uuid.to_string()),
        "grant": if valid { view.grant.clone().map(Value::from).unwrap_or(Value::Null) } else { Value::Null },
        "share": state.share,
        "loadout": loadout,
        "hidden_players": state.hidden_players,
        "emotes": state.emotes,
    })
}

fn write_file_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, file)
}

/// Write the mod file with an explicit grant view (tests / callers that already resolved it).
pub fn write_cosmetics_file_with(
    engine: &Engine,
    uuid: &str,
    game_dir: &Path,
    canonical_game_dir: Option<&Path>,
    view: &GrantView,
) {
    let doc = match serde_json::to_string_pretty(&cosmetics_doc(engine, uuid, view)) {
        Ok(d) => d,
        Err(e) => return tracing::warn!("[cosmetics] could not write espectral-cosmetics.json: {e}"),
    };
    let mut dirs: Vec<PathBuf> = vec![];
    for d in [Some(game_dir), canonical_game_dir].into_iter().flatten() {
        if d.as_os_str().is_empty() {
            continue;
        }
        let abs = crate::util::normalize_path(&std::path::absolute(d).unwrap_or_else(|_| d.to_path_buf()));
        if !dirs.contains(&abs) {
            dirs.push(abs);
        }
    }
    for dir in dirs {
        if let Err(e) = write_file_atomic(&crate::paths::cosmetics_config_in(&dir), &doc) {
            tracing::warn!("[cosmetics] could not write espectral-cosmetics.json: {e}");
        }
    }
}

/// Write `<gameDir>/config/espectral-cosmetics.json` (and the same bytes into
/// the canonical game dir when it differs). Never fails (warns).
pub fn write_cosmetics_file(
    engine: &Engine,
    flat_uuid: &str,
    game_dir: &Path,
    canonical_game_dir: Option<&Path>,
) -> EngineResult<()> {
    let view = normalize_uuid(flat_uuid).map(|f| grant_view(engine, &f)).unwrap_or_default();
    write_cosmetics_file_with(engine, flat_uuid, game_dir, canonical_game_dir, &view);
    Ok(())
}

/// Game dirs of the live launches owned by a uuid (running-games.json, live pids).
pub fn running_game_dirs(engine: &Engine, flat_uuid: &str) -> Vec<PathBuf> {
    crate::game_sessions::read_sessions(engine)
        .into_values()
        .filter(|s| s.uuid.as_deref().and_then(normalize_uuid).as_deref() == Some(flat_uuid))
        .filter(|s| s.pid != 0 && crate::platform::is_pid_alive(s.pid))
        .filter_map(|s| s.game_dir.filter(|g| !g.is_empty()).map(PathBuf::from))
        .collect()
}

/// Rewrite the file for every running launch owned by `uuid`.
pub fn rewrite_for_running(engine: &Engine, uuid: &str) {
    let Some(flat) = normalize_uuid(uuid) else { return };
    let view = grant_view(engine, &flat);
    for dir in running_game_dirs(engine, &flat) {
        write_cosmetics_file_with(engine, &flat, &dir, None, &view);
    }
}

fn player_name_ok(n: &str) -> bool {
    crate::accounts::is_valid_username(n)
}

/// Validate + apply a PUT /api/cosmetics/:uuid patch (raw JSON body, JS
/// validation codes). `flags` are the caller's verified flags. Side effects:
/// persist, presence share sync (when share/loadout changed), rewrite the mod
/// file of every running game of the uuid. Returns the stored state.
pub fn patch_cosmetics(engine: &Engine, uuid: &str, patch: &Value, flags: &[String]) -> EngineResult<AccountCosmetics> {
    let flat = normalize_uuid(uuid)
        .ok_or_else(|| EngineError::bad_request("BAD_UUID", "uuid must be a 32-char hex string"))?;
    let Value::Object(p) = patch else {
        return Err(EngineError::bad_request("BAD_PATCH", "body must be a JSON object"));
    };
    let unknown: Vec<&str> = p
        .keys()
        .map(String::as_str)
        .filter(|k| !matches!(*k, "share" | "loadout" | "hidden_players" | "emotes"))
        .collect();
    if !unknown.is_empty() {
        return Err(EngineError::bad_request("BAD_PATCH", format!("unknown fields: {}", unknown.join(", "))));
    }
    let _guard = engine.accounts_skins_state().cosmetics_lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut current = get_cosmetics_state(engine, &flat);
    if let Some(v) = p.get("share") {
        current.share = v.as_bool().ok_or_else(|| EngineError::bad_request("BAD_SHARE", "share must be a boolean"))?;
    }
    if let Some(v) = p.get("emotes") {
        current.emotes =
            v.as_bool().ok_or_else(|| EngineError::bad_request("BAD_EMOTES", "emotes must be a boolean"))?;
    }
    if let Some(v) = p.get("hidden_players") {
        let bad = || {
            EngineError::bad_request(
                "BAD_HIDDEN",
                format!("hidden_players must be an array of at most {MAX_HIDDEN} names"),
            )
        };
        let arr = v.as_array().filter(|a| a.len() <= MAX_HIDDEN).ok_or_else(bad)?;
        let mut names: IndexSet<String> = IndexSet::new();
        for n in arr {
            match n.as_str() {
                Some(s) if player_name_ok(s) => {
                    names.insert(s.to_string());
                }
                _ => {
                    return Err(EngineError::bad_request(
                        "BAD_HIDDEN",
                        "hidden player names must match /^[A-Za-z0-9_]{1,16}$/",
                    ))
                }
            }
        }
        current.hidden_players = names.into_iter().collect();
    }
    if let Some(v) = p.get("loadout") {
        current.loadout = validate_loadout(v, flags)?;
    }
    let mut accounts = load_accounts(engine);
    accounts.insert(flat.clone(), current.to_stored());
    persist(engine, &accounts);
    drop(_guard);

    if p.contains_key("share") || p.contains_key("loadout") {
        if let Err(e) = crate::presence::sync_share(engine, &flat) {
            if !matches!(e, EngineError::NotImplemented(_)) {
                tracing::warn!("[cosmetics] presence republish failed: {e}");
            }
        }
    }
    rewrite_for_running(engine, &flat);
    Ok(current)
}

/// Public view of one uuid for GET /api/cosmetics.
pub fn cosmetics_account_view(engine: &Engine, uuid: &str, flags: &[String]) -> CosmeticsAccountState {
    let state = get_cosmetics_state(engine, uuid);
    CosmeticsAccountState {
        share: state.share,
        loadout: state.loadout,
        hidden_players: state.hidden_players,
        emotes: state.emotes,
        flags: flags.to_vec(),
    }
}

/// `getCosmeticsState(uuid).share` — affirmative consent to publish presence.
pub fn share_enabled(engine: &Engine, flat_uuid: &str) -> EngineResult<bool> {
    Ok(get_cosmetics_state(engine, flat_uuid).share)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    const UUID: &str = "aabbccddeeff00112233445566778899";

    fn mvp() -> Vec<String> {
        vec!["supporter".into(), "mvp".into()]
    }

    fn lo(pairs: &[(&str, Option<&str>)]) -> IndexMap<String, Option<String>> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(str::to_string))).collect()
    }

    fn code<T: std::fmt::Debug>(r: EngineResult<T>) -> String {
        r.unwrap_err().envelope().code
    }

    #[test]
    fn catalog_loads_from_the_js_source() {
        let c = catalog();
        assert_eq!(c.slots.iter().collect::<Vec<_>>(), ["espalda", "cuello", "cabeza", "elitros"]);
        assert!(c.items.contains_key("alas_eco"));
        assert!(!c.slots.contains("brazos"));
    }

    #[test]
    fn legal_loadout_round_trips_with_all_slots() {
        let out = validate_loadout(
            &json!({ "espalda": "alas_eco", "cuello": "bufanda_llama_alma", "cabeza": "orejas_vulpinas" }),
            &mvp(),
        )
        .unwrap();
        assert_eq!(out["espalda"].as_deref(), Some("alas_eco"));
        assert_eq!(out["cuello"].as_deref(), Some("bufanda_llama_alma"));
        assert_eq!(out["cabeza"].as_deref(), Some("orejas_vulpinas"));
        assert_eq!(out["elitros"], None);
        assert!(!out.contains_key("brazos"));
    }

    #[test]
    fn loadout_rejections() {
        assert_eq!(code(validate_loadout(&json!({ "piernas": "alas_eco" }), &mvp())), "BAD_LOADOUT");
        assert_eq!(code(validate_loadout(&json!({ "espalda": "nope" }), &mvp())), "UNKNOWN_COSMETIC");
        assert_eq!(code(validate_loadout(&json!({ "cuello": "alas_eco" }), &mvp())), "WRONG_SLOT");
        assert_eq!(code(validate_loadout(&json!("manto_espectral"), &mvp())), "BAD_LOADOUT");
        assert_eq!(code(validate_loadout(&json!({ "espalda": 3 }), &mvp())), "BAD_LOADOUT");
        assert_eq!(code(validate_loadout(&json!({ "espalda": "manto_espectral" }), &[])), "FLAG_REQUIRED");
        assert_eq!(
            code(validate_loadout(&json!({ "espalda": "manto_espectral" }), &["supporter".into()])),
            "FLAG_REQUIRED"
        );
    }

    #[test]
    fn exclusivity_is_enforced_wings_are_fine() {
        for cloak in ["manto_espectral", "jirones_ceniza"] {
            assert_eq!(
                code(validate_loadout(&json!({ "cuello": "bufanda_llama_alma", "espalda": cloak }), &mvp())),
                "EXCLUSIVE_CONFLICT"
            );
        }
        let ok = validate_loadout(&json!({ "cuello": "bufanda_llama_alma", "espalda": "alas_eco" }), &mvp()).unwrap();
        assert_eq!(ok["espalda"].as_deref(), Some("alas_eco"));
    }

    #[test]
    fn sanitize_drops_disallowed_and_resolves_exclusivity() {
        let state = lo(&[
            ("espalda", Some("manto_espectral")),
            ("cuello", Some("bufanda_llama_alma")),
            ("cabeza", Some("nope")),
        ]);
        let clean = sanitize_loadout(&state, &mvp());
        assert_eq!(clean["espalda"], None);
        assert_eq!(clean["cuello"].as_deref(), Some("bufanda_llama_alma"));
        assert_eq!(clean["cabeza"], None);
        assert!(!clean.contains_key("brazos"));
        assert!(sanitize_loadout(&state, &[]).values().all(Option::is_none));
    }

    #[test]
    fn sanitize_tolerates_the_withdrawn_brazos_slot() {
        let stored = lo(&[
            ("espalda", Some("alas_eco")),
            ("cuello", None),
            ("cabeza", Some("orejas_felinas")),
            ("brazos", Some("vendas_runicas")),
            ("elitros", None),
        ]);
        assert_eq!(
            sanitize_loadout(&stored, &mvp()),
            lo(&[
                ("espalda", Some("alas_eco")),
                ("cuello", None),
                ("cabeza", Some("orejas_felinas")),
                ("elitros", None)
            ])
        );
        let err = validate_loadout(&serde_json::to_value(&stored).unwrap(), &mvp()).unwrap_err().envelope();
        assert!(err.code == "BAD_LOADOUT" && err.message.contains("brazos"));
    }

    #[test]
    fn cosmetics_file_carries_valid_grants_and_sanitized_loadout() {
        let (d, e) = temp_engine();
        let game_dir = d.path().join("game");
        patch_cosmetics(
            &e,
            UUID,
            &json!({ "share": true, "loadout": { "espalda": "alas_eco", "cuello": "bufanda_llama_alma" }, "hidden_players": ["Griefer_1"] }),
            &mvp(),
        )
        .unwrap();
        let view = GrantView { flags: mvp(), grant: Some("h.p.s".into()), grant_valid: true };
        let canonical = d.path().join("canonical");
        write_cosmetics_file_with(&e, UUID, &game_dir, Some(&canonical), &view);
        let read = |dir: &Path| -> Value {
            serde_json::from_str(&std::fs::read_to_string(dir.join("config/espectral-cosmetics.json")).unwrap())
                .unwrap()
        };
        let doc = read(&game_dir);
        assert_eq!(read(&canonical), doc);
        assert_eq!(doc["schema"], 1);
        assert_eq!(doc["uuid"], UUID);
        assert_eq!(doc["share"], true);
        assert_eq!(doc["emotes"], true);
        assert_eq!(doc["grant"], "h.p.s");
        assert_eq!(
            doc["loadout"],
            json!({ "espalda": "alas_eco", "cuello": "bufanda_llama_alma", "cabeza": null, "elitros": null })
        );
        assert_eq!(doc["hidden_players"], json!(["Griefer_1"]));
        assert_eq!(
            doc.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["schema", "uuid", "grant", "share", "loadout", "hidden_players", "emotes"]
        );

        // invalid / expired grant -> grant null + empty loadout; prefs survive
        let dead = GrantView { flags: mvp(), grant: Some("tampered".into()), grant_valid: false };
        write_cosmetics_file_with(&e, UUID, &game_dir, None, &dead);
        let doc = read(&game_dir);
        assert_eq!(doc["grant"], Value::Null);
        assert!(doc["loadout"].as_object().unwrap().values().all(Value::is_null));
        assert_eq!(doc["share"], true);
        assert_eq!(doc["hidden_players"], json!(["Griefer_1"]));

        // the contract fn never fails and (until identity lands) writes the dead-grant doc
        write_cosmetics_file(&e, UUID, &game_dir, None).unwrap();
        assert_eq!(read(&game_dir)["grant"], Value::Null);
    }

    #[test]
    fn patch_validation_codes() {
        let (_d, e) = temp_engine();
        let u2 = "ffffffffffffffffffffffffffffffff";
        assert_eq!(code(patch_cosmetics(&e, u2, &json!({ "bogus": 1 }), &[])), "BAD_PATCH");
        assert_eq!(code(patch_cosmetics(&e, u2, &json!([1]), &[])), "BAD_PATCH");
        assert_eq!(code(patch_cosmetics(&e, u2, &json!({ "share": "yes" }), &[])), "BAD_SHARE");
        assert_eq!(code(patch_cosmetics(&e, u2, &json!({ "emotes": 1 }), &[])), "BAD_EMOTES");
        assert_eq!(code(patch_cosmetics(&e, u2, &json!({ "hidden_players": ["not a name!"] }), &[])), "BAD_HIDDEN");
        assert_eq!(code(patch_cosmetics(&e, u2, &json!({ "hidden_players": "x" }), &[])), "BAD_HIDDEN");
        assert_eq!(code(patch_cosmetics(&e, "nope", &json!({}), &[])), "BAD_UUID");
        let out =
            patch_cosmetics(&e, u2, &json!({ "share": true, "emotes": false, "hidden_players": ["A", "A", "B"] }), &[])
                .unwrap();
        assert!(out.share && !out.emotes);
        assert_eq!(out.hidden_players, ["A", "B"]);
        // persisted in the JS layout
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(cosmetics_path(&e)).unwrap()).unwrap();
        assert_eq!(raw["schema"], 1);
        assert_eq!(raw["accounts"][u2]["emotes"], false);
        assert!(!share_enabled(&e, UUID).is_err());
        assert!(share_enabled(&e, u2).unwrap());
        // dashed uuid normalizes to the same record
        assert_eq!(get_cosmetics_state(&e, "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF").hidden_players, ["A", "B"]);
    }

    #[test]
    fn state_reads_leniently() {
        let (_d, e) = temp_engine();
        std::fs::write(
            cosmetics_path(&e),
            json!({ "accounts": { UUID: { "share": "x", "emotes": false, "loadout": { "espalda": "alas_eco", "cuello": null, "brazos": "v" }, "hidden_players": ["a", 3, ""], "future": 1 } } }).to_string(),
        )
        .unwrap();
        let s = get_cosmetics_state(&e, UUID);
        assert!(s.share && !s.emotes);
        assert_eq!(s.loadout, lo(&[("espalda", Some("alas_eco"))]));
        assert_eq!(s.hidden_players, ["a"]);
        let v = serde_json::to_value(cosmetics_account_view(&e, UUID, &mvp())).unwrap();
        assert_eq!(v["loadout"], json!({ "espalda": "alas_eco" }));
        assert_eq!(v["flags"], json!(["supporter", "mvp"]));
        assert_eq!(get_cosmetics_state(&e, "bad"), AccountCosmetics::default());
    }
}
