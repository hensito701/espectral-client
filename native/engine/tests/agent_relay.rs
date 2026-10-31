//! Agent access — ported behaviours from `tests/agent-api.test.mjs`:
//! launch-option validation, the command registry + queue semantics
//! (release/poll/timeout/one-waiter), the bridge owner rules and the
//! file-result rewriting. Everything runs in a temp data dir; nothing leaves
//! the machine.

use std::collections::HashSet;

use espectral_engine::agent::{self, AgentState};
use espectral_engine::bridge::{self, BridgeLaunch};
use espectral_engine::test_support::temp_engine;
use espectral_engine::Engine;
use serde_json::{json, Value};

fn registered(st: &AgentState, key: &str, instance: &str) -> std::sync::Arc<agent::AgentLaunch> {
    agent::register_launch(st, key, instance, None)
}

/// A bridge-authenticated launch (what the game gets at spawn).
fn attach_bridge(e: &Engine, instance: &str) -> String {
    let token = bridge::mint_token();
    bridge::attach(e, instance, Some("a".repeat(32).as_str()), e.paths().data_dir().to_path_buf(), &token);
    token
}

fn owner(e: &Engine, token: &str) -> BridgeLaunch {
    bridge::authenticate(e, token).expect("bridge token registered")
}

fn code_of(err: &espectral_engine::EngineError) -> (u16, String) {
    let env = err.envelope();
    (env.status, env.code)
}

#[test]
fn normalize_join_world_and_server() {
    assert_eq!(
        agent::normalize_join(Some(&json!({ "world": "Mundo 1" }))).unwrap(),
        Some(agent::JoinOption { world: Some("Mundo 1".into()), server: None })
    );
    assert_eq!(
        agent::normalize_join(Some(&json!({ "server": "mc.example.com:25565" }))).unwrap(),
        Some(agent::JoinOption { world: None, server: Some("mc.example.com:25565".into()) })
    );
    assert_eq!(
        agent::normalize_join(Some(&json!({ "server": "[::1]:25565" }))).unwrap(),
        Some(agent::JoinOption { world: None, server: Some("[::1]:25565".into()) })
    );
    assert_eq!(agent::normalize_join(None).unwrap(), None);
    assert_eq!(agent::normalize_join(Some(&Value::Null)).unwrap(), None);

    let bad = [
        json!({ "world": ".." }),
        json!({ "world": "../etc" }),
        json!({ "world": "a/b" }),
        json!({ "world": "a\\b" }),
        json!({ "world": "w", "server": "h" }),
        json!({}),
        json!({ "server": "has space" }),
        json!({ "server": "host:notaport" }),
        json!("localhost"),
        // A leading '-' would be parsed as the next option instead of a value.
        json!({ "world": "--width" }),
        json!({ "world": "-Survival" }),
    ];
    for input in bad {
        let err = agent::normalize_join(Some(&input)).expect_err(&format!("expected BAD_JOIN for {input}"));
        assert_eq!(code_of(&err), (400, "BAD_JOIN".into()), "{input}");
    }
}

#[test]
fn normalize_window_limits_and_agent_only_hidden() {
    assert_eq!(
        agent::normalize_window(Some(&json!({ "width": 1080, "height": 1920 })), false).unwrap(),
        Some(agent::WindowOption { width: 1080, height: 1920, hidden: false })
    );
    assert_eq!(
        agent::normalize_window(Some(&json!({ "width": 1080, "height": 1920, "hidden": true })), true).unwrap(),
        Some(agent::WindowOption { width: 1080, height: 1920, hidden: true })
    );
    let err = agent::normalize_window(Some(&json!({ "width": 1080, "height": 1920, "hidden": true })), false)
        .expect_err("hidden is agent-only");
    assert_eq!(code_of(&err), (400, "BAD_WINDOW".into()));

    for win in [
        json!({ "width": 63, "height": 100 }),
        json!({ "width": 100, "height": 63 }),
        json!({ "width": 7681, "height": 100 }),
        json!({ "width": 100, "height": 4321 }),
        json!({ "width": 1.5, "height": 100 }),
        json!({ "width": "wide", "height": 100 }),
        json!({ "width": 100 }),
    ] {
        let err = agent::normalize_window(Some(&win), true).expect_err(&format!("expected BAD_WINDOW for {win}"));
        assert_eq!(code_of(&err), (400, "BAD_WINDOW".into()), "{win}");
    }
}

#[test]
fn normalize_agent_only_booleans() {
    assert!(!agent::normalize_agent(None).unwrap());
    assert!(!agent::normalize_agent(Some(&Value::Null)).unwrap());
    assert!(agent::normalize_agent(Some(&Value::Bool(true))).unwrap());
    let err = agent::normalize_agent(Some(&json!("yes"))).expect_err("a string is not a boolean");
    assert_eq!(code_of(&err), (400, "BAD_AGENT".into()));
}

#[test]
fn launch_options_force_agent_and_validate_the_bundle() {
    let opts = agent::normalize_launch_options(&json!({ "join": { "world": "W" } }), true).unwrap();
    assert!(opts.agent);
    assert_eq!(opts.join.unwrap().world.as_deref(), Some("W"));
    // `agent` in the body is honored for plain launches.
    assert!(agent::normalize_launch_options(&json!({ "agent": true }), false).unwrap().agent);
    let err = agent::normalize_launch_options(&json!({ "agent": "yes" }), false).unwrap_err();
    assert_eq!(code_of(&err), (400, "BAD_AGENT".into()));
    // A non-object body is treated as {} (JS `body && typeof body === 'object'`).
    assert_eq!(agent::normalize_launch_options(&Value::Null, false).unwrap(), agent::LaunchOptions::default());
}

#[test]
fn game_arg_fragment_order() {
    let opts = agent::LaunchOptions {
        agent: true,
        join: Some(agent::JoinOption { world: Some("Mundo 1".into()), server: None }),
        window: Some(agent::WindowOption { width: 1080, height: 1920, hidden: true }),
    };
    assert_eq!(
        agent::game_arg_fragment(&opts),
        vec!["--quickPlaySingleplayer", "Mundo 1", "--width", "1080", "--height", "1920"]
    );
    let server = agent::LaunchOptions {
        agent: true,
        join: Some(agent::JoinOption { world: None, server: Some("play.example.com:25566".into()) }),
        window: None,
    };
    assert_eq!(agent::game_arg_fragment(&server), vec!["--quickPlayMultiplayer", "play.example.com:25566"]);
    assert!(agent::game_arg_fragment(&agent::LaunchOptions::default()).is_empty());
}

#[test]
fn registry_re_register_on_spawn_keeps_queued_commands() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let key = "lmerge";
    agent::register_launch(st, key, "merge-inst", None);
    let pending = agent::call_tool(st, key, "screen_tree", None, 5000).unwrap();
    let entry = agent::register_launch(st, key, "merge-inst", Some("/games/merge".into()));
    assert_eq!(entry.game_dir(), Some("/games/merge".into()));

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let commands = rt.block_on(agent::await_commands(&entry, std::time::Duration::from_millis(50)));
    assert_eq!(commands.len(), 1, "the command issued before spawn is still queued");
    // The result must reach the ORIGINAL call: a re-register must not orphan it.
    assert!(agent::complete_command(&entry, &commands[0].id, true, Some(&json!({ "screen": "TitleScreen" })), None));
    let data = rt.block_on(pending.wait()).unwrap();
    assert_eq!(data, json!({ "screen": "TitleScreen" }));
}

#[tokio::test]
async fn batch_is_delivered_to_exactly_one_parked_poll() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let key = "lone-waiter";
    let entry = registered(st, key, "one-inst");

    let first = agent::await_commands(&entry, std::time::Duration::from_millis(200));
    let second = agent::await_commands(&entry, std::time::Duration::from_millis(200));
    let pending = agent::call_tool(st, key, "click", Some(&json!({ "x": 10, "y": 20 })), 5000).unwrap();
    let (a, b) = tokio::join!(first, second);
    let delivered: Vec<_> = [a, b].into_iter().filter(|c| !c.is_empty()).collect();
    assert_eq!(delivered.len(), 1, "the batch is handed out once, never replayed to a second poll");
    assert_eq!(delivered[0][0].tool, "click");
    drop(pending);
    agent::release_launch(st, key);
}

#[tokio::test]
async fn timed_out_call_is_dropped_not_executed_late() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let key = "llate";
    let entry = registered(st, key, "late-inst");

    let pending = agent::call_tool(st, key, "record_start", None, 1000).unwrap();
    let err = pending.wait().await.expect_err("no mod answered");
    assert_eq!(code_of(&err), (504, "AGENT_TIMEOUT".into()));
    let leftover = agent::await_commands(&entry, std::time::Duration::from_millis(10)).await;
    assert!(leftover.is_empty(), "the abandoned command must not reach the mod on its next poll");
}

#[test]
fn queue_is_bounded() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let key = "lqueue";
    registered(st, key, "queue-inst");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut pending = Vec::new();
    for _ in 0..agent::MAX_QUEUE {
        pending.push(agent::call_tool(st, key, "screenshot", None, 1000).unwrap());
    }
    let err = agent::call_tool(st, key, "screenshot", None, 1000).err().expect("queue full");
    assert_eq!(code_of(&err), (429, "QUEUE_FULL".into()));
    agent::release_launch(st, key);
    rt.block_on(async {
        for p in pending {
            let _ = p.wait().await;
        }
    });
}

#[tokio::test]
async fn release_fails_inflight_and_releases_parked_polls() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let key = "lrelay-gone";
    let entry = registered(st, key, "relay-gone");

    // A parked poll is released empty on exit (the mod thread must not hang).
    let parked = tokio::spawn({
        let entry = entry.clone();
        async move { agent::await_commands(&entry, std::time::Duration::from_secs(5)).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let pending = agent::call_tool(st, key, "screenshot", None, 30_000).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let first = parked.await.unwrap();
    assert_eq!(first.len(), 1, "the queued call reached the parked poll");

    let parked2 = tokio::spawn({
        let entry = entry.clone();
        async move { agent::await_commands(&entry, std::time::Duration::from_secs(5)).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    agent::release_launch(st, key);
    let err = pending.wait().await.expect_err("the game exited");
    assert_eq!(code_of(&err), (502, "GAME_GONE".into()));
    assert!(parked2.await.unwrap().is_empty(), "the parked poll is released empty");
    // The key is gone: further calls are refused, not queued into the void.
    let after = agent::call_tool(st, key, "screenshot", None, 1000).err().expect("launch released");
    assert_eq!(code_of(&after), (404, "NO_LAUNCH".into()));
}

#[tokio::test]
async fn bridge_owner_and_agent_gates() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    // A bridge-authenticated launch that was never registered as an agent launch.
    let plain_token = attach_bridge(&e, "plain-inst");
    let plain = owner(&e, &plain_token);
    let err = agent::launch_for_bridge(st, Some(&plain), None).unwrap_err();
    assert_eq!(code_of(&err), (409, "NOT_AGENT_LAUNCH".into()));

    // A bad token is BRIDGE_AUTH.
    let err = agent::launch_for_bridge(st, None, None).unwrap_err();
    assert_eq!(code_of(&err), (403, "BRIDGE_AUTH".into()));

    // Keys are not secret: a token must only address its own launch.
    let attacker_token = attach_bridge(&e, "attacker-inst");
    let attacker = owner(&e, &attacker_token);
    registered(st, "lattacker", "attacker-inst");
    registered(st, "lvictim", "victim-inst");
    let err = agent::launch_for_bridge(st, Some(&attacker), Some("lvictim")).unwrap_err();
    assert_eq!(code_of(&err), (403, "BRIDGE_AUTH".into()));
    let ok = agent::launch_for_bridge(st, Some(&attacker), Some("lattacker")).unwrap();
    assert_eq!(ok.key(), "lattacker");
    // No key: resolved by instance (the mod only knows its bridge token).
    assert_eq!(agent::launch_for_bridge(st, Some(&attacker), None).unwrap().key(), "lattacker");
}

#[tokio::test]
async fn a_bridge_token_only_gets_its_own_launch_commands() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let attacker_token = attach_bridge(&e, "attacker-inst");
    let attacker = owner(&e, &attacker_token);
    registered(st, "lattacker", "attacker-inst");
    registered(st, "lvictim", "victim-inst");

    let victim = agent::call_tool(st, "lvictim", "screenshot", None, 400).unwrap();
    let own = agent::call_tool(st, "lattacker", "screen_tree", None, 5000).unwrap();
    let own_id = own.command().id.clone();
    let entry = agent::launch_for_bridge(st, Some(&attacker), None).unwrap();
    let polled = agent::await_commands(&entry, std::time::Duration::from_millis(500)).await;
    assert_eq!(polled.len(), 1);
    assert_eq!(polled[0].id, own_id, "only its own launch commands are delivered");

    let err = victim.wait().await.expect_err("never answered");
    assert_eq!(code_of(&err), (504, "AGENT_TIMEOUT".into()));
    agent::release_launch(st, "lvictim");
    agent::release_launch(st, "lattacker");
}

#[tokio::test]
async fn state_is_stored_per_launch_and_echoed() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let key = "lrelay-state";
    let token = attach_bridge(&e, "relay-state");
    let entry = registered(st, key, "relay-state");
    registered(st, "other-state", "other-inst");
    assert_eq!(agent::state_of(&entry), None, "no state before the first mod report");

    let state = agent::set_state(
        &entry,
        &json!({
            "phase": "in_world", "screen": "GameScreen", "server": null, "world": "Mundo 1",
            "context": "singleplayer", "paused": false, "recording": true,
        }),
    );
    assert_eq!(state.phase.as_deref(), Some("in_world"));
    assert_eq!(state.server, None, "non-strings become null");
    let view = agent::state_of(&entry).unwrap();
    assert_eq!(view.world.as_deref(), Some("Mundo 1"));
    assert!(view.recording);
    assert!(view.updated_at.is_some());
    // Another launch does not see it.
    let other = agent::get_launch(st, "other-state").unwrap();
    assert_eq!(agent::state_of(&other), None);
    let _ = token;
}

#[test]
fn complete_command_error_code_rules() {
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let entry = registered(st, "lcomplete", "complete-inst");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    let pending = agent::call_tool(st, "lcomplete", "chat", Some(&json!({ "text": "hi" })), 5000).unwrap();
    let id = pending.command().id.clone();
    assert!(agent::complete_command(
        &entry,
        &id,
        false,
        None,
        Some(&json!({ "code": "TIER_REFUSED", "message": "act tier refused" })),
    ));
    let err = rt.block_on(pending.wait()).expect_err("mod reported a failure");
    assert_eq!(code_of(&err), (502, "TIER_REFUSED".into()));
    assert!(err.to_string().contains("act tier refused"));

    // A lowercase/invalid code collapses to FAILED.
    let pending = agent::call_tool(st, "lcomplete", "chat", None, 5000).unwrap();
    let id = pending.command().id.clone();
    agent::complete_command(&entry, &id, false, None, Some(&json!({ "code": "lowercase" })));
    let err = rt.block_on(pending.wait()).expect_err("mod reported a failure");
    assert_eq!(code_of(&err), (502, "FAILED".into()));
    // The message falls back to `<tool> failed`.
    assert!(err.to_string().contains("'chat' failed"));

    // A late/duplicate result is not accepted.
    assert!(!agent::complete_command(&entry, "nope", true, None, None));
}

#[test]
fn policy_carries_blocked_hosts_and_the_allowlist() {
    let (_d, e) = temp_engine();
    let p = agent::policy(e.config());
    assert_eq!(p["blocked_hosts"], json!(["uhce.top", "espectral.es"]));
    assert_eq!(p["allowed_servers"], json!([]));
}

fn write_capture(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let caps = dir.join(agent::CAPTURES_DIR);
    std::fs::create_dir_all(&caps).unwrap();
    let p = caps.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn rewrite_file_results_absolutizes_and_inlines_a_preview() {
    let d = tempfile::tempdir().unwrap();
    let game_dir = d.path().join("instances").join("cap");
    // 1x1 PNG
    let png = base64_decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    );
    let shot = write_capture(&game_dir, "shot.png", &png);

    let out = agent::rewrite_file_results(
        &json!({ "image_path": "espectral-captures/shot.png", "preview_path": "espectral-captures/shot.png", "width": 1920 }),
        Some(&game_dir),
    );
    assert_eq!(out["image_path"], json!(shot.to_string_lossy()));
    assert_eq!(out["preview_path"], json!(shot.to_string_lossy()));
    assert_eq!(out["width"], 1920);
    assert_eq!(out["preview"]["mime"], "image/png");
    assert_eq!(out["preview"]["base64"], json!(base64_encode(&png)));
}

#[test]
fn rewrite_file_results_refuses_escapes_and_symlinks() {
    let d = tempfile::tempdir().unwrap();
    let game_dir = d.path().join("instances").join("cap2");
    std::fs::create_dir_all(game_dir.join(agent::CAPTURES_DIR)).unwrap();
    std::fs::write(game_dir.join("options.txt"), "gamma:1.0\n").unwrap();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut rels = vec![
        "../options.txt".to_string(),
        "espectral-captures/../../options.txt".to_string(),
        "options.txt".to_string(),
        game_dir.join("..").join("cap2").join("options.txt").to_string_lossy().into_owned(),
        "/etc/passwd".to_string(),
    ];
    // A symlink inside the captures dir pointing at a launcher file: lexically
    // inside, really outside.
    #[cfg(unix)]
    {
        let caps = game_dir.join(agent::CAPTURES_DIR);
        let secret = d.path().join("secret.txt");
        std::fs::write(&secret, "launcher internals").unwrap();
        std::os::unix::fs::symlink(&secret, caps.join("leak.png")).unwrap();
        rels.push("espectral-captures/leak.png".to_string());
    }
    for rel in &rels {
        let out = agent::rewrite_file_results(&json!({ "video_path": rel, "preview_path": rel }), Some(&game_dir));
        assert_eq!(out["video_path"], Value::Null, "{rel} must be refused");
        assert!(out.get("preview").is_none(), "no preview for a refused path ({rel})");
    }
    // Non-path fields pass through untouched.
    assert_eq!(
        agent::rewrite_file_results(&json!({ "frames": 30, "note": "x" }), Some(&game_dir)),
        json!({ "frames": 30, "note": "x" })
    );
}

#[test]
fn rewrite_file_results_is_a_noop_without_a_game_dir() {
    assert_eq!(agent::rewrite_file_results(&json!({ "video_path": "a.mp4" }), None), json!({ "video_path": "a.mp4" }));
    assert_eq!(agent::rewrite_file_results(&Value::Null, Some(std::path::Path::new("/tmp"))), Value::Null);
    // Arrays pass through (JS copies the entries index by index).
    let arr = json!([1, 2, { "x_path": "a" }]);
    assert_eq!(agent::rewrite_file_results(&arr, Some(std::path::Path::new("/tmp"))), arr);
}

#[test]
fn resolve_capture_path_accepts_absolute_paths_inside_the_root() {
    let d = tempfile::tempdir().unwrap();
    let game_dir = d.path().join("instances").join("cap3");
    let shot = write_capture(&game_dir, "abs.webp", b"x");
    let got = agent::resolve_capture_path(&shot.to_string_lossy(), &game_dir, None);
    assert_eq!(got, Some(shot));
    // A path outside the root is refused even when it exists.
    let outside = d.path().join("outside.png");
    std::fs::write(&outside, b"x").unwrap();
    assert_eq!(agent::resolve_capture_path(&outside.to_string_lossy(), &game_dir, None), None);
    assert_eq!(agent::resolve_capture_path("", &game_dir, None), None);
}

#[test]
fn agent_state_helpers_are_isolated_per_engine() {
    // Two engines never share registry entries (JS module state is per engine).
    let (_d1, e1) = temp_engine();
    let (_d2, e2) = temp_engine();
    registered(e1.agent_state(), "lshared", "a");
    assert!(agent::is_agent_launch(e1.agent_state(), "lshared"));
    assert!(!agent::is_agent_launch(e2.agent_state(), "lshared"));
    let keys: HashSet<_> = agent::all_launches(e1.agent_state()).iter().map(|e| e.key().to_string()).collect();
    assert!(keys.contains("lshared"));
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

#[tokio::test]
async fn a_dropped_parked_poll_never_steals_the_next_batch() {
    // Axum drops the handler future when the HTTP client hangs up: the parked
    // waiter must leave the waiter set so a later call reaches a LIVE poll
    // instead of being handed to a dead one (and lost).
    let (_d, e) = temp_engine();
    let st = e.agent_state();
    let key = "ldropped";
    let entry = registered(st, key, "dropped-inst");

    let parked = tokio::spawn({
        let entry = entry.clone();
        async move { agent::await_commands(&entry, std::time::Duration::from_secs(30)).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    parked.abort();
    let _ = parked.await;
    // Let the guard's Drop run before the next call.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let pending = agent::call_tool(st, key, "click", None, 5000).unwrap();
    let id = pending.command().id.clone();
    // A fresh poll takes the command at once (it was never consumed).
    let got = agent::await_commands(&entry, std::time::Duration::from_millis(200)).await;
    assert_eq!(got.len(), 1, "the command stayed queued for the live poll");
    assert_eq!(got[0].id, id);
    agent::complete_command(&entry, &id, true, Some(&json!({ "ok": 1 })), None);
    assert_eq!(pending.wait().await.unwrap(), json!({ "ok": 1 }));
}
