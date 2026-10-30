//! Port of tests/resolver.test.mjs (rulesAllow cases live in crate::rules)
//! and the resolver half of tests/neoforge.test.mjs.

use super::*;
use crate::api::game_resolve::ResolveEndpoints;
use crate::test_support::{fixture, temp_engine, MockRequest, MockResponse, MockServer};
use crate::types::{LibraryDownload, Loader};
use serde_json::json;

fn mocked(e: &Engine, srv: &MockServer) {
    e.game_resolve_state().set_endpoints(ResolveEndpoints::all_on(&srv.url("")));
    e.game_resolve_state().isolate();
}

/// An engine whose endpoints are a server that must never be hit.
async fn offline_engine() -> (tempfile::TempDir, Engine, MockServer) {
    let (d, e) = temp_engine();
    let srv = MockServer::start(|r| panic!("unexpected fetch: {}", r.path)).await;
    mocked(&e, &srv);
    (d, e, srv)
}

fn fixture_json(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture(name)).unwrap()).unwrap()
}

async fn wait_for_json(file: &Path, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(v) = std::fs::read_to_string(file).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            if pred(&v) {
                return v;
            }
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {}", file.display());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// helpers / foundation pieces
// ---------------------------------------------------------------------------

#[test]
fn classpath_order_and_filters() {
    let (_d, e) = temp_engine();
    let inst =
        InstanceMeta { name: "x".into(), version: "1.21.1".into(), loader: Loader::Fabric, ..Default::default() };
    let vj = json!({ "libraries": [
        { "name": "a:b:1" },
        { "name": "org.lwjgl:lwjgl:3:natives-linux" },
        { "name": "c:d:2", "rules": [{ "action": "allow", "os": { "name": "nonexistent" } }] }
    ] });
    let fabric = FabricProfile {
        mc_version: "1.21.1".into(),
        loader_version: "0.16".into(),
        loader_maven: None,
        intermediary: None,
        main_class: "K".into(),
        jvm_args: vec![],
        libraries: vec![LibraryDownload {
            name: "f:g:1".into(),
            rel_path: "f/g/1/g-1.jar".into(),
            url: "u".into(),
            sha1: None,
            size: None,
        }],
        min_java_version: None,
        library_count: None,
        seeded: None,
        downloaded: None,
    };
    let cp = resolve_classpath(&e, &inst, &vj, Some(&fabric));
    let libs = e.paths().libraries_dir();
    assert_eq!(cp, vec![libs.join("f/g/1/g-1.jar"), libs.join("a/b/1/b-1.jar"), e.paths().client_jar("1.21.1")]);
    // Native separators throughout (Node path.join parity).
    let s = cp[0].to_string_lossy().into_owned();
    if cfg!(windows) {
        assert!(!s.contains('/'), "{s}");
    }
}

#[test]
fn load_instance_meta_codes() {
    let (_d, e) = temp_engine();
    assert_eq!(load_instance_meta(&e, "bad/name").unwrap_err().envelope().code, "invalid_instance_name");
    assert_eq!(load_instance_meta(&e, "missing").unwrap_err().status(), 404);
    crate::paths::write_json_atomic(
        &e.paths().instance_json("ok"),
        &json!({ "name": "ok", "version": "1.21.1", "loader": "vanilla" }),
    )
    .unwrap();
    assert_eq!(load_instance_meta(&e, "ok").unwrap().version, "1.21.1");
}

#[test]
fn encode_uri_component_parity() {
    assert_eq!(encode_uri_component("1.21.1"), "1.21.1");
    assert_eq!(encode_uri_component("1.20 pre/1+x"), "1.20%20pre%2F1%2Bx");
    assert_eq!(encode_uri_component("a~b*c'(d)!"), "a~b*c'(d)!");
}

#[test]
fn join_rel_splits_segments() {
    let b = Path::new("/base");
    assert_eq!(join_rel(b, "a/b/c.jar"), Path::new("/base").join("a").join("b").join("c.jar"));
    assert_eq!(join_rel(b, "./a//b"), Path::new("/base").join("a").join("b"));
}

// ---------------------------------------------------------------------------
// fabricSupportedForVersion
// ---------------------------------------------------------------------------

async fn fabric_supported_with(resp: MockResponse, mc: &str) -> (bool, Vec<MockRequest>) {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(move |_| resp.clone()).await;
    mocked(&e, &srv);
    let r = fabric_supported_for_version(&e, mc).await.unwrap();
    (r, srv.requests())
}

#[tokio::test]
async fn fabric_supported_non_empty_list_is_true() {
    let (r, reqs) =
        fabric_supported_with(MockResponse::json(&json!([{ "version": "0.16.5" }, { "version": "0.16.4" }])), "26.2")
            .await;
    assert!(r);
    assert_eq!(reqs[0].path, "/fabric-meta/v2/versions/loader/26.2");
}

#[tokio::test]
async fn fabric_supported_empty_list_is_false() {
    assert!(!fabric_supported_with(MockResponse::json(&json!([])), "1.99.99").await.0);
}

#[tokio::test]
async fn fabric_supported_http_400_is_definitive_false() {
    assert!(!fabric_supported_with(MockResponse::status(400), "1.20.4").await.0);
}

#[tokio::test]
async fn fabric_supported_http_500_is_optimistic_true() {
    assert!(fabric_supported_with(MockResponse::status(500), "26.2").await.0);
}

#[tokio::test]
async fn fabric_supported_network_failure_is_optimistic_true() {
    assert!(fabric_supported_with(MockResponse::drop_connection(), "26.2").await.0);
}

// ---------------------------------------------------------------------------
// getFabricLoaders / getVersionManifest caches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fabric_loaders_expired_cache_survives_meta_outage() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| MockResponse::drop_connection()).await;
    mocked(&e, &srv);
    let file = e.paths().versions_dir().join("fabric-loaders.json");
    crate::paths::write_json_atomic(
        &file,
        &json!({
            "fetched_at": crate::platform::now_ms() - 7 * 24 * 60 * 60 * 1000,
            "loaders": [{ "version": "0.17.2", "stable": true, "build": 10 }, { "version": "0.17.1", "stable": false, "build": 9 }],
            "latest_stable": "0.17.2",
        }),
    )
    .unwrap();
    // force=true skips the SWR path and goes straight to the (failing) fetch.
    let out = get_fabric_loaders(&e, true).await.unwrap();
    assert_eq!(out.latest_stable.as_deref(), Some("0.17.2"));
    assert_eq!(out.loaders.len(), 2);
}

#[tokio::test]
async fn fabric_loaders_outage_without_cache_fails_loudly() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| MockResponse::drop_connection()).await;
    mocked(&e, &srv);
    let err = get_fabric_loaders(&e, false).await.unwrap_err();
    assert!(err.has_code("fabric_loader_unreachable"), "{err:?}");
}

#[tokio::test]
async fn manifest_swr_returns_stale_and_refreshes_in_background() {
    let (_d, e) = temp_engine();
    let fresh = json!({
        "latest": { "release": "1.21.2", "snapshot": "25w02a" },
        "versions": [{ "id": "1.21.2", "type": "release", "releaseTime": "2024-09-01T00:00:00Z", "sha1": "abc", "url": "https://example.com/1.21.2.json" }],
    });
    let srv = MockServer::start(move |_| MockResponse::json(&fresh).with_delay(Duration::from_millis(20))).await;
    mocked(&e, &srv);
    let file = e.paths().version_manifest_cache();
    crate::paths::write_json_atomic(
        &file,
        &json!({
            "fetched_at": crate::platform::now_ms() - MANIFEST_TTL_MS - 1000,
            "latest_release": "1.21.1",
            "latest_snapshot": "25w01a",
            "versions": [{ "id": "1.21.1", "type": "release", "release_time": "2024-08-08T00:00:00Z", "sha1": null, "url": "https://example.com/1.21.1.json" }],
        }),
    )
    .unwrap();
    let out = get_version_manifest(&e, false).await.unwrap();
    assert_eq!(out.latest_release, "1.21.1");
    assert_eq!(out.versions[0].id, "1.21.1");
    assert_eq!(out.versions[0].sha1, "", "null sha1 surfaces as an empty string");
    let updated = wait_for_json(&file, |m| m["latest_release"] == "1.21.2").await;
    assert_eq!(updated["versions"][0]["release_time"], "2024-09-01T00:00:00Z");
    assert_eq!(updated["latest_snapshot"], "25w02a");
    assert!(srv.hits() >= 1);
}

#[tokio::test]
async fn fabric_loaders_swr_returns_stale_and_refreshes_in_background() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| {
        MockResponse::json(&json!([{ "version": "0.16.0", "stable": true, "build": 6 }]))
            .with_delay(Duration::from_millis(20))
    })
    .await;
    mocked(&e, &srv);
    let file = e.paths().versions_dir().join("fabric-loaders.json");
    crate::paths::write_json_atomic(
        &file,
        &json!({
            "fetched_at": crate::platform::now_ms() - MANIFEST_TTL_MS - 1000,
            "loaders": [{ "version": "0.15.0", "stable": true, "build": 5 }],
            "latest_stable": "0.15.0",
        }),
    )
    .unwrap();
    let out = get_fabric_loaders(&e, false).await.unwrap();
    assert_eq!(out.latest_stable.as_deref(), Some("0.15.0"));
    assert_eq!(out.loaders[0]["version"], "0.15.0");
    let updated = wait_for_json(&file, |m| m["latest_stable"] == "0.16.0").await;
    assert_eq!(updated["loaders"][0]["build"], 6);
    assert_eq!(srv.requests()[0].path, "/fabric-meta/v2/versions/loader");
}

#[tokio::test]
async fn manifest_force_bypasses_stale() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| {
        MockResponse::json(&json!({
            "latest": { "release": "fresh", "snapshot": "fresh" },
            "versions": [{ "id": "fresh", "type": "release", "releaseTime": "2024-09-01T00:00:00Z", "sha1": null, "url": "https://example.com/fresh.json" }],
        }))
    })
    .await;
    mocked(&e, &srv);
    crate::paths::write_json_atomic(
        &e.paths().version_manifest_cache(),
        &json!({
            "fetched_at": crate::platform::now_ms() - MANIFEST_TTL_MS - 1000,
            "latest_release": "old", "latest_snapshot": "old",
            "versions": [{ "id": "old", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/old.json" }],
        }),
    )
    .unwrap();
    let out = get_version_manifest(&e, true).await.unwrap();
    assert_eq!(out.latest_release, "fresh");
}

#[tokio::test]
async fn manifest_offline_falls_back_to_any_cache_and_fails_without_one() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| MockResponse::status(503)).await;
    mocked(&e, &srv);
    assert!(get_version_manifest(&e, true).await.is_err());
    crate::paths::write_json_atomic(
        &e.paths().version_manifest_cache(),
        &json!({ "fetched_at": 1, "latest_release": "x", "latest_snapshot": "y", "versions": [] }),
    )
    .unwrap();
    assert_eq!(get_version_manifest(&e, true).await.unwrap().latest_release, "x");
}

// ---------------------------------------------------------------------------
// C14: natives freshness markers
// ---------------------------------------------------------------------------

fn natives_version_json(lib: &str, rel: &str, content: &[u8]) -> Value {
    json!({
        "libraries": [{
            "name": lib,
            "downloads": { "artifact": {
                "url": "https://example.com/fake-natives.jar",
                "path": rel,
                "sha1": sha1_hex(content),
                "size": content.len(),
            } },
        }],
        "downloads": {},
    })
}

fn natives_entries(dir: &Path, prefix: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix))
        .collect()
}

#[tokio::test]
async fn natives_marker_skips_second_install_and_force_extract_reextracts() {
    let (_d, e, srv) = offline_engine().await;
    let instance = InstanceMeta { name: "test-natives".into(), version: "1.21.1".into(), ..Default::default() };
    let content = format!("dummy native dll content {}", crate::platform::now_ms()).into_bytes();
    let rel = "org/test/natives-test/1.0/natives-test-1.0-natives-windows.jar";
    let vj = natives_version_json("org.test:natives-test:1.0:natives-windows", rel, &content);
    let lib_dest = join_rel(&e.paths().libraries_dir(), rel);
    std::fs::create_dir_all(lib_dest.parent().unwrap()).unwrap();
    std::fs::write(&lib_dest, &content).unwrap();
    let no_seed = InstallLibrariesOptions { seed_from_fast_client: false, force_extract: false };

    let r1 = install_libraries(&e, &instance, &vj, no_seed).await.unwrap();
    assert_eq!(r1.natives_extracted, 1);
    assert_eq!(r1.library_count, 0);
    let natives_dir = e.paths().instance_natives_dir(&instance.name);
    let extracted = natives_dir.join(lib_dest.file_name().unwrap());
    assert!(extracted.exists(), "non-zip natives are copied as-is");
    let markers = natives_entries(&natives_dir, ".extract-ok-");
    let metas = natives_entries(&natives_dir, ".extract-meta-");
    assert_eq!((markers.len(), metas.len()), (1, 1));
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(natives_dir.join(&metas[0])).unwrap()).unwrap();
    let st = std::fs::metadata(&lib_dest).unwrap();
    assert_eq!(meta["size"].as_u64(), Some(st.len()));
    assert!((meta["mtimeMs"].as_f64().unwrap() - crate::jvm::node_mtime_ms(&st)).abs() < 1e-3);

    std::fs::remove_file(&extracted).unwrap();
    install_libraries(&e, &instance, &vj, no_seed).await.unwrap();
    assert!(!extracted.exists(), "fresh marker -> extraction skipped");
    assert!(natives_dir.join(&markers[0]).exists());

    install_libraries(&e, &instance, &vj, InstallLibrariesOptions { force_extract: true, ..no_seed }).await.unwrap();
    assert!(extracted.exists(), "forceExtract re-extracts");
    assert_eq!(natives_entries(&natives_dir, ".extract-ok-").len(), 1);
    assert_eq!(srv.hits(), 0);
}

#[tokio::test]
async fn natives_marker_invalidated_by_source_change() {
    let (_d, e, srv) = offline_engine().await;
    let instance = InstanceMeta { name: "test-natives2".into(), version: "1.21.1".into(), ..Default::default() };
    let rel = "org/test/natives-mtime/1.0/natives-mtime-1.0-natives-windows.jar";
    let lib = "org.test:natives-mtime:1.0:natives-windows";
    let lib_dest = join_rel(&e.paths().libraries_dir(), rel);
    std::fs::create_dir_all(lib_dest.parent().unwrap()).unwrap();
    std::fs::write(&lib_dest, b"content v1").unwrap();
    let no_seed = InstallLibrariesOptions { seed_from_fast_client: false, force_extract: false };
    install_libraries(&e, &instance, &natives_version_json(lib, rel, b"content v1"), no_seed).await.unwrap();
    let extracted = e.paths().instance_natives_dir(&instance.name).join(lib_dest.file_name().unwrap());
    std::fs::remove_file(&extracted).unwrap();
    tokio::time::sleep(Duration::from_millis(15)).await;
    std::fs::write(&lib_dest, b"content v2 different").unwrap();
    install_libraries(&e, &instance, &natives_version_json(lib, rel, b"content v2 different"), no_seed).await.unwrap();
    assert_eq!(std::fs::read_to_string(&extracted).unwrap(), "content v2 different");
    assert_eq!(srv.hits(), 0);
}

// ---------------------------------------------------------------------------
// C16: version-json single read + manifest memo
// ---------------------------------------------------------------------------

fn write_manifest(e: &Engine, versions: Value) {
    crate::paths::write_json_atomic(
        &e.paths().version_manifest_cache(),
        &json!({ "fetched_at": crate::platform::now_ms(), "latest_release": "", "latest_snapshot": "", "versions": versions }),
    )
    .unwrap();
}

#[tokio::test]
async fn version_json_cache_hit_is_sha1_verified_and_mismatch_refetches() {
    let (_d, e) = temp_engine();
    let fresh_text = json!({ "id": "single-read-test", "mainClass": "fresh" }).to_string();
    let ft = fresh_text.clone();
    let srv = MockServer::start(move |_| MockResponse::ok(&ft)).await;
    mocked(&e, &srv);
    let id = "single-read-test";
    let text =
        json!({ "id": id, "mainClass": "net.minecraft.client.main.Main", "minimumLauncherVersion": 21 }).to_string();
    let file = e.paths().versions_dir().join(format!("{id}.json"));
    std::fs::create_dir_all(e.paths().versions_dir()).unwrap();
    std::fs::write(&file, &text).unwrap();
    let url = srv.url("/v/single-read-test.json");
    write_manifest(
        &e,
        json!([{ "id": id, "type": "release", "release_time": "", "sha1": sha1_hex(text.as_bytes()), "url": url }]),
    );
    let out = get_version_json(&e, id).await.unwrap();
    assert_eq!(out["mainClass"], "net.minecraft.client.main.Main");
    assert_eq!(srv.hits(), 0, "cached sha matches -> no network");

    std::fs::write(&file, json!({ "id": id, "corrupted": true }).to_string()).unwrap();
    write_manifest(
        &e,
        json!([{ "id": id, "type": "release", "release_time": "", "sha1": sha1_hex(fresh_text.as_bytes()), "url": url }]),
    );
    e.game_resolve_state().clear_manifest_memo();
    let out2 = get_version_json(&e, id).await.unwrap();
    assert_eq!(out2["mainClass"], "fresh");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), fresh_text);

    // A download that does not match the manifest sha1 is rejected.
    write_manifest(
        &e,
        json!([{ "id": id, "type": "release", "release_time": "", "sha1": "0".repeat(40), "url": url }]),
    );
    e.game_resolve_state().clear_manifest_memo();
    let err = get_version_json(&e, id).await.unwrap_err();
    assert!(err.has_code("sha1_mismatch"));
    assert_eq!(err.status(), 502);
}

#[tokio::test]
async fn version_json_manifest_memo_hits_when_file_unchanged() {
    let (_d, e, srv) = offline_engine().await;
    write_manifest(
        &e,
        json!([
            { "id": "memo-a", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/memo-a.json" },
            { "id": "memo-b", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/memo-b.json" },
        ]),
    );
    for v in ["memo-a", "memo-b"] {
        std::fs::write(e.paths().versions_dir().join(format!("{v}.json")), json!({ "id": v }).to_string()).unwrap();
    }
    assert_eq!(get_version_json(&e, "memo-a").await.unwrap()["id"], "memo-a");
    assert!(e.game_resolve_state().manifest_memo.lock().unwrap().is_some());
    // Same manifest mtime+size -> served from the memo.
    assert_eq!(get_version_json(&e, "memo-b").await.unwrap()["id"], "memo-b");
    e.game_resolve_state().clear_manifest_memo();
    assert_eq!(get_version_json(&e, "memo-a").await.unwrap()["id"], "memo-a");
    let err = get_version_json(&e, "nope").await.unwrap_err();
    assert!(err.has_code("version_not_found"));
    assert_eq!(srv.hits(), 0);
}

#[tokio::test]
async fn resolve_version_reports_java_tier_support() {
    let (_d, e, _srv) = offline_engine().await;
    write_manifest(
        &e,
        json!([
            { "id": "1.21.1", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/a.json" },
            { "id": "old", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/b.json" },
            { "id": "weird", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/c.json" },
        ]),
    );
    let vd = e.paths().versions_dir();
    std::fs::write(vd.join("1.21.1.json"), json!({ "javaVersion": { "majorVersion": 21 } }).to_string()).unwrap();
    std::fs::write(vd.join("old.json"), json!({ "id": "old" }).to_string()).unwrap();
    std::fs::write(vd.join("weird.json"), json!({ "javaVersion": { "majorVersion": 11 } }).to_string()).unwrap();
    let a = resolve_version(&e, "1.21.1").await.unwrap();
    assert_eq!((a.java_major, a.supported, a.reason.as_str()), (21, true, ""));
    assert_eq!(resolve_version(&e, "old").await.unwrap().java_major, 8);
    let w = resolve_version(&e, "weird").await.unwrap();
    assert!(!w.supported);
    assert_eq!(w.reason, "requires JDK 11 — the launcher provisions Temurin 8/16/17/21/25");
    assert!(resolve_version(&e, "missing").await.unwrap_err().has_code("version_not_found"));
}

// ---------------------------------------------------------------------------
// resolveFabric sha1 hydration (AOT-cache churn fix)
// ---------------------------------------------------------------------------

const INTERMEDIARY_SHA1: &str = "29daec33f16264c910f74a926406bae5cf36d81e";
const LOADER_SHA1: &str = "354dfaa02d0552e11867f85dff7cdbfaf813ba3e";

fn seed_fabric_cache(e: &Engine) -> PathBuf {
    let vd = e.paths().versions_dir();
    crate::paths::write_json_atomic(
        &vd.join("fabric-loaders.json"),
        &json!({
            "fetched_at": crate::platform::now_ms(),
            "loaders": [{ "version": "0.19.3", "stable": true, "build": 3, "maven": "net.fabricmc:fabric-loader:0.19.3" }],
            "latest_stable": "0.19.3",
        }),
    )
    .unwrap();
    let combo_file = vd.join("fabric-loader-26.2-0.19.3.json");
    crate::paths::write_json_atomic(
        &combo_file,
        &json!({
            "loader": { "version": "0.19.3", "maven": "net.fabricmc:fabric-loader:0.19.3" },
            "intermediary": { "version": "0.0.0", "maven": "net.fabricmc:intermediary:0.0.0" },
            "launcherMeta": {
                "version": 2,
                "mainClass": { "client": "net.fabricmc.loader.impl.launch.knot.KnotClient" },
                "libraries": { "common": [
                    { "name": "org.ow2.asm:asm:9.10.1", "url": "https://maven.fabricmc.net/", "sha1": "a".repeat(40), "size": 126151 }
                ] },
            },
        }),
    )
    .unwrap();
    combo_file
}

fn sidecars(r: &MockRequest) -> MockResponse {
    if r.path.ends_with("intermediary-0.0.0.jar.sha1") {
        MockResponse::ok(&format!("{INTERMEDIARY_SHA1}\n"))
    } else if r.path.ends_with("fabric-loader-0.19.3.jar.sha1") {
        MockResponse::ok(LOADER_SHA1)
    } else {
        panic!("unexpected fetch: {}", r.path)
    }
}

fn by_path(p: &FabricProfile) -> HashMap<String, LibraryDownload> {
    p.libraries.iter().map(|l| (l.rel_path.clone(), l.clone())).collect()
}

#[tokio::test]
async fn resolve_fabric_hydrates_intermediary_and_loader_sha1() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(sidecars).await;
    mocked(&e, &srv);
    seed_fabric_cache(&e);
    let profile = resolve_fabric(&e, "26.2", None).await.unwrap();
    let m = by_path(&profile);
    assert_eq!(m["net/fabricmc/intermediary/0.0.0/intermediary-0.0.0.jar"].sha1.as_deref(), Some(INTERMEDIARY_SHA1));
    assert_eq!(m["net/fabricmc/fabric-loader/0.19.3/fabric-loader-0.19.3.jar"].sha1.as_deref(), Some(LOADER_SHA1));
    assert_eq!(m["org/ow2/asm/asm/9.10.1/asm-9.10.1.jar"].sha1.as_deref(), Some(&*"a".repeat(40)));
    let paths: Vec<String> = srv.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(paths.iter().filter(|p| p.ends_with(".sha1")).count(), 2);
    assert!(!paths.iter().any(|p| p.contains("asm")));
    assert!(paths.iter().all(|p| p.starts_with("/fabric-maven/net/fabricmc/")));
    assert_eq!(profile.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
    assert_eq!(profile.jvm_args, vec![Value::from("-DFabricMcEmu= net.minecraft.client.main.Main ")]);
    assert_eq!(profile.min_java_version, Some(8));
    assert_eq!(profile.loader_maven.as_deref(), Some("net.fabricmc:fabric-loader:0.19.3"));
}

#[tokio::test]
async fn resolve_fabric_memoizes_hydrated_hashes_in_combo_cache() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(sidecars).await;
    mocked(&e, &srv);
    let combo_file = seed_fabric_cache(&e);
    resolve_fabric(&e, "26.2", None).await.unwrap();
    let cached: Value = serde_json::from_str(&std::fs::read_to_string(&combo_file).unwrap()).unwrap();
    assert_eq!(
        cached["espectral_maven_sha1"]["net/fabricmc/intermediary/0.0.0/intermediary-0.0.0.jar"],
        INTERMEDIARY_SHA1
    );
    // Second resolve: memo satisfies both hashes with no network at all.
    let offline = MockServer::start(|r| panic!("memoized hashes must not re-fetch: {}", r.path)).await;
    mocked(&e, &offline);
    let again = resolve_fabric(&e, "26.2", None).await.unwrap();
    let m = by_path(&again);
    assert_eq!(m["net/fabricmc/intermediary/0.0.0/intermediary-0.0.0.jar"].sha1.as_deref(), Some(INTERMEDIARY_SHA1));
    assert_eq!(m["net/fabricmc/fabric-loader/0.19.3/fabric-loader-0.19.3.jar"].sha1.as_deref(), Some(LOADER_SHA1));
    assert_eq!(offline.hits(), 0);
}

#[tokio::test]
async fn resolve_fabric_survives_unreachable_sidecar() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| MockResponse::drop_connection()).await;
    mocked(&e, &srv);
    seed_fabric_cache(&e);
    let profile = resolve_fabric(&e, "26.2", None).await.unwrap();
    let inter = profile.libraries.iter().find(|l| l.rel_path.contains("intermediary")).unwrap();
    assert_eq!(inter.sha1, None, "no sidecar -> hashless, not a failed launch");
    assert_eq!(profile.loader_version, "0.19.3");
}

#[tokio::test]
async fn resolve_fabric_unknown_loader_is_404() {
    let (_d, e, _srv) = offline_engine().await;
    seed_fabric_cache(&e);
    let err = resolve_fabric(&e, "26.2", Some("9.9.9")).await.unwrap_err();
    assert!(err.has_code("fabric_loader_not_found"));
    assert_eq!(err.to_string(), "fabric loader not found: 9.9.9");
}

#[tokio::test]
async fn resolve_fabric_fetches_and_caches_missing_combo() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|r| {
        if r.path == "/fabric-meta/v2/versions/loader/1.21.1/0.16.10" {
            MockResponse::json(&json!({
                "intermediary": { "maven": "net.fabricmc:intermediary:1.21.1" },
                "launcherMeta": { "min_java_version": 21, "libraries": { "common": [] }, "mainClass": { "client": "K" } },
                "arguments": { "jvm": ["-Dx=1"] },
            }))
        } else if r.path.ends_with(".sha1") {
            MockResponse::status(404)
        } else {
            panic!("unexpected {}", r.path)
        }
    })
    .await;
    mocked(&e, &srv);
    crate::paths::write_json_atomic(
        &e.paths().versions_dir().join("fabric-loaders.json"),
        &json!({ "fetched_at": crate::platform::now_ms(), "latest_stable": "0.16.10",
                 "loaders": [{ "version": "0.16.10", "stable": true, "maven": "net.fabricmc:fabric-loader:0.16.10" }] }),
    )
    .unwrap();
    let p = resolve_fabric(&e, "1.21.1", None).await.unwrap();
    assert_eq!((p.main_class.as_str(), p.min_java_version), ("K", Some(21)));
    assert_eq!(p.jvm_args, vec![Value::from("-Dx=1")]);
    assert_eq!(p.libraries.len(), 2);
    assert!(e.paths().versions_dir().join("fabric-loader-1.21.1-0.16.10.json").exists());
}

// ---------------------------------------------------------------------------
// neoforge.test.mjs — local version json + inheritance chain merge
// ---------------------------------------------------------------------------

#[test]
fn read_local_version_json_reads_local_only_profiles() {
    let (_d, e) = temp_engine();
    let id = "neoforge-21.1.242";
    let p = e.paths().version_json(id);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::copy(fixture("neoforge-21.1.242-version.json"), &p).unwrap();
    assert_eq!(read_local_version_json(&e, id).unwrap()["id"], id);
    assert!(read_local_version_json(&e, "does-not-exist").is_none());
}

async fn merged_fixture() -> Value {
    let parent = fixture_json("vanilla-1.21.1-mini.json");
    resolve_version_chain_with(fixture_json("neoforge-21.1.242-version.json"), move |_| {
        let p = parent.clone();
        async move { Ok(Some(p)) }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn chain_merges_neoforge_profile_over_vanilla_parent() {
    let merged = merged_fixture().await;
    assert_eq!(merged["id"], "neoforge-21.1.242");
    assert_eq!(merged["mainClass"], "cpw.mods.bootstraplauncher.BootstrapLauncher");
    assert_eq!(merged["assetIndex"]["id"], "17");
    assert_eq!(merged["javaVersion"]["majorVersion"], 21);
    // 47 child + 4 parent libs minus the gson collision = 50
    let libs = merged["libraries"].as_array().unwrap();
    assert_eq!(libs.len(), 50);
    let first = libs[0]["name"].as_str().unwrap();
    assert!(first.starts_with("cpw.mods:") || first.starts_with("net.neoforged"), "{first}");
    assert!(merged["downloads"]["client"]["url"].is_string(), "client download must come from the parent");
}

#[tokio::test]
async fn chain_jvm_args_merge_child_first() {
    let merged = merged_fixture().await;
    let jvm = merged["arguments"]["jvm"].as_array().unwrap();
    assert!(jvm.contains(&Value::from("-p")));
    assert!(jvm.contains(&Value::from("-cp")));
    assert!(jvm.iter().any(|a| a.as_str().map(|s| s.starts_with("-DlibraryDirectory=")).unwrap_or(false)));
    assert!(jvm.iter().any(|a| a.as_str().map(|s| s.contains("${classpath_separator}")).unwrap_or(false)));
    assert_eq!(jvm[0], "-Djava.net.preferIPv6Addresses=system");
    assert!(jvm.iter().any(|a| a.get("rules").map(Value::is_array).unwrap_or(false)));
    // Repeated flags survive (NeoForge repeats --add-opens once per export).
    assert!(jvm.iter().filter(|a| a.as_str() == Some("--add-opens")).count() > 1);
}

#[tokio::test]
async fn chain_game_args_merge_child_first() {
    let merged = merged_fixture().await;
    let game = merged["arguments"]["game"].as_array().unwrap();
    assert_eq!(game[0], "--fml.neoForgeVersion");
    assert!(game.contains(&Value::from("--launchTarget")));
    assert!(game.contains(&Value::from("--username")));
    assert!(game.iter().any(|a| a.get("rules").map(Value::is_array).unwrap_or(false)));
}

#[tokio::test]
async fn chain_multi_level_and_cycle_guard() {
    let a = json!({ "id": "a", "mainClass": "A", "libraries": [{ "name": "x:1", "url": "u" }] });
    let b = json!({ "id": "b", "inheritsFrom": "a", "libraries": [{ "name": "b:1" }] });
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let c2 = calls.clone();
    let merged = resolve_version_chain_with(b, move |id| {
        c2.lock().unwrap().push(id.clone());
        let a = a.clone();
        async move { Ok((id == "a").then_some(a)) }
    })
    .await
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["a".to_string()]);
    assert_eq!(merged["id"], "b");
    assert_eq!(merged["mainClass"], "A");
    assert_eq!(merged["libraries"].as_array().unwrap().len(), 2);

    let cyc = json!({ "id": "c1", "inheritsFrom": "c2", "libraries": [] });
    let err = resolve_version_chain_with(cyc, |_| async {
        Ok(Some(json!({ "id": "c2", "inheritsFrom": "c1", "libraries": [] })))
    })
    .await
    .unwrap_err();
    assert!(err.has_code("INHERITS_BREAK"));

    let err = resolve_version_chain_with(json!({ "id": "x", "inheritsFrom": "nope", "libraries": [] }), |_| async {
        Ok(None)
    })
    .await
    .unwrap_err();
    assert!(err.has_code("INHERITS_BREAK"));
    assert_eq!(err.to_string(), "parent version nope not found for x");
    assert_eq!(err.status(), 502);
}

#[tokio::test]
async fn chain_default_getter_uses_version_json_cache() {
    let (_d, e, _srv) = offline_engine().await;
    write_manifest(
        &e,
        json!([{ "id": "1.21.1", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/x.json" }]),
    );
    std::fs::copy(fixture("vanilla-1.21.1-mini.json"), e.paths().versions_dir().join("1.21.1.json")).unwrap();
    let merged = resolve_version_chain(&e, fixture_json("neoforge-21.1.242-version.json")).await.unwrap();
    assert_eq!(merged["libraries"].as_array().unwrap().len(), 50);
}

#[test]
fn merge_args_keeps_flags_and_dedupes_values() {
    let out = merge_args(
        Some(&[json!("--a"), json!("--b")]),
        Some(&[json!("--b"), json!("--c"), json!({ "rules": [], "value": "--z" })]),
    );
    assert_eq!(
        out,
        vec![json!("--a"), json!("--b"), json!("--b"), json!("--c"), json!({ "rules": [], "value": "--z" })]
    );
    let out = merge_args(Some(&[json!("x"), json!({ "v": 1 })]), Some(&[json!("x"), json!({ "v": 1 }), json!("y")]));
    assert_eq!(out, vec![json!("x"), json!({ "v": 1 }), json!("y")]);
}

#[test]
fn dedupe_by_name_earlier_lists_win() {
    let a = [json!({ "name": "x:1", "url": "child" }), json!({ "name": "y:1", "url": "child" })];
    let b = [
        json!({ "name": "x:1", "url": "parent" }),
        json!({ "name": "z:1", "url": "parent" }),
        json!({ "url": "noname" }),
    ];
    let out = dedupe_by_name(&[&a, &b]);
    let urls: Vec<&str> = out.iter().map(|l| l["url"].as_str().unwrap()).collect();
    assert_eq!(urls, vec!["child", "child", "parent"]);
}

// ---------------------------------------------------------------------------
// Libraries / assets / seed
// ---------------------------------------------------------------------------

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[tokio::test]
async fn install_libraries_downloads_classpath_legacy_natives_and_client_jar() {
    let (_d, e) = temp_engine();
    let lib_bytes = b"jar-a".to_vec();
    let natives_zip = zip_bytes(&[("liblegacy.so", b"so"), ("META-INF/MANIFEST.MF", b"m")]);
    let client = b"client-jar".to_vec();
    let (lb, nz, cb) = (lib_bytes.clone(), natives_zip.clone(), client.clone());
    let srv = MockServer::start(move |r| match r.path.as_str() {
        "/lib/a.jar" => MockResponse::bytes(lb.clone()),
        "/lib/natives.jar" => MockResponse::bytes(nz.clone()),
        "/client.jar" => MockResponse::bytes(cb.clone()),
        p => panic!("unexpected {p}"),
    })
    .await;
    mocked(&e, &srv);
    let os = crate::platform::os_name();
    let vj = json!({
        "libraries": [
            {
                "name": "org.legacy:lwjgl-platform:2.9.4",
                "downloads": {
                    "artifact": { "url": srv.url("/lib/a.jar"), "sha1": sha1_hex(&lib_bytes), "size": lib_bytes.len() },
                    "classifiers": { format!("natives-{os}"): {
                        "url": srv.url("/lib/natives.jar"), "path": "org/legacy/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4-natives.jar",
                        "sha1": sha1_hex(&natives_zip), "size": natives_zip.len()
                    } }
                },
                "natives": { os: format!("natives-{os}") },
                "extract": { "exclude": ["META-INF/"] }
            },
            { "name": "never:on:this-os", "rules": [{ "action": "allow", "os": { "name": "nonexistent" } }] }
        ],
        "downloads": { "client": { "url": srv.url("/client.jar"), "sha1": sha1_hex(&client), "size": client.len() } }
    });
    let inst = InstanceMeta { name: "legacy".into(), version: "1.12.2".into(), ..Default::default() };
    let mut rx = e.subscribe();
    let r = install_libraries(
        &e,
        &inst,
        &vj,
        InstallLibrariesOptions { seed_from_fast_client: false, force_extract: false },
    )
    .await
    .unwrap();
    assert_eq!((r.library_count, r.downloaded, r.natives_extracted, r.seeded), (1, 3, 1, 0));
    assert_eq!(r.client_jar.as_deref(), Some(e.paths().client_jar("1.12.2").as_path()));
    assert_eq!(std::fs::read(e.paths().client_jar("1.12.2")).unwrap(), client);
    let natives = e.paths().instance_natives_dir("legacy");
    assert!(natives.join("liblegacy.so").exists());
    assert!(!natives.join("META-INF").exists());
    let mut kinds = vec![];
    while let Ok(ev) = rx.try_recv() {
        if let EngineEvent::ModProgress(p) = ev {
            if p.phase.as_deref() == Some("done") {
                kinds.push(p.kind.unwrap());
            }
        }
    }
    kinds.sort();
    assert_eq!(kinds, vec!["client", "library", "natives"]);
}

#[tokio::test]
async fn install_libraries_seeds_from_fastclient_root() {
    let (d, e, srv) = offline_engine().await;
    let root = d.path().join("fastclient");
    let rel = "com/x/y/1/y-1.jar";
    let bytes = b"seeded jar".to_vec();
    let src = join_rel(&root.join("libraries"), rel);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, &bytes).unwrap();
    e.game_resolve_state().update_hooks(|h| h.fast_client_root = Some(Some(root.clone())));
    let vj = json!({ "libraries": [{ "name": "com.x:y:1", "downloads": { "artifact": {
        "url": "https://example.com/never", "sha1": sha1_hex(&bytes), "size": bytes.len() } } }] });
    let inst = InstanceMeta { name: "seeded".into(), version: "1.21.1".into(), ..Default::default() };
    let r = install_libraries(&e, &inst, &vj, InstallLibrariesOptions::default()).await.unwrap();
    assert_eq!((r.seeded, r.downloaded), (1, 0));
    assert_eq!(std::fs::read(join_rel(&e.paths().libraries_dir(), rel)).unwrap(), bytes);
    assert_eq!(srv.hits(), 0);
    assert!(install_libraries(&e, &InstanceMeta { name: "bad/x".into(), ..inst }, &vj, Default::default())
        .await
        .unwrap_err()
        .has_code("invalid_instance_name"));
}

#[tokio::test]
async fn install_assets_dedupes_hashes_and_emits_done() {
    let (_d, e) = temp_engine();
    let obj_a = b"asset-a".to_vec();
    let obj_b = b"asset-b".to_vec();
    let (ha, hb) = (sha1_hex(&obj_a), sha1_hex(&obj_b));
    let index = json!({ "objects": {
        "minecraft/sounds/a.ogg": { "hash": ha, "size": obj_a.len() },
        "minecraft/sounds/a-copy.ogg": { "hash": ha, "size": obj_a.len() },
        "icons/b.png": { "hash": hb, "size": obj_b.len() },
    } })
    .to_string();
    let idx = index.clone();
    let (oa, ob, ha2, hb2) = (obj_a.clone(), obj_b.clone(), ha.clone(), hb.clone());
    let srv = MockServer::start(move |r| {
        if r.path == "/idx/17.json" {
            MockResponse::ok(&idx)
        } else if r.path == format!("/assets/{}/{}", &ha2[..2], ha2) {
            MockResponse::bytes(oa.clone())
        } else if r.path == format!("/assets/{}/{}", &hb2[..2], hb2) {
            MockResponse::bytes(ob.clone())
        } else {
            panic!("unexpected {}", r.path)
        }
    })
    .await;
    mocked(&e, &srv);
    let vj =
        json!({ "assetIndex": { "id": "17", "url": srv.url("/idx/17.json"), "sha1": sha1_hex(index.as_bytes()) } });
    let inst = InstanceMeta { name: "assets".into(), version: "1.21.1".into(), ..Default::default() };
    let mut rx = e.subscribe();
    let r = install_assets(&e, &inst, &vj).await.unwrap();
    assert_eq!(r, AssetsInstall { index: Some("17".into()), objects: 3 });
    assert_eq!(srv.hits(), 3, "index + one download per unique hash");
    assert!(e.paths().assets_dir().join("objects").join(&ha[..2]).join(&ha).exists());
    assert!(e.paths().asset_index("17").exists());
    let mut done = None;
    while let Ok(ev) = rx.try_recv() {
        if let EngineEvent::AssetProgress(p) = ev {
            if p.kind.as_deref() == Some("index") {
                done = Some(p);
            }
        }
    }
    let done = done.expect("asset index done event");
    assert_eq!(done.index, Some(Value::from("17")));
    assert_eq!(done.message.as_deref(), Some("asset index 17: 3 objects"));
    assert_eq!(install_assets(&e, &inst, &json!({})).await.unwrap(), AssetsInstall { index: None, objects: 0 });
}

#[tokio::test]
async fn seed_instance_fabric_writes_profile_and_creates_dirs() {
    let (_d, e) = temp_engine();
    // Every fabric jar is served by the mock; sidecars carry the real sha1.
    let jar = |p: &str| format!("jar:{p}").into_bytes();
    let srv = MockServer::start(move |r| {
        if let Some(p) = r.path.strip_suffix(".sha1") {
            MockResponse::ok(&sha1_hex(&jar(p)))
        } else if r.path.starts_with("/fabric-maven/") {
            MockResponse::bytes(jar(&r.path))
        } else {
            panic!("unexpected {}", r.path)
        }
    })
    .await;
    mocked(&e, &srv);
    let vd = e.paths().versions_dir();
    crate::paths::write_json_atomic(
        &vd.join("fabric-loaders.json"),
        &json!({ "fetched_at": crate::platform::now_ms(), "latest_stable": "0.19.3",
                 "loaders": [{ "version": "0.19.3", "stable": true, "maven": "net.fabricmc:fabric-loader:0.19.3" }] }),
    )
    .unwrap();
    crate::paths::write_json_atomic(
        &vd.join("fabric-loader-26.2-0.19.3.json"),
        &json!({
            "intermediary": { "maven": "net.fabricmc:intermediary:26.2" },
            "launcherMeta": { "libraries": { "common": [{ "name": "org.ow2.asm:asm:9.10.1", "url": srv.url("/fabric-maven/") }] } },
        }),
    )
    .unwrap();
    write_manifest(
        &e,
        json!([{ "id": "26.2", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/26.2.json" }]),
    );
    std::fs::write(vd.join("26.2.json"), json!({ "id": "26.2", "libraries": [] }).to_string()).unwrap();
    let inst =
        InstanceMeta { name: "fab".into(), version: "26.2".into(), loader: Loader::Fabric, ..Default::default() };
    let r = seed_instance(&e, &inst, false).await.unwrap();
    let f = r.fabric.unwrap();
    assert_eq!((f.library_count, f.downloaded, f.seeded), (Some(3), Some(3), Some(0)));
    assert!(f.libraries.iter().all(|l| l.sha1.is_some()), "every jar hydrated");
    for sub in ["mods", "natives", "logs", "saves", "config", "cache"] {
        assert!(e.paths().instance_dir("fab").join(sub).is_dir(), "{sub}");
    }
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(e.paths().instance_fabric_profile("fab")).unwrap()).unwrap();
    let keys: Vec<&str> = doc.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["mc_version", "loader_version", "main_class", "jvm_args", "libraries"]);
    assert_eq!(doc["libraries"][0]["relPath"], "org/ow2/asm/asm/9.10.1/asm-9.10.1.jar");
    assert!(join_rel(&e.paths().libraries_dir(), "net/fabricmc/intermediary/26.2/intermediary-26.2.jar").exists());

    let neo =
        InstanceMeta { name: "neo".into(), version: "26.2".into(), loader: Loader::Neoforge, ..Default::default() };
    assert!(seed_instance(&e, &neo, false).await.unwrap_err().has_code("LOADER_VERSION_REQUIRED"));
}

#[tokio::test]
async fn seed_instance_vanilla_end_to_end() {
    let (_d, e) = temp_engine();
    let index = json!({ "objects": {} }).to_string();
    let idx = index.clone();
    let srv = MockServer::start(move |r| match r.path.as_str() {
        "/idx.json" => MockResponse::ok(&idx),
        "/client.jar" => MockResponse::ok("client"),
        p => panic!("unexpected {p}"),
    })
    .await;
    mocked(&e, &srv);
    write_manifest(
        &e,
        json!([{ "id": "1.21.1", "type": "release", "release_time": "", "sha1": null, "url": "https://example.com/x.json" }]),
    );
    let vj = json!({ "id": "1.21.1", "libraries": [], "assetIndex": { "id": "17", "url": srv.url("/idx.json") },
                     "downloads": { "client": { "url": srv.url("/client.jar") } } });
    std::fs::write(e.paths().versions_dir().join("1.21.1.json"), vj.to_string()).unwrap();
    let inst = InstanceMeta { name: "van".into(), version: "1.21.1".into(), ..Default::default() };
    let r = seed_instance(&e, &inst, false).await.unwrap();
    assert_eq!(r.libraries.downloaded, 1);
    assert_eq!(r.assets.index.as_deref(), Some("17"));
    assert!(r.fabric.is_none() && r.neoforge.is_none());
    assert_eq!(std::fs::read_to_string(e.paths().client_jar("1.21.1")).unwrap(), "client");
}
