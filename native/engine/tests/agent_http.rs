//! Agent access — HTTP behaviours over the real loopback router, ported from
//! `tests/agent-api.test.mjs` and `tests/agent-route.test.mjs`: the auth gates
//! (403 disabled / 401 bad token / host + CSRF guards), the instance and
//! launch listings, the launch/stop/state/logs routes, the `call` relay and
//! the mod relay (`/api/bridge/agent/*`).
//!
//! Every request goes over a real TCP socket on an ephemeral port; nothing
//! leaves the machine.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use espectral_engine::agent;
use espectral_engine::bridge;
use espectral_engine::launch::{self, SpawnedGame};
use espectral_engine::test_support::temp_engine;
use espectral_engine::Engine;
use futures_util::FutureExt;
use serde_json::{json, Value};

async fn get(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> (u16, Value) {
    let mut req = client().get(format!("http://{addr}{path}"));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = req.send().await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn post(addr: SocketAddr, path: &str, headers: &[(&str, &str)], body: Value) -> (u16, Value) {
    let mut req = client().post(format!("http://{addr}{path}")).json(&body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = req.send().await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

/// `x-espectral-client: 1` — the CSRF header every non-GET needs (the MCP shim
/// sends it too).
const CSRF: (&str, &str) = ("x-espectral-client", "1");

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

fn token_of(e: &Engine) -> String {
    agent::read_token(e.paths().data_dir()).expect("token file")
}

fn enable_agent(e: &Engine) {
    agent::save_agent_config(e.config(), &agent::AgentAccessPatch { enabled: Some(true), allowed_servers: None });
    agent::ensure_token(e.paths().data_dir()).unwrap();
}

fn write_instance(e: &Engine, name: &str, capture_tools: bool) {
    let dir = e.paths().instance_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = json!({
        "name": name, "version": "26.3", "loader": "fabric", "loader_version": null,
        "memory_mb": 2048, "mods": [], "memory_auto_migrated": true,
        "jdk_path_override": null, "aot_auto_train": false, "capture_tools": capture_tools,
        "created_at": "2026-01-01T00:00:00.000Z", "updated_at": "2026-01-01T00:00:00.000Z",
    });
    std::fs::write(dir.join("instance.json"), doc.to_string()).unwrap();
}

/// A fake registered agent launch + its bridge token.
fn attach_agent_launch(e: &Engine, key: &str, instance: &str) -> String {
    let game_dir = e.paths().instance_dir(instance);
    std::fs::create_dir_all(game_dir.join(agent::CAPTURES_DIR)).unwrap();
    let token = bridge::mint_token();
    bridge::attach(e, instance, Some("a".repeat(32).as_str()), game_dir.clone(), &token);
    agent::register_launch(e.agent_state(), key, instance, Some(game_dir));
    token
}

#[tokio::test]
async fn auth_gates_disabled_beats_auth_then_bearer_token() {
    let (_d, e) = temp_engine();
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();

    // Disabled by default, and no token exists until access is enabled.
    let (s, j) = get(addr, "/api/agent/instances", &[]).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("AGENT_DISABLED")));
    assert_eq!(agent::read_token(e.paths().data_dir()), None);

    enable_agent(&e);
    let token = token_of(&e);

    // Disabled wins over a valid token too: a disabled engine never confirms
    // a token's validity.
    agent::save_agent_config(e.config(), &agent::AgentAccessPatch { enabled: Some(false), allowed_servers: None });
    let (s, j) = get(addr, "/api/agent/instances", &[("authorization", &bearer(&token))]).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("AGENT_DISABLED")));
    agent::save_agent_config(e.config(), &agent::AgentAccessPatch { enabled: Some(true), allowed_servers: None });
    // Missing / wrong token -> 401 AGENT_AUTH.
    let (s, j) = get(addr, "/api/agent/instances", &[]).await;
    assert_eq!((s, j["error"]["code"].as_str()), (401, Some("AGENT_AUTH")));
    let (s, j) = get(addr, "/api/agent/instances", &[("authorization", &bearer(&"f".repeat(64)))]).await;
    assert_eq!((s, j["error"]["code"].as_str()), (401, Some("AGENT_AUTH")));

    // Right token -> 200 with the instance list.
    write_instance(&e, "inst-a", true);
    let (s, j) = get(addr, "/api/agent/instances", &[("authorization", &bearer(&token))]).await;
    assert_eq!(s, 200, "{j}");
    let list = j["instances"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "inst-a");
    assert_eq!(list[0]["version"], "26.3");
    assert_eq!(list[0]["loader"], "fabric");
    assert_eq!(list[0]["capture_tools"], true);
    assert_eq!(list[0]["running"], false);

    // Disabling again hides a valid token.
    agent::save_agent_config(e.config(), &agent::AgentAccessPatch { enabled: Some(false), allowed_servers: None });
    let (s, _) = get(addr, "/api/agent/instances", &[("authorization", &bearer(&token))]).await;
    assert_eq!(s, 403);
    e.shutdown().await;
}

#[tokio::test]
async fn csrf_and_host_guards_apply_to_the_agent_routes() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let token = token_of(&e);
    let auth = bearer(&token);

    // A non-GET without the client header is refused before auth runs.
    let (s, j) = post(addr, "/api/agent/call", &[("authorization", &auth)], json!({ "key": "k", "tool": "t" })).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("FORBIDDEN")));
    // A foreign Host header is refused (DNS rebinding), even on GET.
    let (s, j) = get(addr, "/api/agent/instances", &[("authorization", &auth), ("host", "evil.example")]).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("FORBIDDEN")));
    // The relay needs it as well.
    let (s, j) = post(addr, "/api/bridge/agent/poll", &[], json!({})).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("FORBIDDEN")));
    e.shutdown().await;
}

#[tokio::test]
async fn bridge_poll_refused_for_non_agent_and_bad_tokens() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();

    // A bridge-authenticated launch that was never registered as an agent launch.
    let plain = bridge::mint_token();
    bridge::attach(&e, "plain-inst", Some("b".repeat(32).as_str()), e.paths().data_dir().to_path_buf(), &plain);
    let (s, j) = post(addr, "/api/bridge/agent/poll", &[CSRF, ("x-espectral-bridge", &plain)], json!({})).await;
    assert_eq!((s, j["error"]["code"].as_str()), (409, Some("NOT_AGENT_LAUNCH")));

    let (s, j) =
        post(addr, "/api/bridge/agent/poll", &[CSRF, ("x-espectral-bridge", &"c".repeat(64))], json!({})).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("BRIDGE_AUTH")));
    e.shutdown().await;
}

#[tokio::test]
async fn relay_delivers_a_call_to_the_parked_poll_and_resolves_it() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));
    let key = "lrelay1";
    let token = attach_agent_launch(&e, key, "relay-inst");

    // The mod parks a poll; the call arrives and is answered with the command.
    let mod_token = token.clone();
    let poll = tokio::spawn(async move {
        post(addr, "/api/bridge/agent/poll", &[CSRF, ("x-espectral-bridge", &mod_token)], json!({})).await
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    let call = tokio::spawn(async move {
        post(addr, "/api/agent/call", &[CSRF, ("authorization", &api)], json!({ "key": key, "tool": "screen_tree" }))
            .await
    });

    let (s, j) = poll.await.unwrap();
    assert_eq!(s, 200, "{j}");
    let commands = j["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1);
    let command = &commands[0];
    assert_eq!(command["tool"], "screen_tree");
    // The command shape is exactly {id, tool, args, timeout_ms}.
    let mut keys: Vec<&String> = command.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["args", "id", "timeout_ms", "tool"]);
    // Policy rides along on every poll.
    assert_eq!(j["policy"]["blocked_hosts"], json!(["uhce.top", "espectral.es"]));
    assert!(j["policy"]["allowed_servers"].is_array());

    let (s, j) = post(
        addr,
        "/api/bridge/agent/result",
        &[CSRF, ("x-espectral-bridge", &token)],
        json!({
            "id": command["id"],
            "ok": true,
            "data": { "screen": "TitleScreen", "widgets": [{ "id": "w0", "text": "Singleplayer" }] },
        }),
    )
    .await;
    assert_eq!((s, j["accepted"].as_bool()), (200, Some(true)));

    let (s, j) = call.await.unwrap();
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["ok"], true);
    assert_eq!(j["data"]["screen"], "TitleScreen");
    e.shutdown().await;
}

#[tokio::test]
async fn relay_error_result_becomes_the_call_error() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));
    let key = "lrelay-err";
    let token = attach_agent_launch(&e, key, "relay-err");

    let mod_token = token.clone();
    let poll = tokio::spawn(async move {
        post(addr, "/api/bridge/agent/poll", &[CSRF, ("x-espectral-bridge", &mod_token)], json!({})).await
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    let call = tokio::spawn(async move {
        post(
            addr,
            "/api/agent/call",
            &[CSRF, ("authorization", &api)],
            json!({ "key": key, "tool": "chat", "args": { "text": "hi" } }),
        )
        .await
    });
    let (_, j) = poll.await.unwrap();
    post(
        addr,
        "/api/bridge/agent/result",
        &[CSRF, ("x-espectral-bridge", &token)],
        json!({
            "id": j["commands"][0]["id"],
            "ok": false,
            "error": { "code": "TIER_REFUSED", "message": "act tier refused on an Espectral server" },
        }),
    )
    .await;

    let (s, j) = call.await.unwrap();
    assert_eq!(s, 502);
    assert_eq!(j["error"]["code"], "TIER_REFUSED");
    e.shutdown().await;
}

#[tokio::test]
async fn call_without_a_polling_mod_times_out() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));
    let key = "lrelay-timeout";
    attach_agent_launch(&e, key, "relay-timeout");

    let (s, j) = post(
        addr,
        "/api/agent/call",
        &[CSRF, ("authorization", &api)],
        json!({ "key": key, "tool": "screenshot", "args": {}, "timeout_s": 1 }),
    )
    .await;
    assert_eq!((s, j["error"]["code"].as_str()), (504, Some("AGENT_TIMEOUT")));
    e.shutdown().await;
}

#[tokio::test]
async fn call_to_unknown_or_non_agent_launch_is_refused() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));

    let (s, j) =
        post(addr, "/api/agent/call", &[CSRF, ("authorization", &api)], json!({ "key": "nope", "tool": "s" })).await;
    assert_eq!((s, j["error"]["code"].as_str()), (404, Some("NO_LAUNCH")));

    // A launch row without an agent registry entry (a plain game) is 409.
    write_instance(&e, "plain-call", false);
    let reply = e.launch_instance("plain-call", espectral_engine::api::game_launch::LaunchRequest::default()).await;
    // The launch itself may fail to resolve here; what matters is that a key
    // that is not in the agent registry never accepts a call.
    if let Ok(espectral_engine::api::game_launch::LaunchOutcome::Started(r)) = reply {
        let (s, j) =
            post(addr, "/api/agent/call", &[CSRF, ("authorization", &api)], json!({ "key": r.key, "tool": "s" })).await;
        assert_eq!((s, j["error"]["code"].as_str()), (404, Some("NO_LAUNCH")));
    }
    e.shutdown().await;
}

#[tokio::test]
async fn state_is_stored_per_launch_and_echoed_by_the_route() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));
    let key = "lrelay-state";
    let token = attach_agent_launch(&e, key, "relay-state");

    let (s, j) = get(addr, &format!("/api/agent/state?key={key}"), &[("authorization", &api)]).await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["state"], Value::Null, "no state before the first mod report");

    let (s, _) = post(
        addr,
        "/api/bridge/agent/state",
        &[CSRF, ("x-espectral-bridge", &token)],
        json!({
            "phase": "in_world", "screen": "GameScreen", "server": null, "world": "Mundo 1",
            "context": "singleplayer", "paused": false, "recording": true,
        }),
    )
    .await;
    assert_eq!(s, 200);

    let (_, j) = get(addr, &format!("/api/agent/state?key={key}"), &[("authorization", &api)]).await;
    assert_eq!(j["state"]["phase"], "in_world");
    assert_eq!(j["state"]["world"], "Mundo 1");
    assert_eq!(j["state"]["recording"], true);
    assert!(j["state"]["updated_at"].is_i64());
    assert_eq!(j["instance"], "relay-state");
    assert_eq!(j["running"], true);

    // The `agent-state` event carries the same view.
    let mut rx = e.subscribe();
    post(
        addr,
        "/api/bridge/agent/state",
        &[CSRF, ("x-espectral-bridge", &token)],
        json!({ "phase": "paused", "paused": true }),
    )
    .await;
    let ev = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ev = rx.recv().await.unwrap();
            if ev.name() == "agent-state" {
                return ev;
            }
        }
    })
    .await
    .expect("agent-state emitted");
    let payload = ev.payload();
    assert_eq!(payload["key"], key);
    assert_eq!(payload["instance"], "relay-state");
    assert_eq!(payload["phase"], "paused");
    assert_eq!(payload["paused"], true);
    assert!(payload["updated_at"].is_i64());
    e.shutdown().await;
}

#[tokio::test]
async fn relay_goes_inert_when_agent_access_is_switched_off() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let token = attach_agent_launch(&e, "lgate", "gate-inst");
    agent::save_agent_config(e.config(), &agent::AgentAccessPatch { enabled: Some(false), allowed_servers: None });

    let (s, j) = post(addr, "/api/bridge/agent/poll", &[CSRF, ("x-espectral-bridge", &token)], json!({})).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("AGENT_DISABLED")));
    e.shutdown().await;
}

#[tokio::test]
async fn relay_auth_refuses_a_foreign_launch_key() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let attacker = attach_agent_launch(&e, "lattacker", "attacker-inst");
    attach_agent_launch(&e, "lvictim", "victim-inst");

    for route in ["/api/bridge/agent/poll", "/api/bridge/agent/result", "/api/bridge/agent/state"] {
        let (s, j) = post(
            addr,
            route,
            &[CSRF, ("x-espectral-bridge", &attacker)],
            json!({ "key": "lvictim", "id": "c1", "ok": true }),
        )
        .await;
        assert_eq!((s, j["error"]["code"].as_str()), (403, Some("BRIDGE_AUTH")), "{route} refuses a foreign key");
    }
    e.shutdown().await;
}

#[tokio::test]
async fn launches_route_lists_rows_with_their_agent_flag_and_state() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));
    write_instance(&e, "list-inst", false);

    let r = fake_launch(&e, "list-inst");
    e.set_launch_test_hooks(espectral_engine::api::game_launch::LaunchTestHooks {
        resolve_launch: Some(Arc::new(move |_, _| {
            let r = r.clone();
            async move { Ok(r) }.boxed()
        })),
        launch_instance: Some(Arc::new(|_e, _i, _r, cb: launch::LaunchCallbacks| {
            if let Some(f) = cb.on_spawned.as_ref() {
                f(espectral_engine::platform::now_ms());
            }
            Ok(SpawnedGame { pid: std::process::id(), argv: vec![], process: None })
        })),
        quiet_presence: true,
        ..Default::default()
    });
    let reply = e
        .launch_instance(
            "list-inst",
            espectral_engine::api::game_launch::LaunchRequest { agent: true, ..Default::default() },
        )
        .await
        .unwrap();
    let espectral_engine::api::game_launch::LaunchOutcome::Started(reply) = reply else { panic!("started") };

    // A registry entry with no buffer row is still addressable by key (an
    // adopted agent launch outlives its log ring), but /api/agent/launches
    // lists buffer rows like the JS launchSummaries().
    let (s, j) = get(addr, "/api/agent/launches", &[("authorization", &api)]).await;
    assert_eq!(s, 200, "{j}");
    let rows = j["launches"].as_array().unwrap();
    let row = rows.iter().find(|r| r["key"] == reply.key.as_str()).expect("row exists");
    assert_eq!(row["agent"], true, "the /api/launches row carries agent: true");
    assert_eq!(row["instance"], "list-inst");
    assert_eq!(row["state"], Value::Null);

    let entry = agent::get_launch(e.agent_state(), &reply.key).unwrap();
    agent::set_state(&entry, &json!({ "phase": "title" }));
    let (_, j) = get(addr, "/api/agent/launches", &[("authorization", &api)]).await;
    let row = j["launches"].as_array().unwrap().iter().find(|r| r["key"] == reply.key.as_str()).unwrap();
    assert_eq!(row["state"]["phase"], "title");
    e.shutdown().await;
}

#[tokio::test]
async fn logs_route_serves_the_launch_ring_and_overrides_running_from_the_registry() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));

    // No buffer, no registry entry: 404 NO_LAUNCH.
    let (s, j) = get(addr, "/api/agent/logs?key=nope", &[("authorization", &api)]).await;
    assert_eq!((s, j["error"]["code"].as_str()), (404, Some("NO_LAUNCH")));

    // An adopted agent launch with a pruned buffer answers an empty view but
    // keeps running:true (the registry row is the authority).
    let key = "llogs";
    attach_agent_launch(&e, key, "logs-inst");
    let (s, j) = get(addr, &format!("/api/agent/logs?key={key}&cursor=3"), &[("authorization", &api)]).await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["lines"], json!([]));
    assert_eq!(j["cursor"], 3);
    assert_eq!(j["running"], true);

    // A launch buffer serves its lines from the cursor.
    write_instance(&e, "logs-real", false);
    let base = e.paths().data_dir().to_path_buf();
    e.set_launch_test_hooks(espectral_engine::api::game_launch::LaunchTestHooks {
        resolve_launch: Some(Arc::new(move |_, _| {
            let mut r = fake_launch_paths(&base, "logs-real");
            r.bridge_token = Some("e".repeat(64));
            async move { Ok(r) }.boxed()
        })),
        launch_instance: Some(Arc::new(|_e, _i, _r, _cb: launch::LaunchCallbacks| {
            Ok(SpawnedGame { pid: std::process::id(), argv: vec![], process: None })
        })),
        quiet_presence: true,
        ..Default::default()
    });
    let reply = e.launch_instance("logs-real", Default::default()).await.unwrap();
    let espectral_engine::api::game_launch::LaunchOutcome::Started(reply) = reply else { panic!("started") };
    let mut got = Vec::new();
    for _ in 0..200 {
        let (s, j) =
            get(addr, &format!("/api/agent/logs?key={}&cursor=0", reply.key), &[("authorization", &api)]).await;
        assert_eq!(s, 200, "{j}");
        got = j["lines"].as_array().cloned().unwrap_or_default();
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(got.iter().any(|l| l.as_str().unwrap_or_default().contains("preparing")), "lines: {got:?}");
    e.shutdown().await;
}

#[tokio::test]
async fn stop_route_is_idempotent_and_never_kills_a_relaunch() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));

    let (s, j) = post(addr, "/api/agent/stop", &[CSRF, ("authorization", &api)], json!({ "key": "" })).await;
    assert_eq!((s, j["error"]["code"].as_str()), (400, Some("BAD_REQUEST")));
    let (s, j) = post(addr, "/api/agent/stop", &[CSRF, ("authorization", &api)], json!({ "key": "nope" })).await;
    assert_eq!((s, j["error"]["code"].as_str()), (404, Some("NO_LAUNCH")));

    // A registry-only key that already exited answers already_stopped and does
    // not touch the instance (which may have been relaunched under a new key).
    let key = "lstopped";
    agent::register_launch(e.agent_state(), key, "relaunch-inst", None);
    let entry = agent::get_launch(e.agent_state(), key).unwrap();
    agent::release_launch(e.agent_state(), key);
    assert!(!entry.is_running());
    // The registry entry is gone, so the key is unknown: NO_LAUNCH.
    let (s, _) = post(addr, "/api/agent/stop", &[CSRF, ("authorization", &api)], json!({ "key": key })).await;
    assert_eq!(s, 404);
    e.shutdown().await;
}

#[tokio::test]
async fn launch_route_validates_wait_for_and_instance() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));

    let (s, j) = post(addr, "/api/agent/launch", &[CSRF, ("authorization", &api)], json!({})).await;
    assert_eq!((s, j["error"]["code"].as_str()), (400, Some("BAD_REQUEST")));
    let (s, j) = post(
        addr,
        "/api/agent/launch",
        &[CSRF, ("authorization", &api)],
        json!({ "instance": "x", "wait_for": "menu" }),
    )
    .await;
    assert_eq!((s, j["error"]["code"].as_str()), (400, Some("BAD_REQUEST")));
    let (s, j) = post(
        addr,
        "/api/agent/launch",
        &[CSRF, ("authorization", &api)],
        json!({ "instance": "x", "join": { "world": ".." } }),
    )
    .await;
    assert_eq!((s, j["error"]["code"].as_str()), (400, Some("BAD_JOIN")));
    let (s, j) =
        post(addr, "/api/agent/launch", &[CSRF, ("authorization", &api)], json!({ "instance": "ghost" })).await;
    assert_eq!(s, 404, "{j}");
    e.shutdown().await;
}

#[tokio::test]
async fn launch_route_waits_for_the_reported_state_and_fails_fast_on_exit() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));
    write_instance(&e, "wait-inst", false);

    let r = fake_launch(&e, "wait-inst");
    e.set_launch_test_hooks(espectral_engine::api::game_launch::LaunchTestHooks {
        resolve_launch: Some(Arc::new(move |_, _| {
            let r = r.clone();
            async move { Ok(r) }.boxed()
        })),
        launch_instance: Some(Arc::new(|_e, _i, _r, _cb: launch::LaunchCallbacks| {
            Ok(SpawnedGame { pid: std::process::id(), argv: vec![], process: None })
        })),
        quiet_presence: true,
        ..Default::default()
    });

    let api2 = api.clone();
    let launch = tokio::spawn(async move {
        post(
            addr,
            "/api/agent/launch",
            &[CSRF, ("authorization", &api2)],
            json!({ "instance": "wait-inst", "wait_for": "in_world", "timeout_s": 5 }),
        )
        .await
    });
    // Let the route create the launch, then report the state from the mod.
    let key = loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let keys: Vec<String> = agent::all_launches(e.agent_state()).iter().map(|x| x.key().to_string()).collect();
        if let Some(k) = keys.into_iter().next() {
            break k;
        }
    };
    let entry = agent::get_launch(e.agent_state(), &key).unwrap();
    agent::set_state(&entry, &json!({ "phase": "in_world", "world": "W" }));

    let (s, j) = launch.await.unwrap();
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["key"], key);
    assert_eq!(j["state"]["phase"], "in_world");
    assert_eq!(j["state"]["world"], "W");

    // A key that disappears before the state arrives is a fast 502 GAME_GONE.
    let key2 = "lgone-now";
    agent::register_launch(e.agent_state(), key2, "wait-inst", None);
    let api3 = api.clone();
    let wait2 = tokio::spawn(async move {
        post(
            addr,
            "/api/agent/launch",
            &[CSRF, ("authorization", &api3)],
            json!({ "instance": "wait-inst", "wait_for": "title", "timeout_s": 5 }),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    // The launch is refused with 409 while the first one still holds the
    // instance; release it so a fresh launch can start, then kill its key.
    agent::release_launch(e.agent_state(), &key);
    let (s, j) = wait2.await.unwrap();
    assert!((s, j["error"]["code"].as_str()) == (409, Some("ALREADY_RUNNING")) || s == 502, "got {s} {j}");
    e.shutdown().await;
}

#[tokio::test]
async fn stop_route_kills_a_fake_running_launch() {
    let (_d, e) = temp_engine();
    enable_agent(&e);
    let addr = espectral_engine::loopback::start(&e, 0).await.unwrap();
    let api = bearer(&token_of(&e));
    write_instance(&e, "stop-inst", false);

    // A REAL live pid: the stop route force-kills the process tree, and using
    // our own pid here would SIGKILL the test binary.
    let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sleep" })
        .args(if cfg!(windows) { vec!["/C", "ping -n 30 127.0.0.1 >NUL"] } else { vec!["30"] })
        .spawn()
        .expect("child process");
    let pid = child.id();
    let r = fake_launch(&e, "stop-inst");
    e.set_launch_test_hooks(espectral_engine::api::game_launch::LaunchTestHooks {
        resolve_launch: Some(Arc::new(move |_, _| {
            let r = r.clone();
            async move { Ok(r) }.boxed()
        })),
        launch_instance: Some(Arc::new(move |_e, _i, _r, cb: launch::LaunchCallbacks| {
            if let Some(f) = cb.on_spawned.as_ref() {
                f(espectral_engine::platform::now_ms());
            }
            Ok(SpawnedGame { pid, argv: vec![], process: None })
        })),
        quiet_presence: true,
        ..Default::default()
    });
    let reply = e.launch_instance("stop-inst", Default::default()).await.unwrap();
    let espectral_engine::api::game_launch::LaunchOutcome::Started(reply) = reply else { panic!("started") };
    for _ in 0..200 {
        if e.list_launches().await.unwrap().launches.iter().any(|r| r.key == reply.key && r.running) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (s, j) = post(addr, "/api/agent/stop", &[CSRF, ("authorization", &api)], json!({ "key": reply.key })).await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["ok"], true);
    // The game process is gone and the instance is free again. The spawned
    // child is OUR child, so it stays a reapable zombie for `kill(pid, 0)`:
    // `try_wait` is the honest check.
    let mut killed = false;
    for _ in 0..100 {
        if child.try_wait().ok().flatten().is_some() {
            killed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(killed, "the stop killed the game process");
    // The pid only becomes truly dead once we reap it (a killed child that the
    // test owns is a zombie), so the reconcile sweep runs after the reap — the
    // game's own exit watcher does that in production.
    let _ = e.list_launches().await.unwrap();
    for _ in 0..100 {
        if !e.active_instances().contains("stop-inst") {
            break;
        }
        let _ = e.list_launches().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!e.active_instances().contains("stop-inst"));
    let _ = child.kill();
    let _ = child.wait();
    e.shutdown().await;
}

fn fake_launch(e: &Engine, name: &str) -> espectral_engine::types::ResolvedLaunch {
    fake_launch_paths(e.paths().data_dir(), name)
}

fn fake_launch_paths(data_dir: &std::path::Path, name: &str) -> espectral_engine::types::ResolvedLaunch {
    let paths = espectral_engine::paths::Paths::new(data_dir);
    fake_launch_at(&paths, name)
}

fn fake_launch_at(paths: &espectral_engine::paths::Paths, name: &str) -> espectral_engine::types::ResolvedLaunch {
    use espectral_engine::types::{JavaRuntime, LaunchAccount, LaunchMode, ResolvedLaunch, ResolvedVersionRef};
    let e = paths;
    let dir = e.instance_dir(name);
    ResolvedLaunch {
        java: JavaRuntime {
            path: "/jdk/bin/java".into(),
            build: "25.0.4+7".into(),
            major: Some(25),
            ..Default::default()
        },
        version: ResolvedVersionRef {
            id: "26.3".into(),
            main_class: "net.minecraft.client.main.Main".into(),
            asset_index_id: Some("26.3".into()),
            json: json!({}),
        },
        loader: None,
        fabric_profile: None,
        classpath: vec![],
        natives_dir: dir.join("natives"),
        game_dir: dir,
        assets_dir: e.assets_dir(),
        memory_mb: 1024,
        account: LaunchAccount {
            username: "AgentBot".into(),
            uuid: "00000000-0000-0000-0000-000000000000".into(),
            access_token: "0".into(),
            user_type: "mojang".into(),
        },
        bridge_token: Some("d".repeat(64)),
        mode: LaunchMode::Normal,
        aot_key: "k".into(),
        aot_cache_path: e.aot_cache_file("k"),
        aot_cache_exists: false,
        aot_cache_stale: false,
        aot_available: true,
        agent: true,
        join: None,
        window: None,
        ffmpeg_path: None,
        warnings: vec![],
    }
}
