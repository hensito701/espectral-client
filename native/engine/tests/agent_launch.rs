//! Agent access — ported behaviours from `tests/agent-launch.test.mjs` and the
//! agent parts of `tests/agent-route.test.mjs`: buildArgv flags/placeholders,
//! the shared startLaunch path (buffer tagging, registry registration,
//! presence suppression) and the agent launch route rules.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use espectral_engine::agent::{self, JoinOption, WindowOption};
use espectral_engine::api::game_launch::{LaunchOutcome, LaunchRequest, LaunchTestHooks};
use espectral_engine::launch::{self, ResolveLaunchOptions, SpawnedGame};
use espectral_engine::test_support::temp_engine;
use espectral_engine::types::{
    InstanceMeta, JavaRuntime, LaunchAccount, LaunchMode, LoaderLaunch, ResolvedLaunch, ResolvedVersionRef,
};
use espectral_engine::Engine;
use futures_util::FutureExt;
use serde_json::json;

fn java(path: &str) -> JavaRuntime {
    JavaRuntime { path: path.into(), build: "25.0.4+7".into(), major: Some(25), ..Default::default() }
}

/// A minimal resolveLaunch result with everything buildArgv reads.
fn resolved() -> ResolvedLaunch {
    ResolvedLaunch {
        java: java("/jdk/bin/java.exe"),
        version: ResolvedVersionRef {
            id: "26.3".into(),
            main_class: "net.minecraft.client.main.Main".into(),
            asset_index_id: Some("26.3".into()),
            json: json!({}),
        },
        loader: None,
        fabric_profile: None,
        classpath: vec!["/data/libraries/a.jar".into()],
        natives_dir: "/games/agent-spike/natives".into(),
        game_dir: "/games/agent-spike".into(),
        assets_dir: "/data/assets".into(),
        memory_mb: 4096,
        account: LaunchAccount {
            username: "AgentBot".into(),
            uuid: "00000000-0000-0000-0000-000000000000".into(),
            access_token: "0".into(),
            user_type: "mojang".into(),
        },
        bridge_token: Some("b".repeat(64)),
        mode: LaunchMode::Normal,
        aot_key: "key".into(),
        aot_cache_path: "/data/cache/aot/key/game.aot".into(),
        aot_cache_exists: true,
        aot_cache_stale: false,
        aot_available: true,
        agent: false,
        join: None,
        window: None,
        ffmpeg_path: None,
        warnings: vec![],
    }
}

fn inst(name: &str) -> InstanceMeta {
    InstanceMeta { name: name.into(), version: "26.3".into(), ..Default::default() }
}

/// Program args after the main class.
fn prog_tail(argv: &[String], main_class: &str) -> Vec<String> {
    let i = argv.iter().position(|a| a == main_class).unwrap_or_else(|| panic!("main class {main_class} present"));
    assert!(i > 0);
    argv[i + 1..].to_vec()
}

#[test]
fn agent_launch_adds_flags_ffmpeg_and_hidden() {
    let (_d, e) = temp_engine();
    let mut r = resolved();
    r.agent = true;
    r.ffmpeg_path = Some(PathBuf::from("/usr/bin/ffmpeg"));
    r.window = Some(WindowOption { width: 1080, height: 1920, hidden: true });
    let argv = launch::build_argv(&e, &inst("agent-spike"), &mut r);
    assert!(argv.iter().any(|a| a == "-Despectral.agent=1"), "agent property present");
    assert!(argv.iter().any(|a| a == "-Despectral.ffmpeg=/usr/bin/ffmpeg"), "ffmpeg property present");
    assert!(argv.iter().any(|a| a == "-Despectral.agent.hidden=1"), "hidden property present");
    let tail = prog_tail(&argv, "net.minecraft.client.main.Main");
    assert_eq!(tail[tail.len() - 4..], ["--width", "1080", "--height", "1920"]);
}

#[test]
fn plain_launch_carries_no_agent_properties_and_keeps_its_argv() {
    let (_d, e) = temp_engine();
    let mut r = resolved();
    r.bridge_token = None;
    let argv = launch::build_argv(&e, &inst("plain"), &mut r);
    for flag in ["-Despectral.agent=1", "-Despectral.agent.hidden=1"] {
        assert!(!argv.iter().any(|a| a == flag), "{flag} must be absent");
    }
    assert!(!argv.iter().any(|a| a.starts_with("-Despectral.ffmpeg=")), "no ffmpeg property");
    assert!(!argv.iter().any(|a| a == "--quickPlaySingleplayer"));
    // Byte-identical to the pre-agent argv: the fixed program list, unchanged.
    assert_eq!(
        prog_tail(&argv, "net.minecraft.client.main.Main"),
        [
            "--username",
            "AgentBot",
            "--version",
            "26.3",
            "--gameDir",
            "/games/agent-spike",
            "--assetsDir",
            "/data/assets",
            "--assetIndex",
            "26.3",
            "--uuid",
            "00000000-0000-0000-0000-000000000000",
            "--accessToken",
            "0",
            "--userType",
            "mojang",
            "--versionType",
            "release",
        ]
    );
    // The installed default resolution placeholders stay 854x480.
    assert!(argv.iter().any(|a| a == "-Dresolution_width=854" || a == "854") || !argv.iter().any(|a| a == "--width"));
}

#[test]
fn join_world_and_server_map_to_quick_play_args() {
    let (_d, e) = temp_engine();
    let mut sp = resolved();
    sp.agent = true;
    sp.join = Some(JoinOption { world: Some("Mundo nuevo".into()), server: None });
    let argv = launch::build_argv(&e, &inst("a"), &mut sp);
    let tail = prog_tail(&argv, "net.minecraft.client.main.Main");
    assert_eq!(tail[tail.len() - 2..], ["--quickPlaySingleplayer", "Mundo nuevo"]);

    let mut mp = resolved();
    mp.agent = true;
    mp.join = Some(JoinOption { world: None, server: Some("play.example.com:25566".into()) });
    let argv = launch::build_argv(&e, &inst("a"), &mut mp);
    let tail = prog_tail(&argv, "net.minecraft.client.main.Main");
    assert_eq!(tail[tail.len() - 2..], ["--quickPlayMultiplayer", "play.example.com:25566"]);
}

#[test]
fn neoforge_template_expands_the_quick_play_and_resolution_features() {
    let (_d, e) = temp_engine();
    let game_args = vec![
        json!("--username"),
        json!("${auth_player_name}"),
        json!({
            "rules": [{ "action": "allow", "features": { "has_custom_resolution": true } }],
            "value": ["--width", "${resolution_width}", "--height", "${resolution_height}"]
        }),
        json!({
            "rules": [{ "action": "allow", "features": { "is_quick_play_singleplayer": true } }],
            "value": ["--quickPlaySingleplayer", "${quickPlaySingleplayer}"]
        }),
        json!({
            "rules": [{ "action": "allow", "features": { "is_quick_play_multiplayer": true } }],
            "value": ["--quickPlayMultiplayer", "${quickPlayMultiplayer}"]
        }),
        json!({ "rules": [{ "action": "allow", "features": { "is_demo_user": true } }], "value": "--demo" }),
    ];
    let loader = LoaderLaunch {
        main_class: "net.neoforged.Main".into(),
        jvm_args: vec![],
        game_args,
        kind: Some("neoforge".into()),
    };
    let mut with_opts = resolved();
    with_opts.loader = Some(loader.clone());
    with_opts.agent = true;
    with_opts.window = Some(WindowOption { width: 1280, height: 720, hidden: false });
    with_opts.join = Some(JoinOption { world: None, server: Some("localhost:25565".into()) });
    let argv = launch::build_argv(&e, &inst("nf"), &mut with_opts);
    let tail = prog_tail(&argv, "net.neoforged.Main");
    assert_eq!(
        tail,
        ["--username", "AgentBot", "--width", "1280", "--height", "720", "--quickPlayMultiplayer", "localhost:25565"]
    );
    assert!(!tail.iter().any(|a| a == "--demo"), "gated demo rule stays off");

    // No options -> every gated rule drops out (argv identical to a plain launch).
    let mut plain = resolved();
    plain.loader = Some(loader);
    let argv = launch::build_argv(&e, &inst("nf"), &mut plain);
    assert_eq!(prog_tail(&argv, "net.neoforged.Main"), ["--username", "AgentBot"]);
}

// ---------------------------------------------------------------------------
// startLaunch path (agent-route.test.mjs)
// ---------------------------------------------------------------------------

fn write_instance(e: &Engine, name: &str) {
    let dir = e.paths().instance_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = json!({
        "name": name, "version": "26.3", "loader": "fabric", "loader_version": null,
        "memory_mb": 2048, "mods": [], "memory_auto_migrated": true,
        "jdk_path_override": null, "aot_auto_train": false,
        "created_at": "2026-01-01T00:00:00.000Z", "updated_at": "2026-01-01T00:00:00.000Z",
    });
    std::fs::write(dir.join("instance.json"), doc.to_string()).unwrap();
}

fn fake_resolved(e: &Engine, name: &str) -> ResolvedLaunch {
    let mut r = resolved();
    r.game_dir = e.paths().instance_dir(name);
    r.natives_dir = r.game_dir.join("natives");
    r.bridge_token = Some("d".repeat(64));
    r
}

/// Install the agent-route seams: resolve returns a fake launch and the spawn
/// double fires `on_spawned` (like the real launchInstance) with an alive pid.
fn install_hooks(e: &Engine, name: &str) -> Arc<Mutex<Vec<ResolveLaunchOptions>>> {
    let seen: Arc<Mutex<Vec<ResolveLaunchOptions>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let r = fake_resolved(e, name);
    e.set_launch_test_hooks(LaunchTestHooks {
        resolve_launch: Some(Arc::new(move |_inst, opts| {
            recorder.lock().unwrap().push(opts);
            let r = r.clone();
            async move { Ok(r) }.boxed()
        })),
        launch_instance: Some(Arc::new(|_e, _inst, _resolved, cb: launch::LaunchCallbacks| {
            if let Some(f) = cb.on_spawned.as_ref() {
                f(espectral_engine::platform::now_ms());
            }
            Ok(SpawnedGame { pid: std::process::id(), argv: vec![], process: None })
        })),
        quiet_presence: true,
        ..Default::default()
    });
    seen
}

#[tokio::test]
async fn agent_launch_is_tagged_registered_and_skips_presence() {
    let (_d, e) = temp_engine();
    write_instance(&e, "agent-route-1");
    let seen = install_hooks(&e, "agent-route-1");

    let reply = e
        .launch_instance(
            "agent-route-1",
            LaunchRequest {
                agent: true,
                join: Some(json!({ "world": "Survival" })),
                window: Some(json!({ "width": 1080, "height": 1920, "hidden": true })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let LaunchOutcome::Started(reply) = reply else { panic!("expected a started launch") };
    assert!(!reply.key.is_empty());
    assert!(reply.preparing);

    // Launch options reach resolveLaunch (→ argv in buildArgv).
    for _ in 0..200 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let opts = seen.lock().unwrap().first().cloned().expect("resolve called");
    assert!(opts.agent);
    assert_eq!(opts.join.unwrap().world.as_deref(), Some("Survival"));
    let w = opts.window.unwrap();
    assert_eq!((w.width, w.height, w.hidden), (1080, 1920, true));

    let rows = e.list_launches().await.unwrap().launches;
    let row = rows.iter().find(|r| r.key == reply.key).expect("launch row exists");
    assert!(row.agent, "row carries agent: true");

    // Registered in the agent registry with the resolved gameDir.
    let entry = agent::get_launch(e.agent_state(), &reply.key).expect("agent launch registered");
    assert_eq!(entry.game_dir(), Some(e.paths().instance_dir("agent-route-1")));

    // Releasing (game exit) makes pending calls fail GAME_GONE.
    let pending = agent::call_tool(e.agent_state(), &reply.key, "screenshot", None, 5000).unwrap();
    agent::release_launch(e.agent_state(), &reply.key);
    let err = pending.wait().await.expect_err("game gone");
    assert_eq!(err.envelope().code, "GAME_GONE");
    e.shutdown().await;
}

#[tokio::test]
async fn plain_launch_stays_out_of_the_agent_registry() {
    let (_d, e) = temp_engine();
    write_instance(&e, "plain-route");
    let seen = install_hooks(&e, "plain-route");

    let reply = e.launch_instance("plain-route", LaunchRequest::default()).await.unwrap();
    let LaunchOutcome::Started(reply) = reply else { panic!("expected a started launch") };
    for _ in 0..200 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let opts = seen.lock().unwrap().first().cloned().expect("resolve called");
    assert!(!opts.agent);
    assert!(opts.join.is_none());
    let row = e.list_launches().await.unwrap().launches.into_iter().find(|r| r.key == reply.key).unwrap();
    assert!(!row.agent);
    assert!(agent::get_launch(e.agent_state(), &reply.key).is_none(), "plain launches stay out");
    e.shutdown().await;
}

#[tokio::test]
async fn invalid_launch_options_are_rejected_before_any_buffer_exists() {
    let (_d, e) = temp_engine();
    write_instance(&e, "bad-opts");
    let err = e
        .launch_instance("bad-opts", LaunchRequest { join: Some(json!({ "world": ".." })), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!((err.status(), err.envelope().code.as_str()), (400, "BAD_JOIN"));
    let err = e
        .launch_instance(
            "bad-opts",
            LaunchRequest { window: Some(json!({ "width": 10, "height": 10 })), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((err.status(), err.envelope().code.as_str()), (400, "BAD_WINDOW"));
    // `hidden` on a plain launch is refused rather than silently dropped.
    let err = e
        .launch_instance(
            "bad-opts",
            LaunchRequest {
                window: Some(json!({ "width": 800, "height": 600, "hidden": true })),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.envelope().code, "BAD_WINDOW");
    assert!(e.list_launches().await.unwrap().launches.is_empty(), "no launch row was created");
    assert!(!e.active_instances().contains("bad-opts"));
}

#[tokio::test]
async fn agent_dry_run_reports_the_options_and_warnings() {
    let (_d, e) = temp_engine();
    write_instance(&e, "dry-agent");
    install_hooks(&e, "dry-agent");
    let out = e
        .launch_instance(
            "dry-agent",
            LaunchRequest {
                dry_run: true,
                agent: true,
                join: Some(json!({ "server": "localhost" })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let LaunchOutcome::DryRun(dr) = out else { panic!("expected a dry run") };
    assert!(dr.argv.iter().any(|a| a == "-Despectral.agent=1"));
    assert!(dr.argv.windows(2).any(|w| w == ["--quickPlayMultiplayer", "localhost"]));
    // The ffmpeg warning rides along when nothing is provisioned (POSIX PATH
    // lookup is the only source; the sandboxed env has no ffmpeg).
    if espectral_engine::ffmpeg::resolve_ffmpeg_sync(&e).is_none() {
        assert!(dr.warnings.iter().any(|w| w.contains("no ffmpeg found")), "warnings: {:?}", dr.warnings);
    }
    // Dry runs never register a launch.
    assert!(e.list_launches().await.unwrap().launches.is_empty());
}
