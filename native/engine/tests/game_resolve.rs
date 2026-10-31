//! game-resolve facade (routes/versions.mjs + GET /api/jvm) end to end on a
//! temp data dir with every upstream on a loopback MockServer.

use espectral_engine::api::game_resolve::ResolveEndpoints;
use espectral_engine::test_support::{temp_engine, MockResponse, MockServer};
use espectral_engine::types::JavaProbe;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn list_versions_maps_the_mojang_manifest() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| {
        MockResponse::json(&json!({
            "latest": { "release": "1.21.11", "snapshot": "26w01a" },
            "versions": [
                { "id": "26w01a", "type": "snapshot", "url": "https://x/26w01a.json", "time": "t", "releaseTime": "2026-01-01T00:00:00+00:00", "sha1": "s1" },
                { "id": "1.21.11", "url": "https://x/1.21.11.json", "time": "t2", "sha1": "s2" },
            ]
        }))
    })
    .await;
    e.game_resolve_state().set_endpoints(ResolveEndpoints::all_on(&srv.url("")));
    let m = e.list_versions().await.unwrap();
    assert_eq!((m.latest_release.as_str(), m.latest_snapshot.as_str()), ("1.21.11", "26w01a"));
    let v = serde_json::to_value(&m.versions[1]).unwrap();
    assert_eq!(
        v,
        json!({ "id": "1.21.11", "type": "release", "release_time": "t2", "sha1": "s2", "url": "https://x/1.21.11.json" })
    );
    // Cached on disk with fetched_at, in the JS layout.
    let cached: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(e.paths().version_manifest_cache()).unwrap()).unwrap();
    assert!(cached["fetched_at"].as_i64().unwrap() > 0);
    assert_eq!(cached["versions"][0]["release_time"], "2026-01-01T00:00:00+00:00");
    // Second call is served from the 6h cache.
    e.list_versions().await.unwrap();
    assert_eq!(srv.hits(), 1);
}

#[tokio::test]
async fn list_versions_failure_is_502_manifest_fetch_failed() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| MockResponse::status(404)).await;
    e.game_resolve_state().set_endpoints(ResolveEndpoints::all_on(&srv.url("")));
    let env = e.list_versions().await.unwrap_err().envelope();
    assert_eq!((env.status, env.code.as_str()), (502, "MANIFEST_FETCH_FAILED"));
    assert!(env.message.starts_with("version manifest fetch failed: "), "{}", env.message);
}

#[tokio::test]
async fn resolve_version_info_codes() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|r| match r.path.as_str() {
        "/manifest.json" => MockResponse::json(&json!({
            "latest": { "release": "1.21.1", "snapshot": "1.21.1" },
            "versions": [
                { "id": "1.21.1", "type": "release", "url": "/v/1.21.1.json" },
                { "id": "broken", "type": "release", "url": "/v/broken.json" },
            ]
        })),
        "/v/1.21.1.json" => MockResponse::json(&json!({ "id": "1.21.1", "javaVersion": { "majorVersion": 21 } })),
        _ => MockResponse::status(500),
    })
    .await;
    e.game_resolve_state().set_endpoints(ResolveEndpoints::all_on(&srv.url("")));
    // Manifest URLs are absolute upstream; rewrite them onto the mock.
    let m = e.list_versions().await.unwrap();
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(e.paths().version_manifest_cache()).unwrap()).unwrap();
    for (i, v) in m.versions.iter().enumerate() {
        doc["versions"][i]["url"] = json!(srv.url(&v.url));
    }
    std::fs::write(e.paths().version_manifest_cache(), doc.to_string()).unwrap();

    let info = e.resolve_version_info("1.21.1").await.unwrap();
    assert_eq!(
        serde_json::to_value(&info).unwrap(),
        json!({ "id": "1.21.1", "java_major": 21, "supported": true, "reason": "" })
    );
    let env = e.resolve_version_info("nope").await.unwrap_err().envelope();
    assert_eq!((env.status, env.code.as_str()), (404, "VERSION_NOT_FOUND"));
    assert_eq!(env.message, "version not found in manifest: nope");
    let env = e.resolve_version_info("broken").await.unwrap_err().envelope();
    assert_eq!((env.status, env.code.as_str()), (502, "VERSION_RESOLVE_FAILED"));
}

#[tokio::test]
async fn jvm_info_adds_supported_majors_and_503_when_discovery_fails() {
    let (d, e) = temp_engine();
    e.game_resolve_state().isolate();
    e.game_resolve_state().set_endpoints(ResolveEndpoints::all_on("http://127.0.0.1:9"));
    let env = e.jvm_info().await.unwrap_err().envelope();
    assert_eq!((env.status, env.code.as_str()), (503, "UNAVAILABLE"));

    let java = d.path().join("jdk").join("bin").join("java");
    std::fs::create_dir_all(java.parent().unwrap()).unwrap();
    std::fs::write(&java, "x").unwrap();
    e.game_resolve_state().update_hooks(|h| h.path_java = Some(Some(java.clone())));
    e.game_resolve_state().set_probe_override(Some(Arc::new(|_: &std::path::Path| JavaProbe {
        ok: true,
        major: 25,
        version: Some("25.0.4".into()),
        build: Some("25.0.4+7".into()),
        vendor: Some("Eclipse Adoptium".into()),
        error: None,
    })));
    let info = e.jvm_info().await.unwrap();
    let v = serde_json::to_value(&info).unwrap();
    assert_eq!(v["supported_majors"], json!([25, 21, 17, 16, 8]));
    assert_eq!(v["major"], 25);
    assert_eq!(v["source"], "path");
    assert_eq!(v["build"], "25.0.4+7");
    assert_eq!(e.config().jvm().unwrap().path, java.to_string_lossy());
}
