//! Content cluster through the public Engine facade: instance CRUD, mods,
//! servers.dat, import detection, server status — temp data dir, injected
//! import roots and loopback mocks only (never the real network / home dir).

use std::time::Duration;

use espectral_engine::api::content::{InstallModrinthRequest, ModrinthSearchQuery};
use espectral_engine::import::ImportRoots;
use espectral_engine::test_support::{temp_engine, MockResponse, MockServer};
use espectral_engine::types::{CreateInstanceRequest, InstancePatch, Loader, ServerEntry};
use espectral_engine::Engine;
use serde_json::json;

fn offline(e: &Engine, base: &std::path::Path, modrinth: &str) {
    let roots =
        ImportRoots { windows: false, appdata: base.join("appdata"), home: base.join("home"), xdg_data_home: None };
    let modrinth = modrinth.to_string();
    e.update_content_settings(move |s| {
        s.background_on_create = false;
        s.modrinth_api = modrinth;
        s.modrinth_retry_backoff = Duration::from_millis(1);
        s.import_roots = Some(roots);
    });
}

#[tokio::test]
async fn instance_lifecycle_json_shapes() {
    let (d, e) = temp_engine();
    offline(&e, d.path(), "http://127.0.0.1:9/v2");
    let s = e
        .create_instance(CreateInstanceRequest {
            name: "Mi Instancia".into(),
            version: "1.21.11".into(),
            memory_mb: Some(4096),
            ..Default::default()
        })
        .await
        .unwrap();
    let v = serde_json::to_value(&s).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "name",
            "version",
            "loader",
            "loader_version",
            "modpack",
            "modpack_version",
            "memory_mb",
            "memory_auto",
            "hue",
            "game_dir",
            "has_icon",
            "mod_count",
            "enabled_mod_count",
            "aot_key",
            "aot_cache_exists",
            "imported_from",
            "created_at"
        ]
    );
    assert_eq!((v["memory_mb"].as_u64(), v["memory_auto"].as_bool()), (Some(4096), Some(false)));

    let list = e.list_instances().await.unwrap();
    assert_eq!(serde_json::to_value(&list).unwrap()[0]["running"], false);

    let servers = vec![ServerEntry {
        name: "Espectral".into(),
        ip: "main.uhce.top".into(),
        hidden: false,
        accept_textures: false,
        has_icon: false,
    }];
    assert_eq!(e.put_instance_servers("Mi Instancia", servers.clone()).await.unwrap().count, 1);
    let detail = e.get_instance_detail("Mi Instancia").await.unwrap();
    assert_eq!(detail.servers, servers);
    assert_eq!(detail.aot.ready_to_train, Some(true));

    let renamed = e
        .patch_instance("Mi Instancia", InstancePatch { new_name: Some("Renombrada".into()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(renamed.name, "Renombrada");
    assert_eq!(e.instance_servers("Renombrada").await.unwrap().servers, servers);
    assert_eq!(e.instance_servers("Mi Instancia").await.unwrap_err().status(), 404);

    let del = e.delete_instance("Renombrada").await.unwrap();
    assert!(del.trash.unwrap().starts_with("Renombrada.deleted."));
    assert!(e.list_instances().await.unwrap().is_empty());
}

#[tokio::test]
async fn mods_toggle_and_modrinth_conflicts() {
    let sha = espectral_engine::download::sha1_hex;
    let srv = MockServer::start(move |req| {
        let p = req.path.as_str();
        if p.ends_with("/version_files") {
            return MockResponse::json(&json!({ sha(b"enemy"): { "project_id": "PENEMY" } }));
        }
        if p.starts_with("/v2/project/PROOT/version") {
            return MockResponse::json(&json!([{
                "id": "r1", "project_id": "PROOT", "version_number": "1", "status": "listed",
                "dependencies": [{ "project_id": "PENEMY", "dependency_type": "incompatible" }],
                "files": [{ "primary": true, "filename": "root.jar", "url": "http://127.0.0.1:9/root.jar", "hashes": {} }]
            }]));
        }
        if p.starts_with("/v2/projects?ids=") {
            return MockResponse::json(&json!([{ "id": "PENEMY", "title": "Enemy Mod" }]));
        }
        MockResponse::status(404)
    })
    .await;
    let (d, e) = temp_engine();
    offline(&e, d.path(), &srv.url("/v2"));
    e.create_instance(CreateInstanceRequest {
        name: "fab".into(),
        version: "1.20.4".into(),
        loader: Some(Loader::Vanilla),
        ..Default::default()
    })
    .await
    .unwrap();
    let mods = e.mods_dir("fab").await.unwrap().path;
    std::fs::write(std::path::Path::new(&mods).join("enemy.jar"), "enemy").unwrap();

    let t = e.set_mod_enabled("fab", "enemy.jar", false).await.unwrap();
    assert_eq!((t.filename.as_str(), t.enabled), ("enemy.jar", false));
    e.set_mod_enabled("fab", "enemy.jar", true).await.unwrap();
    let rows = e.list_mods("fab").await.unwrap();
    assert_eq!(rows.len(), 1);
    let row = serde_json::to_value(&rows[0]).unwrap();
    assert_eq!(row["project_slug"], serde_json::Value::Null);
    assert_eq!(row["sha1"], sha(b"enemy"));

    let err = e
        .install_modrinth_mod("fab", InstallModrinthRequest { project_id: "PROOT".into(), force: false })
        .await
        .unwrap_err()
        .envelope();
    assert_eq!((err.status, err.code.as_str(), err.message.as_str()), (409, "mod_conflicts", "Enemy Mod"));

    let err = e.search_modrinth(ModrinthSearchQuery { q: "x".into(), ..Default::default() }).await.unwrap_err();
    assert_eq!(err.envelope().code, "MODRINTH_SEARCH_FAILED");
}

#[tokio::test]
async fn import_sources_use_injected_roots_and_servers_are_cached() {
    let srv = MockServer::start(|_| MockResponse::json(&json!({ "online": false }))).await;
    let (d, e) = temp_engine();
    offline(&e, d.path(), "http://127.0.0.1:9/v2");
    e.update_content_settings(|s| s.mcsrvstat_api = srv.url(""));
    let mc = d.path().join("home").join(".minecraft");
    std::fs::create_dir_all(&mc).unwrap();
    std::fs::write(mc.join("options.txt"), "version:1\nfov:0.5\n").unwrap();
    let sources = e.import_sources().await.unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].id, "vanilla");
    assert_eq!((sources[0].options_exists, sources[0].options_key_count), (true, 1));

    let first = e.server_statuses().await.unwrap();
    assert_eq!(first.servers.len(), 2);
    assert!(first.servers.iter().all(|s| !s.online && s.error.is_none()));
    e.server_statuses().await.unwrap();
    assert_eq!(srv.hits(), 2, "150 s cache");
}
