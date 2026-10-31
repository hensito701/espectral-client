//! Port of `src/engine/aot.mjs` — JEP 483 AOT cache: keys, classpath stamps,
//! staleness, proof logs, eviction, training runs (graceful close ladder),
//! the training queue.
//!
//! **Owner: game-launch.** Status: PORTED.
//!
//! key = sha256(version | javaBuild | os-arch) — mods are off-classpath, so
//! the mod set is NOT a key input (meta.json still records `mod_set_hash`).
//! Cache: `<data>/cache/aot/<key>/game.aot` + `meta.json`.
//!
//! Training: spawn the JVM with `-XX:AOTCacheOutput=…`, wait for the menu
//! marker in `<gameDir>/logs/latest.log` (init cap armed at first log
//! activity, hard cap regardless), then ask the game to close GRACEFULLY
//! (`taskkill /PID` without /F on Windows = WM_CLOSE to every top-level
//! window; SIGTERM elsewhere) and wait out the whole cache dump. NEVER
//! force-kill after the menu: the cache is written on normal JVM exit only.
//!
//! Proof: parsed from the newest `<gameDir>/aot-<pid>.log` written by any real
//! `-XX:AOTCache` launch (`Using AOT-linked classes: true`).
//!
//! Differences from the JS: a trainer spawn failure ends the run at once
//! (`spawn failed: …`) instead of waiting out the marker caps; a failing
//! `instances::compute_mod_set_hash` records `mod_set_hash: null`
//! (diagnostic-only field) instead of failing the training run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::{BoxFuture, FutureExt, Shared};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::events::{EngineEvent, TrainDoneEvent, TrainProgressEvent};
use crate::launch::process;
use crate::types::{AotProof, InstanceMeta, LaunchMode, Loader};

/// Game-init cap: armed at the first latest.log activity.
pub const MARKER_INIT_CAP: Duration = Duration::from_secs(180);
/// Never wait longer than 10 min for the menu, logged or not.
pub const MARKER_HARD_CAP: Duration = Duration::from_secs(600);
/// The whole AOT dump must fit (≈165 s measured on 26.2-fabric).
pub const EXIT_WAIT: Duration = Duration::from_secs(600);
/// Keep at most this many cache keys.
pub const MAX_CACHE_KEYS: usize = 4;
/// Keep at most this many aot-<pid>.log proof logs (and none older than 24h).
pub const MAX_AOT_LOGS: usize = 8;

/// AOT cache key: `sha256("<version>|<javaBuild>|<osArch>")`, null-safe
/// (`unknown` build, Node `process.arch` default). Existing caches stay valid.
pub fn cache_key(version: &str, java_build: Option<&str>, os_arch: Option<&str>) -> String {
    let b = java_build.unwrap_or("unknown");
    let a = os_arch.unwrap_or(crate::platform::node_arch());
    hex::encode(Sha256::digest(format!("{version}|{b}|{a}").as_bytes()))
}

/// queueTrainInstance() options.
#[derive(Clone, Default)]
pub struct TrainOptions {
    /// Key estimate (train-done carries the authoritative one).
    pub key: Option<String>,
    /// Explicit user action: train even when aot_auto_train is off.
    pub force: bool,
    /// Never spawn the trainer while this returns true (a live game shares the gameDir).
    pub is_blocked: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

// ---------------------------------------------------------------------------
// Classpath identity (JEP 483 path + size + mtime)
// ---------------------------------------------------------------------------

/// One `meta.json` `classpath_stamp` entry. Unstattable entries are -1/-1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StampEntry {
    pub path: String,
    pub size: i64,
    pub mtime_ms: f64,
}

/// Identity stamp of a classpath (path + size + mtime per entry).
pub fn classpath_stamp(classpath: &[PathBuf]) -> Vec<StampEntry> {
    classpath
        .iter()
        .map(|entry| {
            let (size, mtime_ms) = match std::fs::metadata(entry) {
                Ok(m) => (m.len() as i64, crate::platform::mtime_ms(&m)),
                Err(_) => (-1, -1.0),
            };
            StampEntry { path: entry.to_string_lossy().into_owned(), size, mtime_ms }
        })
        .collect()
}

/// Stored stamp vs a fresh one. Missing/empty stored stamp = no evidence =
/// false. mtime compared with a 2 s tolerance (FAT/network volumes).
pub fn stamp_matches(stored: Option<&[StampEntry]>, current: &[StampEntry]) -> bool {
    let Some(stored) = stored.filter(|s| !s.is_empty()) else { return false };
    if stored.len() != current.len() {
        return false;
    }
    stored.iter().zip(current).all(|(a, b)| {
        a.path == b.path
            && a.size == b.size
            && a.size >= 0
            && a.mtime_ms >= 0.0
            && b.mtime_ms >= 0.0
            && (a.mtime_ms - b.mtime_ms).abs() <= 2000.0
    })
}

/// Stored classpath stamp for a key, None when meta.json has none.
pub fn read_cache_stamp(engine: &Engine, key: &str) -> Option<Vec<StampEntry>> {
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(engine.paths().aot_meta(key)).ok()?).ok()?;
    let arr = raw.get("classpath_stamp")?.as_array()?;
    // An odd entry can never prove validity: read it as a drifted (-1) entry.
    Some(
        arr.iter()
            .map(|v| {
                serde_json::from_value(v.clone()).unwrap_or(StampEntry {
                    path: String::new(),
                    size: -1,
                    mtime_ms: -1.0,
                })
            })
            .collect(),
    )
}

/// True when the recorded classpath identity no longer matches disk (or was
/// never recorded — unverifiable is stale so one retrain fixes it).
pub fn is_cache_stale(engine: &Engine, key: &str, classpath: &[PathBuf]) -> bool {
    !stamp_matches(read_cache_stamp(engine, key).as_deref(), &classpath_stamp(classpath))
}

/// Key from the CURRENT config state (no JVM discovery): `config.jvm.build`
/// or 'unknown' until a JVM has been probed.
pub async fn instance_key(engine: &Engine, instance: &InstanceMeta) -> EngineResult<String> {
    let build = engine.config().jvm().map(|j| j.build).filter(|b| !b.is_empty());
    Ok(cache_key(&instance.version, build.as_deref(), None))
}

// ---------------------------------------------------------------------------
// Proof logs
// ---------------------------------------------------------------------------

fn is_aot_log_name(name: &str) -> bool {
    name.strip_prefix("aot-")
        .and_then(|r| r.strip_suffix(".log"))
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

fn aot_logs_in(dir: &Path) -> Vec<(PathBuf, f64)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    rd.flatten()
        .filter(|e| e.file_name().to_str().is_some_and(is_aot_log_name))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (e.path(), crate::platform::mtime_ms(&m)))
        })
        .collect()
}

/// Newest aot-<pid>.log where the game runs (effective gameDir), or None.
pub fn latest_aot_proof_log(engine: &Engine, instance_name: &str) -> Option<PathBuf> {
    let dir = crate::instances::effective_game_dir_by_name(engine, instance_name);
    let mut best: Option<(PathBuf, f64)> = None;
    for (p, m) in aot_logs_in(&dir) {
        if best.as_ref().map_or(true, |(_, bm)| m > *bm) {
            best = Some((p, m));
        }
    }
    best.map(|(p, _)| p)
}

/// First `[warning][aot]` / `[error][aot]` record (the JVM refusing the
/// cache), trimmed and capped at 300 chars; None on a clean log.
pub fn aot_refusal_line(text: &str) -> Option<String> {
    text.split('\n')
        .find(|l| l.contains("[warning][aot]") || l.contains("[error][aot]"))
        .map(|l| l.trim().chars().take(300).collect())
}

/// Proof object from the newest proof log; None when no AOT boot log exists.
pub fn aot_proof(engine: &Engine, instance_name: &str) -> Option<AotProof> {
    let log = latest_aot_proof_log(engine, instance_name)?;
    let text = std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let using = text.contains("Using AOT-linked classes: true");
    Some(AotProof {
        log_path: Some(log.to_string_lossy().into_owned()),
        using_aot_linked_classes: using,
        refusal: if using { None } else { aot_refusal_line(&text) },
    })
}

/// Keep at most 8 newest aot-<pid>.log files and delete any older than 24h.
pub async fn prune_aot_proof_logs(engine: &Engine, instance_name: &str) {
    if instance_name.is_empty() {
        return;
    }
    let dir = crate::instances::effective_game_dir_by_name(engine, instance_name);
    let _ = tokio::task::spawn_blocking(move || {
        let mut logs = aot_logs_in(&dir);
        logs.sort_by(|a, b| a.1.total_cmp(&b.1)); // oldest first
        let now = crate::platform::now_ms() as f64;
        const DAY_MS: f64 = 24.0 * 3_600_000.0;
        let mut fresh = Vec::new();
        for (p, m) in logs {
            if now - m > DAY_MS {
                let _ = std::fs::remove_file(&p);
            } else {
                fresh.push(p);
            }
        }
        if fresh.len() > MAX_AOT_LOGS {
            let excess = fresh.len() - MAX_AOT_LOGS;
            for p in fresh.iter().take(excess) {
                let _ = std::fs::remove_file(p);
            }
        }
    })
    .await;
}

/// Bounded cache eviction: keep at most 4 keys, delete the oldest by
/// meta.json `trained_at` (dir mtime fallback), never `current_key`.
pub async fn evict_old_aot_caches(engine: &Engine, current_key: &str) {
    let root = engine.paths().aot_root();
    let current = current_key.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let Ok(rd) = std::fs::read_dir(&root) else { return };
        let dirs: Vec<String> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        if dirs.len() <= MAX_CACHE_KEYS {
            return;
        }
        let mut candidates: Vec<(String, f64)> = Vec::new();
        for k in dirs.iter().filter(|k| **k != current) {
            let trained = std::fs::read_to_string(root.join(k).join("meta.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v.get("trained_at").and_then(Value::as_str).map(str::to_string))
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.timestamp_millis() as f64);
            let t = trained.unwrap_or_else(|| {
                std::fs::metadata(root.join(k)).map(|m| crate::platform::mtime_ms(&m)).unwrap_or(0.0)
            });
            candidates.push((k.clone(), t));
        }
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
        let to_delete = dirs.len() - MAX_CACHE_KEYS;
        for (k, _) in candidates.iter().take(to_delete) {
            let _ = std::fs::remove_dir_all(root.join(k));
        }
    })
    .await;
}

// ---------------------------------------------------------------------------
// Marker wait
// ---------------------------------------------------------------------------

/// waitForMarker() result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerOutcome {
    pub ok: bool,
    /// The veto fired (a game launched while the trainer was booting).
    pub aborted: bool,
    /// ms epoch.
    pub at: i64,
}

/// Size-tracked latest.log watcher resolving on the menu marker.
/// `game_init_timeout` arms only on FIRST log activity; `hard_timeout`
/// bounds the total wait. `should_abort` is polled every tick AFTER the
/// marker read (a same-tick menu wins); a panicking veto is ignored.
pub async fn wait_for_marker(
    log_file: &Path,
    game_init_timeout: Duration,
    hard_timeout: Duration,
    should_abort: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> MarkerOutcome {
    let mut seen: i64 = std::fs::metadata(log_file).map(|m| m.len() as i64).unwrap_or(-1);
    let started = Instant::now();
    let mut armed_at: Option<Instant> = None;
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let now = Instant::now();
        if now - started > hard_timeout {
            tracing::warn!(
                "[aot] hard cap {}s reached (log activity: {}); aborting marker wait",
                hard_timeout.as_secs(),
                if armed_at.is_some() { "yes" } else { "none" }
            );
            return MarkerOutcome { ok: false, aborted: false, at: crate::platform::now_ms() };
        }
        if armed_at.is_some_and(|a| now - a > game_init_timeout) {
            tracing::warn!(
                "[aot] game reached no menu within {}s of first log line; aborting marker wait",
                game_init_timeout.as_secs()
            );
            return MarkerOutcome { ok: false, aborted: false, at: crate::platform::now_ms() };
        }
        if let Some(chunk) = crate::launch::read_log_growth(log_file, &mut seen) {
            if armed_at.is_none() {
                armed_at = Some(now);
            }
            if chunk.contains(crate::launch::MARKER) {
                return MarkerOutcome { ok: true, aborted: false, at: crate::platform::now_ms() };
            }
        }
        if let Some(f) = should_abort {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(false) {
                return MarkerOutcome { ok: false, aborted: true, at: crate::platform::now_ms() };
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Training
// ---------------------------------------------------------------------------

fn format_resolve_error(err: &EngineError) -> String {
    let msg = err.to_string();
    let cause = std::error::Error::source(err).map(|c| c.to_string()).filter(|c| !c.is_empty());
    let detail = match cause {
        Some(c) if !msg.contains(&c) => format!("{msg} ({c})"),
        _ => msg,
    };
    let url = match err {
        EngineError::Http(h) if !h.url.is_empty() => Some(h.url.clone()),
        _ => regex::Regex::new(r#"https?://[^\s'")]+"#)
            .ok()
            .and_then(|re| re.find(&detail).map(|m| m.as_str().to_string())),
    };
    match url {
        Some(u) if !detail.starts_with(&u) && !detail.contains(&format!("<{u}>")) => {
            format!("resolve failed: <{u}>: {detail}")
        }
        _ => format!("resolve failed: {detail}"),
    }
}

const SUPERSEDED: &str = "superseded: instance relaunched while training resolved; will retry after that game exits";

fn train_payload(instance: &str, key: Option<String>, ok: bool, error: Option<String>) -> TrainDoneEvent {
    TrainDoneEvent {
        instance: instance.to_string(),
        key,
        ok,
        error,
        skipped: None,
        cache_size_bytes: None,
        proof: None,
        extra: Default::default(),
    }
}

/// Background AOT training for an instance (trainInstance). Emits
/// train-progress / train-done; returns the train-done payload. Opt-out and
/// NeoForge skips return without a train-done event (like the JS).
pub async fn train_instance(engine: &Engine, instance: &InstanceMeta, opts: TrainOptions) -> TrainDoneEvent {
    let name = instance.name.clone();
    let mut key = opts.key.clone();
    if !opts.force && instance.aot_auto_train == Some(false) {
        let mut p = train_payload(&name, key, false, Some("aot_auto_train is off for this instance".into()));
        p.skipped = Some(true);
        return p;
    }
    if instance.loader == Loader::Neoforge {
        tracing::info!("[aot] skipped for NeoForge instance {name}");
        return train_payload(
            &name,
            key,
            false,
            Some("AOT unavailable for NeoForge (requires a JDK 25-tier runtime)".into()),
        );
    }
    let progress = |key: &Option<String>, phase: &str, message: String| {
        engine.emit(EngineEvent::TrainProgress(TrainProgressEvent {
            instance: name.clone(),
            key: key.clone(),
            phase: phase.into(),
            message,
            at: crate::platform::now_ms(),
        }));
    };
    let done = |p: TrainDoneEvent| {
        engine.emit(EngineEvent::TrainDone(p.clone()));
        p
    };
    let is_blocked = || opts.is_blocked.as_ref().is_some_and(|f| f());
    if is_blocked() {
        return done(train_payload(&name, key, false, Some(SUPERSEDED.into())));
    }

    let hooks = engine.game_launch_state().hooks();
    let retry_base = hooks.train_retry_base_ms.unwrap_or(1200);
    let mut resolved = None;
    let mut last_error = None;
    const MAX_ATTEMPTS: u64 = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        progress(&key, "resolve", format!("resolving launch configuration ({attempt}/{MAX_ATTEMPTS})…"));
        let ropts = crate::launch::ResolveLaunchOptions { mode: LaunchMode::Train, ..Default::default() };
        let r = match &hooks.resolve_launch {
            Some(h) => h(instance.clone(), ropts).await,
            None => crate::launch::resolve_launch(engine, instance, ropts).await,
        };
        match r {
            Ok(r) => {
                resolved = Some(r);
                last_error = None;
                break;
            }
            Err(e) => {
                if attempt < MAX_ATTEMPTS {
                    let delay = attempt * retry_base;
                    progress(&key, "retry", format!("resolve attempt {attempt} failed ({e}); retrying in {delay}ms…"));
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                last_error = Some(e);
            }
        }
    }
    let Some(mut resolved) = resolved else {
        let err =
            last_error.map(|e| format_resolve_error(&e)).unwrap_or_else(|| "resolve failed: unknown error".into());
        return done(train_payload(&name, key, false, Some(err)));
    };
    if resolved.java.major.unwrap_or(0) < 25 {
        return done(train_payload(&name, key, false, Some("AOT training requires a JDK 25-tier runtime".into())));
    }

    let mod_set_hash = match crate::instances::compute_mod_set_hash(engine, instance).await {
        Ok(h) => Value::String(h),
        Err(e) => {
            tracing::debug!("[aot] mod set hash unavailable: {e}");
            Value::Null
        }
    };
    let build = Some(resolved.java.build.as_str()).filter(|b| !b.is_empty());
    let resolved_key = cache_key(&resolved.version.id, build, None);
    if let Some(k) = key.as_deref().filter(|k| *k != resolved_key) {
        progress(&key, "key", format!("key drifted from estimate {k} -> {resolved_key}"));
    }
    key = Some(resolved_key.clone());
    let cache_file = engine.paths().aot_cache_file(&resolved_key);
    let _ = std::fs::create_dir_all(engine.paths().aot_cache_dir(&resolved_key));
    resolved.aot_cache_path = cache_file.clone();
    resolved.aot_cache_exists = false;

    let game_dir = resolved.game_dir.clone();
    let _ = std::fs::create_dir_all(game_dir.join("logs"));
    let log_file = game_dir.join("logs").join("latest.log");

    if is_blocked() {
        return done(train_payload(&name, key, false, Some(SUPERSEDED.into())));
    }
    progress(&key, "spawn", format!("train JVM ({}) -> {}", resolved.java.build, cache_file.display()));
    let argv = crate::launch::build_argv(engine, instance, &mut resolved);
    let child = match process::spawn_java(&resolved.java.path, &argv, &game_dir, false) {
        Ok(c) => c,
        Err(e) => return done(train_payload(&name, key, false, Some(format!("spawn failed: {e}")))),
    };
    let token = resolved.bridge_token.clone().unwrap_or_default();
    // Register the trainer with the mod bridge (its argv carries a token) and
    // detach it when the process ends.
    let _ = crate::presence::launch_started(
        engine,
        crate::presence::LaunchPresence {
            instance: name.clone(),
            uuid: Some(resolved.account.uuid.clone()).filter(|u| !u.is_empty()),
            game_dir: game_dir.clone(),
            token: token.clone(),
        },
    );
    let handle = {
        let e = engine.clone();
        let name = name.clone();
        process::monitor(child, None, move |_| {
            let _ = crate::presence::launch_exited(&e, &name, Some(&token));
        })
    };
    let pid = handle.pid();
    progress(&key, "running", format!("pid {pid}"));

    let veto = opts.is_blocked.clone();
    let marker = wait_for_marker(&log_file, MARKER_INIT_CAP, MARKER_HARD_CAP, veto.as_deref()).await;

    if marker.aborted {
        // A PRE-MENU trainer has written no cache yet: safe to kill.
        progress(&key, "superseded", "a game launched while the trainer was still booting; closing the trainer".into());
        process::terminate(pid);
        if !handle.wait_timeout(Duration::from_secs(30)).await {
            process::kill_hard(pid);
            handle.wait_timeout(Duration::from_secs(5)).await;
        }
        return done(train_payload(&name, key, false, Some(SUPERSEDED.into())));
    }
    if !marker.ok {
        progress(&key, "timeout", format!("game did not reach the menu within {}s", MARKER_INIT_CAP.as_secs()));
        process::close_gracefully(pid).await;
        handle.wait_timeout(Duration::from_secs(60)).await;
        return done(train_payload(
            &name,
            key,
            false,
            Some(format!("timeout waiting for 'Sound engine started' ({}s cap)", MARKER_INIT_CAP.as_secs())),
        ));
    }

    progress(&key, "menu", "Sound engine started; requesting graceful close".into());
    process::close_gracefully(pid).await;
    if !handle.wait_timeout(EXIT_WAIT).await {
        return done(train_payload(
            &name,
            key,
            false,
            Some(format!(
                "JVM did not exit within {}s of WM_CLOSE; refusing to force-kill (the AOT cache is written on normal exit only)",
                EXIT_WAIT.as_secs()
            )),
        ));
    }

    let cache_size = std::fs::metadata(&cache_file).map(|m| m.len()).unwrap_or(0);
    let ok = cache_size > 0;
    if ok {
        let meta = json!({
            "key": resolved_key,
            "trained_at": crate::platform::now_iso(),
            "classpath_stamp": classpath_stamp(&resolved.classpath),
            "java_build": resolved.java.build,
            "game_version": resolved.version.id,
            "mod_set_hash": mod_set_hash,
        });
        if let Err(e) = crate::paths::write_json_atomic(&engine.paths().aot_meta(&resolved_key), &meta) {
            progress(&key, "warn", format!("meta.json write failed: {e}"));
        }
        evict_old_aot_caches(engine, &resolved_key).await;
    }
    let error = (!ok).then(|| "game.aot missing or empty after training exit".to_string());
    let mut payload = train_payload(&name, key.clone(), ok, error.clone());
    payload.cache_size_bytes = Some(cache_size);
    payload.proof = aot_proof(engine, &name).and_then(|p| serde_json::to_value(p).ok());
    progress(
        &key,
        if ok { "done" } else { "failed" },
        if ok { format!("cache written ({cache_size} bytes)") } else { error.unwrap_or_default() },
    );
    done(payload)
}

/// One shared training run per instance.
pub(crate) type TrainingFuture = Shared<BoxFuture<'static, TrainDoneEvent>>;

/// Map of instance name -> (run id, shared run).
#[derive(Default)]
pub(crate) struct Trainings {
    pub(crate) next_id: u64,
    pub(crate) runs: HashMap<String, (u64, TrainingFuture)>,
}

/// Queue (deduped per instance) a background training run; resolves with the
/// train-done payload. Creation and launch can both request the cache;
/// sharing the run prevents two JVMs from training it at once. The run is a
/// detached task: dropping this future does not cancel the training.
pub async fn queue_train_instance(
    engine: &Engine,
    instance: &InstanceMeta,
    opts: TrainOptions,
) -> EngineResult<TrainDoneEvent> {
    if instance.loader == Loader::Neoforge {
        tracing::info!("[aot] skipped for NeoForge instance {}", instance.name);
        return Ok(train_payload(
            &instance.name,
            opts.key,
            false,
            Some("AOT unavailable for NeoForge (requires a JDK 25-tier runtime)".into()),
        ));
    }
    if instance.name.is_empty() {
        return Err(EngineError::other("instance name is required"));
    }
    let name = instance.name.clone();
    let state = engine.game_launch_state();
    let fut = {
        let mut t = state.trainings.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((_, f)) = t.runs.get(&name) {
            f.clone()
        } else {
            t.next_id += 1;
            let id = t.next_id;
            let weak = std::sync::Arc::downgrade(&engine.inner);
            let e = engine.clone();
            let inst = instance.clone();
            let task = tokio::spawn(async move { train_instance(&e, &inst, opts).await });
            let n = name.clone();
            let fut: TrainingFuture = async move {
                let r = match task.await {
                    Ok(r) => r,
                    Err(err) => train_payload(&n, None, false, Some(format!("training task failed: {err}"))),
                };
                if let Some(inner) = weak.upgrade() {
                    let e = Engine { inner };
                    let mut t = e.game_launch_state().trainings.lock().unwrap_or_else(|p| p.into_inner());
                    if t.runs.get(&n).is_some_and(|(rid, _)| *rid == id) {
                        t.runs.remove(&n);
                    }
                }
                r
            }
            .boxed()
            .shared();
            t.runs.insert(name.clone(), (id, fut.clone()));
            fut
        }
    };
    Ok(fut.await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    fn sha(s: &str) -> String {
        hex::encode(Sha256::digest(s.as_bytes()))
    }

    #[test]
    fn cache_key_matches_js_sha256() {
        let k = cache_key("1.21.11", Some("25.0.4+7-LTS"), Some("x64"));
        assert_eq!(k, sha("1.21.11|25.0.4+7-LTS|x64"));
        assert_eq!(k.len(), 64);
        assert_eq!(k, cache_key("1.21.11", Some("25.0.4+7-LTS"), Some("x64")));
        assert_ne!(k, cache_key("26.2", Some("25.0.4+7-LTS"), Some("x64")));
        assert_ne!(k, cache_key("1.21.11", Some("21.0.1+12-LTS"), Some("x64")));
        assert_ne!(k, cache_key("1.21.11", Some("25.0.4+7-LTS"), Some("arm64")));
        let arch = crate::platform::node_arch();
        assert_eq!(cache_key("1.21.11", None, None), sha(&format!("1.21.11|unknown|{arch}")));
        assert_eq!(cache_key("1.21.11", None, Some("x64")), sha("1.21.11|unknown|x64"));
        assert_eq!(cache_key("1.21.11", Some("25.0.4+7-LTS"), None), sha(&format!("1.21.11|25.0.4+7-LTS|{arch}")));
    }

    fn touch_log(dir: &Path, pid: u32, mtime_ms: i64) {
        let f = dir.join(format!("aot-{pid}.log"));
        std::fs::write(&f, format!("log {pid}")).unwrap();
        set_mtime(&f, mtime_ms);
    }

    fn set_mtime(p: &Path, mtime_ms: i64) {
        let t = std::time::UNIX_EPOCH + Duration::from_millis(mtime_ms as u64);
        let f = std::fs::File::options().write(true).open(p).or_else(|_| std::fs::File::open(p)).unwrap();
        f.set_modified(t).unwrap();
    }

    fn remaining_logs(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| is_aot_log_name(n))
            .collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn prune_keeps_8_newest() {
        let (_d, e) = temp_engine();
        let dir = e.paths().instance_dir("prune-keep8");
        std::fs::create_dir_all(&dir).unwrap();
        let now = crate::platform::now_ms();
        for i in 1..=12 {
            touch_log(&dir, i, now - (12 - i as i64) * 1000);
        }
        prune_aot_proof_logs(&e, "prune-keep8").await;
        let left = remaining_logs(&dir);
        assert_eq!(left.len(), 8);
        for i in 1..=4 {
            assert!(!left.contains(&format!("aot-{i}.log")));
        }
        for i in 5..=12 {
            assert!(left.contains(&format!("aot-{i}.log")));
        }
    }

    #[tokio::test]
    async fn prune_deletes_older_than_24h_and_combined() {
        let (_d, e) = temp_engine();
        let dir = e.paths().instance_dir("prune-24h");
        std::fs::create_dir_all(&dir).unwrap();
        let now = crate::platform::now_ms();
        const H: i64 = 3_600_000;
        touch_log(&dir, 1, now - 25 * H);
        touch_log(&dir, 2, now - 26 * H);
        touch_log(&dir, 3, now - 1000);
        touch_log(&dir, 4, now);
        prune_aot_proof_logs(&e, "prune-24h").await;
        assert_eq!(remaining_logs(&dir), vec!["aot-3.log", "aot-4.log"]);

        let dir = e.paths().instance_dir("prune-combined");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 1..=5 {
            touch_log(&dir, i, now - 25 * H - i as i64 * 1000);
        }
        for i in 6..=12 {
            touch_log(&dir, i, now - (12 - i as i64) * 1000);
        }
        prune_aot_proof_logs(&e, "prune-combined").await;
        assert_eq!(remaining_logs(&dir).len(), 7);
    }

    #[tokio::test]
    async fn prune_ignores_other_files_and_missing_dir() {
        let (_d, e) = temp_engine();
        prune_aot_proof_logs(&e, "prune-missing").await;
        let dir = e.paths().instance_dir("prune-missing");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latest.log"), "hello").unwrap();
        std::fs::write(dir.join("aot-bad.log"), "bad").unwrap();
        touch_log(&dir, 1, crate::platform::now_ms());
        prune_aot_proof_logs(&e, "prune-missing").await;
        assert!(dir.join("latest.log").exists() && dir.join("aot-bad.log").exists() && dir.join("aot-1.log").exists());
    }

    #[test]
    fn refusal_line() {
        assert_eq!(aot_refusal_line("[0.003s][info][aot] opening archive file game.aot"), None);
        assert_eq!(aot_refusal_line(""), None);
        assert_eq!(
            aot_refusal_line(
                "[0.10s][warning][aot] This file is not the one used while building the shared archive file\n\
                 [0.11s][error][aot] Unable to map shared spaces"
            )
            .as_deref(),
            Some("[0.10s][warning][aot] This file is not the one used while building the shared archive file")
        );
    }

    #[test]
    fn proof_reports_usage_and_refusal() {
        let (_d, e) = temp_engine();
        let dir = e.paths().instance_dir("proof-refusal");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(aot_proof(&e, "proof-refusal").is_none());
        let log = dir.join("aot-4242.log");
        std::fs::write(
            &log,
            "[0.003s][info][aot] opening archive file /data/cache/aot/key/game.aot\n[0.004s][error][aot] Unable to map shared spaces\n",
        )
        .unwrap();
        let refused = aot_proof(&e, "proof-refusal").unwrap();
        assert!(!refused.using_aot_linked_classes);
        assert_eq!(refused.refusal.as_deref(), Some("[0.004s][error][aot] Unable to map shared spaces"));
        std::fs::write(
            &log,
            "[4.7s][info][aot] Using AOT-linked classes: true (static archive: has aot-linked classes)\n",
        )
        .unwrap();
        let linked = aot_proof(&e, "proof-refusal").unwrap();
        assert!(linked.using_aot_linked_classes);
        assert_eq!(linked.refusal, None);
        assert_eq!(linked.log_path.as_deref(), Some(log.to_string_lossy().as_ref()));
    }

    fn make_key_dir(root: &Path, key: &str, trained_at_ms: Option<i64>, mtime_ms: Option<i64>) {
        let dir = root.join(key);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.aot"), "fake-aot").unwrap();
        if let Some(t) = trained_at_ms {
            let iso = chrono::DateTime::from_timestamp_millis(t)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            std::fs::write(dir.join("meta.json"), json!({ "trained_at": iso, "key": key }).to_string()).unwrap();
        }
        if let Some(m) = mtime_ms {
            set_mtime(&dir, m);
        }
    }

    fn keys(root: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn eviction_rules() {
        let (_d, e) = temp_engine();
        let root = e.paths().aot_root();
        let now = crate::platform::now_ms();
        for (i, k) in ["k1", "k2", "k3", "k4", "k5"].iter().enumerate() {
            make_key_dir(&root, k, Some(now - (5 - i as i64) * 10_000), None);
        }
        evict_old_aot_caches(&e, "k5").await;
        assert_eq!(keys(&root), vec!["k2", "k3", "k4", "k5"]);
        evict_old_aot_caches(&e, "k5").await; // <= 4: no-op
        assert_eq!(keys(&root).len(), 4);

        std::fs::remove_dir_all(&root).unwrap();
        make_key_dir(&root, "current", Some(now - 50_000), None);
        for (i, k) in ["k2", "k3", "k4", "k5"].iter().enumerate() {
            make_key_dir(&root, k, Some(now - (4 - i as i64) * 10_000), None);
        }
        evict_old_aot_caches(&e, "current").await;
        let left = keys(&root);
        assert!(left.contains(&"current".to_string()) && !left.contains(&"k2".to_string()) && left.len() == 4);

        std::fs::remove_dir_all(&root).unwrap();
        make_key_dir(&root, "k1", None, Some(now - 50_000));
        for (i, k) in ["k2", "k3", "k4", "k5"].iter().enumerate() {
            make_key_dir(&root, k, Some(now - (4 - i as i64) * 10_000), None);
        }
        evict_old_aot_caches(&e, "k5").await;
        assert!(!keys(&root).contains(&"k1".to_string()), "mtime fallback evicts k1");

        std::fs::remove_dir_all(&root).unwrap();
        for (i, k) in ["k1", "k2", "k3", "k4", "k5", "k6"].iter().enumerate() {
            make_key_dir(&root, k, Some(now - (6 - i as i64) * 10_000), None);
        }
        evict_old_aot_caches(&e, "k6").await;
        assert_eq!(keys(&root), vec!["k3", "k4", "k5", "k6"]);
    }

    #[test]
    fn stamps_detect_drift() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.jar");
        std::fs::write(&a, "aaaa").unwrap();
        let missing = d.path().join("nope.jar");
        let st = classpath_stamp(&[a.clone(), missing.clone()]);
        assert_eq!(st[0].size, 4);
        assert!(st[0].mtime_ms > 0.0);
        assert_eq!(st[1], StampEntry { path: missing.to_string_lossy().into(), size: -1, mtime_ms: -1.0 });

        let before = classpath_stamp(std::slice::from_ref(&a));
        assert!(stamp_matches(Some(&before), &classpath_stamp(std::slice::from_ref(&a))));
        set_mtime(&a, crate::platform::now_ms() + 60_000);
        assert!(!stamp_matches(Some(&before), &classpath_stamp(std::slice::from_ref(&a))), "mtime drift");
        std::fs::write(&a, "aaaaa").unwrap();
        assert!(!stamp_matches(Some(&before), &classpath_stamp(std::slice::from_ref(&a))), "size drift");

        let b = d.path().join("b.jar");
        std::fs::write(&b, "bbbb").unwrap();
        let current = classpath_stamp(&[a.clone(), b.clone()]);
        assert!(!stamp_matches(None, &current));
        assert!(!stamp_matches(Some(&[]), &current));
        assert!(!stamp_matches(Some(&classpath_stamp(std::slice::from_ref(&a))), &current));
        assert!(!stamp_matches(Some(&classpath_stamp(&[b.clone(), a.clone()])), &current));
        let ghost = d.path().join("ghost.jar");
        assert!(!stamp_matches(
            Some(&classpath_stamp(std::slice::from_ref(&ghost))),
            &classpath_stamp(std::slice::from_ref(&ghost))
        ));
    }

    #[test]
    fn is_cache_stale_reads_meta() {
        let (d, e) = temp_engine();
        let key = "stalekey";
        let jar = d.path().join("lib.jar");
        std::fs::write(&jar, "contents").unwrap();
        let write_meta = |extra: Value| {
            let mut m = json!({ "key": key });
            m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            crate::paths::write_json_atomic(&e.paths().aot_meta(key), &m).unwrap();
        };
        write_meta(json!({ "classpath_stamp": classpath_stamp(std::slice::from_ref(&jar)) }));
        assert!(!is_cache_stale(&e, key, std::slice::from_ref(&jar)));
        set_mtime(&jar, crate::platform::now_ms() + 60_000);
        assert!(is_cache_stale(&e, key, std::slice::from_ref(&jar)));
        write_meta(json!({ "classpath_manifest": ["lib.jar"] }));
        assert!(is_cache_stale(&e, key, std::slice::from_ref(&jar)));
        std::fs::remove_file(e.paths().aot_meta(key)).unwrap();
        assert!(is_cache_stale(&e, key, std::slice::from_ref(&jar)));
        // A JS-written stamp (integers, mtimeMs float) is read back.
        let st = classpath_stamp(std::slice::from_ref(&jar));
        let js = json!({ "classpath_stamp": [{ "path": st[0].path, "size": st[0].size, "mtime_ms": st[0].mtime_ms }] });
        write_meta(js);
        assert!(!is_cache_stale(&e, key, std::slice::from_ref(&jar)));
    }

    #[tokio::test]
    async fn train_aborts_when_blocked_and_honors_opt_out() {
        let (_d, e) = temp_engine();
        let mut rx = e.subscribe();
        let inst = InstanceMeta { name: "busy-inst".into(), ..Default::default() };
        let r =
            train_instance(&e, &inst, TrainOptions { is_blocked: Some(Arc::new(|| true)), ..Default::default() }).await;
        assert!(!r.ok);
        assert_eq!(r.instance, "busy-inst");
        assert!(r.error.unwrap().contains("superseded"));
        assert_eq!(rx.recv().await.unwrap().name(), "train-done");

        let opt_out = InstanceMeta { name: "optout-inst".into(), aot_auto_train: Some(false), ..Default::default() };
        let r = train_instance(&e, &opt_out, TrainOptions::default()).await;
        assert!(!r.ok);
        assert_eq!(r.skipped, Some(true));
        let nf = InstanceMeta { name: "nf".into(), loader: Loader::Neoforge, ..Default::default() };
        let r = queue_train_instance(&e, &nf, TrainOptions::default()).await.unwrap();
        assert!(r.error.unwrap().contains("NeoForge"));
    }

    #[tokio::test]
    async fn train_resolve_failure_retries_then_reports() {
        let (_d, e) = temp_engine();
        e.set_launch_test_hooks(crate::api::game_launch::LaunchTestHooks {
            resolve_launch: Some(Arc::new(|_, _| {
                async { Err(EngineError::other("boom at https://meta.example.test/v2/x")) }.boxed()
            })),
            train_retry_base_ms: Some(1),
            ..Default::default()
        });
        let inst = InstanceMeta { name: "rf".into(), version: "1.21.11".into(), ..Default::default() };
        let r = train_instance(&e, &inst, TrainOptions { force: true, ..Default::default() }).await;
        assert_eq!(
            r.error.as_deref(),
            Some("resolve failed: <https://meta.example.test/v2/x>: boom at https://meta.example.test/v2/x")
        );
    }

    #[tokio::test]
    async fn wait_for_marker_veto_and_caps() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("latest.log");
        std::fs::write(&log, "").unwrap();
        // Veto fires mid-boot.
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let f2 = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            f2.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let veto = move || flag.load(std::sync::atomic::Ordering::SeqCst);
        let started = Instant::now();
        let r = wait_for_marker(&log, MARKER_INIT_CAP, MARKER_HARD_CAP, Some(&veto)).await;
        assert!(!r.ok && r.aborted);
        assert!(started.elapsed() < Duration::from_secs(10));

        // A same-tick menu wins over the veto.
        let l2 = log.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&l2).unwrap();
            f.write_all(b"blah\nSound engine started\n").unwrap();
        });
        let always = || true;
        let r = wait_for_marker(&log, MARKER_INIT_CAP, MARKER_HARD_CAP, Some(&always)).await;
        assert!(r.ok && !r.aborted);

        // No veto: timeouts.
        let r = wait_for_marker(&log, Duration::from_millis(200), Duration::from_millis(400), None).await;
        assert!(!r.ok && !r.aborted);

        // A panicking veto never wedges the wait.
        let l3 = log.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&l3).unwrap();
            f.write_all(b"Sound engine started\n").unwrap();
        });
        let boom = || -> bool { panic!("veto boom") };
        let r = wait_for_marker(&log, MARKER_INIT_CAP, MARKER_HARD_CAP, Some(&boom)).await;
        assert!(r.ok);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn train_end_to_end_with_fake_jvm() {
        use crate::types::{JavaRuntime, LaunchAccount, ResolvedLaunch, ResolvedVersionRef};
        let (d, e) = temp_engine();
        let java = crate::api::game_launch::test_util::fake_java(d.path(), true);
        let game_dir = e.paths().instance_dir("train-inst");
        std::fs::create_dir_all(game_dir.join("logs")).unwrap();
        let jar = d.path().join("lib.jar");
        std::fs::write(&jar, "x").unwrap();
        let resolved = ResolvedLaunch {
            java: JavaRuntime {
                path: java.to_string_lossy().into(),
                build: "25.0.4+7".into(),
                major: Some(25),
                ..Default::default()
            },
            version: ResolvedVersionRef {
                id: "1.21.11".into(),
                main_class: "M".into(),
                asset_index_id: Some("x".into()),
                json: json!({}),
            },
            loader: None,
            fabric_profile: None,
            classpath: vec![jar.clone()],
            natives_dir: game_dir.join("natives"),
            game_dir: game_dir.clone(),
            assets_dir: d.path().join("assets"),
            memory_mb: 1024,
            account: LaunchAccount {
                username: "Tester".into(),
                uuid: "11111111111111111111111111111111".into(),
                access_token: "tok".into(),
                user_type: "mojang".into(),
            },
            bridge_token: Some("d".repeat(64)),
            mode: LaunchMode::Train,
            aot_key: String::new(),
            aot_cache_path: PathBuf::new(),
            aot_cache_exists: false,
            aot_cache_stale: false,
            aot_available: true,
            warnings: vec![],
        };
        e.set_launch_test_hooks(crate::api::game_launch::LaunchTestHooks {
            resolve_launch: Some(Arc::new(move |_, _| {
                let r = resolved.clone();
                async move { Ok(r) }.boxed()
            })),
            ..Default::default()
        });
        let mut rx = e.subscribe();
        let inst = InstanceMeta {
            name: "train-inst".into(),
            version: "1.21.11".into(),
            loader: Loader::Fabric,
            ..Default::default()
        };
        let (a, b) = tokio::join!(
            queue_train_instance(&e, &inst, TrainOptions { force: true, ..Default::default() }),
            queue_train_instance(&e, &inst, TrainOptions { force: true, ..Default::default() }),
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a, b, "one shared run");
        assert!(a.ok, "{a:?}");
        let key = cache_key("1.21.11", Some("25.0.4+7"), None);
        assert_eq!(a.key.as_deref(), Some(key.as_str()));
        assert!(a.cache_size_bytes.unwrap() > 0);
        let meta: Value = serde_json::from_str(&std::fs::read_to_string(e.paths().aot_meta(&key)).unwrap()).unwrap();
        assert_eq!(meta["game_version"], "1.21.11");
        assert!(!is_cache_stale(&e, &key, &[jar]));
        // argv carried the AOT output flag + the bridge token
        let args = std::fs::read_to_string(game_dir.join("args.txt")).unwrap();
        assert!(args.contains(&format!("-XX:AOTCacheOutput={}", e.paths().aot_cache_file(&key).display())));
        assert!(args.contains(&format!("-Despectral.bridge.token={}", "d".repeat(64))));
        let mut done = 0;
        while let Ok(ev) = rx.try_recv() {
            if ev.name() == "train-done" {
                done += 1;
            }
        }
        assert_eq!(done, 1, "exactly one train-done for the shared run");
        assert!(e.game_launch_state().trainings.lock().unwrap().runs.is_empty());
    }
}
