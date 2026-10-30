//! Integration-level checks of the Engine facade (template for porters:
//! use `espectral_engine::test_support` — temp data dir, loopback MockServer,
//! never the real network or home dir).

use espectral_engine::test_support::{temp_engine, MockResponse, MockServer};
use espectral_engine::{EngineError, EngineEvent};

#[tokio::test]
async fn unported_routes_report_not_implemented_501() {
    let (_d, e) = temp_engine();
    let err = e.list_versions().await.unwrap_err();
    assert!(matches!(err, EngineError::NotImplemented(_)));
    let env = err.envelope();
    assert_eq!((env.status, env.code.as_str()), (501, "NOT_IMPLEMENTED"));
}

#[tokio::test]
async fn core_routes_work_end_to_end() {
    let (_d, e) = temp_engine();
    assert!(e.health().await.unwrap().ok);
    let cfg = e.get_config().await.unwrap();
    assert_eq!(cfg.data_dir, e.paths().data_dir().to_string_lossy());
    assert!(e.paths().config_json().exists());
}

#[tokio::test]
async fn http_and_events_are_shared_by_the_engine() {
    let (_d, e) = temp_engine();
    let srv = MockServer::start(|_| MockResponse::json(&serde_json::json!({ "ok": true }))).await;
    let v: serde_json::Value = e.http().fetch_json(&srv.url("/x"), &Default::default()).await.unwrap();
    assert_eq!(v["ok"], true);
    let mut rx = e.subscribe();
    e.emit(EngineEvent::ImportDone(espectral_engine::events::ImportDoneEvent {
        instance: "p".into(),
        ok: true,
        error: None,
        already_exists: None,
    }));
    assert_eq!(rx.recv().await.unwrap().name(), "import-done");
}
