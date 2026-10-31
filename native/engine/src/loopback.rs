//! Loopback HTTP listener (NEW — no JS module): the only HTTP the native
//! engine still serves, because external processes call it:
//!
//! - `POST /api/bridge/visible` — the in-game Espectral mod (headers
//!   `x-espectral-client: 1` and `x-espectral-bridge: <per-launch token>`,
//!   body `{ uuids: [...] }`) -> [`crate::bridge::visible_for_token`]
//!   (the same logic `Engine::bridge_visible` exposes);
//! - `GET /api/discord/callback` — Discord's OAuth redirect (registered as
//!   `http://127.0.0.1:4199/api/discord/callback`) -> `Engine::discord_callback`,
//!   answering the completion / error HTML from routes/discord.mjs;
//! - `GET|POST /api/agent/*` and `POST /api/bridge/agent/*` — agent access
//!   (routes/agent.mjs, upstream c22767f): the Bearer-token agent API and the
//!   mod long-poll relay, backed by [`crate::agent`].
//!
//! **Owner: game-launch (agent access).** Status: PORTED.
//!
//! Binds 127.0.0.1 only, port `ESPECTRAL_PORT` or 4199. Keeps the JS guards:
//! the Host header must be `127.0.0.1|localhost[:port]` (DNS-rebinding), and
//! every non-GET needs `x-espectral-client: 1` (CSRF) — including the agent
//! API and the mod relay, which is why the MCP shim sends it. Errors use the
//! JS envelope `{ error: { code, message } }`. Stops on `Engine::cancel_token()`.
//!
//! Agent routes answer 403 `AGENT_DISABLED` while `agent.enabled` is off
//! (before any auth so a disabled engine never confirms a token), then 401
//! `AGENT_AUTH` for a bad Bearer token. The relay authenticates with the
//! per-launch bridge token in `x-espectral-bridge` instead.
//!
//! Not served here: the session cookie on the Discord callback (the native UI
//! claims the session via `discord_claim`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};

use crate::agent;
use crate::api::identity_discord::DiscordUser;
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};

/// JS DEFAULT_PORT.
pub const DEFAULT_PORT: u16 = 4199;

/// The custom header every state-changing request must carry.
pub const CLIENT_HEADER: &str = "x-espectral-client";

/// `ESPECTRAL_PORT` env or 4199.
pub fn configured_port() -> u16 {
    std::env::var("ESPECTRAL_PORT").ok().and_then(|p| p.parse().ok()).filter(|p| *p > 0).unwrap_or(DEFAULT_PORT)
}

/// Bound listener address (None until started / when the bind failed).
#[derive(Debug, Default)]
pub struct LoopbackState {
    addr: Mutex<Option<SocketAddr>>,
}

/// The address the loopback listener is bound to, if running.
pub fn bound_addr(engine: &Engine) -> Option<SocketAddr> {
    *engine.identity_discord_state().loopback.addr.lock().unwrap_or_else(|p| p.into_inner())
}

/// `http://127.0.0.1:<port>/api/bridge` — the value game-launch passes as
/// `-Despectral.bridge.url` (bound port when running, else the configured one).
pub fn bridge_url(engine: &Engine) -> String {
    let port = bound_addr(engine).map(|a| a.port()).unwrap_or_else(configured_port);
    format!("http://127.0.0.1:{port}/api/bridge")
}

fn host_ok(headers: &HeaderMap) -> bool {
    let Some(h) = headers.get(header::HOST) else { return true };
    let h = h.to_str().unwrap_or("").to_ascii_lowercase();
    if h.is_empty() {
        return true;
    }
    let (name, port) = match h.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (h.as_str(), None),
    };
    (name == "127.0.0.1" || name == "localhost")
        && port.map_or(true, |p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn json_error(status: u16, code: &str, message: &str) -> Response {
    let s = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (s, axum::Json(json!({ "error": { "code": code, "message": message } }))).into_response()
}

fn engine_error(e: &EngineError) -> Response {
    let env = e.envelope();
    json_error(env.status, &env.code, &env.message)
}

async fn bridge_visible(State(engine): State<Engine>, headers: HeaderMap, body: Bytes) -> Response {
    if !host_ok(&headers) {
        return json_error(403, "FORBIDDEN", "unexpected Host header");
    }
    if headers.get(CLIENT_HEADER).and_then(|v| v.to_str().ok()) != Some("1") {
        return json_error(403, "FORBIDDEN", "missing x-espectral-client header");
    }
    let parsed: Value = if String::from_utf8_lossy(&body).trim().is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return json_error(400, "BAD_JSON", "request body is not valid JSON"),
        }
    };
    // Non-strings keep their slot (the JS slices to 1000 before filtering).
    let uuids: Vec<String> = parsed
        .get("uuids")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|u| u.as_str().unwrap_or_default().to_string()).collect())
        .unwrap_or_default();
    let token = headers.get(crate::bridge::BRIDGE_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    match crate::bridge::visible_for_token(&engine, token, &uuids) {
        Ok(reply) => axum::Json(reply).into_response(),
        Err(e) => engine_error(&e),
    }
}

/// HTML escape (routes/discord.mjs `esc`).
pub fn esc(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The OAuth completion page shown in the system browser.
pub fn render_auth_success_html(user: Option<&DiscordUser>) -> String {
    let safe_name = esc(user.map(|u| u.name.as_str()).filter(|n| !n.is_empty()).unwrap_or("Usuario"));
    let safe_avatar = esc(user.and_then(|u| u.avatar.as_deref()).unwrap_or(""));
    let user_json = serde_json::to_string(&user).unwrap_or_else(|_| "null".into()).replace('<', "\\u003c");
    let avatar_img = if safe_avatar.is_empty() {
        String::new()
    } else {
        format!(r#"<img src="{safe_avatar}" alt="Avatar" class="avatar" />"#)
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="es">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Espectral — Discord Login</title>
  <style>
    :root {{ --bg: #060a14; --card-bg: rgba(15, 23, 42, 0.88); --blurple: #5865f2; --blurple-glow: rgba(88, 101, 242, 0.35); --text: #e8ecf4; --text-muted: #94a3b8; --border: rgba(88, 101, 242, 0.3); }}
    * {{ box-sizing: border-box; }}
    body {{ margin: 0; padding: 24px; min-height: 100vh; display: flex; align-items: center; justify-content: center; background: var(--bg); color: var(--text); font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; text-align: center; }}
    .card {{ max-width: 420px; width: 100%; background: var(--card-bg); border: 1px solid var(--border); border-radius: 20px; padding: 40px 32px; box-shadow: 0 20px 50px rgba(0, 0, 0, 0.6), 0 0 30px var(--blurple-glow); backdrop-filter: blur(16px); }}
    .avatar {{ width: 72px; height: 72px; border-radius: 50%; border: 2px solid var(--blurple); box-shadow: 0 0 16px var(--blurple-glow); margin: 0 auto 16px auto; display: block; object-fit: cover; background: #0f172a; }}
    .badge {{ display: inline-flex; align-items: center; gap: 6px; padding: 4px 12px; border-radius: 9999px; background: rgba(88, 101, 242, 0.15); border: 1px solid rgba(88, 101, 242, 0.4); color: #a5b4fc; font-size: 12px; font-weight: 600; margin-bottom: 12px; text-transform: uppercase; letter-spacing: 0.05em; }}
    h1 {{ margin: 0 0 8px 0; font-size: 22px; font-weight: 700; color: #fff; }}
    p {{ margin: 0 0 20px 0; font-size: 14px; color: var(--text-muted); line-height: 1.5; }}
    .hint {{ font-size: 12px; color: #64748b; }}
  </style>
  <script>
    try {{
      if (window.opener) {{
        window.opener.postMessage({{
          type: 'discord-auth-success',
          user: {user_json}
        }}, '*');
      }}
    }} catch (e) {{}}
    setTimeout(function() {{
      try {{ window.close(); }} catch (e) {{}}
    }}, 1400);
  </script>
</head>
<body>
  <div class="card">
    {avatar_img}
    <div class="badge">Identidad Espectral</div>
    <h1>¡Hola, {safe_name}!</h1>
    <p>Tu cuenta de Discord se ha vinculado correctamente con Espectral Client.</p>
    <div class="hint">Ya puedes volver a Espectral Client — la sesión se completará sola.</div>
  </div>
</body>
</html>"#
    )
}

/// The OAuth error page.
pub fn render_auth_error_html(message: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>Error de autenticación</title>
<style>body{{background:#060a14;color:#ef4444;font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;text-align:center;}}h2{{margin-bottom:8px;}}p{{color:#94a3b8;}}</style>
</head>
<body><div><h2>Error de vinculación con Discord</h2><p>{}</p></div></body>
</html>"#,
        esc(message)
    )
}

async fn discord_callback(
    State(engine): State<Engine>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !host_ok(&headers) {
        return json_error(403, "FORBIDDEN", "unexpected Host header");
    }
    let redirect = query.get("redirect").cloned();
    match engine.discord_callback(query).await {
        Ok(outcome) => {
            if matches!(redirect.as_deref(), Some("tauri") | Some("deep")) {
                return (StatusCode::FOUND, [(header::LOCATION, "tauri://localhost/#/")]).into_response();
            }
            Html(render_auth_success_html(outcome.user.as_ref())).into_response()
        }
        Err(e @ EngineError::Api { .. }) => {
            let s = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST);
            (s, Html(render_auth_error_html(&e.to_string()))).into_response()
        }
        Err(e) => engine_error(&e),
    }
}

// ---------------------------------------------------------------------------
// Agent access (routes/agent.mjs)
// ---------------------------------------------------------------------------

/// Host + CSRF guards shared by every route (JS `makeRequestHandler`):
/// `non_get` mirrors `typeof method !== 'GET'` — the agent API and the relay
/// carry `x-espectral-client: 1` too.
fn guard(headers: &HeaderMap, non_get: bool) -> Option<Response> {
    if !host_ok(headers) {
        return Some(json_error(403, "FORBIDDEN", "unexpected Host header"));
    }
    if non_get && headers.get(CLIENT_HEADER).and_then(|v| v.to_str().ok()) != Some("1") {
        return Some(json_error(403, "FORBIDDEN", "missing x-espectral-client header"));
    }
    None
}

/// JS `readJsonBody`: empty/blank -> null, malformed -> 400 BAD_JSON.
fn parse_body(body: &Bytes) -> Result<Value, Box<Response>> {
    let text = String::from_utf8_lossy(body);
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|_| Box::new(json_error(400, "BAD_JSON", "request body is not valid JSON")))
}

fn bad_request(message: &str) -> Response {
    json_error(400, "BAD_REQUEST", message)
}

fn string_field(body: &Value, key: &str) -> String {
    body.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn agent_auth(engine: &Engine, headers: &HeaderMap) -> EngineResult<()> {
    let header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    agent::authenticate(engine.config(), engine.paths().data_dir(), header)
}

/// 403 AGENT_DISABLED while agent access is off (JS `requireEnabled`).
fn require_enabled(engine: &Engine) -> EngineResult<()> {
    if !agent::agent_config(engine.config()).enabled {
        return Err(agent::disabled_error());
    }
    Ok(())
}

/// The `/api/launches` row for a key, or the registry entry when the buffer
/// was already pruned (an adopted agent game outlives its log ring). 404
/// NO_LAUNCH when neither knows the key.
async fn require_launch(engine: &Engine, key: &str) -> EngineResult<crate::types::LaunchHandle> {
    if key.is_empty() {
        return Err(EngineError::bad_request("BAD_REQUEST", "key is required"));
    }
    let rows = engine.list_launches().await?.launches;
    if let Some(row) = rows.into_iter().find(|r| r.key == key) {
        return Ok(row);
    }
    if let Some(entry) = agent::get_launch(engine.agent_state(), key) {
        return Ok(crate::types::LaunchHandle {
            key: key.to_string(),
            instance: entry.instance().to_string(),
            version: String::new(),
            account: None,
            running: entry.is_running(),
            agent: true,
            started_at: 0,
            menu_at: None,
            ended_at: None,
        });
    }
    Err(EngineError::not_found("NO_LAUNCH", format!("unknown launch key: {key}")))
}

/// Wait for the mod's state to satisfy `want` ('title' → phase title/in_world,
/// 'in_world' → phase in_world). Polls the registry: the mod reports over HTTP,
/// so there is no in-process event to subscribe to.
async fn wait_for_state(engine: &Engine, key: &str, want: &str, timeout: Duration) -> EngineResult<agent::AgentReport> {
    let deadline = tokio::time::Instant::now() + timeout;
    let wanted: &[&str] = if want == "in_world" { &["in_world"] } else { &["title", "in_world"] };
    loop {
        let Some(entry) = agent::get_launch(engine.agent_state(), key) else {
            return Err(EngineError::api(502, "GAME_GONE", "the game exited before reaching the requested state"));
        };
        let state = agent::state_of(&entry);
        if let Some(s) = state.as_ref().filter(|s| s.phase.as_deref().is_some_and(|p| wanted.contains(&p))) {
            return Ok(s.clone());
        }
        if tokio::time::Instant::now() >= deadline {
            let last = state.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "none".into());
            return Err(EngineError::api(
                504,
                "AGENT_TIMEOUT",
                format!("game did not reach '{want}' within {} s (last phase: {last})", timeout.as_secs()),
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// `Number(v)` clamped, falling back to `default` when non-finite/≤0
/// (JS `Number(b.timeout_s ?? 180)` + the `isFinite` check).
fn timeout_s(body: &Value, default: f64) -> f64 {
    let n = match body.get("timeout_s") {
        None | Some(Value::Null) => default,
        v => agent::js_number(v),
    };
    if !n.is_finite() || n <= 0.0 {
        default
    } else {
        n
    }
}

async fn agent_instances(State(engine): State<Engine>, headers: HeaderMap) -> Response {
    if let Some(r) = guard(&headers, false) {
        return r;
    }
    if let Err(e) = agent_auth(&engine, &headers) {
        return engine_error(&e);
    }
    let list = match crate::instances::list_instances(&engine).await {
        Ok(l) => l,
        Err(e) => return engine_error(&e),
    };
    let instances: Vec<Value> = list
        .into_iter()
        .map(|inst| {
            json!({
                "name": inst.name,
                "version": inst.version,
                "loader": inst.loader.as_str(),
                "capture_tools": inst.capture_tools,
                "running": engine.active_instances().contains(&inst.name),
            })
        })
        .collect();
    axum::Json(json!({ "instances": instances })).into_response()
}

async fn agent_launches(State(engine): State<Engine>, headers: HeaderMap) -> Response {
    if let Some(r) = guard(&headers, false) {
        return r;
    }
    if let Err(e) = agent_auth(&engine, &headers) {
        return engine_error(&e);
    }
    let rows = match engine.list_launches().await {
        Ok(r) => r.launches,
        Err(e) => return engine_error(&e),
    };
    let out: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let state = agent::get_launch(engine.agent_state(), &row.key).and_then(|e| agent::state_of(&e));
            json!({
                "key": row.key,
                "instance": row.instance,
                "running": row.running,
                "agent": row.agent,
                "state": state,
            })
        })
        .collect();
    axum::Json(json!({ "launches": out })).into_response()
}

async fn agent_launch(State(engine): State<Engine>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = guard(&headers, true) {
        return r;
    }
    if let Err(e) = agent_auth(&engine, &headers) {
        return engine_error(&e);
    }
    let parsed = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let name = string_field(&parsed, "instance");
    if name.is_empty() {
        return bad_request("instance is required");
    }
    // Validate before the launch facade creates a buffer (the facade
    // re-validates with the same codes; this keeps the 400 ordering obvious).
    if let Err(e) = agent::normalize_launch_options(&parsed, true) {
        return engine_error(&e);
    }
    let wait_for = match parsed.get("wait_for") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "title" || s == "in_world" => Some(s.clone()),
        _ => return bad_request("wait_for must be 'title' or 'in_world'"),
    };
    let timeout = timeout_s(&parsed, 180.0).clamp(5.0, 600.0);

    // `account` is validated by startLaunch in the JS too: a non-string (or an
    // empty one) is a 400, not a silently ignored override.
    let account = match parsed.get("account") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(_) => return engine_error(&EngineError::bad_request("BAD_ACCOUNT", "account must be a non-empty string")),
    };
    let req = crate::api::game_launch::LaunchRequest {
        agent: true,
        account,
        join: parsed.get("join").cloned().filter(|v| !v.is_null()),
        window: parsed.get("window").cloned().filter(|v| !v.is_null()),
        ..Default::default()
    };
    let reply = match engine.launch_instance(&name, req).await {
        Ok(crate::api::game_launch::LaunchOutcome::Started(r)) => r,
        Ok(crate::api::game_launch::LaunchOutcome::DryRun(_)) => {
            return engine_error(&EngineError::other("unexpected dry-run reply"))
        }
        Err(e) => return engine_error(&e),
    };
    if let Some(want) = wait_for {
        return match wait_for_state(&engine, &reply.key, &want, Duration::from_secs_f64(timeout)).await {
            Ok(state) => axum::Json(json!({ "key": reply.key, "state": state })).into_response(),
            Err(e) => engine_error(&e),
        };
    }
    let state = agent::get_launch(engine.agent_state(), &reply.key).and_then(|e| agent::state_of(&e));
    axum::Json(json!({ "key": reply.key, "state": state })).into_response()
}

async fn agent_stop(State(engine): State<Engine>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = guard(&headers, true) {
        return r;
    }
    if let Err(e) = agent_auth(&engine, &headers) {
        return engine_error(&e);
    }
    let parsed = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let key = string_field(&parsed, "key");
    if key.is_empty() {
        return bad_request("key is required");
    }
    let row = match require_launch(&engine, &key).await {
        Ok(r) => r,
        Err(e) => return engine_error(&e),
    };
    // A key addresses ONE launch: once it has exited, the same instance may
    // already have been relaunched under a new key — stopping by the old key
    // must not kill the new game.
    if !row.running {
        return axum::Json(json!({ "ok": true, "already_stopped": true })).into_response();
    }
    match engine.stop_instance(&row.instance).await {
        Ok(_) => axum::Json(json!({ "ok": true })).into_response(),
        Err(e) => engine_error(&e),
    }
}

async fn agent_state_route(
    State(engine): State<Engine>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(r) = guard(&headers, false) {
        return r;
    }
    if let Err(e) = agent_auth(&engine, &headers) {
        return engine_error(&e);
    }
    let key = query.get("key").cloned().unwrap_or_default();
    let row = match require_launch(&engine, &key).await {
        Ok(r) => r,
        Err(e) => return engine_error(&e),
    };
    let state = agent::get_launch(engine.agent_state(), &key).and_then(|e| agent::state_of(&e));
    axum::Json(json!({ "key": key, "instance": row.instance, "running": row.running, "state": state })).into_response()
}

async fn agent_logs(
    State(engine): State<Engine>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(r) = guard(&headers, false) {
        return r;
    }
    if let Err(e) = agent_auth(&engine, &headers) {
        return engine_error(&e);
    }
    let key = query.get("key").cloned().unwrap_or_default();
    let row = match require_launch(&engine, &key).await {
        Ok(r) => r,
        Err(e) => return engine_error(&e),
    };
    // `Number(cursor)` then `Number.isInteger(cursor) && cursor >= 0`.
    let cursor = match query.get("cursor").filter(|c| !c.trim().is_empty()) {
        Some(raw) => match raw.trim().parse::<f64>() {
            Ok(n) if n.is_finite() && n.fract() == 0.0 && n >= 0.0 => n as u64,
            _ => 0,
        },
        None => 0,
    };
    let Some(view) = engine.launch_log(&key, cursor).await.ok() else {
        // A pruned buffer: answer the empty view, not an error (JS bufferView).
        return axum::Json(json!({ "lines": [], "cursor": cursor, "running": row.running })).into_response();
    };
    // The registry row is the authority on whether the game is still up: an
    // adopted agent game outlives its log ring, and the agent must not read
    // "exited" while /api/agent/state says it is running.
    let running = view.running || row.running;
    axum::Json(json!({ "lines": view.lines, "cursor": view.cursor, "running": running })).into_response()
}

async fn agent_call(State(engine): State<Engine>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = guard(&headers, true) {
        return r;
    }
    if let Err(e) = agent_auth(&engine, &headers) {
        return engine_error(&e);
    }
    let parsed = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let key = string_field(&parsed, "key");
    if key.is_empty() {
        return bad_request("key is required");
    }
    let st = engine.agent_state();
    let Some(entry) = agent::get_launch(st, &key) else {
        return engine_error(&EngineError::not_found("NO_LAUNCH", format!("unknown launch key: {key}")));
    };
    if !entry.is_agent() {
        return engine_error(&EngineError::api(
            409,
            "NOT_AGENT_LAUNCH",
            "this launch was not started with agent: true",
        ));
    }
    if !entry.is_running() {
        return engine_error(&EngineError::api(502, "GAME_GONE", "the game is no longer running"));
    }
    // Floor as well as ceiling: a sub-second timeout_s would abort a call
    // before the mod could answer (callTool clamps to >=1 s; keep both in sync).
    let secs = timeout_s(&parsed, agent::CALL_TIMEOUT_S as f64).clamp(1.0, agent::MAX_CALL_TIMEOUT_S as f64);
    let tool = string_field(&parsed, "tool");
    let args = parsed.get("args");
    let pending = match agent::call_tool(st, &key, &tool, args, (secs * 1000.0).round() as u64) {
        Ok(p) => p,
        Err(e) => return engine_error(&e),
    };
    match pending.wait().await {
        Ok(data) => {
            let data = agent::rewrite_file_results(&data, entry.game_dir().as_deref());
            axum::Json(json!({ "ok": true, "data": data })).into_response()
        }
        Err(e) => engine_error(&e),
    }
}

/// The relay's bridge-owner check (403 BRIDGE_AUTH before the enabled gate).
fn bridge_owner(engine: &Engine, headers: &HeaderMap) -> Result<crate::bridge::BridgeLaunch, Box<Response>> {
    let token = headers.get(crate::bridge::BRIDGE_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    crate::bridge::authenticate(engine, token)
        .ok_or_else(|| Box::new(json_error(403, "BRIDGE_AUTH", "missing or invalid x-espectral-bridge token")))
}

async fn bridge_agent_poll(State(engine): State<Engine>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = guard(&headers, true) {
        return r;
    }
    let owner = match bridge_owner(&engine, &headers) {
        Ok(o) => o,
        Err(r) => return *r,
    };
    if let Err(e) = require_enabled(&engine) {
        return engine_error(&e);
    }
    let parsed = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let key = parsed.get("key").and_then(Value::as_str).filter(|k| !k.is_empty());
    let entry = match agent::launch_for_bridge(engine.agent_state(), Some(&owner), key) {
        Ok(e) => e,
        Err(e) => return engine_error(&e),
    };
    // A parked poll is released when the client hangs up: axum drops the
    // handler future, which drops `await_commands`' guard and puts any
    // handed-over batch back in the queue (never swallowed).
    let commands = agent::await_commands(&entry, Duration::from_millis(agent::POLL_TIMEOUT_MS)).await;
    axum::Json(json!({ "commands": commands, "policy": agent::policy(engine.config()) })).into_response()
}

async fn bridge_agent_result(State(engine): State<Engine>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = guard(&headers, true) {
        return r;
    }
    let owner = match bridge_owner(&engine, &headers) {
        Ok(o) => o,
        Err(r) => return *r,
    };
    if let Err(e) = require_enabled(&engine) {
        return engine_error(&e);
    }
    let parsed = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let key = parsed.get("key").and_then(Value::as_str).filter(|k| !k.is_empty());
    let entry = match agent::launch_for_bridge(engine.agent_state(), Some(&owner), key) {
        Ok(e) => e,
        Err(e) => return engine_error(&e),
    };
    let id = string_field(&parsed, "id");
    if id.is_empty() {
        return bad_request("id is required");
    }
    let accepted = agent::complete_command(
        &entry,
        &id,
        parsed.get("ok") == Some(&Value::Bool(true)),
        parsed.get("data"),
        parsed.get("error"),
    );
    axum::Json(json!({ "ok": true, "accepted": accepted })).into_response()
}

async fn bridge_agent_state(State(engine): State<Engine>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = guard(&headers, true) {
        return r;
    }
    let owner = match bridge_owner(&engine, &headers) {
        Ok(o) => o,
        Err(r) => return *r,
    };
    if let Err(e) = require_enabled(&engine) {
        return engine_error(&e);
    }
    let parsed = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let key = parsed.get("key").and_then(Value::as_str).filter(|k| !k.is_empty());
    let entry = match agent::launch_for_bridge(engine.agent_state(), Some(&owner), key) {
        Ok(e) => e,
        Err(e) => return engine_error(&e),
    };
    let state = agent::set_state(&entry, &parsed);
    engine.emit(crate::events::EngineEvent::AgentState(crate::events::AgentStateEvent {
        key: entry.key().to_string(),
        instance: entry.instance().to_string(),
        phase: state.phase.clone(),
        screen: state.screen.clone(),
        server: state.server.clone(),
        world: state.world.clone(),
        context: state.context.clone(),
        paused: state.paused,
        recording: state.recording,
        updated_at: state.updated_at.unwrap_or_default(),
    }));
    axum::Json(json!({ "ok": true })).into_response()
}

async fn not_found(headers: HeaderMap) -> Response {
    if !host_ok(&headers) {
        return json_error(403, "FORBIDDEN", "unexpected Host header");
    }
    json_error(404, "NOT_FOUND", "no such route")
}

/// The loopback router (exposed for tests).
pub fn router(engine: Engine) -> Router {
    Router::new()
        .route("/api/bridge/visible", post(bridge_visible))
        .route("/api/discord/callback", get(discord_callback))
        // routes/agent.mjs — agent API (Bearer token) + the mod relay
        // (per-launch bridge token).
        .route("/api/agent/instances", get(agent_instances))
        .route("/api/agent/launches", get(agent_launches))
        .route("/api/agent/launch", post(agent_launch))
        .route("/api/agent/stop", post(agent_stop))
        .route("/api/agent/state", get(agent_state_route))
        .route("/api/agent/logs", get(agent_logs))
        .route("/api/agent/call", post(agent_call))
        .route("/api/bridge/agent/poll", post(bridge_agent_poll))
        .route("/api/bridge/agent/result", post(bridge_agent_result))
        .route("/api/bridge/agent/state", post(bridge_agent_state))
        .fallback(not_found)
        .with_state(engine)
}

/// Bind 127.0.0.1:`port` (0 = ephemeral) and serve until engine shutdown.
pub async fn start(engine: &Engine, port: u16) -> EngineResult<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| EngineError::other(format!("[loopback] could not bind 127.0.0.1:{port}: {e}")))?;
    let addr = listener.local_addr()?;
    *engine.identity_discord_state().loopback.addr.lock().unwrap_or_else(|p| p.into_inner()) = Some(addr);
    let app = router(engine.clone());
    let cancel = engine.cancel_token().clone();
    tokio::spawn(async move {
        let r = axum::serve(listener, app).with_graceful_shutdown(async move { cancel.cancelled().await }).await;
        if let Err(e) = r {
            tracing::warn!("[loopback] server stopped: {e}");
        }
    });
    tracing::info!("[loopback] listening on http://{addr}");
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    async fn post(addr: SocketAddr, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
        let mut req = no_proxy().post(format!("http://{addr}{path}")).body(body.to_string());
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    fn no_proxy() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn bridge_route_guards_and_answers() {
        let (d, e) = temp_engine();
        let addr = start(&e, 0).await.unwrap();
        assert_eq!(bound_addr(&e), Some(addr));
        assert_eq!(bridge_url(&e), format!("http://127.0.0.1:{}/api/bridge", addr.port()));

        // Missing x-espectral-client beats bridge auth.
        let token = crate::bridge::mint_token();
        let (s, j) = post(addr, "/api/bridge/visible", &[("x-espectral-bridge", &token)], "{}").await;
        assert_eq!((s, j["error"]["code"].as_str()), (403, Some("FORBIDDEN")));
        // Unknown token.
        let (s, j) = post(addr, "/api/bridge/visible", &[("x-espectral-client", "1")], r#"{"uuids":[]}"#).await;
        assert_eq!((s, j["error"]["code"].as_str()), (403, Some("BRIDGE_AUTH")));
        // Bad JSON.
        let (s, j) = post(addr, "/api/bridge/visible", &[("x-espectral-client", "1")], "{nope").await;
        assert_eq!((s, j["error"]["code"].as_str()), (400, Some("BAD_JSON")));
        // Foreign Host header (DNS rebinding).
        let (s, j) =
            post(addr, "/api/bridge/visible", &[("x-espectral-client", "1"), ("host", "evil.example")], "{}").await;
        assert_eq!((s, j["error"]["code"].as_str()), (403, Some("FORBIDDEN")));

        crate::bridge::attach(&e, "inst", Some("11111111111111111111111111111111"), d.path().into(), &token);
        crate::bridge::seed_cache(
            &e,
            "22222222222222222222222222222222",
            crate::bridge::CacheEntry {
                grant: Some("g".into()),
                loadout: None,
                absent: false,
                expires_at: crate::platform::now_ms() + 60_000,
            },
        );
        let (s, j) = post(
            addr,
            "/api/bridge/visible",
            &[("x-espectral-client", "1"), ("x-espectral-bridge", &token), ("content-type", "application/json")],
            r#"{"uuids":["22222222222222222222222222222222", 5]}"#,
        )
        .await;
        assert_eq!(s, 200);
        assert_eq!(
            j,
            json!({ "players": [{ "uuid": "22222222222222222222222222222222", "grant": "g", "loadout": null }] })
        );
        e.shutdown().await;
    }

    #[tokio::test]
    async fn discord_callback_renders_html() {
        let (_d, e) = temp_engine();
        e.identity_discord_state().set_overrides(|o| o.discord_mock = Some(true));
        let addr = start(&e, 0).await.unwrap();
        let (_url, state) = crate::discord_auth::auth_start_url(&e, None);
        let res = no_proxy()
            .get(format!("http://{addr}/api/discord/callback?code=mock_code&state={state}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
        assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
        let html = res.text().await.unwrap();
        assert!(html.contains("¡Hola, TestPlayer!"));
        assert_eq!(crate::discord_auth::claim_session_user(&e, &state).status, "ok");

        let res = no_proxy()
            .get(format!("http://{addr}/api/discord/callback?error=x&error_description=%3Cscript%3E"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 400);
        let html = res.text().await.unwrap();
        assert!(html.contains("&lt;script&gt;") && !html.contains("<script>"));

        let (_url, state) = crate::discord_auth::auth_start_url(&e, None);
        let res = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(format!("http://{addr}/api/discord/callback?code=mock_code&state={state}&redirect=tauri"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 302);
        assert_eq!(res.headers()["location"], "tauri://localhost/#/");
        e.shutdown().await;
    }

    #[test]
    fn html_escaping_blocks_script_breakout() {
        let u = DiscordUser {
            id: "1".into(),
            name: "</script><b>".into(),
            avatar: Some("\"x".into()),
            extra: Default::default(),
        };
        let html = render_auth_success_html(Some(&u));
        assert!(!html.contains("</script><b>"));
        assert!(html.contains("&lt;/script&gt;&lt;b&gt;"));
        assert!(html.contains(r#"src="&quot;x""#));
        assert!(render_auth_success_html(None).contains("¡Hola, Usuario!"));
    }

    #[test]
    fn host_header_rules() {
        let mk = |h: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::HOST, h.parse().unwrap());
            m
        };
        assert!(host_ok(&HeaderMap::new()));
        assert!(host_ok(&mk("127.0.0.1:4199")));
        assert!(host_ok(&mk("LOCALHOST")));
        assert!(!host_ok(&mk("127.0.0.1.evil.example")));
        assert!(!host_ok(&mk("localhost:abc")));
    }
}
