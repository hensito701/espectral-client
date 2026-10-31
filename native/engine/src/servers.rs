//! Port of `src/engine/servers.mjs` — Espectral server status (MAIN + 24/7
//! Meetups) via mcsrvstat v3, 150 s cache.
//!
//! **Owner: content.** Status: PORTED.
//!
//! - A User-Agent is required by the API (the engine client always sends one).
//! - 150 s cache per host (> 2x the UI's 60 s poll, so most polls hit it).
//! - `online:false` answers map to offline WITHOUT `error`; only transport
//!   failures (HTTP status, network, timeout) set `error`.
//!
//! Cache lives in ContentState (api/content.rs); the API base and timeout come
//! from `ContentSettings` (tests point them at a mock server).

use serde_json::Value;

use crate::engine::Engine;
use crate::types::ServerStatus;

/// Hosts shown on the home screen.
pub const HOSTS: [&str; 2] = ["main.uhce.top", "24h.uhce.top"];
/// mcsrvstat API base (`<base>/3/<host>`).
pub const MCSRVSTAT_API: &str = "https://api.mcsrvstat.us";
/// servers.mjs CACHE_TTL_MS.
pub const CACHE_TTL_MS: i64 = 150_000;

/// Status for one host (cached). Never fails: always returns a shape.
pub async fn get_server_status(engine: &Engine, host: &str) -> ServerStatus {
    {
        let cache = engine.content_state().server_status.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, status)) = cache.get(host) {
            if crate::platform::now_ms() - at < CACHE_TTL_MS {
                return status.clone();
            }
        }
    }
    let status = fetch_status(engine, host).await;
    engine
        .content_state()
        .server_status
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(host.to_string(), (crate::platform::now_ms(), status.clone()));
    status
}

/// Status for every host, in HOSTS order.
pub async fn get_all_server_statuses(engine: &Engine) -> Vec<ServerStatus> {
    futures_util::future::join_all(HOSTS.iter().map(|h| get_server_status(engine, h))).await
}

async fn fetch_status(engine: &Engine, host: &str) -> ServerStatus {
    let settings = engine.content_settings();
    let url = format!("{}/3/{}", settings.mcsrvstat_api.trim_end_matches('/'), crate::mods::encode_uri_component(host));
    let res = engine.http().client().get(&url).timeout(settings.server_status_timeout).send().await;
    match res {
        Ok(r) if !r.status().is_success() => {
            offline_status(host, Some(format!("mcsrvstat HTTP {}", r.status().as_u16())))
        }
        Ok(r) => match r.json::<Value>().await {
            Ok(data) => map_status(host, &data),
            Err(e) if e.is_timeout() => offline_status(host, Some("request timed out".into())),
            Err(e) => offline_status(host, Some(e.to_string())),
        },
        Err(e) if e.is_timeout() => offline_status(host, Some("request timed out".into())),
        Err(e) => offline_status(host, Some(e.to_string())),
    }
}

fn int_of(v: Option<&Value>) -> Option<i64> {
    crate::config::json_integer(v)
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => {
            a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect()
        }
        _ => vec![],
    }
}

fn map_status(host: &str, data: &Value) -> ServerStatus {
    let motd = data.get("motd").cloned().unwrap_or(Value::Null);
    let players = data.get("players").cloned().unwrap_or(Value::Null);
    let s = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_string);
    ServerStatus {
        host: host.into(),
        online: data.get("online") == Some(&Value::Bool(true)),
        hostname: s("hostname").unwrap_or_else(|| host.into()),
        ip: s("ip"),
        port: int_of(data.get("port")).and_then(|p| u16::try_from(p).ok()).unwrap_or(25565),
        version: s("version"),
        software: s("software"),
        motd_raw: str_list(motd.get("raw")),
        motd_clean: str_list(motd.get("clean")),
        players_online: int_of(players.get("online")).and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
        players_max: int_of(players.get("max")).and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
        icon: s("icon").filter(|i| !i.is_empty()),
        fetched_at: crate::platform::now_ms(),
        error: None,
    }
}

fn offline_status(host: &str, error: Option<String>) -> ServerStatus {
    ServerStatus {
        host: host.into(),
        online: false,
        hostname: host.into(),
        ip: None,
        port: 25565,
        version: None,
        software: None,
        motd_raw: vec![],
        motd_clean: vec![],
        players_online: 0,
        players_max: 0,
        icon: None,
        fetched_at: crate::platform::now_ms(),
        error: error.filter(|e| !e.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use serde_json::json;

    #[tokio::test]
    async fn maps_online_offline_and_errors_with_cache() {
        let server = MockServer::start(|req| {
            if req.path.ends_with("/main.uhce.top") {
                assert!(req.header("user-agent").is_some());
                MockResponse::json(&json!({
                    "online": true, "ip": "1.2.3.4", "port": 25566, "hostname": "main.uhce.top", "version": "1.21.11",
                    "motd": { "raw": ["§aHi"], "clean": ["Hi"] }, "players": { "online": 7, "max": 100 },
                    "icon": "data:image/png;base64,AAA"
                }))
            } else {
                MockResponse::status(503)
            }
        })
        .await;
        let (_d, e) = temp_engine();
        e.update_content_settings(|s| s.mcsrvstat_api = server.url(""));
        let all = get_all_server_statuses(&e).await;
        assert_eq!(all.len(), 2);
        let main = &all[0];
        assert!(main.online);
        assert_eq!(
            (main.ip.as_deref(), main.port, main.players_online, main.players_max),
            (Some("1.2.3.4"), 25566, 7, 100)
        );
        assert_eq!(main.motd_clean, vec!["Hi"]);
        assert_eq!(main.icon.as_deref(), Some("data:image/png;base64,AAA"));
        assert!(main.error.is_none());
        let off = &all[1];
        assert!(!off.online);
        assert_eq!(off.error.as_deref(), Some("mcsrvstat HTTP 503"));
        assert_eq!(off.ip, None);
        let v = serde_json::to_value(off).unwrap();
        assert_eq!(v["ip"], Value::Null);
        assert!(v.get("icon").is_none());

        let hits = server.hits();
        get_all_server_statuses(&e).await;
        assert_eq!(server.hits(), hits, "second poll is served from the cache");
    }

    #[test]
    fn online_false_has_no_error() {
        let s = map_status("x", &json!({ "online": false, "port": 1.5 }));
        assert!(!s.online && s.error.is_none());
        assert_eq!(s.port, 25565);
        assert_eq!(s.hostname, "x");
    }
}
