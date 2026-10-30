//! Port of `src/engine/accounts.mjs` — accounts stored in `config.accounts`
//! (offline + Microsoft + Lunar-imported), active account, avatar colors and
//! images (`<data>/avatars/<uuid>.png`).
//!
//! **Owner: accounts-skins.** Status: PARTIAL — the read-side helpers other
//! clusters need are implemented here (list/get/active/public/offline uuid);
//! the write side is still to port.
//!
//! JS exports still to port: isValidAvatarColor pickAvatarColor createAccount
//! upsertMsaAccount upsertLunarAccount deleteMsaAccount setActiveAccount
//! deleteAccount setAvatarColor avatarPathFor readAccountAvatar
//! writeAccountAvatar removeAccountAvatar setAvatarAuto.
//! Tests: tests/accounts.test.mjs, personalization.test.mjs (account parts),
//! skin-avatar.test.mjs. Writes: ConfigStore::set_accounts / update (atomic RMW).
//!
//! CONTRACT (used by game-launch, identity-discord, accounts-skins): every
//! `pub fn` below.

use std::path::PathBuf;

use md5::{Digest, Md5};

use crate::engine::Engine;
use crate::types::{Account, PublicAccount, PublicMicrosoft, TokenKind};

/// `^[A-Za-z0-9_]{1,16}$`
pub fn is_valid_username(username: &str) -> bool {
    (1..=16).contains(&username.len()) && username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Java-compatible offline UUID: md5("OfflinePlayer:" + name), version 3, RFC 4122 variant.
pub fn offline_uuid(username: &str) -> String {
    let mut b: [u8; 16] = Md5::digest(format!("OfflinePlayer:{username}").as_bytes()).into();
    b[6] = (b[6] & 0x0f) | 0x30;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use serde_json::json;

    #[test]
    fn offline_uuid_is_java_compatible() {
        // Java: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        let u = offline_uuid("Steve");
        assert_eq!(&u[14..15], "3");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn usernames() {
        assert!(is_valid_username("Steve_123"));
        assert!(!is_valid_username(""));
        assert!(!is_valid_username("seventeen_chars__"));
        assert!(!is_valid_username("bad-name"));
    }

    #[test]
    fn public_account_strips_tokens() {
        let (_d, e) = temp_engine();
        e.config().set(
            "accounts",
            json!([{
                "username": "A", "uuid": "u-1", "token_kind": "msa",
                "microsoft": { "refresh_token": "SECRET", "lunar": true }, "access_token": "SECRET2"
            }]),
        );
        e.config().set("active_username", json!("A"));
        let a = get_active_account(&e).unwrap();
        let v = serde_json::to_string(&public_account(&e, &a)).unwrap();
        assert!(!v.contains("SECRET"), "{v}");
        assert!(v.contains(r#""microsoft":{"lunar":true}"#), "{v}");
        assert!(v.contains(r#""has_avatar":false"#));
    }
}
