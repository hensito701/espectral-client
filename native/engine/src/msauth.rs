//! Port of `src/engine/msauth.mjs` — Microsoft device-code login, XBL/XSTS
//! -> Minecraft token exchange, Lunar refresh-token path, token refresh +
//! `<data>/mc-token-cache.json`, profile skin get/upload/reset.
//!
//! **Owner: accounts-skins.** Status: PORTED.
//!
//! Device-code flow: POST /devicecode -> POST /token (device_code grant,
//! polled by the UI through `Engine::msa_poll`) -> XBL user/authenticate
//! (RPS ticket) -> XSTS authorize (RETAIL) -> Minecraft login_with_xbox ->
//! minecraft/profile.
//!
//! Token model:
//! - the MSA refresh_token is persisted per account in config.json;
//! - Lunar-imported accounts carry `microsoft.lunar: true`; their refresh
//!   token is bound to the legacy launcher client id + login.live.com;
//! - the Minecraft access token is NEVER written to config.json: it lives in
//!   memory ([`crate::api::accounts_skins::AccountsSkinsState`], keyed by
//!   username — the JS kept it on the live config object) and in
//!   `<data>/mc-token-cache.json` (`{ <uuid>: { accessToken, expiresAt } }`,
//!   dashed and dashless keys) so a launch survives a transient MSA outage.
//!
//! Error codes the device-flow poll keys on: MSA_PENDING MSA_SLOW_DOWN
//! MSA_TRANSIENT MSA_NETWORK — surfaced here as [`DevicePoll`] variants
//! (MSA_NETWORK stays an error with that code).
//!
//! Endpoints are injectable ([`Endpoints`], stored in the cluster state) so
//! tests point the whole chain at a loopback mock; production uses the real
//! Microsoft / Xbox / Mojang hosts.
//!
//! Tests: tests/msauth.test.mjs, tests/skin.test.mjs.
//!
//! CONTRACT (called by game-launch resolveAccountFor and identity verify):
//! ensure_minecraft_token, prewarm_msa_token.

use std::sync::Arc;
use std::time::Duration;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::platform::now_ms;
use crate::types::{Account, MsDeviceFlow, SkinVariant, TokenKind};

/// Espectral Client's Azure app registration ("Personal Microsoft accounts",
/// public client flows enabled).
pub const DEFAULT_CLIENT_ID: &str = "071ffa4b-eafb-4b7a-aa71-d8beef4f4f2e";
/// Legacy Minecraft launcher client id (Lunar-imported refresh tokens).
pub const LUNAR_CLIENT_ID: &str = "00000000402b5328";
pub const LUNAR_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
pub const MSA_SCOPE: &str = "XboxLive.signin offline_access";

/// Time a Minecraft access token is considered valid before refresh (safety margin).
pub const TOKEN_SKEW_MS: i64 = 5 * 60 * 1000;
/// Window before expiry that triggers a background refresh at engine start.
pub const PREWARM_WINDOW_MS: i64 = 10 * 60 * 1000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const SKIN_TIMEOUT: Duration = Duration::from_secs(30);

const RETRYABLE_STATUS: [u16; 5] = [429, 500, 502, 503, 504];
const RETRY_FALLBACK_DELAYS_MS: [u64; 2] = [1000, 3000];
/// Ignore absurd Retry-After values rather than hanging a launch on them.
const MAX_RETRY_AFTER_MS: f64 = 4000.0;

/// Every remote endpoint msauth talks to (injectable for tests).
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoints {
    /// `https://login.microsoftonline.com/consumers/oauth2/v2.0` (`/devicecode`, `/token`).
    pub auth_base: String,
    pub xbl_auth: String,
    pub xsts_auth: String,
    pub mc_login: String,
    /// `…/minecraft/profile` (skins live under `/skins`, `/skins/active`).
    pub mc_profile: String,
    pub lunar_refresh: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            auth_base: "https://login.microsoftonline.com/consumers/oauth2/v2.0".into(),
            xbl_auth: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts_auth: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            mc_login: "https://api.minecraftservices.com/authentication/login_with_xbox".into(),
            mc_profile: "https://api.minecraftservices.com/minecraft/profile".into(),
            lunar_refresh: "https://login.live.com/oauth20_token.srf".into(),
        }
    }
}

impl Endpoints {
    /// Every endpoint on one base URL (tests: a loopback MockServer), keeping the real paths.
    pub fn on_base(base: &str) -> Self {
        let b = base.trim_end_matches('/');
        Endpoints {
            auth_base: format!("{b}/consumers/oauth2/v2.0"),
            xbl_auth: format!("{b}/user/authenticate"),
            xsts_auth: format!("{b}/xsts/authorize"),
            mc_login: format!("{b}/authentication/login_with_xbox"),
            mc_profile: format!("{b}/minecraft/profile"),
            lunar_refresh: format!("{b}/oauth20_token.srf"),
        }
    }
}

/// A usable Minecraft access token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McToken {
    pub access_token: String,
    /// ms epoch.
    pub expires_at: i64,
}

fn endpoints(engine: &Engine) -> Endpoints {
    engine.accounts_skins_state().endpoints.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Replace the remote endpoints (tests / self-hosted proxies).
#[doc(hidden)]
pub fn set_endpoints(engine: &Engine, eps: Endpoints) {
    *engine.accounts_skins_state().endpoints.write().unwrap_or_else(|p| p.into_inner()) = eps;
}

/// Client ID resolution: config `msa_client_id` (non-empty string), then
/// `ESPECTRAL_MSA_CLIENT_ID`, then [`DEFAULT_CLIENT_ID`].
pub fn client_id(engine: &Engine) -> String {
    if let Some(id) = engine.config().get_str("msa_client_id").filter(|s| !s.is_empty()) {
        return id;
    }
    std::env::var("ESPECTRAL_MSA_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_string())
}

// ---------------------------------------------------------------------------
// mc-token-cache.json
// ---------------------------------------------------------------------------

fn read_mc_token_cache(engine: &Engine) -> Map<String, Value> {
    std::fs::read_to_string(engine.paths().mc_token_cache())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

fn write_mc_token_cache(engine: &Engine, cache: &Map<String, Value>) {
    let p = engine.paths().mc_token_cache();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(p, text); // best-effort
    }
}

/// Disk-cached token for a uuid (dashed or dashless key), regardless of expiry.
pub fn cached_mc_token(engine: &Engine, uuid: &str) -> Option<McToken> {
    if uuid.is_empty() {
        return None;
    }
    let cache = read_mc_token_cache(engine);
    let entry = cache.get(uuid).or_else(|| cache.get(&uuid.replace('-', "")))?;
    let token = entry.get("accessToken").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let exp = entry.get("expiresAt").and_then(Value::as_f64).filter(|e| *e != 0.0)?;
    Some(McToken { access_token: token.to_string(), expires_at: exp as i64 })
}

fn set_cached_mc_token(engine: &Engine, uuid: &str, token: &McToken) {
    if uuid.is_empty() || token.access_token.is_empty() || token.expires_at == 0 {
        return;
    }
    let mut cache = read_mc_token_cache(engine);
    let entry = json!({ "accessToken": token.access_token, "expiresAt": token.expires_at });
    cache.insert(uuid.to_string(), entry.clone());
    cache.insert(uuid.replace('-', ""), entry);
    write_mc_token_cache(engine, &cache);
}

fn memory_token(engine: &Engine, username: &str) -> Option<McToken> {
    engine.accounts_skins_state().mc_tokens.lock().unwrap_or_else(|p| p.into_inner()).get(username).cloned()
}

fn set_memory_token(engine: &Engine, username: &str, token: &McToken) {
    engine
        .accounts_skins_state()
        .mc_tokens
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(username.to_string(), token.clone());
}

/// Forget the in-memory token of an account (logout).
pub fn forget_memory_token(engine: &Engine, username: &str) {
    engine.accounts_skins_state().mc_tokens.lock().unwrap_or_else(|p| p.into_inner()).remove(username);
}

/// Remember a fresh Minecraft token in memory (e.g. right after a device login).
pub fn remember_token(engine: &Engine, username: &str, token: &McToken) {
    set_memory_token(engine, username, token);
}

fn is_fresh(t: &McToken) -> bool {
    now_ms() < t.expires_at - TOKEN_SKEW_MS
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

/// A parsed auth-API response (`{ ok, status, data, retryAfter }`).
#[derive(Debug, Clone)]
pub struct AuthResponse {
    pub ok: bool,
    pub status: u16,
    /// JSON body (non-JSON / empty -> `{}`).
    pub data: Value,
    /// Retry-After in seconds.
    pub retry_after: Option<f64>,
}

enum Body {
    None,
    Form(Vec<(&'static str, String)>),
    Json(Value),
}

/// RFC 7231 Retry-After (delta-seconds or HTTP-date) -> seconds.
pub fn retry_after_seconds(raw: Option<&str>) -> Option<f64> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(secs) = trimmed.parse::<f64>() {
        return (secs.is_finite() && secs >= 0.0).then_some(secs);
    }
    let at = chrono::DateTime::parse_from_rfc2822(trimmed).ok()?;
    let ms = at.timestamp_millis() - now_ms();
    Some(((ms as f64) / 1000.0).ceil().max(0.0))
}

fn form_encode(pairs: &[(&'static str, String)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

async fn fetch_json(
    engine: &Engine,
    method: reqwest::Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> EngineResult<AuthResponse> {
    let mut req = engine.http().client().request(method, url).timeout(FETCH_TIMEOUT);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req = match body {
        Body::None => req,
        Body::Form(pairs) => req.header("Content-Type", "application/x-www-form-urlencoded").body(form_encode(&pairs)),
        Body::Json(v) => {
            req.header("Content-Type", "application/json").header("Accept", "application/json").body(v.to_string())
        }
    };
    let net = |e: reqwest::Error| {
        let message = if e.is_timeout() { "request timed out".to_string() } else { e.to_string() };
        EngineError::api(502, "MSA_NETWORK", format!("Microsoft request failed: {message}"))
    };
    let res = req.send().await.map_err(net)?;
    let status = res.status().as_u16();
    let retry_after = retry_after_seconds(res.headers().get("retry-after").and_then(|v| v.to_str().ok()));
    let text = res.text().await.map_err(net)?;
    let data =
        if text.is_empty() { json!({}) } else { serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({})) };
    Ok(AuthResponse { ok: (200..300).contains(&status), status, data, retry_after })
}

async fn sleep_ms(engine: &Engine, ms: u64) {
    let fake = {
        let mut g = engine.accounts_skins_state().fake_sleeps.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(log) = g.as_mut() {
            log.push(ms);
            true
        } else {
            false
        }
    };
    if !fake {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

/// Rate-limit / transient retry for the Minecraft auth endpoint: only
/// 429/5xx, at most 3 attempts, Retry-After honoured when <= 4 s, else 1 s then 3 s.
async fn fetch_json_retry(engine: &Engine, url: &str, body: Value) -> EngineResult<AuthResponse> {
    let mut attempt = 0;
    loop {
        let last = fetch_json(engine, reqwest::Method::POST, url, &[], Body::Json(body.clone())).await?;
        if !RETRYABLE_STATUS.contains(&last.status) || attempt == RETRY_FALLBACK_DELAYS_MS.len() {
            return Ok(last);
        }
        let hinted = last.retry_after.map(|s| s * 1000.0);
        let wait = match hinted {
            Some(h) if (0.0..=MAX_RETRY_AFTER_MS).contains(&h) => h as u64,
            _ => RETRY_FALLBACK_DELAYS_MS[attempt],
        };
        sleep_ms(engine, wait).await;
        attempt += 1;
    }
}

fn str_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Short, token-free diagnostic of a failed auth-API body: errorMessage, error,
/// detail, path or raw (first non-null), max 140 chars, prefixed with ": ".
pub fn body_snippet(data: &Value) -> String {
    let Some(obj) = data.as_object() else { return String::new() };
    let interesting =
        ["errorMessage", "error", "detail", "path", "raw"].iter().find_map(|k| obj.get(*k).filter(|v| !v.is_null()));
    match interesting {
        Some(v) if crate::config::json_truthy(Some(v)) => {
            format!(": {}", str_of(v).chars().take(140).collect::<String>())
        }
        _ => String::new(),
    }
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Device-code flow
// ---------------------------------------------------------------------------

/// Start a device-code login: the user-facing code + URL; `flow_id` is the device code.
pub async fn start_device_login(engine: &Engine) -> EngineResult<MsDeviceFlow> {
    let eps = endpoints(engine);
    let cid = client_id(engine);
    let r = fetch_json(
        engine,
        reqwest::Method::POST,
        &format!("{}/devicecode", eps.auth_base),
        &[],
        Body::Form(vec![("client_id", cid.clone()), ("scope", MSA_SCOPE.into())]),
    )
    .await?;
    let Some(device_code) = s(&r.data, "device_code").filter(|_| r.ok) else {
        let error = s(&r.data, "error");
        let desc = s(&r.data, "error_description");
        let is_app_not_found =
            error == Some("unauthorized_client") && desc.map(|d| d.contains("700016")).unwrap_or(false);
        let message = if cid == DEFAULT_CLIENT_ID && is_app_not_found {
            "Microsoft login is not configured yet. Create an Azure app (see the docs) and set msa_client_id in \
             config.json, or set ESPECTRAL_MSA_CLIENT_ID."
                .to_string()
        } else {
            let detail = match (error, desc) {
                (Some(e), Some(d)) => format!(": {e} — {d}"),
                (Some(e), None) => format!(": {e}"),
                _ => String::new(),
            };
            format!("could not start device login (HTTP {}{detail})", r.status)
        };
        return Err(EngineError::api(502, "MSA_DEVICE_FAILED", message));
    };
    // Never synthesize verification_uri + ?otc=<code>: Microsoft rejects that
    // pre-filled link for consumer accounts. Only pass through a complete URI
    // Microsoft itself returns.
    Ok(MsDeviceFlow {
        flow_id: device_code.to_string(),
        user_code: s(&r.data, "user_code").unwrap_or_default().to_string(),
        verification_uri: s(&r.data, "verification_uri").unwrap_or_default().to_string(),
        verification_uri_complete: s(&r.data, "verification_uri_complete").map(str::to_string),
        expires_in: r.data.get("expires_in").and_then(Value::as_f64).map(|f| f as u64).unwrap_or(900),
        interval: r.data.get("interval").and_then(Value::as_f64).map(|f| f as u64).unwrap_or(5),
    })
}

/// MSA tokens from a completed device login.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

/// One device-code token check (JS `checkDeviceToken`; the back-off codes
/// are variants instead of thrown errors).
#[derive(Debug, Clone, PartialEq)]
pub enum DevicePoll {
    Tokens(DeviceTokens),
    /// MSA_PENDING — keep polling.
    Pending,
    /// MSA_SLOW_DOWN — Microsoft's suggested wait (seconds, default 5).
    SlowDown(u64),
    /// MSA_TRANSIENT — temporarily_unavailable / server_error.
    Transient,
}

/// Single-shot device-code token check. Definitive failures are errors:
/// MSA_DECLINED (400), MSA_EXPIRED (408), MSA_POLL_FAILED (502), MSA_NETWORK (502).
pub async fn check_device_token(engine: &Engine, device_code: &str) -> EngineResult<DevicePoll> {
    let eps = endpoints(engine);
    let r = fetch_json(
        engine,
        reqwest::Method::POST,
        &format!("{}/token", eps.auth_base),
        &[],
        Body::Form(vec![
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code".into()),
            ("device_code", device_code.to_string()),
            ("client_id", client_id(engine)),
        ]),
    )
    .await?;
    if let Some(at) = s(&r.data, "access_token").filter(|_| r.ok) {
        return Ok(DevicePoll::Tokens(DeviceTokens {
            access_token: at.to_string(),
            refresh_token: s(&r.data, "refresh_token").map(str::to_string),
        }));
    }
    match r.data.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => Ok(DevicePoll::Pending),
        Some("authorization_declined") => {
            Err(EngineError::api(400, "MSA_DECLINED", "sign-in was declined in the browser"))
        }
        Some("expired_token") => Err(EngineError::api(408, "MSA_EXPIRED", "device code expired — start over")),
        Some("slow_down") => {
            let ra = r.data.get("retry_after").and_then(Value::as_f64).filter(|f| f.is_finite() && *f > 0.0);
            Ok(DevicePoll::SlowDown(ra.map(|f| f.ceil() as u64).unwrap_or(5)))
        }
        Some("temporarily_unavailable") | Some("server_error") => Ok(DevicePoll::Transient),
        other => {
            Err(EngineError::api(502, "MSA_POLL_FAILED", format!("token poll failed: {}", other.unwrap_or("unknown"))))
        }
    }
}

/// Legacy: loop until the user signs in (CLI / direct callers).
pub async fn poll_device_token(engine: &Engine, device_code: &str, interval: u64) -> EngineResult<DeviceTokens> {
    let mut interval = if interval == 0 { 5 } else { interval };
    loop {
        if engine.is_shutting_down() {
            return Err(EngineError::other("engine shutting down"));
        }
        match check_device_token(engine, device_code).await? {
            DevicePoll::Tokens(t) => return Ok(t),
            DevicePoll::Pending => {}
            DevicePoll::SlowDown(s) => interval = s,
            DevicePoll::Transient => interval += 5,
        }
        sleep_ms(engine, interval * 1000).await;
    }
}

// ---------------------------------------------------------------------------
// XBL -> XSTS -> Minecraft
// ---------------------------------------------------------------------------

/// A Minecraft session from the XBL chain.
#[derive(Debug, Clone, PartialEq)]
pub struct McLogin {
    pub access_token: String,
    pub uuid: String,
    pub username: String,
    /// Seconds.
    pub expires_in: f64,
}

async fn exchange_chain(engine: &Engine, rps_ticket: String, contract_header: bool) -> EngineResult<McLogin> {
    let eps = endpoints(engine);
    let hdrs: &[(&str, &str)] = if contract_header { &[("x-xbl-contract-version", "1")] } else { &[] };
    // 1. XBL
    let xbl = fetch_json(
        engine,
        reqwest::Method::POST,
        &eps.xbl_auth,
        hdrs,
        Body::Json(json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": rps_ticket },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        })),
    )
    .await?;
    let Some(xbl_token) = s(&xbl.data, "Token").filter(|_| xbl.ok) else {
        return Err(EngineError::api(502, "MSA_XBL_FAILED", "Xbox Live authentication failed"));
    };
    // 2. XSTS
    let xsts = fetch_json(
        engine,
        reqwest::Method::POST,
        &eps.xsts_auth,
        hdrs,
        Body::Json(json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        })),
    )
    .await?;
    let Some(xsts_token) = s(&xsts.data, "Token").filter(|_| xsts.ok) else {
        let xerr = xsts.data.get("XErr").and_then(Value::as_f64).map(|f| f as i64);
        return Err(match xerr {
            Some(2148916233) => {
                EngineError::api(403, "MSA_NO_GAMEPASS", "this Microsoft account does not own Minecraft")
            }
            Some(2148916235) => {
                EngineError::api(403, "MSA_UNDERAGE", "Xbox profile is under 18 — parental consent required")
            }
            _ => EngineError::api(
                502,
                "MSA_XSTS_FAILED",
                format!(
                    "Xbox security token failed (XErr {})",
                    xsts.data.get("XErr").filter(|v| !v.is_null()).map(str_of).unwrap_or_else(|| "unknown".into())
                ),
            ),
        });
    };
    let uhs = xsts.data.pointer("/DisplayClaims/xui/0/uhs").map(str_of).filter(|u| !u.is_empty() && u != "null");
    let Some(uhs) = uhs else {
        return Err(EngineError::api(502, "MSA_XSTS_FAILED", "XSTS response missing user hash"));
    };
    // 3. Minecraft login — the API answers snake_case (access_token / expires_in).
    let mc =
        fetch_json_retry(engine, &eps.mc_login, json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") }))
            .await?;
    let Some(mc_token) = s(&mc.data, "access_token").filter(|_| mc.ok) else {
        return Err(EngineError::api(
            502,
            "MSA_MC_LOGIN_FAILED",
            format!("Minecraft login failed (HTTP {}{})", mc.status, body_snippet(&mc.data)),
        ));
    };
    // 4. Profile
    let bearer = format!("Bearer {mc_token}");
    let pf =
        fetch_json(engine, reqwest::Method::GET, &eps.mc_profile, &[("Authorization", &bearer)], Body::None).await?;
    let (Some(id), Some(name)) = (s(&pf.data, "id"), s(&pf.data, "name")) else {
        return Err(EngineError::api(502, "MSA_PROFILE_FAILED", "could not fetch the Minecraft profile"));
    };
    if !pf.ok {
        return Err(EngineError::api(502, "MSA_PROFILE_FAILED", "could not fetch the Minecraft profile"));
    }
    Ok(McLogin {
        access_token: mc_token.to_string(),
        uuid: id.to_string(),
        username: name.to_string(),
        expires_in: mc.data.get("expires_in").and_then(Value::as_f64).unwrap_or(86400.0),
    })
}

/// Exchange an MSA access token for a Minecraft access token + profile ("d=" ticket).
pub async fn exchange_for_minecraft(engine: &Engine, msa_access_token: &str) -> EngineResult<McLogin> {
    exchange_chain(engine, format!("d={msa_access_token}"), false).await
}

/// Exchange a Lunar-refreshed MSA token ("t=" compact ticket, XBL contract v1).
pub async fn exchange_lunar_for_minecraft(engine: &Engine, msa_access_token: &str) -> EngineResult<McLogin> {
    exchange_chain(engine, format!("t={msa_access_token}"), true).await
}

/// Refresh a Lunar-imported account's MSA token via login.live.com.
/// Returns (msa access token, rotated refresh token).
pub async fn refresh_lunar_msa_token(engine: &Engine, refresh_token: &str) -> EngineResult<(String, String)> {
    let eps = endpoints(engine);
    let r = fetch_json(
        engine,
        reqwest::Method::POST,
        &eps.lunar_refresh,
        &[],
        Body::Form(vec![
            ("client_id", LUNAR_CLIENT_ID.into()),
            ("grant_type", "refresh_token".into()),
            ("scope", LUNAR_SCOPE.into()),
            ("refresh_token", refresh_token.to_string()),
        ]),
    )
    .await?;
    let Some(at) = s(&r.data, "access_token").filter(|_| r.ok) else {
        return Err(EngineError::api(
            401,
            "MSA_REFRESH_FAILED",
            "session expired — re-import the account from Lunar Client",
        ));
    };
    let rotated = s(&r.data, "refresh_token").unwrap_or(refresh_token).to_string();
    Ok((at.to_string(), rotated))
}

/// Persist a rotated refresh token onto the stored account (by username).
fn persist_rotated_refresh(engine: &Engine, username: &str, new_refresh: &str) {
    crate::accounts::modify_accounts(engine, |accounts, _| {
        if let Some(m) = accounts.iter_mut().find(|a| a.username == username).and_then(|a| a.microsoft.as_mut()) {
            m.refresh_token = Some(new_refresh.to_string());
        }
    });
}

/// Refresh a stored Microsoft account's Minecraft token using its refresh token.
pub async fn refresh_minecraft_token(engine: &Engine, account: &Account) -> EngineResult<McToken> {
    let ms = account.microsoft.as_ref();
    let refresh = ms.and_then(|m| m.refresh_token.clone()).filter(|r| !r.is_empty());
    if ms.and_then(|m| m.lunar) == Some(true) {
        if let Some(refresh) = &refresh {
            let (msa, rotated) = refresh_lunar_msa_token(engine, refresh).await?;
            let mc = exchange_lunar_for_minecraft(engine, &msa).await?;
            if &rotated != refresh {
                persist_rotated_refresh(engine, &account.username, &rotated);
            }
            return Ok(McToken {
                access_token: mc.access_token,
                expires_at: now_ms() + (mc.expires_in * 1000.0) as i64,
            });
        }
    }
    let Some(refresh) = refresh else {
        return Err(EngineError::bad_request("MSA_NO_REFRESH", "account has no refresh token"));
    };
    let eps = endpoints(engine);
    let r = fetch_json(
        engine,
        reqwest::Method::POST,
        &format!("{}/token", eps.auth_base),
        &[],
        Body::Form(vec![
            ("grant_type", "refresh_token".into()),
            ("client_id", client_id(engine)),
            ("scope", MSA_SCOPE.into()),
            ("refresh_token", refresh.clone()),
        ]),
    )
    .await?;
    let Some(msa) = s(&r.data, "access_token").filter(|_| r.ok).map(str::to_string) else {
        return Err(EngineError::api(401, "MSA_REFRESH_FAILED", "session expired — sign in with Microsoft again"));
    };
    let new_refresh = s(&r.data, "refresh_token").map(str::to_string).unwrap_or_else(|| refresh.clone());
    let mc = exchange_for_minecraft(engine, &msa).await?;
    if new_refresh != refresh {
        persist_rotated_refresh(engine, &account.username, &new_refresh);
    }
    Ok(McToken { access_token: mc.access_token, expires_at: now_ms() + (mc.expires_in * 1000.0) as i64 })
}

/// Fresh-or-cached MC token for an MSA account (None for offline accounts).
///
/// Order: in-memory token -> still-valid disk cache -> refresh (persisted to
/// memory + disk). A failed refresh falls back to a still-valid disk token so
/// a transient outage never downgrades a launch to an offline token.
pub async fn ensure_minecraft_token(engine: &Engine, account: &Account) -> EngineResult<Option<McToken>> {
    tracing::debug!(
        "[msauth] ensureMinecraftToken called for {} uuid={} kind={:?} lunar={:?}",
        account.username,
        account.uuid,
        account.token_kind,
        account.microsoft.as_ref().and_then(|m| m.lunar)
    );
    if account.token_kind != TokenKind::Msa {
        return Ok(None);
    }
    if let Some(t) = memory_token(engine, &account.username).filter(is_fresh) {
        tracing::debug!("[msauth] -> memory cache hit");
        return Ok(Some(t));
    }
    if let Some(t) = cached_mc_token(engine, &account.uuid).filter(is_fresh) {
        tracing::debug!("[msauth] -> disk cache hit");
        set_memory_token(engine, &account.username, &t);
        return Ok(Some(t));
    }
    let fresh = match refresh_minecraft_token(engine, account).await {
        Ok(t) => t,
        Err(e) => {
            if let Some(fallback) = cached_mc_token(engine, &account.uuid).filter(is_fresh) {
                tracing::warn!(
                    "[msauth] refresh failed ({}: {e}) — using cached Minecraft token for '{}'",
                    e.envelope().code,
                    account.username
                );
                set_memory_token(engine, &account.username, &fallback);
                return Ok(Some(fallback));
            }
            return Err(e);
        }
    };
    set_memory_token(engine, &account.username, &fresh);
    set_cached_mc_token(engine, &account.uuid, &fresh);
    Ok(Some(fresh))
}

/// Effective cached expiry of an account's token (memory first, then disk).
fn cached_expiry(engine: &Engine, account: &Account) -> Option<i64> {
    memory_token(engine, &account.username).or_else(|| cached_mc_token(engine, &account.uuid)).map(|t| t.expires_at)
}

fn needs_prewarm(engine: &Engine, account: &Account) -> bool {
    !matches!(cached_expiry(engine, account), Some(exp) if exp - now_ms() > PREWARM_WINDOW_MS)
}

/// Startup prewarm of the active MSA account's token when it expires within
/// 10 minutes. Best-effort: errors are logged, never returned. Concurrent
/// calls for the same account dedupe (the second waits, then sees the fresh token).
pub async fn prewarm_msa_token(engine: &Engine) -> EngineResult<()> {
    let Some(account) = crate::accounts::get_active_account(engine) else { return Ok(()) };
    if account.token_kind != TokenKind::Msa || !needs_prewarm(engine, &account) {
        return Ok(());
    }
    let lock: Arc<tokio::sync::Mutex<()>> = engine
        .accounts_skins_state()
        .prewarm_locks
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(account.username.clone())
        .or_default()
        .clone();
    let _guard = lock.lock().await;
    if !needs_prewarm(engine, &account) {
        return Ok(());
    }
    if let Err(e) = ensure_minecraft_token(engine, &account).await {
        tracing::warn!("[msauth] prewarm failed for '{}': {e}", account.username);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Minecraft profile skins (Bearer = Minecraft access token)
// ---------------------------------------------------------------------------

/// `getProfileSkin` result.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileSkin {
    pub variant: SkinVariant,
    pub has_skin: bool,
    pub cape: bool,
    pub skin_url: Option<String>,
    pub uuid: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum SkinOp {
    Fetch,
    Upload,
    Reset,
}

/// Response body -> object; non-JSON text -> `{ raw: <first 140 chars> }`; empty -> `{}`.
fn parse_skin_body(text: &str) -> Value {
    if text.is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Object(_)) | Ok(v @ Value::Array(_)) => v,
        Ok(_) => json!({}),
        Err(_) => json!({ "raw": text.chars().take(140).collect::<String>() }),
    }
}

fn skin_error_for_status(op: SkinOp, status: u16, retry_after: Option<f64>, data: &Value) -> EngineError {
    if status == 401 {
        return EngineError::api(401, "MSA_REFRESH_FAILED", "session expired — sign in with Microsoft again");
    }
    if status == 429 {
        // The JS attached err.retry_after; the envelope carries it in the message.
        let msg = match retry_after {
            Some(s) => format!("skin rate limited — retry in {s}s"),
            None => "skin rate limited — try again later".to_string(),
        };
        return EngineError::api(429, "SKIN_RATE_LIMITED", msg);
    }
    let (action, code) = match op {
        SkinOp::Fetch => ("fetch profile skin", "SKIN_FETCH_FAILED"),
        SkinOp::Upload => ("upload profile skin", "SKIN_UPLOAD_FAILED"),
        SkinOp::Reset => ("reset profile skin", "SKIN_UPLOAD_FAILED"),
    };
    EngineError::api(status, code, format!("could not {action} (HTTP {status}{})", body_snippet(data)))
}

struct SkinResponse {
    status: u16,
    retry_after: Option<f64>,
    data: Value,
}

async fn skin_send(engine: &Engine, op: SkinOp, req: reqwest::RequestBuilder) -> EngineResult<SkinResponse> {
    let transport = |e: reqwest::Error| {
        let (code, action) = match op {
            SkinOp::Fetch => ("SKIN_FETCH_FAILED", "fetch"),
            SkinOp::Upload => ("SKIN_UPLOAD_FAILED", "upload"),
            SkinOp::Reset => ("SKIN_UPLOAD_FAILED", "reset"),
        };
        EngineError::api(502, code, format!("could not {action} profile skin: {e}"))
    };
    let _ = engine;
    let res = req.timeout(SKIN_TIMEOUT).send().await.map_err(transport)?;
    let status = res.status().as_u16();
    let retry_after = retry_after_seconds(res.headers().get("retry-after").and_then(|v| v.to_str().ok()));
    let text = res.text().await.unwrap_or_default();
    Ok(SkinResponse { status, retry_after, data: parse_skin_body(&text) })
}

fn skin_ok(r: &SkinResponse) -> bool {
    (200..300).contains(&r.status)
}

/// Read the Mojang profile for skin/cape state.
pub async fn get_profile_skin(engine: &Engine, access_token: &str) -> EngineResult<ProfileSkin> {
    let url = endpoints(engine).mc_profile;
    let req = engine.http().client().get(&url).header("Authorization", format!("Bearer {access_token}"));
    let r = skin_send(engine, SkinOp::Fetch, req).await?;
    if !skin_ok(&r) {
        return Err(skin_error_for_status(SkinOp::Fetch, r.status, r.retry_after, &r.data));
    }
    let empty = vec![];
    let skins = r.data.get("skins").and_then(Value::as_array).unwrap_or(&empty);
    let active = skins.iter().find(|s| {
        s.get("state").and_then(Value::as_str) == Some("ACTIVE")
            && s.get("url").and_then(Value::as_str).map(|u| !u.is_empty()).unwrap_or(false)
    });
    let capes = r.data.get("capes").and_then(Value::as_array).unwrap_or(&empty);
    let slim = active
        .and_then(|a| a.get("variant"))
        .map(|v| match v {
            Value::Null => String::new(),
            other => str_of(other),
        })
        .map(|v| v.to_uppercase() == "SLIM")
        .unwrap_or(false);
    Ok(ProfileSkin {
        variant: if slim { SkinVariant::Slim } else { SkinVariant::Classic },
        has_skin: active.is_some(),
        cape: capes.iter().any(|c| c.get("state").and_then(Value::as_str) == Some("ACTIVE")),
        skin_url: active.and_then(|a| a.get("url")).and_then(Value::as_str).map(str::to_string),
        uuid: r.data.get("id").and_then(Value::as_str).map(str::to_string),
        name: r.data.get("name").and_then(Value::as_str).map(str::to_string),
    })
}

fn multipart_skin_body(boundary: &str, variant: &str, png: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(png.len() + 512);
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\n{variant}\r\n").as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

async fn send_skin_upload(
    engine: &Engine,
    access_token: &str,
    png: &[u8],
    mapped: &str,
    method: reqwest::Method,
) -> EngineResult<SkinResponse> {
    let mut rnd = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut rnd);
    let boundary = format!("----espectral{}", hex::encode(rnd));
    let url = format!("{}/skins", endpoints(engine).mc_profile);
    let req = engine
        .http()
        .client()
        .request(method, &url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
        .body(multipart_skin_body(&boundary, mapped, png));
    skin_send(engine, SkinOp::Upload, req).await
}

/// Upload a new skin: multipart `{ variant: CLASSIC|SLIM, file }`. POST first,
/// with a single PUT retry on 405 only. Returns the Mojang response body.
pub async fn upload_profile_skin(
    engine: &Engine,
    access_token: &str,
    png: &[u8],
    variant: SkinVariant,
) -> EngineResult<Value> {
    let mapped = match variant {
        SkinVariant::Classic => "CLASSIC",
        SkinVariant::Slim => "SLIM",
    };
    if png.is_empty() {
        return Err(EngineError::bad_request("BAD_IMAGE", "skin image is empty"));
    }
    let mut r = send_skin_upload(engine, access_token, png, mapped, reqwest::Method::POST).await?;
    if !skin_ok(&r) && r.status == 405 {
        tracing::error!("[msauth] uploadProfileSkin POST -> 405, retrying once with PUT");
        r = send_skin_upload(engine, access_token, png, mapped, reqwest::Method::PUT).await?;
    }
    if !skin_ok(&r) {
        tracing::error!("[msauth] uploadProfileSkin failed (HTTP {}{})", r.status, body_snippet(&r.data));
        return Err(skin_error_for_status(SkinOp::Upload, r.status, r.retry_after, &r.data));
    }
    Ok(r.data)
}

/// Reset to the Steve/Alex default: DELETE …/skins/active. Empty body -> `{ reset: true }`.
pub async fn reset_profile_skin(engine: &Engine, access_token: &str) -> EngineResult<Value> {
    let url = format!("{}/skins/active", endpoints(engine).mc_profile);
    let req = engine.http().client().delete(&url).header("Authorization", format!("Bearer {access_token}"));
    let r = skin_send(engine, SkinOp::Reset, req).await?;
    if !skin_ok(&r) {
        return Err(skin_error_for_status(SkinOp::Reset, r.status, r.retry_after, &r.data));
    }
    Ok(match &r.data {
        Value::Object(m) if !m.is_empty() => r.data.clone(),
        _ => json!({ "reset": true }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockRequest, MockResponse, MockServer};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const UHS: &str = "1234567890";
    const XSTS_TOKEN: &str = "xsts-jwt-body";
    const MC_TOKEN: &str = "mc-access-token-value";

    fn json_resp(v: Value) -> MockResponse {
        MockResponse::json(&v)
    }

    /// The standard happy chain; `login` decides the MC login answer.
    fn chain(
        login: impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static,
    ) -> impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static {
        move |r: &MockRequest| match r.path.as_str() {
            "/user/authenticate" => {
                json_resp(json!({ "Token": "xbl-token", "DisplayClaims": { "xui": [{ "uhs": UHS }] } }))
            }
            "/xsts/authorize" => {
                json_resp(json!({ "Token": XSTS_TOKEN, "DisplayClaims": { "xui": [{ "uhs": UHS }] } }))
            }
            "/authentication/login_with_xbox" => login(r),
            "/minecraft/profile" => {
                assert_eq!(r.header("authorization").as_deref(), Some(format!("Bearer {MC_TOKEN}").as_str()));
                json_resp(json!({ "id": "abc123def4564789", "name": "Tester" }))
            }
            "/consumers/oauth2/v2.0/token" => json_resp(
                json!({ "access_token": "new-msa-access", "refresh_token": "new-refresh", "expires_in": 3600 }),
            ),
            _ => json_resp(json!({ "error": "unexpected endpoint" })).with_status(404),
        }
    }

    async fn setup(
        handler: impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static,
    ) -> (tempfile::TempDir, Engine, MockServer) {
        let srv = MockServer::start(handler).await;
        let (d, e) = temp_engine();
        set_endpoints(&e, Endpoints::on_base(&srv.url("")));
        *e.accounts_skins_state().fake_sleeps.lock().unwrap() = Some(vec![]);
        (d, e, srv)
    }

    fn sleeps(e: &Engine) -> Vec<u64> {
        e.accounts_skins_state().fake_sleeps.lock().unwrap().clone().unwrap()
    }

    #[test]
    fn retry_after_parsing() {
        assert_eq!(retry_after_seconds(Some("2")), Some(2.0));
        assert_eq!(retry_after_seconds(Some(" ")), None);
        assert_eq!(retry_after_seconds(None), None);
        assert_eq!(retry_after_seconds(Some("-1")), None);
        assert_eq!(retry_after_seconds(Some("Wed, 21 Oct 2015 07:28:00 GMT")), Some(0.0));
        assert_eq!(retry_after_seconds(Some("garbage")), None);
    }

    #[test]
    fn snippet_prefers_error_message() {
        assert_eq!(body_snippet(&json!({ "error": "E", "errorMessage": "M" })), ": M");
        assert_eq!(body_snippet(&json!({ "path": "/p" })), ": /p");
        assert_eq!(body_snippet(&json!({})), "");
        assert_eq!(body_snippet(&json!("x")), "");
        assert_eq!(body_snippet(&json!({ "raw": "x".repeat(300) })).len(), 142);
    }

    #[test]
    fn client_id_prefers_config() {
        let (_d, e) = temp_engine();
        if std::env::var("ESPECTRAL_MSA_CLIENT_ID").is_err() {
            assert_eq!(client_id(&e), DEFAULT_CLIENT_ID);
        }
        e.config().set("msa_client_id", json!("abcdef-1234"));
        assert_eq!(client_id(&e), "abcdef-1234");
    }

    #[tokio::test]
    async fn start_device_login_never_builds_a_prefilled_link() {
        let (_d, e, srv) = setup(|r| {
            assert_eq!(r.path, "/consumers/oauth2/v2.0/devicecode");
            let body = String::from_utf8_lossy(&r.body).to_string();
            assert!(body.contains("scope=XboxLive.signin+offline_access"), "{body}");
            json_resp(json!({
                "device_code": "dev-code", "user_code": "ABCD1234",
                "verification_uri": "https://www.microsoft.com/link", "expires_in": 900, "interval": 5
            }))
        })
        .await;
        let flow = start_device_login(&e).await.unwrap();
        assert_eq!(flow.flow_id, "dev-code");
        assert_eq!(flow.verification_uri, "https://www.microsoft.com/link");
        assert_eq!(flow.user_code, "ABCD1234");
        assert_eq!(flow.verification_uri_complete, None);
        assert_eq!(srv.hits(), 1);
    }

    #[tokio::test]
    async fn start_device_login_explains_unconfigured_app() {
        let (_d, e, _srv) = setup(|_| {
            json_resp(json!({ "error": "unauthorized_client", "error_description": "AADSTS700016: app not found" }))
                .with_status(400)
        })
        .await;
        let err = start_device_login(&e).await.unwrap_err().envelope();
        assert_eq!(err.code, "MSA_DEVICE_FAILED");
        if std::env::var("ESPECTRAL_MSA_CLIENT_ID").is_err() {
            assert!(err.message.contains("not configured"), "{}", err.message);
        }
        e.config().set("msa_client_id", json!("0000-1111-2222"));
        let err = start_device_login(&e).await.unwrap_err().envelope();
        assert!(err.message.contains("HTTP 400: unauthorized_client — AADSTS700016"), "{}", err.message);
    }

    #[tokio::test]
    async fn device_token_outcomes() {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let (_d, e, _srv) = setup(move |_| {
            let body = match n2.fetch_add(1, Ordering::SeqCst) {
                0 => json!({ "error": "authorization_pending" }),
                1 => json!({ "error": "slow_down", "retry_after": 12 }),
                2 => json!({ "error": "slow_down" }),
                3 => json!({ "error": "server_error" }),
                4 => json!({ "error": "authorization_declined" }),
                5 => json!({ "error": "expired_token" }),
                6 => json!({ "error": "weird" }),
                _ => return json_resp(json!({ "access_token": "AT", "refresh_token": "RT" })),
            };
            json_resp(body).with_status(400)
        })
        .await;
        assert_eq!(check_device_token(&e, "dc").await.unwrap(), DevicePoll::Pending);
        assert_eq!(check_device_token(&e, "dc").await.unwrap(), DevicePoll::SlowDown(12));
        assert_eq!(check_device_token(&e, "dc").await.unwrap(), DevicePoll::SlowDown(5));
        assert_eq!(check_device_token(&e, "dc").await.unwrap(), DevicePoll::Transient);
        let code = |r: EngineResult<DevicePoll>| {
            let x = r.unwrap_err().envelope();
            (x.status, x.code)
        };
        assert_eq!(code(check_device_token(&e, "dc").await), (400, "MSA_DECLINED".into()));
        assert_eq!(code(check_device_token(&e, "dc").await), (408, "MSA_EXPIRED".into()));
        assert_eq!(code(check_device_token(&e, "dc").await), (502, "MSA_POLL_FAILED".into()));
        assert_eq!(
            check_device_token(&e, "dc").await.unwrap(),
            DevicePoll::Tokens(DeviceTokens { access_token: "AT".into(), refresh_token: Some("RT".into()) })
        );
    }

    #[tokio::test]
    async fn network_failure_is_msa_network() {
        let (_d, e, _srv) = setup(|_| MockResponse::drop_connection()).await;
        let err = check_device_token(&e, "dc").await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (502, "MSA_NETWORK"));
    }

    #[tokio::test]
    async fn exchange_happy_path_with_snake_case_api() {
        let (_d, e, srv) = setup(chain(|_| json_resp(json!({ "access_token": MC_TOKEN, "expires_in": 86400 })))).await;
        let mc = exchange_for_minecraft(&e, "msa-access-token").await.unwrap();
        assert_eq!(mc.access_token, MC_TOKEN);
        assert_eq!(mc.username, "Tester");
        assert_eq!(mc.uuid, "abc123def4564789");
        assert_eq!(mc.expires_in, 86400.0);
        let reqs = srv.requests();
        let login = reqs.iter().find(|r| r.path.contains("login_with_xbox")).unwrap();
        assert!(String::from_utf8_lossy(&login.body).contains(&format!("XBL3.0 x={UHS};{XSTS_TOKEN}")));
        let xbl = reqs.iter().find(|r| r.path == "/user/authenticate").unwrap();
        assert_eq!(xbl.body_json().unwrap()["Properties"]["RpsTicket"], "d=msa-access-token");
        assert!(xbl.header("x-xbl-contract-version").is_none());
    }

    #[tokio::test]
    async fn mc_login_rejection_carries_status_and_body() {
        let (_d, e, _srv) = setup(chain(|_| {
            json_resp(json!({ "path": "/authentication/login_with_xbox", "error": "Forbidden" })).with_status(403)
        }))
        .await;
        let err = exchange_for_minecraft(&e, "t").await.unwrap_err().envelope();
        assert_eq!(err.code, "MSA_MC_LOGIN_FAILED");
        assert!(err.message.contains("HTTP 403") && err.message.contains("Forbidden"), "{}", err.message);
        assert!(sleeps(&e).is_empty(), "403 is a real rejection — no retry");
    }

    #[tokio::test]
    async fn mc_login_retries_429_bursts() {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let (_d, e, _srv) = setup(chain(move |_| {
            if n2.fetch_add(1, Ordering::SeqCst) < 2 {
                json_resp(json!({ "error": "Too Many Requests" })).with_status(429)
            } else {
                json_resp(json!({ "access_token": MC_TOKEN, "expires_in": 86400 }))
            }
        }))
        .await;
        let mc = exchange_for_minecraft(&e, "t").await.unwrap();
        assert_eq!(mc.access_token, MC_TOKEN);
        assert_eq!(n.load(Ordering::SeqCst), 3);
        assert_eq!(sleeps(&e), vec![1000, 3000]);
    }

    #[tokio::test]
    async fn mc_login_honours_sane_retry_after() {
        let (_d, e, _srv) = setup(chain(|_| {
            json_resp(json!({ "error": "Too Many Requests" })).with_status(429).with_header("retry-after", "2")
        }))
        .await;
        let err = exchange_for_minecraft(&e, "t").await.unwrap_err().envelope();
        assert!(err.code == "MSA_MC_LOGIN_FAILED" && err.message.contains("HTTP 429"));
        assert_eq!(sleeps(&e), vec![2000, 2000]);
    }

    #[tokio::test]
    async fn lunar_exchange_uses_compact_ticket_and_survives_429() {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let (_d, e, srv) = setup(chain(move |_| {
            if n2.fetch_add(1, Ordering::SeqCst) < 2 {
                json_resp(json!({})).with_status(429)
            } else {
                json_resp(json!({ "access_token": MC_TOKEN, "expires_in": 86400 }))
            }
        }))
        .await;
        let mc = exchange_lunar_for_minecraft(&e, "lunar-msa").await.unwrap();
        assert_eq!(mc.access_token, MC_TOKEN);
        assert_eq!(mc.username, "Tester");
        let xbl = srv.requests().into_iter().find(|r| r.path == "/user/authenticate").unwrap();
        assert_eq!(xbl.body_json().unwrap()["Properties"]["RpsTicket"], "t=lunar-msa");
        assert_eq!(xbl.header("x-xbl-contract-version").as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn xsts_errors_map_to_codes() {
        let (_d, e, _srv) = setup(|r| match r.path.as_str() {
            "/user/authenticate" => json_resp(json!({ "Token": "x" })),
            _ => json_resp(json!({ "XErr": 2148916233u64 })).with_status(401),
        })
        .await;
        assert_eq!(exchange_for_minecraft(&e, "t").await.unwrap_err().envelope().code, "MSA_NO_GAMEPASS");
    }

    fn msa_account(username: &str, uuid: &str, refresh: &str) -> Value {
        json!({ "username": username, "uuid": uuid, "token_kind": "msa", "microsoft": { "refresh_token": refresh } })
    }

    #[tokio::test]
    async fn ensure_token_refreshes_persists_and_rotates() {
        let (_d, e, srv) = setup(chain(|_| json_resp(json!({ "access_token": MC_TOKEN, "expires_in": 86400 })))).await;
        e.config().set("accounts", json!([msa_account("Msa", "u-1", "old-refresh")]));
        let acct = crate::accounts::get_account(&e, "Msa").unwrap();
        let t = ensure_minecraft_token(&e, &acct).await.unwrap().unwrap();
        assert_eq!(t.access_token, MC_TOKEN);
        assert_eq!(srv.hits(), 5);
        // rotated refresh token persisted, access token never in config.json
        let raw = std::fs::read_to_string(e.paths().config_json()).unwrap();
        assert!(raw.contains("new-refresh") && !raw.contains(MC_TOKEN), "{raw}");
        // disk cache (dashed + dashless)
        let cache: Value = serde_json::from_str(&std::fs::read_to_string(e.paths().mc_token_cache()).unwrap()).unwrap();
        assert_eq!(cache["u-1"]["accessToken"], MC_TOKEN);
        assert_eq!(cache["u1"]["accessToken"], MC_TOKEN);
        // memory hit: no more network
        ensure_minecraft_token(&e, &acct).await.unwrap().unwrap();
        assert_eq!(srv.hits(), 5);
        // offline -> None
        let off = Account { username: "o".into(), ..Default::default() };
        assert_eq!(ensure_minecraft_token(&e, &off).await.unwrap(), None);
    }

    #[tokio::test]
    async fn ensure_token_uses_disk_cache_and_falls_back_on_refresh_failure() {
        let (_d, e, srv) = setup(|_| json_resp(json!({ "error": "invalid_grant" })).with_status(400)).await;
        e.config().set("accounts", json!([msa_account("Msa", "u-2", "r")]));
        let acct = crate::accounts::get_account(&e, "Msa").unwrap();
        // no cache -> the refresh error surfaces
        let err = ensure_minecraft_token(&e, &acct).await.unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (401, "MSA_REFRESH_FAILED"));
        // valid disk cache -> used without network
        let hits = srv.hits();
        let tok = McToken { access_token: "disk".into(), expires_at: now_ms() + 3_600_000 };
        set_cached_mc_token(&e, "u-2", &tok);
        assert_eq!(ensure_minecraft_token(&e, &acct).await.unwrap().unwrap(), tok);
        assert_eq!(srv.hits(), hits);
    }

    #[tokio::test]
    async fn lunar_accounts_refresh_through_login_live() {
        let (_d, e, srv) = setup(chain(|_| json_resp(json!({ "access_token": MC_TOKEN, "expires_in": 600 })))).await;
        e.config().set(
            "accounts",
            json!([{ "username": "Lun", "uuid": "u-3", "token_kind": "msa", "microsoft": { "refresh_token": "lr", "lunar": true } }]),
        );
        // login.live answers through the generic handler's 404 -> MSA_REFRESH_FAILED (re-import)
        let acct = crate::accounts::get_account(&e, "Lun").unwrap();
        let err = ensure_minecraft_token(&e, &acct).await.unwrap_err().envelope();
        assert_eq!(err.code, "MSA_REFRESH_FAILED");
        assert!(err.message.contains("Lunar"), "{}", err.message);
        let req = srv.requests().into_iter().find(|r| r.path == "/oauth20_token.srf").unwrap();
        let body = String::from_utf8_lossy(&req.body).to_string();
        assert!(body.contains("client_id=00000000402b5328") && body.contains("grant_type=refresh_token"), "{body}");
    }

    // ---- prewarm (C11) ----

    #[tokio::test]
    async fn prewarm_no_account_or_offline_is_a_no_op() {
        let (_d, e, srv) = setup(|_| json_resp(json!({}))).await;
        prewarm_msa_token(&e).await.unwrap();
        e.config().set("accounts", json!([{ "username": "OfflineJoe", "uuid": "x", "token_kind": "offline" }]));
        e.config().set("active_username", json!("OfflineJoe"));
        prewarm_msa_token(&e).await.unwrap();
        assert_eq!(srv.hits(), 0);
    }

    #[tokio::test]
    async fn prewarm_fresh_token_is_a_no_op() {
        let (_d, e, srv) = setup(|_| json_resp(json!({}))).await;
        e.config().set("accounts", json!([msa_account("MsaFresh", "fresh-uuid-1234", "r")]));
        e.config().set("active_username", json!("MsaFresh"));
        remember_token(&e, "MsaFresh", &McToken { access_token: "cached".into(), expires_at: now_ms() + 3_600_000 });
        prewarm_msa_token(&e).await.unwrap();
        assert_eq!(srv.hits(), 0);
    }

    #[tokio::test]
    async fn prewarm_near_expiry_refreshes_once_for_concurrent_calls() {
        let (_d, e, srv) = setup(chain(|_| json_resp(json!({ "access_token": MC_TOKEN, "expires_in": 86400 })))).await;
        e.config().set("accounts", json!([msa_account("MsaNear", "near-uuid-5678", "refresh-near")]));
        e.config().set("active_username", json!("MsaNear"));
        remember_token(&e, "MsaNear", &McToken { access_token: "old".into(), expires_at: now_ms() + 120_000 });
        let (a, b) = tokio::join!(prewarm_msa_token(&e), prewarm_msa_token(&e));
        a.unwrap();
        b.unwrap();
        assert_eq!(srv.hits(), 5, "token + xbl + xsts + mc login + profile, once");
        prewarm_msa_token(&e).await.unwrap();
        assert_eq!(srv.hits(), 5, "fresh token -> no-op");
    }

    #[tokio::test]
    async fn prewarm_swallows_refresh_errors() {
        let (_d, e, _srv) = setup(|_| json_resp(json!({ "error": "invalid_grant" })).with_status(400)).await;
        e.config().set("accounts", json!([msa_account("MsaFail", "fail-uuid-9999", "bad")]));
        e.config().set("active_username", json!("MsaFail"));
        prewarm_msa_token(&e).await.unwrap();
    }

    // ---- profile skins (tests/skin.test.mjs) ----

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0u8; 29];
        b[..8].copy_from_slice(&crate::util::PNG_MAGIC);
        b[8..12].copy_from_slice(&13u32.to_be_bytes());
        b[12..16].copy_from_slice(b"IHDR");
        b[16..20].copy_from_slice(&w.to_be_bytes());
        b[20..24].copy_from_slice(&h.to_be_bytes());
        b[24] = 8;
        b[25] = 2;
        b
    }

    #[tokio::test]
    async fn profile_skin_maps_active_slim_skin_and_cape() {
        let (_d, e, _srv) = setup(|r| {
            assert_eq!(r.path, "/minecraft/profile");
            assert_eq!(r.header("authorization").as_deref(), Some("Bearer mc-token-123"));
            json_resp(json!({
                "id": "uuid-1", "name": "Steve",
                "skins": [
                    { "id": "s1", "state": "INACTIVE", "url": "https://old-skin", "variant": "CLASSIC" },
                    { "id": "s2", "state": "ACTIVE", "url": "https://textures.minecraft.net/texture/abc", "variant": "SLIM" }
                ],
                "capes": [{ "id": "c1", "state": "ACTIVE", "url": "https://cape", "alias": "Migrator" }]
            }))
        })
        .await;
        let p = get_profile_skin(&e, "mc-token-123").await.unwrap();
        assert_eq!(
            p,
            ProfileSkin {
                variant: SkinVariant::Slim,
                has_skin: true,
                cape: true,
                skin_url: Some("https://textures.minecraft.net/texture/abc".into()),
                uuid: Some("uuid-1".into()),
                name: Some("Steve".into()),
            }
        );
    }

    #[tokio::test]
    async fn profile_without_skin_defaults_to_classic() {
        let (_d, e, _srv) = setup(|_| json_resp(json!({ "id": "u", "name": "Alex", "skins": [], "capes": [] }))).await;
        let p = get_profile_skin(&e, "tok").await.unwrap();
        assert_eq!((p.variant, p.has_skin, p.cape, p.skin_url), (SkinVariant::Classic, false, false, None));
        assert_eq!((p.uuid.as_deref(), p.name.as_deref()), (Some("u"), Some("Alex")));
    }

    #[tokio::test]
    async fn upload_posts_multipart_variant_and_file() {
        let (_d, e, srv) = setup(|_| json_resp(json!({}))).await;
        let bytes = png(64, 64);
        upload_profile_skin(&e, "mc-token-123", &bytes, SkinVariant::Classic).await.unwrap();
        let r = &srv.requests()[0];
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/minecraft/profile/skins");
        assert_eq!(r.header("authorization").as_deref(), Some("Bearer mc-token-123"));
        let ct = r.header("content-type").unwrap();
        assert!(ct.starts_with("multipart/form-data; boundary="), "{ct}");
        let body = String::from_utf8_lossy(&r.body).to_string();
        let vi = body.find("name=\"variant\"\r\n\r\nCLASSIC\r\n").expect("variant field");
        let fi = body.find("name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png").expect("file field");
        assert!(vi < fi, "variant before file");
        assert!(r.body.windows(bytes.len()).any(|w| w == bytes.as_slice()), "png bytes carried");
        // slim
        upload_profile_skin(&e, "t", &png(64, 32), SkinVariant::Slim).await.unwrap();
        assert!(String::from_utf8_lossy(&srv.requests()[1].body).contains("\r\n\r\nSLIM\r\n"));
        // empty bytes
        assert_eq!(
            upload_profile_skin(&e, "t", &[], SkinVariant::Slim).await.unwrap_err().envelope().code,
            "BAD_IMAGE"
        );
    }

    #[tokio::test]
    async fn upload_405_retries_once_with_put() {
        let (_d, e, srv) = setup(|r| {
            if r.method == "POST" {
                json_resp(json!({ "error": "METHOD_NOT_ALLOWED" })).with_status(405)
            } else {
                json_resp(json!({ "ok": true }))
            }
        })
        .await;
        let out = upload_profile_skin(&e, "t", &png(64, 64), SkinVariant::Classic).await.unwrap();
        assert_eq!(out, json!({ "ok": true }));
        assert_eq!(srv.requests().iter().map(|r| r.method.clone()).collect::<Vec<_>>(), ["POST", "PUT"]);
    }

    #[tokio::test]
    async fn upload_non_405_failure_surfaces_at_once() {
        let (_d, e, srv) = setup(|_| json_resp(json!({ "error": "Bad Request" })).with_status(400)).await;
        let err = upload_profile_skin(&e, "t", &png(64, 64), SkinVariant::Classic).await.unwrap_err().envelope();
        assert_eq!(err.status, 400);
        assert_eq!(srv.hits(), 1);
    }

    #[tokio::test]
    async fn upload_surfaces_mojang_error_message_and_text_bodies() {
        let (_d, e, _srv) = setup(|_| {
            json_resp(json!({
                "error": "Method Not Allowed", "errorMessage": "The skin upload endpoint rejects this content type"
            }))
            .with_status(405)
        })
        .await;
        let err = upload_profile_skin(&e, "SECRET-TOKEN-XYZ", &png(64, 64), SkinVariant::Classic)
            .await
            .unwrap_err()
            .envelope();
        assert_eq!((err.status, err.code.as_str()), (405, "SKIN_UPLOAD_FAILED"));
        assert!(err.message.contains("405") && err.message.contains("rejects this content type"), "{}", err.message);
        assert!(!err.message.contains("SECRET-TOKEN-XYZ"));
        let (_d2, e2, _srv2) = setup(|_| MockResponse::ok("Method Not Allowed").with_status(405)).await;
        let err = upload_profile_skin(&e2, "t", &png(64, 64), SkinVariant::Classic).await.unwrap_err().envelope();
        assert!(err.message.contains("405") && err.message.contains("Method Not Allowed"), "{}", err.message);
    }

    #[tokio::test]
    async fn reset_deletes_active_skin() {
        let (_d, e, srv) = setup(|_| MockResponse::bytes(vec![]).with_status(204)).await;
        assert_eq!(reset_profile_skin(&e, "t").await.unwrap(), json!({ "reset": true }));
        let r = &srv.requests()[0];
        assert_eq!((r.method.as_str(), r.path.as_str()), ("DELETE", "/minecraft/profile/skins/active"));
        assert_eq!(r.header("authorization").as_deref(), Some("Bearer t"));
    }

    #[tokio::test]
    async fn skin_error_mapping_never_leaks_the_token() {
        let (_d, e, _srv) = setup(|_| json_resp(json!({ "error": "Unauthorized" })).with_status(401)).await;
        let tok = "SECRET-TOKEN-XYZ";
        let g = get_profile_skin(&e, tok).await.unwrap_err().envelope();
        let u = upload_profile_skin(&e, tok, &png(64, 64), SkinVariant::Classic).await.unwrap_err().envelope();
        let r = reset_profile_skin(&e, tok).await.unwrap_err().envelope();
        for x in [&g, &u, &r] {
            assert_eq!((x.status, x.code.as_str()), (401, "MSA_REFRESH_FAILED"));
            assert!(!x.message.contains(tok));
        }
        let (_d, e, _srv) = setup(|r| {
            if r.method == "DELETE" {
                json_resp(json!({})).with_status(429)
            } else {
                json_resp(json!({ "error": "Too Many Requests" })).with_status(429).with_header("retry-after", "7")
            }
        })
        .await;
        let g = get_profile_skin(&e, tok).await.unwrap_err().envelope();
        assert_eq!((g.status, g.code.as_str()), (429, "SKIN_RATE_LIMITED"));
        assert!(g.message.contains("retry in 7s"), "{}", g.message);
        let u = upload_profile_skin(&e, tok, &png(64, 64), SkinVariant::Classic).await.unwrap_err().envelope();
        assert_eq!(u.code, "SKIN_RATE_LIMITED");
        let r = reset_profile_skin(&e, tok).await.unwrap_err().envelope();
        assert_eq!(r.code, "SKIN_RATE_LIMITED");
        assert!(r.message.contains("try again later"));
        let (_d, e, _srv) =
            setup(|_| json_resp(json!({ "error": "ForbiddenOperationException" })).with_status(403)).await;
        let g = get_profile_skin(&e, tok).await.unwrap_err().envelope();
        assert!(g.code == "SKIN_FETCH_FAILED" && g.message.contains("403") && !g.message.contains(tok));
        let u = upload_profile_skin(&e, tok, &png(64, 64), SkinVariant::Classic).await.unwrap_err().envelope();
        assert!(u.code == "SKIN_UPLOAD_FAILED" && u.message.contains("403"));
        let (_d, e, _srv) = setup(|_| MockResponse::drop_connection()).await;
        assert_eq!(get_profile_skin(&e, tok).await.unwrap_err().envelope().code, "SKIN_FETCH_FAILED");
    }
}
