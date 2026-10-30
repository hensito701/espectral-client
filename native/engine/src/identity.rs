//! Port of `src/engine/identity.mjs` — espectral-id client: uuid ownership
//! proof (Mojang join + service verify), signed grants (crate::grant),
//! flags, Discord link/unlink, `<data>/identity.json`.
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! State lives at `<dataDir>/identity.json`, keyed by the account's 32-hex uuid:
//!
//! ```text
//! { schema:1, accounts: { <uuid>: {
//!     token: <session token>, token_expires_at: <iso>,
//!     discord: { id, name, avatar } | null,
//!     flags: [..], grant: <jws>, grant_exp: <unix s> } } }
//! ```
//!
//! Entries are kept as raw JSON objects so unknown keys written by any engine
//! version survive. The service base URL is [`identity_url`]
//! (ESPECTRAL_ID_URL env > config `identity_url` > default). The Mojang
//! session server can be stubbed via ESPECTRAL_SESSION_SERVER (tests).
//!
//! NOTHING here may fail a launch — launch paths use [`refresh_grant_soft`].
//!
//! CONTRACT (called by game-launch, accounts-skins cosmetics, bridge):
//! normalize_uuid, identity_url, flags_for, refresh_grant_soft. Also public
//! for accounts-skins (cosmetics doc / account view): [`flags_info`],
//! [`token_for`], [`service_fetch`].

use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::api::identity_discord::DiscordUser;
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::grant;
use crate::types::{Account, IdentityAccount, IdentityDiscord, TokenKind};

pub use crate::config::DEFAULT_IDENTITY_URL;

const STATE_SCHEMA: u64 = 1;
/// Default Mojang session server.
pub const DEFAULT_SESSION_SERVER: &str = "https://sessionserver.mojang.com";

/// In-memory identity.json (JS module-level `cachedState`): the `accounts` map.
#[derive(Debug, Default)]
pub struct IdentityStore {
    accounts: Mutex<Option<Map<String, Value>>>,
}

/// Any uuid form (dashes ok) -> 32 lowercase hex, None when malformed.
pub fn normalize_uuid(uuid: &str) -> Option<String> {
    let flat: String = uuid.chars().filter(|c| *c != '-').collect::<String>().to_lowercase();
    (flat.len() == 32 && flat.chars().all(|c| c.is_ascii_hexdigit())).then_some(flat)
}

fn clean_url(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_string()
}

/// Effective service URL: (test override) > ESPECTRAL_ID_URL env > config > default, no trailing slash.
pub fn identity_url(engine: &Engine) -> String {
    if let Some(u) = engine.identity_discord_state().overrides().identity_url {
        return clean_url(&u);
    }
    let raw = std::env::var("ESPECTRAL_ID_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| engine.config().identity_url().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| DEFAULT_IDENTITY_URL.to_string());
    clean_url(&raw)
}

/// Mojang session server base (env override for stubbed tests).
pub fn session_server_url(engine: &Engine) -> String {
    if let Some(u) = engine.identity_discord_state().overrides().session_server {
        return clean_url(&u);
    }
    let raw = std::env::var("ESPECTRAL_SESSION_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SESSION_SERVER.to_string());
    clean_url(&raw)
}

/// `<data>/identity.json`.
pub fn identity_path(engine: &Engine) -> std::path::PathBuf {
    engine.paths().identity_json()
}

fn store(engine: &Engine) -> &IdentityStore {
    &engine.identity_discord_state().identity
}

/// Run `f` on the (lazily loaded) accounts map.
fn with_accounts<R>(engine: &Engine, f: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
    let mut g = store(engine).accounts.lock().unwrap_or_else(|p| p.into_inner());
    if g.is_none() {
        let raw: Value = std::fs::read_to_string(identity_path(engine))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let accounts = match raw.get("accounts") {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        };
        *g = Some(accounts);
    }
    f(g.as_mut().expect("loaded"))
}

fn persist(engine: &Engine, accounts: &Map<String, Value>) {
    let doc = json!({ "schema": STATE_SCHEMA, "accounts": accounts });
    if let Err(e) = crate::paths::write_json_atomic(&identity_path(engine), &doc) {
        tracing::warn!("[identity] could not persist identity.json: {e}");
    }
}

/// Snapshot of the whole document (`{ schema, accounts }`).
pub fn load_identity_state(engine: &Engine) -> Value {
    with_accounts(engine, |a| json!({ "schema": STATE_SCHEMA, "accounts": a }))
}

/// The stored identity entry for a uuid, or None.
pub fn get_entry(engine: &Engine, uuid: &str) -> Option<Map<String, Value>> {
    let flat = normalize_uuid(uuid)?;
    with_accounts(engine, |a| match a.get(&flat) {
        Some(Value::Object(m)) => Some(m.clone()),
        _ => None,
    })
}

/// Shallow-merge fields into the uuid's identity entry and persist (400 BAD_UUID).
pub fn update_entry(engine: &Engine, uuid: &str, patch: Map<String, Value>) -> EngineResult<Map<String, Value>> {
    let Some(flat) = normalize_uuid(uuid) else {
        return Err(EngineError::bad_request("BAD_UUID", "uuid must be a 32-char hex string"));
    };
    Ok(with_accounts(engine, |a| {
        let mut entry = match a.get(&flat) {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        };
        for (k, v) in patch {
            entry.insert(k, v);
        }
        a.insert(flat.clone(), Value::Object(entry.clone()));
        persist(engine, a);
        entry
    }))
}

/// JS `Date.parse` for the ISO strings the service returns; None = NaN.
fn parse_date_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.timestamp_millis())
}

/// Bearer token for service calls, or None when absent/expired.
pub fn token_for(engine: &Engine, uuid: &str) -> Option<String> {
    token_for_at(engine, uuid, crate::platform::now_ms())
}

/// [`token_for`] with an explicit "now" (ms epoch).
pub fn token_for_at(engine: &Engine, uuid: &str, now_ms: i64) -> Option<String> {
    let entry = get_entry(engine, uuid)?;
    let token = entry.get("token").and_then(Value::as_str).filter(|t| !t.is_empty())?.to_string();
    let exp = entry.get("token_expires_at").and_then(Value::as_str).and_then(parse_date_ms);
    match exp {
        Some(exp) if exp <= now_ms => None,
        _ => Some(token),
    }
}

/// identity.flagsFor result.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlagsInfo {
    pub flags: Vec<String>,
    /// The grant, only when it currently verifies for this uuid.
    pub grant: Option<String>,
    pub grant_valid: bool,
}

fn string_array(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect(),
        _ => vec![],
    }
}

/// The flags a uuid may act on: a currently-valid grant's flags win; when the
/// grant is absent/expired but a live session token remains, fall back to the
/// flags the service returned with the last grant. Never trust a stored
/// grant's flags once the grant itself is dead.
pub fn flags_info(engine: &Engine, uuid: &str) -> FlagsInfo {
    let Some(entry) = get_entry(engine, uuid) else { return FlagsInfo::default() };
    if let Some(g) = entry.get("grant").and_then(Value::as_str).filter(|g| !g.is_empty()) {
        let flat = normalize_uuid(uuid);
        if let Ok(p) = grant::verify_grant(g, flat.as_deref(), None) {
            return FlagsInfo { flags: p.flags, grant: Some(g.to_string()), grant_valid: true };
        }
    }
    if token_for(engine, uuid).is_some() {
        return FlagsInfo { flags: string_array(entry.get("flags")), grant: None, grant_valid: false };
    }
    FlagsInfo::default()
}

/// Verified flags for a uuid (grant-valid flags, else last-known while verified).
pub fn flags_for(engine: &Engine, flat_uuid: &str) -> EngineResult<Vec<String>> {
    Ok(flags_info(engine, flat_uuid).flags)
}

// ---------------------------------------------------------------------------
// Service HTTP client
// ---------------------------------------------------------------------------

/// HTTP verb for [`service_fetch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    fn reqwest(self) -> reqwest::Method {
        match self {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Delete => reqwest::Method::DELETE,
        }
    }
}

/// Request against espectral-id with a timeout. `token` adds the Bearer
/// header. Service error envelopes (`{error:{code,message}}`) map to an API
/// error with the service's status/code (5xx -> 502); transport failures map
/// to 502 IDENTITY_UNREACHABLE. Returns the parsed JSON (Null when none).
pub async fn service_fetch(
    engine: &Engine,
    method: Method,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
    timeout_ms: u64,
) -> EngineResult<Value> {
    let url = format!("{}{}", identity_url(engine), path);
    let mut req = engine
        .http()
        .client()
        .request(method.reqwest(), &url)
        .timeout(Duration::from_millis(timeout_ms))
        .header("Accept", "application/json");
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    if let Some(b) = &body {
        req = req.header("Content-Type", "application/json").body(b.to_string());
    }
    let res = req
        .send()
        .await
        .map_err(|e| EngineError::api(502, "IDENTITY_UNREACHABLE", format!("identity service unreachable: {e}")))?;
    let status = res.status().as_u16();
    let ok = (200..300).contains(&status);
    let text = match res.text().await {
        Ok(t) => t,
        Err(_) if !ok => String::new(),
        Err(e) => {
            return Err(EngineError::api(502, "IDENTITY_UNREACHABLE", format!("identity service unreachable: {e}")));
        }
    };
    let data: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !ok {
        let err = data.get("error");
        let code = err
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("HTTP_{status}"));
        let msg = err
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("identity service returned {status}"));
        let out_status = if (400..500).contains(&status) { status } else { 502 };
        return Err(EngineError::api(out_status, code, msg));
    }
    Ok(data)
}

/// Normalize a service-side discord identity (`{id, username, avatar}`) into
/// the engine shape (`{id, name, avatar-url}`). `avatar` may be a full URL or
/// a bare Discord avatar hash; no avatar -> the default embed avatar.
pub fn normalize_discord_identity(d: &Value) -> Option<DiscordUser> {
    let obj = d.as_object()?;
    let id = match obj.get("id") {
        None | Some(Value::Null) => return None,
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
    };
    let mut avatar = obj.get("avatar").and_then(Value::as_str).filter(|a| !a.is_empty()).map(str::to_string);
    if let Some(a) = &avatar {
        if !(a.starts_with("http://") || a.starts_with("https://")) {
            avatar = Some(format!("https://cdn.discordapp.com/avatars/{id}/{a}.png?size=128"));
        }
    }
    let avatar = avatar.unwrap_or_else(|| {
        let idx = id.trim().parse::<u64>().map(|n| (n >> 22) % 6).unwrap_or(0);
        format!("https://cdn.discordapp.com/embed/avatars/{idx}.png")
    });
    let str_of = |k: &str| match obj.get(k) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(v) => Some(v.to_string()),
    };
    let name = str_of("username").or_else(|| str_of("global_name")).unwrap_or_else(|| format!("User {id}"));
    Some(DiscordUser { id, name, avatar: Some(avatar), extra: Map::new() })
}

fn discord_user_value(u: &DiscordUser) -> Value {
    json!({ "id": u.id, "name": u.name, "avatar": u.avatar })
}

/// GET /v1/grant result.
#[derive(Debug, Clone, PartialEq)]
pub struct GrantRefresh {
    pub grant: Option<String>,
    pub flags: Vec<String>,
    pub exp: Option<f64>,
}

/// Fetch a fresh grant for a verified uuid (GET /v1/grant); stores grant +
/// flags. 401 NOT_VERIFIED without a live session.
pub async fn refresh_grant(engine: &Engine, uuid: &str, timeout_ms: u64) -> EngineResult<GrantRefresh> {
    let Some(token) = token_for(engine, uuid) else {
        return Err(EngineError::api(401, "NOT_VERIFIED", "uuid has no live identity session"));
    };
    let data = service_fetch(engine, Method::Get, "/v1/grant", None, Some(&token), timeout_ms).await?;
    let grant = data.get("grant").and_then(Value::as_str).map(str::to_string);
    let flags = string_array(data.get("flags"));
    let exp = data.get("exp").filter(|v| v.is_number()).cloned();
    let mut patch = Map::new();
    patch.insert("grant".into(), grant.clone().map(Value::String).unwrap_or(Value::Null));
    patch.insert("flags".into(), json!(flags));
    patch.insert("grant_exp".into(), exp.clone().unwrap_or(Value::Null));
    update_entry(engine, uuid, patch)?;
    Ok(GrantRefresh { grant, flags, exp: exp.and_then(|v| v.as_f64()) })
}

/// Grant refresh for launch/put paths: bounded by `budget_ms` (min 250 ms);
/// Ok(true) when a fresh grant was stored, Ok(false) on any failure — never
/// fails or delays a launch beyond its small budget.
pub async fn refresh_grant_soft(engine: &Engine, flat_uuid: &str, budget_ms: u64) -> EngineResult<bool> {
    Ok(refresh_grant(engine, flat_uuid, budget_ms.max(250)).await.is_ok())
}

/// Prove uuid ownership: Mojang join with the account's live MC token, then
/// the service challenge/verify handshake. Returns the stored entry.
pub async fn verify_account(engine: &Engine, account: &Account) -> EngineResult<Map<String, Value>> {
    if normalize_uuid(&account.uuid).is_none() {
        return Err(EngineError::bad_request("BAD_UUID", "account uuid is missing or malformed"));
    }
    if account.token_kind != TokenKind::Msa {
        return Err(EngineError::bad_request("ACCOUNT_NOT_MSA", "only Microsoft accounts can prove uuid ownership"));
    }
    let mc = crate::msauth::ensure_minecraft_token(engine, account).await?;
    let Some(mc) = mc.filter(|t| !t.access_token.is_empty()) else {
        return Err(EngineError::api(
            401,
            "MSA_REFRESH_FAILED",
            "could not obtain a Minecraft session token — sign in again",
        ));
    };
    verify_account_with_token(engine, account, &mc.access_token).await
}

/// [`verify_account`] after the MC token was obtained (split out so the
/// handshake is testable without Microsoft).
pub async fn verify_account_with_token(
    engine: &Engine,
    account: &Account,
    mc_access_token: &str,
) -> EngineResult<Map<String, Value>> {
    let Some(flat) = normalize_uuid(&account.uuid) else {
        return Err(EngineError::bad_request("BAD_UUID", "account uuid is missing or malformed"));
    };
    // 1. challenge
    let challenge = service_fetch(engine, Method::Post, "/v1/auth/challenge", Some(json!({})), None, 5000).await?;
    let challenge_id = challenge.get("challenge_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let server_id = challenge.get("server_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let (Some(challenge_id), Some(server_id)) = (challenge_id, server_id) else {
        return Err(EngineError::api(502, "BAD_CHALLENGE", "identity service returned a malformed challenge"));
    };

    // 2. Mojang join (proof the launcher holds the session for `username`)
    let join = engine
        .http()
        .client()
        .post(format!("{}/session/minecraft/join", session_server_url(engine)))
        .timeout(Duration::from_millis(8000))
        .header("Content-Type", "application/json")
        .body(json!({ "accessToken": mc_access_token, "selectedProfile": flat, "serverId": server_id }).to_string())
        .send()
        .await
        .map_err(|e| EngineError::api(502, "MOJANG_UNREACHABLE", format!("Mojang session server unreachable: {e}")))?;
    let js = join.status().as_u16();
    if js != 204 && !(200..300).contains(&js) {
        return Err(EngineError::api(
            401,
            "MOJANG_JOIN_REJECTED",
            format!("Mojang rejected the join (HTTP {js}) — session may be stale"),
        ));
    }

    // 3. verify -> session token
    let verified = service_fetch(
        engine,
        Method::Post,
        "/v1/auth/verify",
        Some(json!({ "challenge_id": challenge_id, "username": account.username })),
        None,
        5000,
    )
    .await?;
    let Some(token) = verified.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
        return Err(EngineError::api(502, "BAD_VERIFY", "identity service did not return a session token"));
    };
    if let Some(vu) = verified.get("uuid").and_then(Value::as_str).and_then(normalize_uuid) {
        if vu != flat {
            return Err(EngineError::conflict(
                "UUID_MISMATCH",
                "identity service verified a different uuid than the account",
            ));
        }
    }
    let mut patch = Map::new();
    patch.insert("token".into(), json!(token));
    patch.insert(
        "token_expires_at".into(),
        verified.get("expires_at").filter(|v| v.is_string()).cloned().unwrap_or(Value::Null),
    );
    let entry = update_entry(engine, &flat, patch)?;
    // Grant straight away so the account starts out with live flags.
    if let Err(e) = refresh_grant(engine, &flat, 5000).await {
        let code = e.raw_code().map(str::to_string).unwrap_or_else(|| e.to_string());
        tracing::warn!("[identity] initial grant fetch for {}… failed: {code}", &flat[..8]);
    }
    Ok(entry)
}

/// POST /v1/discord/identify — plain launcher Discord login (no link).
pub async fn discord_identify(engine: &Engine, code: &str, redirect_uri: &str) -> EngineResult<DiscordUser> {
    let data = service_fetch(
        engine,
        Method::Post,
        "/v1/discord/identify",
        Some(json!({ "code": code, "redirect_uri": redirect_uri })),
        None,
        5000,
    )
    .await?;
    normalize_discord_identity(data.get("discord").unwrap_or(&Value::Null)).ok_or_else(|| {
        EngineError::api(502, "DISCORD_IDENTIFY_FAILED", "identity service did not return a discord identity")
    })
}

/// POST /v1/discord/link — bind a discord identity to a verified uuid.
pub async fn discord_link(engine: &Engine, uuid: &str, code: &str, redirect_uri: &str) -> EngineResult<DiscordUser> {
    let Some(token) = token_for(engine, uuid) else {
        return Err(EngineError::api(401, "NOT_VERIFIED", "uuid has no live identity session"));
    };
    let data = service_fetch(
        engine,
        Method::Post,
        "/v1/discord/link",
        Some(json!({ "code": code, "redirect_uri": redirect_uri })),
        Some(&token),
        5000,
    )
    .await?;
    let user = normalize_discord_identity(data.get("discord").unwrap_or(&Value::Null)).ok_or_else(|| {
        EngineError::api(502, "DISCORD_LINK_FAILED", "identity service did not return a discord identity")
    })?;
    let mut patch = Map::new();
    patch.insert("discord".into(), discord_user_value(&user));
    if matches!(data.get("flags"), Some(Value::Array(_))) {
        patch.insert("flags".into(), json!(string_array(data.get("flags"))));
    }
    update_entry(engine, uuid, patch)?;
    let _ = refresh_grant(engine, uuid, 5000).await;
    Ok(user)
}

/// Store a discord identity on a uuid (mock-mode link path).
pub(crate) fn store_discord(engine: &Engine, uuid: &str, user: &DiscordUser) -> EngineResult<()> {
    let mut patch = Map::new();
    patch.insert("discord".into(), discord_user_value(user));
    update_entry(engine, uuid, patch).map(|_| ())
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Public account view for GET /api/identity.
pub fn account_identity_entry(engine: &Engine, account: &Account) -> IdentityAccount {
    let flat = normalize_uuid(&account.uuid);
    let entry = flat.as_deref().and_then(|f| get_entry(engine, f));
    let token = flat.as_deref().and_then(|f| token_for(engine, f));
    let mut grant_exp = entry
        .as_ref()
        .and_then(|e| e.get("grant_exp"))
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite())
        .map(|f| f as i64);
    let flags = flat.as_deref().map(|f| flags_info(engine, f)).unwrap_or_default();
    if flags.grant_valid {
        if let Some(g) = &flags.grant {
            if let Ok(p) = grant::verify_grant(g, flat.as_deref(), None) {
                grant_exp = Some(p.exp as i64);
            }
        }
    }
    let discord = entry.as_ref().and_then(|e| e.get("discord")).and_then(Value::as_object).map(|d| IdentityDiscord {
        id: d.get("id").map(value_to_string).unwrap_or_default(),
        username: d
            .get("name")
            .filter(|v| !v.is_null())
            .or_else(|| d.get("username").filter(|v| !v.is_null()))
            .map(value_to_string)
            .unwrap_or_default(),
        avatar: d.get("avatar").and_then(Value::as_str).map(str::to_string),
    });
    IdentityAccount {
        uuid: flat.unwrap_or_else(|| account.uuid.clone()),
        username: account.username.clone(),
        verified: token.is_some(),
        discord,
        flags: flags.flags,
        grant_exp,
    }
}

/// DELETE /v1/discord/link — drop the discord binding for a uuid.
pub async fn discord_unlink(engine: &Engine, uuid: &str) -> EngineResult<()> {
    let Some(token) = token_for(engine, uuid) else {
        return Err(EngineError::api(401, "NOT_VERIFIED", "uuid has no live identity session"));
    };
    service_fetch(engine, Method::Delete, "/v1/discord/link", None, Some(&token), 5000).await?;
    let mut patch = Map::new();
    patch.insert("discord".into(), Value::Null);
    update_entry(engine, uuid, patch)?;
    let _ = refresh_grant(engine, uuid, 5000).await;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockResponse, MockServer};

    pub(crate) const UUID: &str = "aabbccddeeff00112233445566778899";

    #[test]
    fn normalize_uuid_parity() {
        assert_eq!(
            normalize_uuid("B50AD385-829D-3141-A216-7E7D7539BA7F").as_deref(),
            Some("b50ad385829d3141a2167e7d7539ba7f")
        );
        assert_eq!(
            normalize_uuid("b50ad385829d3141a2167e7d7539ba7f").as_deref(),
            Some("b50ad385829d3141a2167e7d7539ba7f")
        );
        assert_eq!(normalize_uuid("xyz"), None);
        assert_eq!(normalize_uuid(""), None);
    }

    #[test]
    fn identity_url_defaults_and_trims() {
        let (_d, e) = temp_engine();
        if std::env::var("ESPECTRAL_ID_URL").is_err() {
            assert_eq!(identity_url(&e), "https://id.uhce.top");
            e.config().set("identity_url", serde_json::json!("https://id.example.test/v1/"));
            assert_eq!(identity_url(&e), "https://id.example.test/v1");
        }
        e.identity_discord_state().set_overrides(|o| o.identity_url = Some("http://127.0.0.1:9/".into()));
        assert_eq!(identity_url(&e), "http://127.0.0.1:9");
    }

    /// Point identity + Mojang at a mock server.
    pub(crate) fn point_at(e: &Engine, server: &MockServer) {
        let url = server.url("");
        e.identity_discord_state().set_overrides(|o| {
            o.identity_url = Some(url.clone());
            o.session_server = Some(url.clone());
        });
    }

    pub(crate) fn seed_token(e: &Engine, uuid: &str, token: &str) {
        let exp = chrono::Utc::now() + chrono::Duration::hours(1);
        let mut patch = Map::new();
        patch.insert("token".into(), json!(token));
        patch.insert("token_expires_at".into(), json!(exp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
        update_entry(e, uuid, patch).unwrap();
    }

    fn msa_account() -> Account {
        Account { username: "TestMsa".into(), uuid: UUID.into(), token_kind: TokenKind::Msa, ..Default::default() }
    }

    fn service_stub() -> impl Fn(&crate::test_support::MockRequest) -> MockResponse + Send + Sync + 'static {
        let grant = crate::grant::tests::sign_test_grant(UUID, &["supporter", "mvp"], 86400);
        move |r| {
            let body = r.body_json();
            match (r.method.as_str(), r.path.as_str()) {
                ("POST", "/v1/auth/challenge") => MockResponse::json(
                    &json!({ "challenge_id": "ch-1", "server_id": "a".repeat(40), "expires_in": 120 }),
                ),
                ("POST", "/session/minecraft/join") => {
                    let b = body.unwrap_or_default();
                    if b["accessToken"] == "fake-mc-token" && b["selectedProfile"] == UUID {
                        MockResponse::status(204).with_status(204)
                    } else {
                        MockResponse::json(&json!({ "error": "Forbidden" })).with_status(403)
                    }
                }
                ("POST", "/v1/auth/verify") => MockResponse::json(&json!({
                    "token": "sess-token-1", "uuid": UUID, "name": "TestMsa",
                    "expires_at": (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
                })),
                ("GET", "/v1/grant") => {
                    if r.header("authorization").as_deref() == Some("Bearer sess-token-1") {
                        MockResponse::json(
                            &json!({ "grant": grant, "flags": ["supporter", "mvp"], "exp": grant::now_s() + 86400 }),
                        )
                    } else {
                        MockResponse::json(&json!({ "error": { "code": "UNAUTHORIZED", "message": "bad token" } }))
                            .with_status(401)
                    }
                }
                _ => {
                    MockResponse::json(&json!({ "error": { "code": "NOT_FOUND", "message": "stub" } })).with_status(404)
                }
            }
        }
    }

    #[tokio::test]
    async fn unverified_account_view() {
        let (_d, e) = temp_engine();
        let v = account_identity_entry(&e, &msa_account());
        assert_eq!(v.uuid, UUID);
        assert_eq!(v.username, "TestMsa");
        assert!(!v.verified);
        assert_eq!(v.discord, None);
        assert!(v.flags.is_empty());
        assert_eq!(v.grant_exp, None);
    }

    #[tokio::test]
    async fn verify_runs_challenge_join_verify_grant_and_persists() {
        let (d, e) = temp_engine();
        let server = MockServer::start(service_stub()).await;
        point_at(&e, &server);
        verify_account_with_token(&e, &msa_account(), "fake-mc-token").await.unwrap();
        let v = account_identity_entry(&e, &msa_account());
        assert!(v.verified);
        assert_eq!(v.flags, vec!["supporter", "mvp"]);
        assert!(v.grant_exp.unwrap() > grant::now_s() + 86000);

        let calls: Vec<String> = server.requests().iter().map(|r| format!("{} {}", r.method, r.path)).collect();
        for want in ["POST /v1/auth/challenge", "POST /session/minecraft/join", "POST /v1/auth/verify", "GET /v1/grant"]
        {
            assert!(calls.contains(&want.to_string()), "{want} in {calls:?}");
        }
        let doc: Value =
            serde_json::from_str(&std::fs::read_to_string(d.path().join("identity.json")).unwrap()).unwrap();
        assert_eq!(doc["schema"], 1);
        assert_eq!(doc["accounts"][UUID]["token"], "sess-token-1");
        assert!(doc["accounts"][UUID]["grant"].is_string());

        // A fresh engine on the same dir reads it back (on-disk format).
        let e2 = crate::Engine::new(crate::test_support::test_options(d.path().to_path_buf())).unwrap();
        assert_eq!(token_for(&e2, UUID).as_deref(), Some("sess-token-1"));
    }

    #[tokio::test]
    async fn verify_with_bad_mc_token_is_join_rejected() {
        let (_d, e) = temp_engine();
        let server = MockServer::start(service_stub()).await;
        point_at(&e, &server);
        let err = verify_account_with_token(&e, &msa_account(), "wrong").await.unwrap_err();
        assert!(err.has_code("MOJANG_JOIN_REJECTED"));
        assert_eq!(err.status(), 401);
    }

    #[tokio::test]
    async fn offline_accounts_cannot_verify() {
        let (_d, e) = temp_engine();
        let acc = Account { username: "Off".into(), uuid: UUID.into(), ..Default::default() };
        assert!(verify_account(&e, &acc).await.unwrap_err().has_code("ACCOUNT_NOT_MSA"));
        let bad = Account { uuid: "nope".into(), token_kind: TokenKind::Msa, ..Default::default() };
        assert!(verify_account(&e, &bad).await.unwrap_err().has_code("BAD_UUID"));
    }

    #[tokio::test]
    async fn service_errors_map_status_and_code() {
        let (_d, e) = temp_engine();
        let server = MockServer::start(|r| match r.path.as_str() {
            "/four" => MockResponse::json(&json!({ "error": { "code": "NOPE", "message": "no" } })).with_status(403),
            "/five" => MockResponse::status(503),
            _ => MockResponse::status(204),
        })
        .await;
        point_at(&e, &server);
        let err = service_fetch(&e, Method::Get, "/four", None, None, 2000).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (403, Some("NOPE")));
        let err = service_fetch(&e, Method::Get, "/five", None, None, 2000).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (502, Some("HTTP_503")));
        assert_eq!(service_fetch(&e, Method::Get, "/ok", None, None, 2000).await.unwrap(), Value::Null);
        // Transport failure.
        e.identity_discord_state().set_overrides(|o| o.identity_url = Some("http://127.0.0.1:1".into()));
        let err = service_fetch(&e, Method::Get, "/x", None, None, 2000).await.unwrap_err();
        assert_eq!((err.status(), err.raw_code()), (502, Some("IDENTITY_UNREACHABLE")));
        // Soft refresh never fails.
        seed_token(&e, UUID, "t");
        assert!(!refresh_grant_soft(&e, UUID, 10).await.unwrap());
    }

    #[test]
    fn token_expiry_and_flags_fallback() {
        let (_d, e) = temp_engine();
        let mut patch = Map::new();
        patch.insert("token".into(), json!("t"));
        patch.insert("token_expires_at".into(), json!("2000-01-01T00:00:00.000Z"));
        patch.insert("flags".into(), json!(["supporter", 3]));
        update_entry(&e, UUID, patch).unwrap();
        assert_eq!(token_for(&e, UUID), None);
        assert_eq!(flags_info(&e, UUID), FlagsInfo::default());
        seed_token(&e, UUID, "t2");
        let f = flags_info(&e, UUID);
        assert_eq!(f.flags, vec!["supporter"]);
        assert!(!f.grant_valid);
        // A valid grant wins.
        let g = crate::grant::tests::sign_test_grant(UUID, &["mvp"], 600);
        let mut patch = Map::new();
        patch.insert("grant".into(), json!(g));
        update_entry(&e, UUID, patch).unwrap();
        let f = flags_info(&e, UUID);
        assert_eq!(f.flags, vec!["mvp"]);
        assert!(f.grant_valid);
        // A grant for another uuid never unlocks this one.
        let other = "00000000000000000000000000000001";
        seed_token(&e, other, "t3");
        let mut patch = Map::new();
        patch.insert("grant".into(), json!(g));
        update_entry(&e, other, patch).unwrap();
        assert!(!flags_info(&e, other).grant_valid);
        assert!(update_entry(&e, "bad", Map::new()).unwrap_err().has_code("BAD_UUID"));
    }

    #[test]
    fn discord_identity_normalization() {
        let u = normalize_discord_identity(&json!({ "id": "9999", "username": "A", "avatar": null })).unwrap();
        assert_eq!(u.name, "A");
        assert_eq!(u.avatar.as_deref(), Some("https://cdn.discordapp.com/embed/avatars/0.png"));
        let u = normalize_discord_identity(&json!({ "id": "718293847561829304", "avatar": "abc" })).unwrap();
        assert_eq!(u.name, "User 718293847561829304");
        assert_eq!(u.avatar.as_deref(), Some("https://cdn.discordapp.com/avatars/718293847561829304/abc.png?size=128"));
        let u = normalize_discord_identity(&json!({ "id": "718293847561829304", "global_name": "G" })).unwrap();
        assert_eq!(u.name, "G");
        let idx = (718293847561829304u64 >> 22) % 6;
        assert_eq!(u.avatar, Some(format!("https://cdn.discordapp.com/embed/avatars/{idx}.png")));
        assert!(normalize_discord_identity(&json!({ "username": "x" })).is_none());
        assert!(normalize_discord_identity(&Value::Null).is_none());
    }
}
