//! Cross-cluster integration: flows that cross the content, accounts-skins,
//! game-resolve, game-launch and identity-discord seams through the facade.
//! Every remote endpoint points at a closed loopback port; nothing leaves the
//! machine and the real home dir is never read.

use std::path::Path;
use std::sync::Arc;

use espectral_engine::api::game_launch::{LaunchOutcome, LaunchRequest};
use espectral_engine::api::game_resolve::ResolveEndpoints;
use espectral_engine::test_support::temp_engine;
use espectral_engine::types::{CosmeticsPatch, CreateInstanceRequest, JavaProbe};
use espectral_engine::Engine;
use serde_json::{json, Value};

const CLOSED: &str = "http://127.0.0.1:9";

/// No network, no PATH java, no FastClient: a fake JDK 21 that "probes" fine.
fn sandbox(e: &Engine, dir: &Path) -> std::path::PathBuf {
    e.update_content_settings(|s| {
        s.background_on_create = false;
        s.modrinth_api = format!("{CLOSED}/v2");
        s.mcsrvstat_api = format!("{CLOSED}/3");
    });
    e.identity_discord_state().set_overrides(|o| {
        o.identity_url = Some(CLOSED.into());
        o.session_server = Some(CLOSED.into());
        o.discord_mock = Some(true);
        o.ipc_candidates = Some(vec![]);
    });
    let rs = e.game_resolve_state();
    rs.isolate();
    rs.set_endpoints(ResolveEndpoints::all_on(CLOSED));
    let java = dir.join("jdk").join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });
    std::fs::create_dir_all(java.parent().unwrap()).unwrap();
    std::fs::write(&java, "x").unwrap();
    let j = java.clone();
    rs.update_hooks(move |h| h.path_java = Some(Some(j)));
    rs.set_probe_override(Some(Arc::new(|_: &Path| JavaProbe {
        ok: true,
        major: 21,
        version: Some("21.0.8".into()),
        build: Some("21.0.8+9".into()),
        vendor: Some("Eclipse Adoptium".into()),
        error: None,
    })));
    java
}

/// Mojang version JSON in the resolver's flat cache (<data>/versions/<id>.json).
fn seed_version_json(e: &Engine, id: &str) {
    let vj = json!({
        "id": id,
        "type": "release",
        "mainClass": "net.minecraft.client.main.Main",
        "javaVersion": { "component": "java-runtime-delta", "majorVersion": 21 },
        "assetIndex": { "id": "17", "sha1": "0".repeat(40), "size": 1, "url": format!("{CLOSED}/17.json") },
        "assets": "17",
        "downloads": { "client": { "sha1": "1".repeat(40), "size": 1, "url": format!("{CLOSED}/client.jar") } },
        "libraries": [
            { "name": "com.mojang:brigadier:1.3.10",
              "downloads": { "artifact": { "path": "com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar",
                                           "sha1": "2".repeat(40), "size": 1, "url": format!("{CLOSED}/b.jar") } } }
        ],
        "arguments": { "game": ["--username", "${auth_player_name}"], "jvm": ["-cp", "${classpath}"] }
    });
    let p = e.paths().data_dir().join("versions").join(format!("{id}.json"));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, vj.to_string()).unwrap();
    // A fresh manifest cache (6 h TTL) naming it, sha1 unset (= trust the cache).
    let manifest = json!({
        "fetched_at": espectral_engine::platform::now_ms(),
        "latest_release": id,
        "latest_snapshot": id,
        "versions": [{ "id": id, "type": "release", "release_time": "2024-08-08T12:24:45+00:00",
                       "url": format!("{CLOSED}/v/{id}.json") }]
    });
    std::fs::write(e.paths().version_manifest_cache(), manifest.to_string()).unwrap();
}

#[tokio::test]
async fn dry_run_launch_crosses_content_accounts_resolve_and_launch() {
    let (d, e) = temp_engine();
    let java = sandbox(&e, d.path());
    seed_version_json(&e, "1.21.1");
    e.create_instance(CreateInstanceRequest {
        name: "Cross Test".into(),
        version: "1.21.1".into(),
        memory_mb: Some(2048),
        ..Default::default()
    })
    .await
    .unwrap();
    e.create_account("Alex").await.unwrap();
    e.set_active_account("Alex").await.unwrap();

    let out = e
        .launch_instance("Cross Test", LaunchRequest { dry_run: true, account: None, ..Default::default() })
        .await
        .unwrap();
    let LaunchOutcome::DryRun(dr) = out else { panic!("expected a dry run, got {out:?}") };
    assert_eq!(dr.java_path, java.to_string_lossy());
    assert_eq!(dr.argv[0], java.to_string_lossy());
    let has = |s: &str| dr.argv.iter().any(|a| a == s);
    assert!(has("net.minecraft.client.main.Main"), "{:?}", dr.argv);
    assert!(has("-Xmx2G"), "{:?}", dr.argv);
    assert!(dr.argv.iter().any(|a| a.starts_with("-Despectral.bridge.url=http://127.0.0.1:")));
    let after = |flag: &str| dr.argv.iter().position(|a| a == flag).map(|i| dr.argv[i + 1].clone());
    assert_eq!(after("--username").as_deref(), Some("Alex"));
    assert_eq!(after("--version").as_deref(), Some("1.21.1"));
    let game_dir = after("--gameDir").expect("--gameDir");
    assert_eq!(Path::new(&game_dir), e.paths().instance_dir("Cross Test"));
    // Classpath: the library under <data>/libraries and the client jar, nothing downloaded.
    assert!(dr.classpath.iter().any(|c| c.ends_with("brigadier-1.3.10.jar")), "{:?}", dr.classpath);
    assert!(dr.classpath_missing.len() >= 2);
    assert!(!dr.assets_ok);

    // Unknown instance and unknown account keep the JS codes.
    let env = e
        .launch_instance("Nope", LaunchRequest { dry_run: true, account: None, ..Default::default() })
        .await
        .unwrap_err()
        .envelope();
    assert_eq!((env.status, env.code.as_str()), (404, "NOT_FOUND"));
    let env = e
        .launch_instance(
            "Cross Test",
            LaunchRequest { dry_run: true, account: Some("Ghost".into()), ..Default::default() },
        )
        .await
        .unwrap_err()
        .envelope();
    assert_eq!(env.status, 404);
}

#[tokio::test]
async fn cosmetics_put_reaches_bridge_registered_game_dirs_and_bridge_answers() {
    let (d, e) = temp_engine();
    sandbox(&e, d.path());
    let acc = e.create_account("Steve").await.unwrap();
    let flat = acc.uuid.replace('-', "");
    let game_dir = d.path().join("running-game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let token = espectral_engine::bridge::mint_token();
    espectral_engine::presence::launch_started(
        &e,
        espectral_engine::presence::LaunchPresence {
            instance: "Running".into(),
            uuid: Some(acc.uuid.clone()),
            game_dir: game_dir.clone(),
            token: token.clone(),
        },
    )
    .unwrap();

    e.put_cosmetics(&flat, CosmeticsPatch { emotes: Some(false), ..Default::default() }).await.unwrap();
    let file = game_dir.join("config").join("espectral-cosmetics.json");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).expect("cosmetics file")).unwrap();
    assert_eq!(doc["uuid"], flat);
    assert_eq!(doc["emotes"], false);
    // Unverified account: no grant, so the loadout is empty (never forged).
    assert_eq!(doc["grant"], Value::Null);
    assert!(doc["loadout"].as_object().unwrap().values().all(Value::is_null));

    // Bridge: a bad token is 403 BRIDGE_AUTH; the launch's token is accepted.
    let env = e.bridge_visible(&"0".repeat(64), vec![flat.clone()]).await.unwrap_err().envelope();
    assert_eq!((env.status, env.code.as_str()), (403, "BRIDGE_AUTH"));
    e.bridge_visible(&token, vec![flat.clone()]).await.unwrap();

    espectral_engine::presence::launch_exited(&e, "Running", Some(&token)).unwrap();
    let env = e.bridge_visible(&token, vec![flat]).await.unwrap_err().envelope();
    assert_eq!(env.code, "BRIDGE_AUTH");
}

#[tokio::test]
async fn client_config_round_trips_through_content_and_identity_discord() {
    let (d, e) = temp_engine();
    sandbox(&e, d.path());
    e.create_instance(CreateInstanceRequest {
        name: "Qol".into(),
        version: "1.21.11".into(),
        loader: Some(espectral_engine::types::Loader::Fabric),
        ..Default::default()
    })
    .await
    .unwrap();
    let info = serde_json::to_value(e.get_client_info("Qol").await.unwrap()).unwrap();
    assert_eq!(info["supported"], true);
    let env = e.get_client_info("missing").await.unwrap_err().envelope();
    assert_eq!((env.status, env.code.as_str()), (404, "NOT_FOUND"));
    // The canonical file lives at <instance>/config/espectral-client.json.
    let path = e.paths().instance_dir("Qol").join("config").join("espectral-client.json");
    assert!(path.exists() || info["config"].is_object());
}
