//! Loopback HTTP listener (NEW — no JS module): the only HTTP the native
//! engine still serves, because external processes call it:
//!
//! - `POST /api/bridge/visible` — the in-game Espectral mod (headers
//!   `x-espectral-client: 1` and `x-espectral-bridge: <per-launch token>`,
//!   body `{ uuids: [...] }`) -> [`crate::bridge::visible_for_token`]
//!   (the same logic `Engine::bridge_visible` exposes);
//! - `GET /api/discord/callback` — Discord's OAuth redirect (registered as
//!   `http://127.0.0.1:4199/api/discord/callback`) -> `Engine::discord_callback`,
//!   answering the completion / error HTML from routes/discord.mjs.
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! Binds 127.0.0.1 only, port `ESPECTRAL_PORT` or 4199. Keeps the JS guards:
//! the Host header must be `127.0.0.1|localhost[:port]` (DNS-rebinding), and
//! every non-GET needs `x-espectral-client: 1` (CSRF). Errors use the JS
//! envelope `{ error: { code, message } }`. Stops on `Engine::cancel_token()`.
//!
//! Not served here (owned elsewhere / not ported): the agent relay
//! `/api/bridge/agent/*` (routes/agent.mjs) and the session cookie on the
//! callback (the native UI claims the session via `discord_claim`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};

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
