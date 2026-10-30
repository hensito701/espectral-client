//! Port of `src/engine/accounts.mjs` — accounts stored in `config.accounts`
//! (offline + Microsoft + Lunar-imported), active account, avatar colors and
//! images (`<data>/avatars/<uuid>.png`).
//!
//! **Owner: accounts-skins.** Status: PORTED.
//!
//! Offline UUID derivation (Java-compatible):
//!   md5 = MD5('OfflinePlayer:' + username), md5[6] = (md5[6] & 0x0f) | 0x30,
//!   md5[8] = (md5[8] & 0x3f) | 0x80, canonical 8-4-4-4-12 hex.
//!
//! Accounts live inside `<data>/config.json` (`accounts` + `active_username`).
//! Every write is an atomic read-modify-write of the live config document
//! ([`crate::config::ConfigStore::update`]), so unknown account fields
//! written by the JS engine survive. Secrets (`access_token` …) are stripped
//! by the config store on every write; Minecraft access tokens live in the
//! accounts-skins cluster state + `mc-token-cache.json` (see `msauth.rs`).
//!
//! Tests: tests/accounts.test.mjs, personalization.test.mjs (account parts),
//! personalization-routes.test.mjs (avatar routes), skin-avatar.test.mjs.
//!
//! CONTRACT (used by game-launch, identity-discord, accounts-skins): every
//! `pub fn` below.

use std::path::PathBuf;

use md5::{Digest, Md5};
use rand::Rng;
use serde_json::{Map, Value};

use crate::api::DeletedReply;
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{Account, MicrosoftAuth, PublicAccount, PublicMicrosoft, TokenKind};

/// `^[A-Za-z0-9_]{1,16}$`
pub fn is_valid_username(username: &str) -> bool {
    (1..=16).contains(&username.len()) && username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Avatar color is an integer on the 0-359 wheel.
pub fn is_valid_avatar_color(c: i64) -> bool {
    (0..=359).contains(&c)
}

/// Circular distance between two hues on the 0-359 wheel.
fn hue_distance(a: i64, b: i64) -> i64 {
    let d = (a - b).abs() % 360;
    d.min(360 - d)
}

/// Random avatar color >= 30 degrees (circular) from every color in use.
/// 50 tries, then accept anything.
pub fn pick_avatar_color(accounts: &[Account]) -> i64 {
    let taken: Vec<i64> =
        accounts.iter().filter_map(|a| a.avatar_color).filter(|c| is_valid_avatar_color(*c)).collect();
    let mut rng = rand::thread_rng();
    for _ in 0..50 {
        let h: i64 = rng.gen_range(0..360);
        if taken.iter().all(|t| hue_distance(h, *t) >= 30) {
            return h;
        }
    }
    rng.gen_range(0..360)
}

/// Java-compatible offline UUID: md5("OfflinePlayer:" + name), version 3, RFC 4122 variant.
pub fn offline_uuid(username: &str) -> String {
    let mut b: [u8; 16] = Md5::digest(format!("OfflinePlayer:{username}").as_bytes()).into();
    b[6] = (b[6] & 0x0f) | 0x30;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// `<data>/avatars`
pub fn avatars_dir(engine: &Engine) -> PathBuf {
    engine.paths().avatars_dir()
}

/// `<data>/avatars/<uuid>.png`
pub fn avatar_path_for_uuid(engine: &Engine, uuid: &str) -> PathBuf {
    engine.paths().avatars_dir().join(format!("{uuid}.png"))
}

/// True when the account has an avatar image on disk.
pub fn has_avatar(engine: &Engine, account: &Account) -> bool {
    !account.uuid.is_empty() && avatar_path_for_uuid(engine, &account.uuid).exists()
}

/// Every stored account (config.accounts order).
pub fn list_accounts(engine: &Engine) -> Vec<Account> {
    engine.config().accounts()
}

pub fn get_account(engine: &Engine, username: &str) -> Option<Account> {
    list_accounts(engine).into_iter().find(|a| a.username == username)
}

/// The account named by `config.active_username`.
pub fn get_active_account(engine: &Engine) -> Option<Account> {
    engine.config().active_username().and_then(|u| get_account(engine, &u))
}

/// H2: the UI-safe shape — never the refresh token or any access-token material.
/// `avatar_auto` (engine-internal marker) never leaks: it is a typed field of
/// [`Account`], not part of `extra`.
pub fn public_account(engine: &Engine, a: &Account) -> PublicAccount {
    let microsoft = match (&a.token_kind, &a.microsoft) {
        (TokenKind::Msa, Some(m)) => Some(PublicMicrosoft { lunar: m.lunar == Some(true) }),
        _ => None,
    };
    let mut extra = a.extra.clone();
    for k in crate::config::SECRET_KEYS {
        extra.remove(*k);
    }
    PublicAccount {
        username: a.username.clone(),
        uuid: a.uuid.clone(),
        token_kind: a.token_kind,
        avatar_color: a.avatar_color,
        microsoft,
        created_at: a.created_at.clone(),
        updated_at: a.updated_at.clone(),
        last_used: a.last_used.clone(),
        has_avatar: has_avatar(engine, a),
        extra,
    }
}

fn not_found(username: &str) -> EngineError {
    EngineError::not_found("NOT_FOUND", format!("account '{username}' does not exist"))
}

/// Atomic read-modify-write of `config.accounts` (+ the whole document for
/// `active_username`). Entries that are not objects are dropped on write.
pub(crate) fn modify_accounts<R>(
    engine: &Engine,
    f: impl FnOnce(&mut Vec<Account>, &mut Map<String, Value>) -> R,
) -> R {
    engine.config().update(|doc| {
        let mut accounts: Vec<Account> = match doc.get("accounts") {
            Some(Value::Array(a)) => a.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect(),
            _ => vec![],
        };
        let r = f(&mut accounts, doc);
        doc.insert("accounts".into(), serde_json::to_value(&accounts).unwrap_or(Value::Array(vec![])));
        r
    })
}

/// POST /api/accounts — create an offline account and make it active.
pub fn create_account(engine: &Engine, username: &str) -> EngineResult<Account> {
    if !is_valid_username(username) {
        return Err(EngineError::bad_request("INVALID_USERNAME", "username must be 1-16 characters of [A-Za-z0-9_]"));
    }
    modify_accounts(engine, |accounts, doc| {
        if accounts.iter().any(|a| a.username == username) {
            return Err(EngineError::conflict("ALREADY_EXISTS", format!("account '{username}' already exists")));
        }
        let now = crate::platform::now_iso();
        let account = Account {
            username: username.to_string(),
            uuid: offline_uuid(username),
            token_kind: TokenKind::Offline,
            avatar_color: Some(pick_avatar_color(accounts)),
            created_at: Some(now.clone()),
            last_used: Some(now),
            ..Default::default()
        };
        accounts.push(account.clone());
        doc.insert("active_username".into(), Value::from(username));
        Ok(account)
    })
}

/// A completed Microsoft device login (`ms` in accounts.mjs upsertMsaAccount).
#[derive(Debug, Clone, PartialEq)]
pub struct MsaLogin {
    pub username: String,
    /// Mojang profile id.
    pub uuid: String,
    pub refresh_token: Option<String>,
    /// Seconds (default 86400).
    pub expires_in: Option<f64>,
}

fn upsert_online(engine: &Engine, username: &str, uuid: &str, microsoft: MicrosoftAuth) -> Account {
    modify_accounts(engine, |accounts, doc| {
        let now = crate::platform::now_iso();
        let existing = accounts.iter().position(|a| a.token_kind == TokenKind::Msa && a.uuid == uuid);
        let prev = existing.map(|i| accounts[i].clone());
        let avatar_color = match prev.as_ref().and_then(|p| p.avatar_color) {
            Some(c) if is_valid_avatar_color(c) => c,
            _ => pick_avatar_color(accounts),
        };
        let account = Account {
            username: username.to_string(),
            uuid: uuid.to_string(),
            token_kind: TokenKind::Msa,
            avatar_color: Some(avatar_color),
            microsoft: Some(microsoft),
            created_at: Some(prev.as_ref().and_then(|p| p.created_at.clone()).unwrap_or_else(|| now.clone())),
            updated_at: Some(now.clone()),
            // last_used is set so the UI's active derivation (max last_used)
            // matches the engine's active_username.
            last_used: Some(now),
            ..Default::default()
        };
        match existing {
            Some(i) => accounts[i] = account.clone(),
            None => accounts.push(account.clone()),
        }
        doc.insert("active_username".into(), Value::from(account.username.clone()));
        account
    })
}

/// Add (or replace) a Microsoft account from a completed device login.
/// Re-logging into the same Microsoft account updates the existing entry.
pub fn upsert_msa_account(engine: &Engine, ms: &MsaLogin) -> Account {
    let microsoft = MicrosoftAuth {
        refresh_token: ms.refresh_token.clone(),
        // access token cached in memory only — NOT persisted (see msauth.rs)
        expires_at: Some(crate::platform::now_ms() as f64 + ms.expires_in.unwrap_or(86400.0) * 1000.0),
        ..Default::default()
    };
    upsert_online(engine, &ms.username, &ms.uuid, microsoft)
}

/// Add (or replace) a Lunar-sourced Microsoft account (refresh token from
/// Lunar Client's `accounts.json`). token_kind stays 'msa'; msauth refreshes
/// it through the legacy login.live.com endpoint (`microsoft.lunar = true`).
pub fn upsert_lunar_account(engine: &Engine, username: &str, uuid: &str, refresh_token: &str) -> Account {
    let microsoft = MicrosoftAuth {
        refresh_token: Some(refresh_token.to_string()),
        lunar: Some(true),
        expires_at: Some(0.0),
        ..Default::default()
    };
    upsert_online(engine, username, uuid, microsoft)
}

fn remove_account_where(engine: &Engine, username: &str, msa_only: bool) -> EngineResult<DeletedReply> {
    let removed = modify_accounts(engine, |accounts, doc| {
        let idx =
            accounts.iter().position(|a| a.username == username && (!msa_only || a.token_kind == TokenKind::Msa))?;
        let removed = accounts.remove(idx);
        if doc.get("active_username").and_then(Value::as_str) == Some(username) {
            doc.insert("active_username".into(), Value::Null);
        }
        Some(removed)
    });
    let Some(removed) = removed else {
        return Err(if msa_only {
            EngineError::not_found("NOT_FOUND", format!("Microsoft account '{username}' does not exist"))
        } else {
            not_found(username)
        });
    };
    if !removed.uuid.is_empty() {
        remove_avatar_file(engine, &removed.uuid);
    }
    Ok(DeletedReply { deleted: username.to_string(), trash: None })
}

/// Remove a Microsoft account (and its stored refresh token).
pub fn delete_msa_account(engine: &Engine, username: &str) -> EngineResult<DeletedReply> {
    remove_account_where(engine, username, true)
}

/// Remove any account.
pub fn delete_account(engine: &Engine, username: &str) -> EngineResult<DeletedReply> {
    remove_account_where(engine, username, false)
}

/// Mark `username` active (and bump its `last_used`). 404 when missing.
pub fn set_active_account(engine: &Engine, username: &str) -> EngineResult<String> {
    modify_accounts(engine, |accounts, doc| {
        let account = accounts.iter_mut().find(|a| a.username == username).ok_or_else(|| not_found(username))?;
        account.last_used = Some(crate::platform::now_iso());
        doc.insert("active_username".into(), Value::from(username));
        Ok(username.to_string())
    })
}

// ---------------------------------------------------------------------------
// Avatar color + image (<dataDir>/avatars/<uuid>.png)
// ---------------------------------------------------------------------------

/// Set an account's avatar color (0-359 | None to clear). 400 BAD_AVATAR_COLOR, 404 NOT_FOUND.
pub fn set_avatar_color(engine: &Engine, username: &str, color: Option<i64>) -> EngineResult<PublicAccount> {
    if let Some(c) = color {
        if !is_valid_avatar_color(c) {
            return Err(EngineError::bad_request(
                "BAD_AVATAR_COLOR",
                "avatar_color must be an integer between 0 and 359 or null",
            ));
        }
    }
    let account = modify_accounts(engine, |accounts, _| {
        let account = accounts.iter_mut().find(|a| a.username == username).ok_or_else(|| not_found(username))?;
        account.avatar_color = color;
        Ok::<_, EngineError>(account.clone())
    })?;
    Ok(public_account(engine, &account))
}

/// Avatar image path for an account (by username). 404 when missing.
pub fn avatar_path_for(engine: &Engine, username: &str) -> EngineResult<PathBuf> {
    let account = get_account(engine, username).ok_or_else(|| not_found(username))?;
    Ok(avatar_path_for_uuid(engine, &account.uuid))
}

/// Read an account avatar (PNG bytes). 404 when the account or image is missing.
pub fn read_account_avatar(engine: &Engine, username: &str) -> EngineResult<Vec<u8>> {
    let p = avatar_path_for(engine, username)?;
    std::fs::read(p).map_err(|_| EngineError::not_found("NOT_FOUND", format!("account '{username}' has no avatar")))
}

/// Store an account avatar (validated PNG bytes). A custom upload always wins:
/// the skin-derived marker is dropped. Returns `has_avatar` (always true).
pub fn write_account_avatar(engine: &Engine, username: &str, png: &[u8]) -> EngineResult<bool> {
    let p = avatar_path_for(engine, username)?;
    std::fs::create_dir_all(avatars_dir(engine))?;
    std::fs::write(&p, png)?;
    clear_avatar_auto_flag(engine, username);
    Ok(true)
}

/// Remove an account avatar (missing file is a no-op). Returns `has_avatar` (always false).
pub fn remove_account_avatar(engine: &Engine, username: &str) -> EngineResult<bool> {
    let p = avatar_path_for(engine, username)?;
    let _ = std::fs::remove_file(p);
    clear_avatar_auto_flag(engine, username);
    Ok(false)
}

/// Mark (or unmark) an account avatar as skin-derived. 404 when missing.
pub fn set_avatar_auto(engine: &Engine, username: &str, auto: bool) -> EngineResult<Account> {
    modify_accounts(engine, |accounts, _| {
        let account = accounts.iter_mut().find(|a| a.username == username).ok_or_else(|| not_found(username))?;
        account.avatar_auto = auto.then_some(true);
        Ok(account.clone())
    })
}

/// Best-effort drop of the skin-derived marker (never fails).
fn clear_avatar_auto_flag(engine: &Engine, username: &str) {
    let has_marker = get_account(engine, username).map(|a| a.avatar_auto.is_some()).unwrap_or(false);
    if has_marker {
        let _ = set_avatar_auto(engine, username, false);
    }
}

/// Best-effort avatar file cleanup when an account goes away.
fn remove_avatar_file(engine: &Engine, uuid: &str) {
    let _ = std::fs::remove_file(avatar_path_for_uuid(engine, uuid));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use serde_json::json;

    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    /// Independent reimplementation of the Java offline-UUID algorithm (tests/accounts.test.mjs).
    fn local_offline_uuid(name: &str) -> String {
        let mut b: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}")).into();
        b[6] = (b[6] & 0x0f) | 0x30;
        b[8] = (b[8] & 0x3f) | 0x80;
        let h = hex::encode(b);
        format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
    }

    #[test]
    fn offline_uuid_is_java_compatible() {
        // Java: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(offline_uuid("Steve"), "5627dd98-e6be-3c21-b8a8-e92344183641");
        let re = regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-3[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$").unwrap();
        for name in ["Notch", "Espectral", "Steve", "Player_01", &"a".repeat(16), "x"] {
            assert_eq!(offline_uuid(name), local_offline_uuid(name), "{name}");
            assert!(re.is_match(&offline_uuid(name)), "{name}");
        }
    }

    #[test]
    fn usernames() {
        assert!(is_valid_username("Steve_123"));
        assert!(is_valid_username("Notch"));
        assert!(is_valid_username("Player_01"));
        assert!(!is_valid_username(""));
        assert!(!is_valid_username("has space"));
        assert!(!is_valid_username(&"x".repeat(17)));
        assert!(!is_valid_username("bad-name"));
    }

    #[test]
    fn public_account_strips_tokens() {
        let (_d, e) = temp_engine();
        e.config().set(
            "accounts",
            json!([{
                "username": "A", "uuid": "u-1", "token_kind": "msa", "avatar_auto": true,
                "microsoft": { "refresh_token": "SECRET", "lunar": true }, "access_token": "SECRET2"
            }]),
        );
        e.config().set("active_username", json!("A"));
        let a = get_active_account(&e).unwrap();
        let v = serde_json::to_string(&public_account(&e, &a)).unwrap();
        assert!(!v.contains("SECRET"), "{v}");
        assert!(!v.contains("avatar_auto"), "{v}");
        assert!(v.contains(r#""microsoft":{"lunar":true}"#), "{v}");
        assert!(v.contains(r#""has_avatar":false"#));
    }

    #[test]
    fn offline_accounts_drop_the_microsoft_block() {
        let (_d, e) = temp_engine();
        e.config().set("accounts", json!([{ "username": "O", "uuid": "u", "token_kind": "offline", "microsoft": {} }]));
        let v = serde_json::to_value(public_account(&e, &get_account(&e, "O").unwrap())).unwrap();
        assert!(v.get("microsoft").is_none());
        assert_eq!(v["avatar_color"], Value::Null);
    }

    #[test]
    fn create_account_validates_and_assigns_spaced_colors() {
        let (_d, e) = temp_engine();
        let a = create_account(&e, "AvatarAl").unwrap();
        let b = create_account(&e, "AvatarBo").unwrap();
        let (ca, cb) = (a.avatar_color.unwrap(), b.avatar_color.unwrap());
        assert!(is_valid_avatar_color(ca) && is_valid_avatar_color(cb));
        assert!(hue_distance(ca, cb) >= 30, "{ca}/{cb}");
        assert_eq!(e.config().active_username().as_deref(), Some("AvatarBo"));
        let pub_a = public_account(&e, &get_account(&e, "AvatarAl").unwrap());
        assert_eq!(pub_a.avatar_color, Some(ca));
        assert!(!pub_a.has_avatar);
        assert_eq!(a.uuid, offline_uuid("AvatarAl"));
        let env = |r: EngineResult<Account>| {
            let x = r.unwrap_err().envelope();
            (x.status, x.code)
        };
        assert_eq!(env(create_account(&e, "AvatarAl")), (409, "ALREADY_EXISTS".into()));
        assert_eq!(env(create_account(&e, "bad name")), (400, "INVALID_USERNAME".into()));
        // persisted
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(e.paths().config_json()).unwrap()).unwrap();
        assert_eq!(raw["accounts"].as_array().unwrap().len(), 2);
        assert_eq!(raw["active_username"], "AvatarBo");
    }

    #[test]
    fn set_active_and_delete() {
        let (_d, e) = temp_engine();
        create_account(&e, "One").unwrap();
        create_account(&e, "Two").unwrap();
        assert_eq!(set_active_account(&e, "One").unwrap(), "One");
        assert_eq!(set_active_account(&e, "Ghost").unwrap_err().envelope().status, 404);
        assert_eq!(get_active_account(&e).unwrap().username, "One");
        // offline accounts are not Microsoft accounts
        assert_eq!(delete_msa_account(&e, "One").unwrap_err().envelope().status, 404);
        assert_eq!(delete_account(&e, "One").unwrap().deleted, "One");
        assert!(e.config().active_username().is_none());
        assert_eq!(delete_account(&e, "One").unwrap_err().envelope().code, "NOT_FOUND");
    }

    #[test]
    fn msa_upsert_replaces_the_same_profile_and_keeps_color() {
        let (_d, e) = temp_engine();
        let first = upsert_msa_account(
            &e,
            &MsaLogin { username: "Old".into(), uuid: "p1".into(), refresh_token: Some("r1".into()), expires_in: None },
        );
        let second = upsert_msa_account(
            &e,
            &MsaLogin { username: "New".into(), uuid: "p1".into(), refresh_token: Some("r2".into()), expires_in: None },
        );
        assert_eq!(first.avatar_color, second.avatar_color);
        assert_eq!(first.created_at, second.created_at);
        let all = list_accounts(&e);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].username, "New");
        assert_eq!(all[0].microsoft.as_ref().unwrap().refresh_token.as_deref(), Some("r2"));
        assert_eq!(e.config().active_username().as_deref(), Some("New"));
        let lunar = upsert_lunar_account(&e, "Lun", "p2", "lr");
        assert_eq!(lunar.microsoft.as_ref().unwrap().lunar, Some(true));
        assert_eq!(public_account(&e, &lunar).microsoft, Some(PublicMicrosoft { lunar: true }));
        assert_eq!(delete_msa_account(&e, "Lun").unwrap().deleted, "Lun");
        assert_eq!(list_accounts(&e).len(), 1);
    }

    #[test]
    fn avatar_color_validates_clears_and_404s() {
        let (_d, e) = temp_engine();
        create_account(&e, "ColorUser").unwrap();
        assert_eq!(set_avatar_color(&e, "ColorUser", Some(42)).unwrap().avatar_color, Some(42));
        assert_eq!(public_account(&e, &get_account(&e, "ColorUser").unwrap()).avatar_color, Some(42));
        assert_eq!(set_avatar_color(&e, "ColorUser", None).unwrap().avatar_color, None);
        let err = set_avatar_color(&e, "ColorUser", Some(400)).unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_AVATAR_COLOR"));
        assert_eq!(set_avatar_color(&e, "NobodyHere", Some(10)).unwrap_err().envelope().status, 404);
    }

    #[test]
    fn avatar_roundtrip_flips_has_avatar() {
        let (_d, e) = temp_engine();
        let tiny = crate::util::lenient_base64_decode(TINY_PNG_B64);
        create_account(&e, "AvatarUser").unwrap();
        let has = |e: &Engine| public_account(e, &get_account(e, "AvatarUser").unwrap()).has_avatar;
        assert!(!has(&e));
        assert_eq!(read_account_avatar(&e, "AvatarUser").unwrap_err().envelope().status, 404);
        assert!(write_account_avatar(&e, "AvatarUser", &tiny).unwrap());
        assert!(has(&e));
        assert_eq!(read_account_avatar(&e, "AvatarUser").unwrap(), tiny);
        assert!(!remove_account_avatar(&e, "AvatarUser").unwrap());
        assert!(!has(&e));
        assert_eq!(read_account_avatar(&e, "NobodyHere").unwrap_err().envelope().status, 404);
        assert_eq!(write_account_avatar(&e, "NobodyHere", &tiny).unwrap_err().envelope().status, 404);
        // deleting the account drops the avatar file
        write_account_avatar(&e, "AvatarUser", &tiny).unwrap();
        let p = avatar_path_for(&e, "AvatarUser").unwrap();
        delete_account(&e, "AvatarUser").unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn avatar_auto_marker_lifecycle() {
        let (_d, e) = temp_engine();
        create_account(&e, "M").unwrap();
        assert_eq!(set_avatar_auto(&e, "M", true).unwrap().avatar_auto, Some(true));
        write_account_avatar(&e, "M", b"\x89PNG\r\n\x1a\n").unwrap();
        assert_eq!(get_account(&e, "M").unwrap().avatar_auto, None);
        set_avatar_auto(&e, "M", true).unwrap();
        remove_account_avatar(&e, "M").unwrap();
        assert_eq!(get_account(&e, "M").unwrap().avatar_auto, None);
        assert_eq!(set_avatar_auto(&e, "Ghost", true).unwrap_err().envelope().status, 404);
    }

    #[test]
    fn unknown_account_fields_survive_writes() {
        let (_d, e) = temp_engine();
        e.config().set("accounts", json!([{ "username": "K", "uuid": "u", "token_kind": "offline", "future": [1] }]));
        set_avatar_color(&e, "K", Some(3)).unwrap();
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(e.paths().config_json()).unwrap()).unwrap();
        assert_eq!(raw["accounts"][0]["future"], json!([1]));
        assert_eq!(raw["accounts"][0]["avatar_color"], 3);
    }
}
