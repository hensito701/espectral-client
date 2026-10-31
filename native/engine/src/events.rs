//! Port of `src/engine/events.mjs` — the engine event stream.
//!
//! The JS engine pushed Server-Sent Events (`event: <name>\ndata: <json>`) to
//! every open `GET /api/events` connection. The native engine replaces SSE
//! with a [`tokio::sync::broadcast`] channel of typed [`EngineEvent`]s: the UI
//! calls [`crate::Engine::subscribe`] and receives every event emitted after
//! that point. Emitting never blocks and never fails (no subscriber = dropped,
//! like SSE with no open connection; a lagging subscriber loses the oldest
//! events and sees `RecvError::Lagged`).
//!
//! This enum enumerates EVERY event the JS engine emits across all modules
//! (the SSE contract the UI listens to):
//!
//! | SSE name          | emitted by (JS)                                         | variant |
//! |-------------------|---------------------------------------------------------|---------|
//! | `launch-log`      | routes/launch.mjs (prepare progress, game output)       | [`EngineEvent::LaunchLog`] |
//! | `launch-exit`     | routes/launch.mjs (exit, resolve failure, cancel)       | [`EngineEvent::LaunchExit`] |
//! | `launch-ready`    | routes/launch.mjs (game reached the menu marker)        | [`EngineEvent::LaunchReady`] |
//! | `mod-progress`    | mods, instances (seeding/import), resolver libs, neoforge, mrpack files (via download_all) | [`EngineEvent::ModProgress`] |
//! | `asset-progress`  | resolver.mjs installAssets (via download_all + done)    | [`EngineEvent::AssetProgress`] |
//! | `train-progress`  | aot.mjs trainInstance                                   | [`EngineEvent::TrainProgress`] |
//! | `train-done`      | aot.mjs trainInstance, routes/launch.mjs autoTrain      | [`EngineEvent::TrainDone`] |
//! | `import-progress` | mrpack.mjs (files / overrides)                          | [`EngineEvent::ImportProgress`] |
//! | `import-done`     | mrpack.mjs importMrpack                                 | [`EngineEvent::ImportDone`] |
//! | `agent-state`     | routes/agent.mjs mod relay (`POST /api/bridge/agent/state`) | [`EngineEvent::AgentState`] |
//!
//! Payload structs serialize to exactly the JS JSON payload (optional fields
//! are skipped when `None` unless the JS always sent them, e.g. `launch-exit`
//! carries explicit `null`s). `mod-progress` / `asset-progress` payloads were
//! free-form in JS (`{ instance, phase, ...context }`), so [`ProgressEvent`]
//! has the common keys typed plus a flattened `extra` map.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::broadcast;

/// Channel capacity: generous because asset installs emit thousands of
/// `asset-progress` events in bursts.
pub const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// The SSE event names, in the JS `EVENT_NAMES` order (+ asset-progress).
pub const EVENT_NAMES: &[&str] = &[
    "launch-log",
    "launch-exit",
    "launch-ready",
    "mod-progress",
    "asset-progress",
    "train-progress",
    "train-done",
    "import-progress",
    "import-done",
];

/// `launch-log` — one line appended to a launch's ring buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchLogEvent {
    pub key: String,
    pub seq: u64,
    pub line: String,
}

/// `launch-exit` — a launch finished (process exit, prepare failure, cancel,
/// spawn error, adopted game ended). All keys are always present (JS sent
/// explicit `null`s).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchExitEvent {
    pub key: String,
    pub instance: String,
    pub account: Option<String>,
    pub code: Option<i32>,
    pub signal: Option<String>,
    /// true = the game reached the menu marker; null when it never spawned.
    pub marker: Option<bool>,
    pub error: Option<String>,
    pub menu_ms: Option<i64>,
    pub played_ms: Option<i64>,
    pub spawn_ms: Option<i64>,
    pub boot_ms: Option<i64>,
    /// phase name -> ms epoch.
    #[serde(default)]
    pub phases: Map<String, Value>,
}

/// `launch-ready` — the game reached its main menu (UI may close the launcher).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchReadyEvent {
    pub key: String,
    pub instance: String,
}

/// `mod-progress` / `asset-progress` payload. JS shape:
/// `{ instance, phase: 'start'|'done'|'error'|…, message?, filename?, index?, total?, kind?, ...context }`.
///
/// `index` is a `Value` because `asset-progress` phase `done` sends the asset
/// index *id* (a string) in it while every other emitter sends a number.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProgressEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// instances.mjs seeding/import events use `status` ('start'|'done'|'error').
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// 'library' | 'natives' | 'client' | 'fabric-library' | 'neoforge-loader' | 'mrpack' | 'index' | 'object' …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest: Option<String>,
    /// download_all 'done' always sends `sha1` (null when unknown) — use
    /// `Some(Value::Null)` for that (an explicit null survives deserialization).
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub sha1: Option<Value>,
    /// asset-progress objects: the asset's logical path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Any other context key.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Key present (even as null) -> Some(value); absent -> None (via `default`).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

impl ProgressEvent {
    pub fn for_instance(instance: impl Into<String>) -> Self {
        ProgressEvent { instance: Some(instance.into()), ..Default::default() }
    }
    pub fn phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
    pub fn index(mut self, index: u64) -> Self {
        self.index = Some(Value::from(index));
        self
    }
    pub fn total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }
    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }
}

/// `train-progress` — `{ instance, key, phase, message, at }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainProgressEvent {
    pub instance: String,
    pub key: Option<String>,
    /// 'resolve' | 'retry' | 'key' | 'spawn' | 'running' | 'superseded' | 'timeout' | 'menu' | 'warn' | 'done' | 'failed'
    pub phase: String,
    pub message: String,
    /// ms epoch.
    pub at: i64,
}

/// `train-done` — `{ instance, key, ok, error?, skipped?, cache_size_bytes?, proof? }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainDoneEvent {
    pub instance: String,
    pub key: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_size_bytes: Option<u64>,
    /// AotProof object (see types::AotProof).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `import-progress` — `{ instance, phase: 'files'|'overrides', index, total, filename }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportProgressEvent {
    pub instance: String,
    pub phase: String,
    pub index: Option<u64>,
    pub total: u64,
    pub filename: Option<String>,
}

/// `import-done` — `{ instance, ok, error?, already_exists? }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportDoneEvent {
    pub instance: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already_exists: Option<bool>,
}

/// `agent-state` — the mod's latest state report for an agent launch
/// (relayed from `POST /api/bridge/agent/state`; JS emits it outside
/// `EVENT_NAMES`, so it is not part of that list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStateEvent {
    pub key: String,
    pub instance: String,
    pub phase: Option<String>,
    pub screen: Option<String>,
    pub server: Option<String>,
    pub world: Option<String>,
    pub context: Option<String>,
    pub paused: bool,
    pub recording: bool,
    pub updated_at: i64,
}

/// Every event the engine emits. Serialized as `{ "event": "<sse-name>", "data": {…} }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "kebab-case")]
pub enum EngineEvent {
    LaunchLog(LaunchLogEvent),
    LaunchExit(LaunchExitEvent),
    LaunchReady(LaunchReadyEvent),
    ModProgress(ProgressEvent),
    AssetProgress(ProgressEvent),
    TrainProgress(TrainProgressEvent),
    TrainDone(TrainDoneEvent),
    ImportProgress(ImportProgressEvent),
    ImportDone(ImportDoneEvent),
    AgentState(AgentStateEvent),
}

impl EngineEvent {
    /// The SSE event name (`event:` line) of the JS engine.
    pub fn name(&self) -> &'static str {
        match self {
            EngineEvent::LaunchLog(_) => "launch-log",
            EngineEvent::LaunchExit(_) => "launch-exit",
            EngineEvent::LaunchReady(_) => "launch-ready",
            EngineEvent::ModProgress(_) => "mod-progress",
            EngineEvent::AssetProgress(_) => "asset-progress",
            EngineEvent::TrainProgress(_) => "train-progress",
            EngineEvent::TrainDone(_) => "train-done",
            EngineEvent::ImportProgress(_) => "import-progress",
            EngineEvent::ImportDone(_) => "import-done",
            EngineEvent::AgentState(_) => "agent-state",
        }
    }

    /// The SSE `data:` payload as JSON (exactly what the JS engine sent).
    pub fn payload(&self) -> Value {
        let r = match self {
            EngineEvent::LaunchLog(p) => serde_json::to_value(p),
            EngineEvent::LaunchExit(p) => serde_json::to_value(p),
            EngineEvent::LaunchReady(p) => serde_json::to_value(p),
            EngineEvent::ModProgress(p) | EngineEvent::AssetProgress(p) => serde_json::to_value(p),
            EngineEvent::TrainProgress(p) => serde_json::to_value(p),
            EngineEvent::TrainDone(p) => serde_json::to_value(p),
            EngineEvent::ImportProgress(p) => serde_json::to_value(p),
            EngineEvent::ImportDone(p) => serde_json::to_value(p),
            EngineEvent::AgentState(p) => serde_json::to_value(p),
        };
        r.unwrap_or(Value::Null)
    }

    /// Instance the event is about, when it names one.
    pub fn instance(&self) -> Option<&str> {
        match self {
            EngineEvent::LaunchLog(_) => None,
            EngineEvent::LaunchExit(p) => Some(&p.instance),
            EngineEvent::LaunchReady(p) => Some(&p.instance),
            EngineEvent::ModProgress(p) | EngineEvent::AssetProgress(p) => p.instance.as_deref(),
            EngineEvent::TrainProgress(p) => Some(&p.instance),
            EngineEvent::TrainDone(p) => Some(&p.instance),
            EngineEvent::ImportProgress(p) => Some(&p.instance),
            EngineEvent::ImportDone(p) => Some(&p.instance),
            EngineEvent::AgentState(p) => Some(&p.instance),
        }
    }
}

/// Which progress channel a download batch reports on (download.mjs `eventName`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressChannel {
    Mod,
    Asset,
}

impl ProgressChannel {
    pub fn wrap(self, p: ProgressEvent) -> EngineEvent {
        match self {
            ProgressChannel::Mod => EngineEvent::ModProgress(p),
            ProgressChannel::Asset => EngineEvent::AssetProgress(p),
        }
    }
}

/// Broadcast hub (replaces the JS SSE client registry).
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<EngineEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        EventBus { tx }
    }

    /// Push one event to every subscriber. Never fails, never blocks
    /// (JS: the event sink must never break the emitter).
    pub fn emit(&self, event: EngineEvent) {
        tracing::trace!(event = event.name(), "emit");
        let _ = self.tx.send(event);
    }

    /// Receive every event emitted after this call.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn emit_reaches_every_subscriber_and_never_fails_without_one() {
        let bus = EventBus::new();
        bus.emit(EngineEvent::LaunchReady(LaunchReadyEvent { key: "k".into(), instance: "i".into() }));
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.emit(EngineEvent::LaunchLog(LaunchLogEvent { key: "k".into(), seq: 1, line: "x".into() }));
        assert_eq!(a.recv().await.unwrap().name(), "launch-log");
        assert_eq!(b.recv().await.unwrap().name(), "launch-log");
    }

    #[test]
    fn progress_payload_matches_js_shape() {
        let mut p = ProgressEvent::for_instance("inst").phase("start").index(0).total(3).filename("a.jar");
        p.extra.insert("preset".into(), json!("ignored-because-typed")); // flatten never duplicates
        p.preset = Some("performance".into());
        p.extra.remove("preset");
        let ev = EngineEvent::ModProgress(p);
        assert_eq!(
            ev.payload(),
            json!({ "instance": "inst", "phase": "start", "index": 0, "total": 3, "filename": "a.jar", "preset": "performance" })
        );
    }

    #[test]
    fn launch_exit_always_serializes_nulls() {
        let ev = EngineEvent::LaunchExit(LaunchExitEvent {
            key: "k".into(),
            instance: "i".into(),
            account: None,
            code: None,
            signal: None,
            marker: None,
            error: Some("cancelled during prepare".into()),
            menu_ms: None,
            played_ms: None,
            spawn_ms: None,
            boot_ms: None,
            phases: Map::new(),
        });
        let v = ev.payload();
        assert_eq!(v["account"], Value::Null);
        assert_eq!(v["marker"], Value::Null);
        assert_eq!(v["phases"], json!({}));
        assert_eq!(v.as_object().unwrap().len(), 12);
    }

    #[test]
    fn tagged_serialization_uses_sse_names() {
        let ev = EngineEvent::ImportDone(ImportDoneEvent {
            instance: "p".into(),
            ok: true,
            error: None,
            already_exists: Some(true),
        });
        let s = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            s,
            json!({ "event": "import-done", "data": { "instance": "p", "ok": true, "already_exists": true } })
        );
        let back: EngineEvent = serde_json::from_value(s).unwrap();
        assert_eq!(back, ev);
        for ev_name in EVENT_NAMES {
            assert!(ev_name.contains('-'));
        }
    }
}
