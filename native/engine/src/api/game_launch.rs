//! Facade — cluster **game-launch** (routes/launch.mjs + the version-change
//! routes of routes/instances.mjs, whose logic is version-change.mjs).
//!
//! | JS route                                        | method |
//! |-------------------------------------------------|--------|
//! | POST /api/instances/:name/launch                | [`Engine::launch_instance`] |
//! | POST /api/instances/:name/stop                  | [`Engine::stop_instance`] |
//! | POST /api/instances/:name/train                 | [`Engine::train_instance`] |
//! | GET  /api/instances/:name/aot                   | [`Engine::instance_aot_status`] |
//! | GET  /api/launch/:key/log?cursor=N              | [`Engine::launch_log`] |
//! | GET  /api/launches                              | [`Engine::list_launches`] |
//! | GET  /api/stats/launches?limit=N                | [`Engine::launch_stats`] |
//! | POST /api/instances/:name/version-change/plan   | [`Engine::plan_version_change`] |
//! | POST /api/instances/:name/version-change        | [`Engine::apply_version_change`] |
//!
//! State ([`GameLaunchState`]): launch ring buffers (2000 lines, finalized
//! buffers kept 10 min, LRU cap 8 — running ones are never evicted),
//! liveLaunches (Discord presence refcount), runningProcesses (process handle,
//! pid and single-flight finalizer), pendingStops, the AOT training queue and
//! the running-games.json lock. `activeInstances` is shared and lives on the
//! Engine (`Engine::active_instances`). Startup: adopt games still running
//! from a previous session ([`Engine::adopt_running_games`]).
//!
//! Discord Rich Presence helpers from routes/launch.mjs
//! ([`presence_for_instance`], [`presence_for_server`],
//! [`presence_event_from_log`]) live here too.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::engine::{Engine, Inner};
use crate::error::{EngineError, EngineResult};
use crate::events::{EngineEvent, LaunchExitEvent, LaunchLogEvent, LaunchReadyEvent, TrainDoneEvent};
use crate::launch::{
    self, GameExit, LaunchAccountInput, LaunchCallbacks, ProcessHandle, ResolveLaunchOptions, SpawnedGame,
};
use crate::types::{
    AotStatus, DryRunResult, InstanceMeta, LaunchHandle, LaunchReply, LaunchStat, Loader, LogChunk, ResolvedLaunch,
    VersionChangePlan, VersionChangeResult, VersionTarget,
};

/// Ring capacity per launch buffer.
pub const RING_CAPACITY: usize = 2000;
/// Finalized buffers are kept this long (UI backfill window).
pub const BUFFER_TTL_MS: i64 = 10 * 60 * 1000;
/// Max retained buffers (running ones are never evicted).
pub const BUFFER_LRU_CAP: usize = 8;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// Test hooks (JS __testHooks)
// ---------------------------------------------------------------------------

/// Replacement for launch::resolve_launch (route + trainer).
pub type ResolveHook =
    Arc<dyn Fn(InstanceMeta, ResolveLaunchOptions) -> BoxFuture<'static, EngineResult<ResolvedLaunch>> + Send + Sync>;
/// Replacement for launch::launch_instance.
pub type SpawnHook = Arc<
    dyn Fn(&Engine, &InstanceMeta, &mut ResolvedLaunch, LaunchCallbacks) -> EngineResult<SpawnedGame> + Send + Sync,
>;

/// Test seams (JS routes/launch.mjs + aot.mjs `__testHooks`).
#[derive(Clone, Default)]
pub struct LaunchTestHooks {
    pub resolve_launch: Option<ResolveHook>,
    pub launch_instance: Option<SpawnHook>,
    /// Skip Discord presence updates (JS: set whenever `__testHooks.emit` was).
    pub quiet_presence: bool,
    /// Trainer resolve retry base delay (JS 1200 ms × attempt).
    pub train_retry_base_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One launch's log buffer.
#[derive(Debug, Clone)]
struct LaunchBuffer {
    instance: String,
    version: String,
    account: Option<String>,
    seq: u64,
    entries: VecDeque<(u64, String)>,
    running: bool,
    started_at: i64,
    menu_at: Option<i64>,
    ended_at: Option<i64>,
    last_read: i64,
    spawned_at: Option<i64>,
    phases: Map<String, Value>,
    needs_auto_train: bool,
    /// Agent launch (`agent: true`): tagged so `/api/launches` rows carry it,
    /// Discord presence is skipped and the agent registry tracks this key.
    agent: bool,
}

impl LaunchBuffer {
    fn push(&mut self, line: String) -> (u64, String) {
        self.seq += 1;
        self.entries.push_back((self.seq, line.clone()));
        while self.entries.len() > RING_CAPACITY {
            self.entries.pop_front();
        }
        (self.seq, line)
    }
    fn finish(&mut self) {
        self.running = false;
        self.ended_at = Some(crate::platform::now_ms());
    }
    fn spawn_boot_ms(&self) -> (Option<i64>, Option<i64>) {
        let spawn = self.spawned_at.map(|s| s - self.started_at);
        let boot = match (self.menu_at, self.spawned_at) {
            (Some(m), Some(s)) => Some(m - s),
            _ => None,
        };
        (spawn, boot)
    }
}

/// A running (or adopted) game.
struct RunningProcess {
    pid: u32,
    key: String,
    /// None for a game adopted from an earlier launcher session (pid only).
    process: Option<ProcessHandle>,
    finalize: Arc<dyn Fn(GameExit) + Send + Sync>,
}

/// game-launch in-memory state.
#[derive(Default)]
pub struct GameLaunchState {
    buffers: Mutex<HashMap<String, LaunchBuffer>>,
    /// Keys of launches still preparing/running (presence refcount, Contract C).
    live_launches: Mutex<HashSet<String>>,
    running: Mutex<HashMap<String, RunningProcess>>,
    /// Stop requests that arrived while a launch was still preparing.
    pending_stops: Mutex<HashSet<String>>,
    pub(crate) trainings: Mutex<crate::aot::Trainings>,
    pub(crate) sessions_lock: Mutex<()>,
    hooks: Mutex<LaunchTestHooks>,
}

impl std::fmt::Debug for GameLaunchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameLaunchState")
            .field("buffers", &lock(&self.buffers).len())
            .field("running", &lock(&self.running).keys().cloned().collect::<Vec<_>>())
            .finish()
    }
}

impl GameLaunchState {
    pub(crate) fn hooks(&self) -> LaunchTestHooks {
        lock(&self.hooks).clone()
    }

    /// Drop stale finalized buffers (TTL) and LRU-evict finished ones over the cap.
    fn prune_buffers(&self) {
        let now = crate::platform::now_ms();
        let mut b = lock(&self.buffers);
        b.retain(|_, buf| buf.running || buf.ended_at.map_or(true, |e| now - e < BUFFER_TTL_MS));
        if b.len() <= BUFFER_LRU_CAP {
            return;
        }
        let mut finished: Vec<(i64, String)> =
            b.iter().filter(|(_, v)| !v.running).map(|(k, v)| (v.last_read, k.clone())).collect();
        finished.sort();
        for (_, k) in finished {
            if b.len() <= BUFFER_LRU_CAP {
                break;
            }
            b.remove(&k);
        }
    }

    fn create_buffer(&self, key: &str, instance: &str, version: &str, account: Option<String>) {
        self.prune_buffers();
        let now = crate::platform::now_ms();
        lock(&self.buffers).insert(
            key.to_string(),
            LaunchBuffer {
                instance: instance.into(),
                version: version.into(),
                account,
                seq: 0,
                entries: VecDeque::new(),
                running: true,
                started_at: now,
                menu_at: None,
                ended_at: None,
                last_read: now,
                spawned_at: None,
                phases: Map::new(),
                needs_auto_train: false,
                agent: false,
            },
        );
    }

    fn with_buffer<R>(&self, key: &str, f: impl FnOnce(&mut LaunchBuffer) -> R) -> Option<R> {
        lock(&self.buffers).get_mut(key).map(f)
    }

    /// Raw output: split on newlines (CRLF/CR normalized), empty lines dropped.
    fn append_output(&self, key: &str, chunk: &str) -> Vec<(u64, String)> {
        let text = chunk.replace("\r\n", "\n").replace('\r', "\n");
        self.with_buffer(key, |b| text.split('\n').filter(|l| !l.is_empty()).map(|l| b.push(l.to_string())).collect())
            .unwrap_or_default()
    }

    fn append_system(&self, key: &str, line: &str) -> Vec<(u64, String)> {
        self.with_buffer(key, |b| vec![b.push(line.to_string())]).unwrap_or_default()
    }

    fn buffer_view(&self, key: &str, cursor: u64) -> Option<LogChunk> {
        self.with_buffer(key, |b| {
            b.last_read = crate::platform::now_ms();
            let lines: Vec<&(u64, String)> = b.entries.iter().filter(|(s, _)| *s > cursor).collect();
            LogChunk {
                cursor: lines.last().map(|(s, _)| *s).unwrap_or(cursor),
                lines: lines.into_iter().map(|(_, t)| t.clone()).collect(),
                running: b.running,
            }
        })
    }

    fn live_is_empty(&self) -> bool {
        lock(&self.live_launches).is_empty()
    }
}

impl Engine {
    /// This cluster's in-memory state.
    pub(crate) fn game_launch_state(&self) -> &GameLaunchState {
        &self.inner.game_launch
    }

    /// Install test seams (resolve / spawn doubles, quiet presence).
    #[doc(hidden)]
    pub fn set_launch_test_hooks(&self, hooks: LaunchTestHooks) {
        *lock(&self.game_launch_state().hooks) = hooks;
    }

    /// Forget in-memory launch state (a "fresh engine"; JS __resetLaunchState).
    #[doc(hidden)]
    pub fn reset_launch_state(&self) {
        let st = self.game_launch_state();
        lock(&st.buffers).clear();
        lock(&st.live_launches).clear();
        lock(&st.running).clear();
        lock(&st.pending_stops).clear();
        self.active_instances().clear();
    }
}

fn upgrade(weak: &Weak<Inner>) -> Option<Engine> {
    weak.upgrade().map(|inner| Engine { inner })
}

/// Engine start hook: adopt running games (JS `void adoptRunningGames()` at route registration).
pub(crate) async fn on_engine_start(engine: &Engine) -> EngineResult<()> {
    let e = engine.clone();
    engine.spawn_background("launch", async move {
        e.adopt_running_games().await;
        Ok(())
    });
    Ok(())
}

pub(crate) async fn on_engine_shutdown(_engine: &Engine) {}

// ---------------------------------------------------------------------------
// Request / reply shapes
// ---------------------------------------------------------------------------

/// POST /api/instances/:name/launch body.
///
/// `agent`/`join`/`window` are the validated launch options (agent access);
/// `join`/`window` stay raw JSON here so one [`crate::agent::normalize_launch_options`]
/// validates both the UI route and the agent route (same error codes).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LaunchRequest {
    #[serde(default)]
    pub dry_run: bool,
    /// Launch under this (non-active) account; 400 BAD_ACCOUNT when empty,
    /// 404 NOT_FOUND when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Agent launch (`true` only when the caller is the agent API; the UI
    /// sends false/absent and its argv stays byte-identical).
    #[serde(default)]
    pub agent: bool,
    /// `{world}` or `{server}` quick-play target (400 BAD_JOIN on invalid).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<Value>,
    /// `{width, height, hidden?}` (400 BAD_WINDOW on invalid; `hidden` is
    /// agent-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Value>,
}

/// Launch reply: a started launch or a dry-run preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LaunchOutcome {
    Started(LaunchReply),
    DryRun(DryRunResult),
}

/// POST /api/instances/:name/stop -> `{ ok, instance, cancelled?, already_stopped? }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopReply {
    pub ok: bool,
    pub instance: String,
    /// 'starting' when the stop landed during the prepare phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub already_stopped: Option<bool>,
}

/// POST /api/instances/:name/train -> `{ key, deferred? }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainReply {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred: Option<bool>,
}

/// GET /api/launches -> `{ launches }` (newest first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchesReply {
    pub launches: Vec<LaunchHandle>,
}

/// GET /api/stats/launches -> `{ launches }` (newest first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchStatsReply {
    pub launches: Vec<LaunchStat>,
}

// ---------------------------------------------------------------------------
// Discord Rich Presence (routes/launch.mjs)
// ---------------------------------------------------------------------------

/// A presence line (JS `{ details?, state, image?, largeText? }`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PresenceLine {
    pub details: Option<String>,
    pub state: String,
    pub image: Option<String>,
    pub large_text: Option<String>,
}

impl PresenceLine {
    fn simple(state: &str, image: Option<&str>) -> Self {
        PresenceLine { details: None, state: state.into(), image: image.map(str::to_string), large_text: None }
    }
}

/// Known Espectral instance names -> the presence shown before joining anything.
const SERVER_STATE: [(&str, &str); 3] =
    [("uhc2", "Jugando en main.uhce.top"), ("uhc", "Jugando en main.uhce.top"), ("24h", "Jugando en 24h.uhce.top")];

struct EspectralServer {
    hosts: &'static [&'static str],
    details: &'static str,
    state: &'static str,
    image: &'static str,
    large_text: &'static str,
}

const ESPECTRAL_SERVERS: [EspectralServer; 3] = [
    EspectralServer {
        hosts: &[
            "main.uhce.top",
            "uhc.espectral.es",
            "uhc2.espectral.es",
            "uhc3.espectral.es",
            "bingo.espectral.es",
            "2.27.233.26",
        ],
        details: "Espectral UHC · MAIN",
        state: "main.uhce.top",
        image: "espectral_logo",
        large_text: "Espectral UHC",
    },
    EspectralServer {
        hosts: &["mini.uhce.top", "150.136.181.209"],
        details: "Espectral UHC · Mini",
        state: "mini.uhce.top",
        image: "espectral_logo",
        large_text: "Espectral UHC Mini",
    },
    EspectralServer {
        hosts: &["24h.uhce.top", "24h.espectral.es", "247.espectral.es"],
        details: "Espectral Meetups 24/7",
        state: "24h.uhce.top",
        image: "24h",
        large_text: "Espectral Meetups 24/7",
    },
];

/// Mod marker logged when the client leaves a server/world (MinecraftMixin).
pub const PRESENCE_MENU_MARKER: &str = "[espectral] presence: menu";

/// Presence for a server the game is connected to; non-Espectral servers
/// stay generic (never leak a private address into a public profile).
pub fn presence_for_server(host: &str) -> PresenceLine {
    let h = host.trim().to_lowercase();
    let h = h.strip_suffix('.').unwrap_or(&h);
    match ESPECTRAL_SERVERS.iter().find(|s| s.hosts.contains(&h)) {
        Some(s) => PresenceLine {
            details: Some(s.details.into()),
            state: s.state.into(),
            image: Some(s.image.into()),
            large_text: Some(s.large_text.into()),
        },
        None => PresenceLine::simple("Jugando servidor multijugador", Some("espectral_logo")),
    }
}

/// A presence transition parsed from one game-log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceLogEvent {
    /// vanilla `Connecting to <host>, <port>`
    Server(String),
    /// the integrated server starting (a world opened)
    Singleplayer,
    /// our mod's leave marker, or a failed connection
    Menu,
}

/// Map one game-log line to a presence transition (None = no change).
pub fn presence_event_from_log(line: &str) -> Option<PresenceLogEvent> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\bConnecting to ([^\s,]+), \d+").expect("regex"));
    if let Some(c) = re.captures(line) {
        return Some(PresenceLogEvent::Server(c[1].to_string()));
    }
    if line.contains("Starting integrated minecraft server") {
        return Some(PresenceLogEvent::Singleplayer);
    }
    if line.contains(PRESENCE_MENU_MARKER) || line.contains("Couldn't connect to server") {
        return Some(PresenceLogEvent::Menu);
    }
    None
}

/// The presence shown before the game has joined anything: a modpack's
/// name, a known Espectral host in the instance name, a snapshot's
/// singleplayer line, else the main menu. Strings stay under 128 chars.
pub fn presence_for_instance(instance: Option<&InstanceMeta>) -> PresenceLine {
    let Some(inst) = instance.filter(|i| !i.name.is_empty()) else {
        return PresenceLine::simple("En el launcher", None);
    };
    if let Some(pack) = inst.modpack.as_deref().filter(|p| !p.is_empty()) {
        let state: String = format!("Jugando {pack}").chars().take(128).collect();
        return PresenceLine { state, image: Some("espectral_logo".into()), ..Default::default() };
    }
    let name = inst.name.to_lowercase();
    for (key, state) in SERVER_STATE {
        if name.contains(key) {
            let image = if key == "24h" { "24h" } else { "espectral_logo" };
            return PresenceLine::simple(state, Some(image));
        }
    }
    static SNAP: OnceLock<regex::Regex> = OnceLock::new();
    let snap = SNAP.get_or_init(|| regex::Regex::new(r"^\d{2}w\d{1,2}[a-z]$").expect("regex"));
    if snap.is_match(&inst.version) {
        PresenceLine::simple("Jugando singleplayer", Some("espectral_logo"))
    } else {
        PresenceLine::simple("En el menú principal", Some("espectral_logo"))
    }
}

fn presence_for_event(ev: &PresenceLogEvent) -> Option<PresenceLine> {
    match ev {
        PresenceLogEvent::Server(h) => Some(presence_for_server(h)),
        PresenceLogEvent::Singleplayer => Some(PresenceLine::simple("Jugando singleplayer", Some("espectral_logo"))),
        PresenceLogEvent::Menu => None,
    }
}

fn presence_quiet(engine: &Engine) -> bool {
    engine.game_launch_state().hooks().quiet_presence || !engine.config().discord_enabled()
}

/// Best-effort presence update (fire-and-forget; never fails the launch).
fn push_presence(engine: &Engine, instance: Option<&InstanceMeta>, start_ts: i64, line: Option<PresenceLine>) {
    if presence_quiet(engine) {
        return;
    }
    let l = line.unwrap_or_else(|| presence_for_instance(instance));
    let p = crate::discord::Presence {
        details: l.details,
        state: Some(l.state),
        start_timestamp: Some(start_ts),
        image: l.image,
        large_text: l.large_text,
        ..Default::default()
    };
    let e = engine.clone();
    engine.spawn_background("discord", async move {
        let _ = crate::discord::set_presence(&e, p).await;
        Ok(())
    });
}

/// Best-effort presence clear (game closed).
fn push_clear(engine: &Engine) {
    if presence_quiet(engine) {
        return;
    }
    let e = engine.clone();
    engine.spawn_background("discord", async move {
        let _ = crate::discord::clear_presence(&e).await;
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// Launch plumbing
// ---------------------------------------------------------------------------

fn base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(D[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// `l<base36 ms><6 hex>` (JS makeKey).
fn make_key() -> String {
    let mut b = [0u8; 3];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    format!("l{}{}", base36(crate::platform::now_ms().max(0) as u64), hex::encode(b))
}

fn emit_lines(engine: &Engine, key: &str, lines: Vec<(u64, String)>) {
    for (seq, line) in lines {
        engine.emit(EngineEvent::LaunchLog(LaunchLogEvent { key: key.into(), seq, line }));
    }
}

fn system_line(engine: &Engine, key: &str, line: &str) {
    let lines = engine.game_launch_state().append_system(key, line);
    emit_lines(engine, key, lines);
}

/// Background auto-train (fire-and-forget; emits its own train-* events).
fn auto_train(engine: &Engine, instance: InstanceMeta, estimate_key: Option<String>, force: bool) {
    let e = engine.clone();
    engine.spawn_background("aot", async move {
        let weak = Arc::downgrade(&e.inner);
        let name = instance.name.clone();
        // The trainer shares the instance's gameDir: never spawn it under a live game.
        let blocked: Arc<dyn Fn() -> bool + Send + Sync> =
            Arc::new(move || upgrade(&weak).is_some_and(|e| e.active_instances().contains(&name)));
        let opts = crate::aot::TrainOptions { key: estimate_key.clone(), force, is_blocked: Some(blocked) };
        if let Err(err) = crate::aot::queue_train_instance(&e, &instance, opts).await {
            e.emit(EngineEvent::TrainDone(TrainDoneEvent {
                instance: instance.name.clone(),
                key: estimate_key,
                ok: false,
                error: Some(err.to_string()),
                skipped: None,
                cache_size_bytes: None,
                proof: None,
                extra: Default::default(),
            }));
        }
        Ok(())
    });
}

fn exit_event(
    key: &str,
    instance: &str,
    account: Option<String>,
    error: &str,
    phases: Map<String, Value>,
) -> LaunchExitEvent {
    LaunchExitEvent {
        key: key.into(),
        instance: instance.into(),
        account,
        code: None,
        signal: None,
        marker: None,
        error: Some(error.into()),
        menu_ms: None,
        played_ms: None,
        spawn_ms: None,
        boot_ms: None,
        phases,
    }
}

/// Reconcile spawned processes against the OS: dead entries are finalized
/// through their own (single-flight) exit path. Preparing launches (no
/// process yet) are never touched — pending_stops owns those.
fn reconcile_running(engine: &Engine) {
    let st = engine.game_launch_state();
    let dead: Vec<RunningProcess> = {
        let mut running = lock(&st.running);
        let names: Vec<String> = running
            .iter()
            .filter(|(_, p)| {
                let exited = p.process.as_ref().is_some_and(ProcessHandle::has_exited);
                exited || !crate::platform::is_pid_alive(p.pid)
            })
            .map(|(n, _)| n.clone())
            .collect();
        names.into_iter().filter_map(|n| running.remove(&n)).collect()
    };
    for p in dead {
        let marker = st.with_buffer(&p.key, |b| b.menu_at.is_some()).unwrap_or(false);
        let status = p.process.as_ref().and_then(ProcessHandle::exit_status).unwrap_or_default();
        let finalize = p.finalize.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            finalize(GameExit { code: status.code, signal: status.signal, marker, error: None })
        }));
    }
}

/// Force-kill a running game (tree on Windows) and wait for it to go.
async fn force_kill_process(pid: u32, process: Option<ProcessHandle>) {
    if process.as_ref().is_some_and(ProcessHandle::has_exited) {
        return;
    }
    launch::process::force_kill_tree(pid).await;
    match process {
        None => {
            // Adopted game: no exit event to wait for — poll the pid.
            for _ in 0..50 {
                if !crate::platform::is_pid_alive(pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Some(p) => {
            p.wait_timeout(Duration::from_secs(5)).await;
        }
    }
}

/// The resolve + spawn half of a launch (runs in the background).
#[allow(clippy::too_many_arguments)]
async fn run_launch(
    engine: Engine,
    key: String,
    instance: InstanceMeta,
    account_override: Option<crate::types::Account>,
    presence_start: i64,
    progress: launch::LineFn,
    options: crate::agent::LaunchOptions,
) {
    let st = engine.game_launch_state();
    let name = instance.name.clone();
    let account_name = account_override.as_ref().map(|a| a.username.clone());
    let agent = options.agent;
    let hooks = st.hooks();

    let account = match &account_override {
        Some(acc) => {
            let (a, warn) = launch::resolve_account_for(&engine, Some(acc)).await;
            if let Some(w) = warn {
                progress(&if w.starts_with("[espectral]") { w } else { format!("[espectral] {w}") });
            }
            Some(LaunchAccountInput::Resolved(a))
        }
        None => None,
    };
    let opts = ResolveLaunchOptions {
        on_progress: Some(progress.clone()),
        account,
        agent,
        join: options.join.clone(),
        window: options.window.clone(),
        ..Default::default()
    };
    let resolved = match &hooks.resolve_launch {
        Some(h) => h(instance.clone(), opts).await,
        None => launch::resolve_launch(&engine, &instance, opts).await,
    };
    let mut resolved = match resolved {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("[espectral] launch error: {e}");
            engine.active_instances().remove(&name);
            lock(&st.pending_stops).remove(&name);
            if agent {
                crate::agent::release_launch(engine.agent_state(), &key);
            }
            tracing::warn!("{msg}");
            system_line(&engine, &key, &msg);
            let (spawn_ms, boot_ms, phases) = st
                .with_buffer(&key, |b| {
                    b.finish();
                    let (s, bt) = b.spawn_boot_ms();
                    (s, bt, b.phases.clone())
                })
                .unwrap_or_default();
            if st.live_is_empty() {
                push_clear(&engine);
            }
            let mut ev = exit_event(&key, &name, account_name, &e.to_string(), phases);
            ev.spawn_ms = spawn_ms;
            ev.boot_ms = boot_ms;
            engine.emit(EngineEvent::LaunchExit(ev));
            return;
        }
    };

    // C6: cancelled during prepare — a stop arrived while resolve was in flight.
    if lock(&st.pending_stops).remove(&name) {
        engine.active_instances().remove(&name);
        if agent {
            crate::agent::release_launch(engine.agent_state(), &key);
        }
        let phases = st
            .with_buffer(&key, |b| {
                b.finish();
                b.phases.clone()
            })
            .unwrap_or_default();
        if st.live_is_empty() {
            push_clear(&engine);
        }
        engine.emit(EngineEvent::LaunchExit(exit_event(&key, &name, account_name, "cancelled during prepare", phases)));
        return;
    }

    // Agent launches never touch Discord rich presence: a bot-driven game is
    // not what the player is playing. They are also NOT counted in the presence
    // refcount — otherwise a player game exiting would leave the presence line
    // stuck for as long as the bot game runs.
    if !agent {
        lock(&st.live_launches).insert(key.clone());
        push_presence(&engine, Some(&instance), presence_start, None);
    }

    let weak = Arc::downgrade(&engine.inner);
    let exit_seen = Arc::new(Mutex::new(false));
    let live_presence: Arc<Mutex<Option<PresenceLine>>> = Arc::new(Mutex::new(None));
    let bridge_token = resolved.bridge_token.clone().unwrap_or_default();

    // Single-flight exit finalizer (process exit and the reconcile sweep race).
    let handle_exit: Arc<dyn Fn(GameExit) + Send + Sync> = {
        let (weak, key, name, instance) = (weak.clone(), key.clone(), name.clone(), instance.clone());
        let exit_seen = exit_seen.clone();
        let token = bridge_token.clone();
        let aot_key = resolved.aot_key.clone();
        let agent_exit = agent;
        Arc::new(move |x: GameExit| {
            let Some(engine) = upgrade(&weak) else { return };
            let st = engine.game_launch_state();
            {
                let mut seen = lock(&exit_seen);
                if *seen {
                    return;
                }
                *seen = true;
                lock(&st.running).remove(&name);
                crate::game_sessions::remove_session(&engine, &name);
            }
            lock(&st.pending_stops).remove(&name);
            let ended_at = crate::platform::now_ms();
            let snap = st.with_buffer(&key, |b| {
                b.running = false;
                b.ended_at = Some(ended_at);
                let auto = std::mem::take(&mut b.needs_auto_train);
                (b.clone(), auto)
            });
            let (buf, needs_auto_train) = match snap {
                Some((b, a)) => (Some(b), a),
                None => (None, false),
            };
            let started_at = buf.as_ref().map_or(ended_at, |b| b.started_at);
            let menu_at = buf.as_ref().and_then(|b| b.menu_at);
            let menu_ms = menu_at.map(|m| m - started_at);
            let played_ms = ended_at - started_at;
            let spawn_at = buf.as_ref().and_then(|b| b.spawned_at);
            let spawn_ms = spawn_at.map(|s| s - started_at);
            let boot_ms = match (menu_at, spawn_at) {
                (Some(m), Some(s)) => Some(m - s),
                _ => None,
            };
            let phases = buf.as_ref().map(|b| b.phases.clone()).unwrap_or_default();
            lock(&st.live_launches).remove(&key);
            engine.active_instances().remove(&name);
            // Any in-flight agent call is failed with GAME_GONE and parked
            // polls are released now — a waiter must never outlive the game.
            if agent_exit {
                crate::agent::release_launch(engine.agent_state(), &key);
            }
            let _ = crate::presence::launch_exited(&engine, &name, Some(&token));
            if st.live_is_empty() {
                push_clear(&engine);
            }
            let summary = match &x.error {
                Some(err) => format!("[espectral] launch error: {err}"),
                None => format!(
                    "[espectral] game exited (code {}{})",
                    x.code.map_or("null".to_string(), |c| c.to_string()),
                    x.signal.as_ref().map(|s| format!(", signal {s}")).unwrap_or_default()
                ),
            };
            system_line(&engine, &key, &summary);
            let inst_name = buf.as_ref().map_or(name.clone(), |b| b.instance.clone());
            engine.emit(EngineEvent::LaunchExit(LaunchExitEvent {
                key: key.clone(),
                instance: inst_name.clone(),
                account: buf.as_ref().and_then(|b| b.account.clone()),
                code: x.code,
                signal: x.signal.clone(),
                marker: Some(x.marker),
                error: x.error.clone(),
                menu_ms,
                played_ms: Some(played_ms),
                spawn_ms,
                boot_ms,
                phases: phases.clone(),
            }));
            crate::stats::record_launch_stat(
                engine.paths(),
                &LaunchStat {
                    key: key.clone(),
                    instance: inst_name,
                    version: buf.as_ref().map(|b| b.version.clone()),
                    started_at: Some(started_at),
                    menu_at,
                    menu_ms,
                    played_ms: Some(played_ms),
                    spawn_ms,
                    boot_ms,
                    phases: Some(phases),
                    extra: Default::default(),
                },
            );
            // Deferred auto-train: the gameDir is free now (one window at a time).
            if needs_auto_train {
                auto_train(&engine, instance.clone(), Some(aot_key.clone()), false);
            }
        })
    };

    let callbacks = {
        let on_log: Arc<dyn Fn(&str) + Send + Sync> = {
            let (e, key, instance, live_presence) =
                (engine.clone(), key.clone(), instance.clone(), live_presence.clone());
            let agent_log = agent;
            Arc::new(move |chunk: &str| {
                let lines = e.game_launch_state().append_output(&key, chunk);
                for (seq, line) in lines {
                    let ev = if agent_log { None } else { presence_event_from_log(&line) };
                    e.emit(EngineEvent::LaunchLog(LaunchLogEvent { key: key.clone(), seq, line }));
                    if let Some(ev) = ev {
                        let p = presence_for_event(&ev);
                        *lock(&live_presence) = p.clone();
                        push_presence(&e, Some(&instance), presence_start, p);
                    }
                }
            })
        };
        let on_spawned: Arc<dyn Fn(i64) + Send + Sync> = {
            let (e, key, name) = (engine.clone(), key.clone(), name.clone());
            let uuid = Some(resolved.account.uuid.clone()).filter(|u| !u.is_empty());
            let game_dir = resolved.game_dir.clone();
            let token = bridge_token.clone();
            let spawned_agent = agent;
            Arc::new(move |at| {
                e.game_launch_state().with_buffer(&key, |b| b.spawned_at = Some(at));
                // The agent entry was registered when the key was minted; the
                // resolved gameDir (per-account profile dirs) is only known now.
                if spawned_agent {
                    crate::agent::register_launch(e.agent_state(), &key, &name, Some(game_dir.clone()));
                }
                let _ = crate::presence::launch_started(
                    &e,
                    crate::presence::LaunchPresence {
                        instance: name.clone(),
                        uuid: uuid.clone(),
                        game_dir: game_dir.clone(),
                        token: token.clone(),
                    },
                );
            })
        };
        let on_phase: Arc<dyn Fn(&str) + Send + Sync> = {
            let (e, key) = (engine.clone(), key.clone());
            Arc::new(move |phase: &str| {
                e.game_launch_state().with_buffer(&key, |b| {
                    if !b.phases.contains_key(phase) {
                        b.phases.insert(phase.into(), Value::from(crate::platform::now_ms()));
                    }
                });
            })
        };
        let on_marker: Arc<dyn Fn(i64) + Send + Sync> = {
            let (e, key, name, instance, live_presence) =
                (engine.clone(), key.clone(), name.clone(), instance.clone(), live_presence.clone());
            let needs_cache = !resolved.aot_cache_exists || resolved.aot_cache_stale;
            let aot_available = resolved.aot_available;
            let marker_agent = agent;
            Arc::new(move |at| {
                let st = e.game_launch_state();
                let has_buf = st.with_buffer(&key, |b| b.menu_at = Some(at)).is_some();
                let iso = chrono::DateTime::from_timestamp_millis(at)
                    .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                    .unwrap_or_default();
                system_line(&e, &key, &format!("[espectral] Sound engine started ({iso})"));
                // Presence refreshes at the main menu with the same elapsed
                // clock — except for agent launches, which must never appear in
                // Discord presence (this is the only push left on this path).
                if !marker_agent {
                    push_presence(&e, Some(&instance), presence_start, lock(&live_presence).clone());
                }
                crate::game_sessions::record_session(&e, &name, json!({ "menu_at": at }));
                // The game is up: the UI may close the launcher now — never for
                // an agent launch (the launcher IS the relay: closing it would
                // cut the agent off from the game it just started).
                if !marker_agent {
                    e.emit(EngineEvent::LaunchReady(LaunchReadyEvent { key: key.clone(), instance: name.clone() }));
                }
                // AOT auto-train on a missing/refused cache — deferred until this
                // game EXITS (the trainer is a second game in the same gameDir).
                let auto_on = instance.aot_auto_train.unwrap_or_else(|| e.config().aot_auto_train());
                if has_buf && auto_on && needs_cache && aot_available {
                    st.with_buffer(&key, |b| b.needs_auto_train = true);
                }
            })
        };
        let on_exit = handle_exit.clone();
        LaunchCallbacks {
            on_log: Some(on_log),
            on_marker: Some(on_marker),
            on_exit: Some(Arc::new(move |x| on_exit(x))),
            on_spawned: Some(on_spawned),
            on_phase: Some(on_phase),
        }
    };

    let spawned = match &hooks.launch_instance {
        Some(h) => h(&engine, &instance, &mut resolved, callbacks),
        None => launch::launch_instance(&engine, &instance, &mut resolved, callbacks),
    };
    match spawned {
        Ok(sp) => {
            let seen = lock(&exit_seen);
            let alive = sp.process.as_ref().map_or(true, |p| !p.has_exited());
            if !*seen && alive {
                lock(&st.running).insert(
                    name.clone(),
                    RunningProcess {
                        pid: sp.pid,
                        key: key.clone(),
                        process: sp.process.clone(),
                        finalize: handle_exit,
                    },
                );
                let (account, started_at) =
                    st.with_buffer(&key, |b| (b.account.clone(), b.started_at)).unwrap_or((None, presence_start));
                crate::game_sessions::record_session(
                    &engine,
                    &name,
                    json!({
                        "key": key,
                        "pid": sp.pid,
                        "version": instance.version,
                        "account": account,
                        "uuid": Some(resolved.account.uuid.clone()).filter(|u| !u.is_empty()),
                        "game_dir": resolved.game_dir.to_string_lossy(),
                        "started_at": started_at,
                        "bridge_token": bridge_token,
                        "agent": agent,
                    }),
                );
            }
        }
        Err(spawn_err) => {
            lock(&st.running).remove(&name);
            lock(&st.pending_stops).remove(&name);
            let summary = spawn_err.to_string();
            tracing::warn!("[espectral] spawn error: {summary}");
            let (spawn_ms, boot_ms, phases) = st
                .with_buffer(&key, |b| {
                    b.finish();
                    let (s, bt) = b.spawn_boot_ms();
                    (s, bt, b.phases.clone())
                })
                .unwrap_or_default();
            lock(&st.live_launches).remove(&key);
            engine.active_instances().remove(&name);
            if agent {
                crate::agent::release_launch(engine.agent_state(), &key);
            }
            let _ = crate::presence::launch_exited(&engine, &name, Some(&bridge_token));
            if st.live_is_empty() {
                push_clear(&engine);
            }
            let mut ev = exit_event(&key, &name, account_name, &summary, phases);
            ev.spawn_ms = spawn_ms;
            ev.boot_ms = boot_ms;
            engine.emit(EngineEvent::LaunchExit(ev));
        }
    }
}

// ---------------------------------------------------------------------------
// Facade
// ---------------------------------------------------------------------------

impl Engine {
    /// POST /api/instances/:name/launch. Dry run -> DryRunResult. Otherwise
    /// 409 ALREADY_RUNNING when active; answers immediately with
    /// `{ key, pid: null, preparing: true, instance, version, account }` and
    /// resolves + spawns in the background (launch-log / launch-ready /
    /// launch-exit events, stats row, presence, deferred auto-train).
    pub async fn launch_instance(&self, name: &str, req: LaunchRequest) -> EngineResult<LaunchOutcome> {
        let instance = launch::load_instance(self, name).await?;
        // Contract C: optional account override (its own per-account run dir).
        let account_override = match req.account.as_deref() {
            None => None,
            Some("") => return Err(EngineError::bad_request("BAD_ACCOUNT", "account must be a non-empty string")),
            Some(u) => Some(
                crate::accounts::get_account(self, u)
                    .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("unknown account: {u}")))?,
            ),
        };
        // One validation for the UI route and the agent route (agent.mjs
        // normalizeLaunchOptions): a bad join/window is a 400 here, before a
        // buffer exists.
        let body = json!({ "agent": req.agent, "join": req.join, "window": req.window });
        let options = crate::agent::normalize_launch_options(&body, false)?;
        if req.dry_run {
            let opts = ResolveLaunchOptions {
                dry_run: true,
                account: account_override.map(|a| LaunchAccountInput::Stored(Box::new(a))),
                agent: options.agent,
                join: options.join,
                window: options.window,
                ..Default::default()
            };
            let resolved = launch::resolve_launch(self, &instance, opts).await?;
            // The preview carries the resolved options (the JS `resolved`
            // object does): `warnings` already includes the ffmpeg one.
            return Ok(LaunchOutcome::DryRun(launch::dry_run_result(self, &instance, &resolved)?));
        }
        let agent = options.agent;
        if !self.active_instances().insert(name) {
            return Err(EngineError::conflict(
                "ALREADY_RUNNING",
                format!("instance is already launching or running: {name}"),
            ));
        }
        let st = self.game_launch_state();
        let key = make_key();
        let account_name = account_override.as_ref().map(|a| a.username.clone());
        st.create_buffer(&key, name, &instance.version, account_name.clone());
        // Agent launches are tagged on the buffer (→ /api/launches rows) so the
        // UI and the agent routes can tell a bot game from a player game.
        st.with_buffer(&key, |b| b.agent = agent);
        // Register with the agent registry BEFORE the key is returned: the
        // mod's first poll races the spawn, and an agent call to a key the
        // caller already holds must never answer "unknown launch". The
        // gameDir is refreshed on spawn (per-account profile dirs).
        if agent {
            crate::agent::register_launch(
                self.agent_state(),
                &key,
                name,
                instance.game_dir.as_deref().filter(|d| !d.is_empty()).map(std::path::PathBuf::from),
            );
        }

        // Stream the prepare phase as launch-log lines (the buffer exists even
        // if resolve later fails).
        let progress: launch::LineFn = {
            let (e, key) = (self.clone(), key.clone());
            Arc::new(move |msg: &str| {
                tracing::info!("{msg}");
                system_line(&e, &key, msg);
            })
        };
        progress(&format!("[espectral] preparing {name}…"));
        let presence_start = crate::platform::now_ms();
        let e = self.clone();
        let (k, inst) = (key.clone(), instance.clone());
        self.spawn_background("launch", async move {
            run_launch(e, k, inst, account_override, presence_start, progress, options).await;
            Ok(())
        });
        Ok(LaunchOutcome::Started(LaunchReply {
            key,
            pid: None,
            preparing: true,
            instance: name.to_string(),
            version: instance.version,
            account: account_name,
        }))
    }

    /// POST /api/instances/:name/stop — idempotent force-kill (taskkill /T /F
    /// on Windows, SIGKILL elsewhere); a stop during prepare cancels the launch.
    pub async fn stop_instance(&self, name: &str) -> EngineResult<StopReply> {
        reconcile_running(self);
        let reply = |cancelled: Option<&str>, already: Option<bool>| StopReply {
            ok: true,
            instance: name.to_string(),
            cancelled: cancelled.map(str::to_string),
            already_stopped: already,
        };
        if !self.active_instances().contains(name) {
            return Ok(reply(None, Some(true)));
        }
        let st = self.game_launch_state();
        let proc = lock(&st.running).get(name).map(|p| (p.pid, p.process.clone()));
        let Some((pid, process)) = proc else {
            lock(&st.pending_stops).insert(name.to_string());
            return Ok(reply(Some("starting"), None));
        };
        force_kill_process(pid, process).await;
        reconcile_running(self);
        Ok(reply(None, None))
    }

    /// POST /api/instances/:name/train — 400 AOT_UNAVAILABLE on NeoForge;
    /// deferred while the game runs; else background train (train-* events).
    pub async fn train_instance(&self, name: &str) -> EngineResult<TrainReply> {
        let instance = launch::load_instance(self, name).await?;
        if instance.loader == Loader::Neoforge {
            return Err(EngineError::bad_request("AOT_UNAVAILABLE", "AOT unavailable on NeoForge (requires JDK 25)"));
        }
        let key = crate::aot::instance_key(self, &instance).await?;
        if self.active_instances().contains(name) {
            // Never train under the live game: the launch's exit trains instead.
            let st = self.game_launch_state();
            let run_key = lock(&st.running).get(name).map(|p| p.key.clone());
            if let Some(k) = run_key {
                st.with_buffer(&k, |b| b.needs_auto_train = true);
            }
            return Ok(TrainReply { key, deferred: Some(true) });
        }
        auto_train(self, instance, Some(key.clone()), true);
        Ok(TrainReply { key, deferred: None })
    }

    /// GET /api/instances/:name/aot -> AotStatus (instances::aot_status).
    pub async fn instance_aot_status(&self, name: &str) -> EngineResult<AotStatus> {
        let instance = launch::load_instance(self, name).await?;
        crate::instances::aot_status(self, &instance).await
    }

    /// GET /api/launch/:key/log?cursor=N -> `{ lines, cursor, running }`;
    /// 404 NOT_FOUND for an unknown key.
    pub async fn launch_log(&self, key: &str, cursor: u64) -> EngineResult<LogChunk> {
        self.game_launch_state()
            .buffer_view(key, cursor)
            .ok_or_else(|| EngineError::not_found("NOT_FOUND", format!("unknown launch key: {key}")))
    }

    /// GET /api/launches — reconciles dead pids first; newest first.
    pub async fn list_launches(&self) -> EngineResult<LaunchesReply> {
        reconcile_running(self);
        let mut rows: Vec<LaunchHandle> = lock(&self.game_launch_state().buffers)
            .iter()
            .map(|(k, b)| LaunchHandle {
                key: k.clone(),
                instance: b.instance.clone(),
                version: b.version.clone(),
                account: b.account.clone(),
                running: b.running,
                agent: b.agent,
                started_at: b.started_at,
                menu_at: b.menu_at,
                ended_at: b.ended_at,
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.started_at));
        Ok(LaunchesReply { launches: rows })
    }

    /// GET /api/stats/launches?limit=N (default 10, clamped 1..=100).
    pub async fn launch_stats(&self, limit: Option<i64>) -> EngineResult<LaunchStatsReply> {
        let limit = match limit {
            Some(l) if l >= 1 => l.min(100) as usize,
            _ => 10,
        };
        Ok(LaunchStatsReply { launches: crate::stats::read_launch_stats(self.paths(), limit) })
    }

    /// POST /api/instances/:name/version-change/plan `{ version?, loader? }`.
    pub async fn plan_version_change(&self, name: &str, target: VersionTarget) -> EngineResult<VersionChangePlan> {
        crate::version_change::plan_version_change(self, name, &target).await
    }

    /// POST /api/instances/:name/version-change — 409 ALREADY_RUNNING while
    /// active; marks the instance active while applying.
    pub async fn apply_version_change(&self, name: &str, target: VersionTarget) -> EngineResult<VersionChangeResult> {
        if !self.active_instances().insert(name) {
            return Err(EngineError::conflict(
                "ALREADY_RUNNING",
                format!("instance '{name}' is running; close the game first"),
            ));
        }
        let r = crate::version_change::apply_version_change(self, name, &target).await;
        self.active_instances().remove(name);
        r
    }

    /// Re-attach games still running from an earlier launcher session (the
    /// game is detached, so it survives the launcher closing). Each becomes a
    /// pid-only running entry: Play is blocked, Stop kills it, the bridge +
    /// presence come back, and the reconcile sweep finalizes it when the
    /// process ends. Returns the adopted instance names.
    pub async fn adopt_running_games(&self) -> Vec<String> {
        let Ok(live) = crate::game_sessions::live_sessions(self).await else { return vec![] };
        let st = self.game_launch_state();
        let mut adopted = Vec::new();
        for (name, s) in live {
            if self.active_instances().contains(&name) {
                continue;
            }
            let instance = match launch::load_instance(self, &name).await {
                Ok(i) => i,
                Err(_) => InstanceMeta {
                    name: name.clone(),
                    version: s.version.clone().unwrap_or_default(),
                    ..Default::default()
                },
            };
            let key = s.key.clone().filter(|k| !k.is_empty()).unwrap_or_else(make_key);
            let version = s.version.clone().unwrap_or_else(|| instance.version.clone());
            st.create_buffer(&key, &name, &version, s.account.clone());
            // An adopted AGENT game comes back as an agent launch: the relay and
            // /api/agent/* keep working after a launcher restart.
            let adopted_agent = s.agent == Some(true);
            st.with_buffer(&key, |b| b.agent = adopted_agent);
            let started_at = s.started_at.unwrap_or_else(crate::platform::now_ms);
            st.with_buffer(&key, |b| {
                b.started_at = started_at;
                b.spawned_at = Some(started_at);
                b.menu_at = s.menu_at;
            });
            st.append_system(
                &key,
                &format!("[espectral] {name} sigue abierto desde antes de reabrir el launcher (pid {})", s.pid),
            );
            self.active_instances().insert(&name);
            if !adopted_agent {
                lock(&st.live_launches).insert(key.clone());
            }
            let token = s.bridge_token.clone().unwrap_or_default();
            let _ = crate::presence::launch_started(
                self,
                crate::presence::LaunchPresence {
                    instance: name.clone(),
                    uuid: s.uuid.clone(),
                    game_dir: s.game_dir.clone().unwrap_or_default().into(),
                    token: token.clone(),
                },
            );
            if adopted_agent {
                // The game is still the one an agent started: re-register it so
                // the agent routes and the mod relay keep working.
                crate::agent::register_launch(
                    self.agent_state(),
                    &key,
                    &name,
                    s.game_dir.as_deref().filter(|d| !d.is_empty()).map(std::path::PathBuf::from),
                );
            } else {
                push_presence(self, Some(&instance), started_at, None);
            }
            let done = Arc::new(AtomicBool::new(false));
            let weak = Arc::downgrade(&self.inner);
            let (n, k) = (name.clone(), key.clone());
            let finalize: Arc<dyn Fn(GameExit) + Send + Sync> = Arc::new(move |_x| {
                if done.swap(true, Ordering::SeqCst) {
                    return;
                }
                let Some(e) = upgrade(&weak) else { return };
                let st = e.game_launch_state();
                lock(&st.running).remove(&n);
                crate::game_sessions::remove_session(&e, &n);
                let ended_at = crate::platform::now_ms();
                let snap = st.with_buffer(&k, |b| {
                    b.running = false;
                    b.ended_at = Some(ended_at);
                    (b.account.clone(), b.started_at, b.menu_at)
                });
                lock(&st.live_launches).remove(&k);
                e.active_instances().remove(&n);
                if adopted_agent {
                    crate::agent::release_launch(e.agent_state(), &k);
                }
                let _ = crate::presence::launch_exited(&e, &n, Some(&token));
                if st.live_is_empty() {
                    push_clear(&e);
                }
                system_line(&e, &k, "[espectral] game exited");
                let (account, started, menu_at) = snap.unwrap_or((None, ended_at, None));
                e.emit(EngineEvent::LaunchExit(LaunchExitEvent {
                    key: k.clone(),
                    instance: n.clone(),
                    account,
                    code: None,
                    signal: None,
                    marker: Some(menu_at.is_some()),
                    error: None,
                    menu_ms: menu_at.map(|m| m - started),
                    played_ms: Some(ended_at - started),
                    spawn_ms: None,
                    boot_ms: None,
                    phases: Map::new(),
                }));
            });
            lock(&st.running).insert(name.clone(), RunningProcess { pid: s.pid, key, process: None, finalize });
            tracing::info!("[launch] adopted running game {name} (pid {})", s.pid);
            adopted.push(name);
        }
        adopted
    }
}

#[cfg(test)]
pub(crate) mod test_util {

    /// A stand-in `java` (unix shell script). Writes its argv to
    /// `<cwd>/args.txt`, prints a boot phase + the menu marker (stdout and
    /// logs/latest.log). `stay`: keep running until SIGTERM (which writes the
    /// `-XX:AOTCacheOutput` file, like a JVM dumping its AOT cache) /
    /// SIGKILL; otherwise exit 3 shortly after the menu.
    #[cfg(unix)]
    pub fn fake_java(dir: &std::path::Path, stay: bool) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let tail = if stay { "while :; do sleep 0.05; done" } else { "sleep 0.3\nexit 3" };
        let script = format!(
            r#"#!/bin/sh
out=""
for a in "$@"; do
  case "$a" in -XX:AOTCacheOutput=*) out="${{a#-XX:AOTCacheOutput=}}";; esac
done
printf '%s\n' "$@" > args.txt
trap 'if [ -n "$out" ]; then printf aotcache > "$out"; fi; exit 0' TERM
mkdir -p logs
echo "[main/INFO]: Loading Minecraft 1.21.11"
echo "[Render thread/INFO]: Connecting to main.uhce.top, 25565"
printf 'boot\nSound engine started\n' >> logs/latest.log
echo "[Render thread/INFO]: Sound engine started"
{tail}
"#
        );
        let p = dir.join(if stay { "fake-java-stay" } else { "fake-java" });
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use crate::types::{JavaRuntime, LaunchAccount, LaunchMode, ResolvedVersionRef};
    use futures_util::FutureExt;

    #[tokio::test]
    async fn launch_stats_clamps_limit() {
        let (_d, e) = temp_engine();
        for i in 0..120 {
            crate::stats::record_launch_stat(e.paths(), &LaunchStat { key: format!("k{i}"), ..Default::default() });
        }
        assert_eq!(e.launch_stats(None).await.unwrap().launches.len(), 10);
        assert_eq!(e.launch_stats(Some(0)).await.unwrap().launches.len(), 10);
        assert_eq!(e.launch_stats(Some(500)).await.unwrap().launches.len(), 100);
        assert_eq!(e.launch_stats(Some(3)).await.unwrap().launches[0].key, "k119");
    }

    fn write_instance(e: &Engine, name: &str, extra: Value) {
        let mut doc = json!({ "name": name, "version": "1.21.11", "loader": "vanilla", "aot_auto_train": false });
        doc.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        crate::paths::write_json_atomic(&e.paths().instance_json(name), &doc).unwrap();
    }

    fn fake_resolved(e: &Engine, name: &str, java: &str) -> ResolvedLaunch {
        let dir = e.paths().instance_dir(name);
        ResolvedLaunch {
            java: JavaRuntime { path: java.into(), build: "25-test".into(), major: Some(25), ..Default::default() },
            version: ResolvedVersionRef {
                id: "1.21.11".into(),
                main_class: "net.minecraft.client.main.Main".into(),
                asset_index_id: Some("1.21".into()),
                json: json!({}),
            },
            loader: None,
            fabric_profile: None,
            classpath: vec![],
            natives_dir: dir.join("natives"),
            game_dir: dir,
            assets_dir: e.paths().assets_dir(),
            memory_mb: 1024,
            account: LaunchAccount {
                username: "Tester".into(),
                uuid: "00000000-0000-0000-0000-000000000000".into(),
                access_token: "0".into(),
                user_type: "mojang".into(),
            },
            bridge_token: None,
            mode: LaunchMode::Normal,
            aot_key: "test-key".into(),
            aot_cache_path: e.paths().aot_cache_file("test-key"),
            aot_cache_exists: false,
            aot_cache_stale: false,
            aot_available: true,
            agent: false,
            join: None,
            window: None,
            ffmpeg_path: None,
            warnings: vec![],
        }
    }

    async fn next_named(rx: &mut tokio::sync::broadcast::Receiver<EngineEvent>, name: &str) -> EngineEvent {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let ev = rx.recv().await.expect("event");
                if ev.name() == name {
                    return ev;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {name}"))
    }

    #[tokio::test]
    async fn stop_during_starting_cancels_and_never_spawns() {
        let (_d, e) = temp_engine();
        let name = "inst-stop-test";
        write_instance(&e, name, json!({}));
        let gate = Arc::new(tokio::sync::Notify::new());
        let resolve_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let spawn_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let resolved = fake_resolved(&e, name, "/fake/java");
        {
            let (gate, rc, sc) = (gate.clone(), resolve_calls.clone(), spawn_calls.clone());
            e.set_launch_test_hooks(LaunchTestHooks {
                resolve_launch: Some(Arc::new(move |_, _| {
                    rc.fetch_add(1, Ordering::SeqCst);
                    let (gate, r) = (gate.clone(), resolved.clone());
                    async move {
                        gate.notified().await;
                        Ok(r)
                    }
                    .boxed()
                })),
                launch_instance: Some(Arc::new(move |_, _, _, _| {
                    sc.fetch_add(1, Ordering::SeqCst);
                    Ok(SpawnedGame { pid: 9999, argv: vec![], process: None })
                })),
                quiet_presence: true,
                ..Default::default()
            });
        }
        let mut rx = e.subscribe();
        let LaunchOutcome::Started(reply) = e.launch_instance(name, LaunchRequest::default()).await.unwrap() else {
            panic!("expected a started launch");
        };
        assert!(reply.preparing);
        assert!(reply.key.starts_with('l'));
        assert!(e.active_instances().contains(name));
        // Second Play while preparing is refused.
        let err = e.launch_instance(name, LaunchRequest::default()).await.unwrap_err();
        assert_eq!(err.envelope().code, "ALREADY_RUNNING");

        let stop = e.stop_instance(name).await.unwrap();
        assert_eq!(
            stop,
            StopReply { ok: true, instance: name.into(), cancelled: Some("starting".into()), already_stopped: None }
        );
        assert!(lock(&e.game_launch_state().pending_stops).contains(name));
        tokio::time::sleep(Duration::from_millis(20)).await;
        gate.notify_one();
        let EngineEvent::LaunchExit(x) = next_named(&mut rx, "launch-exit").await else { unreachable!() };
        assert_eq!(x.error.as_deref(), Some("cancelled during prepare"));
        assert_eq!(x.instance, name);
        assert_eq!((x.spawn_ms, x.boot_ms), (None, None));
        assert_eq!(spawn_calls.load(Ordering::SeqCst), 0);
        assert_eq!(resolve_calls.load(Ordering::SeqCst), 1);
        assert!(!e.active_instances().contains(name));
        assert!(!lock(&e.game_launch_state().pending_stops).contains(name));
        // Stop again: idempotent.
        assert_eq!(e.stop_instance(name).await.unwrap().already_stopped, Some(true));
        // The prepare log is backfillable.
        let log = e.launch_log(&reply.key, 0).await.unwrap();
        assert_eq!(log.lines[0], format!("[espectral] preparing {name}…"));
        assert!(!log.running);
    }

    #[tokio::test]
    async fn resolve_failure_reports_launch_exit() {
        let (_d, e) = temp_engine();
        write_instance(&e, "broken", json!({}));
        e.set_launch_test_hooks(LaunchTestHooks {
            resolve_launch: Some(Arc::new(|_, _| async { Err(EngineError::other("no JDK")) }.boxed())),
            quiet_presence: true,
            ..Default::default()
        });
        let mut rx = e.subscribe();
        e.launch_instance("broken", LaunchRequest::default()).await.unwrap();
        let EngineEvent::LaunchExit(x) = next_named(&mut rx, "launch-exit").await else { unreachable!() };
        assert_eq!(x.error.as_deref(), Some("no JDK"));
        assert_eq!(x.marker, None);
        assert!(!e.active_instances().contains("broken"));
        let rows = e.list_launches().await.unwrap().launches;
        let log = e.launch_log(&rows[0].key, 0).await.unwrap();
        assert_eq!(log.lines.last().unwrap(), "[espectral] launch error: no JDK");
    }

    #[tokio::test]
    async fn account_override_validation_and_unknown_key() {
        let (_d, e) = temp_engine();
        write_instance(&e, "acc", json!({}));
        let err = e
            .launch_instance("acc", LaunchRequest { account: Some(String::new()), ..Default::default() })
            .await
            .unwrap_err();
        assert_eq!((err.status(), err.envelope().code.as_str()), (400, "BAD_ACCOUNT"));
        let err = e
            .launch_instance("acc", LaunchRequest { account: Some("ghost".into()), ..Default::default() })
            .await
            .unwrap_err();
        assert_eq!((err.status(), err.envelope().code.as_str()), (404, "NOT_FOUND"));
        assert!(!e.active_instances().contains("acc"));
        let err = e.launch_log("nope", 0).await.unwrap_err();
        assert_eq!(err.status(), 404);
    }

    #[tokio::test]
    async fn train_route_rules() {
        let (_d, e) = temp_engine();
        write_instance(&e, "nf", json!({ "loader": "neoforge", "loader_version": "21.1.1" }));
        let err = e.train_instance("nf").await.unwrap_err();
        assert_eq!(err.envelope().code, "AOT_UNAVAILABLE");
        write_instance(&e, "busy", json!({}));
        e.active_instances().insert("busy");
        let r = e.train_instance("busy").await.unwrap();
        assert_eq!(r.deferred, Some(true));
        assert_eq!(r.key, crate::aot::cache_key("1.21.11", None, None));
        let err = e.apply_version_change("busy", VersionTarget::default()).await.unwrap_err();
        assert_eq!(err.envelope().code, "ALREADY_RUNNING");
        assert!(e.active_instances().contains("busy"), "guard never clears someone else's flag");
    }

    #[test]
    fn ring_buffer_split_cap_and_cursor() {
        let st = GameLaunchState::default();
        st.create_buffer("k", "i", "v", None);
        let out = st.append_output("k", "a\r\nb\rc\n\n");
        assert_eq!(out.iter().map(|(s, l)| (*s, l.as_str())).collect::<Vec<_>>(), vec![(1, "a"), (2, "b"), (3, "c")]);
        for i in 0..RING_CAPACITY + 10 {
            st.append_system("k", &format!("l{i}"));
        }
        let v = st.buffer_view("k", 0).unwrap();
        assert_eq!(v.lines.len(), RING_CAPACITY);
        assert_eq!(v.cursor, (RING_CAPACITY + 13) as u64);
        let v2 = st.buffer_view("k", v.cursor).unwrap();
        assert!(v2.lines.is_empty());
        assert_eq!(v2.cursor, v.cursor);
        assert!(st.buffer_view("missing", 0).is_none());
        assert!(st.append_output("missing", "x").is_empty());
    }

    #[test]
    fn buffers_are_pruned_by_ttl_and_lru_but_never_running() {
        let st = GameLaunchState::default();
        st.create_buffer("running", "i", "v", None);
        for i in 0..12 {
            let k = format!("done{i}");
            st.create_buffer(&k, "i", "v", None);
            st.with_buffer(&k, |b| {
                b.finish();
                b.last_read = i;
            });
        }
        st.create_buffer("old", "i", "v", None);
        st.with_buffer("old", |b| {
            b.running = false;
            b.ended_at = Some(crate::platform::now_ms() - BUFFER_TTL_MS - 1);
            b.last_read = 1_000_000_000_000_000;
        });
        st.create_buffer("new", "i", "v", None);
        let b = lock(&st.buffers);
        assert!(b.len() <= BUFFER_LRU_CAP + 1);
        assert!(b.contains_key("running") && b.contains_key("new"));
        assert!(!b.contains_key("old"), "TTL-expired buffer dropped");
        assert!(!b.contains_key("done0"), "least recently read evicted first");
    }

    #[test]
    fn make_key_shape() {
        let k = make_key();
        assert!(k.starts_with('l') && k.len() > 8);
        assert_ne!(k, make_key());
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }

    // --- presence.test.mjs / launch.test.mjs (presenceForInstance) ---------

    fn pi(name: &str, version: &str, modpack: Option<&str>) -> InstanceMeta {
        InstanceMeta {
            name: name.into(),
            version: version.into(),
            modpack: modpack.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn presence_for_instance_rules() {
        let p = presence_for_instance(Some(&pi("Keke Cliente 1.0", "1.21.1", Some("Keke Cliente"))));
        assert_eq!((p.state.as_str(), p.image.as_deref()), ("Jugando Keke Cliente", Some("espectral_logo")));
        assert_eq!(
            presence_for_instance(Some(&pi("mi 24h modpack", "1.21.1", Some("Pack 24h")))).state,
            "Jugando Pack 24h"
        );
        let s = presence_for_instance(Some(&pi("uhc2", "1.21.11", None)));
        assert_eq!((s.state.as_str(), s.image.as_deref()), ("Jugando en main.uhce.top", Some("espectral_logo")));
        assert_eq!(presence_for_instance(Some(&pi("my 24h", "1.21.11", None))).image.as_deref(), Some("24h"));
        let long = "x".repeat(300);
        let p = presence_for_instance(Some(&pi("long", "1.21.1", Some(&long))));
        assert_eq!(p.state, format!("Jugando {long}").chars().take(128).collect::<String>());
        for v in ["26.2", "25.1", "21.0", "1.21.11", "1.21"] {
            assert_eq!(presence_for_instance(Some(&pi("modern", v, None))).state, "En el menú principal", "{v}");
        }
        for v in ["24w11a", "25w11a"] {
            assert_eq!(presence_for_instance(Some(&pi("snap", v, None))).state, "Jugando singleplayer");
        }
        assert_eq!(presence_for_instance(Some(&pi("uhc2", "26.2", None))).state, "Jugando en main.uhce.top");
        assert_eq!(presence_for_instance(None).state, "En el launcher");
        assert_eq!(presence_for_instance(Some(&InstanceMeta::default())).state, "En el launcher");
    }

    #[test]
    fn presence_events_from_log_lines() {
        assert_eq!(
            presence_event_from_log("[22:12:11] [Render thread/INFO]: Connecting to main.uhce.top, 25565"),
            Some(PresenceLogEvent::Server("main.uhce.top".into()))
        );
        assert_eq!(
            presence_event_from_log(
                "[19:03:40] [Server thread/INFO]: Starting integrated minecraft server version 26.2"
            ),
            Some(PresenceLogEvent::Singleplayer)
        );
        assert_eq!(
            presence_event_from_log(&format!("[19:20:01] [Render thread/INFO]: {PRESENCE_MENU_MARKER}")),
            Some(PresenceLogEvent::Menu)
        );
        assert_eq!(
            presence_event_from_log("[19:20:01] [Server Connector #1/ERROR]: Couldn't connect to server"),
            Some(PresenceLogEvent::Menu)
        );
        assert_eq!(presence_event_from_log("[CHAT] <Yoshu> Connecting to main ya voy"), None);
        assert_eq!(presence_event_from_log("[Render thread/INFO]: Loaded 1234 recipes"), None);
    }

    #[test]
    fn presence_for_server_lines() {
        let main = presence_for_server("main.uhce.top");
        assert_eq!(main.details.as_deref(), Some("Espectral UHC · MAIN"));
        assert_eq!(main.state, "main.uhce.top");
        for h in ["uhc2.espectral.es", "2.27.233.26", "MAIN.uhce.top", "main.uhce.top."] {
            assert_eq!(presence_for_server(h).details.as_deref(), Some("Espectral UHC · MAIN"), "{h}");
        }
        assert_eq!(presence_for_server("mini.uhce.top").details.as_deref(), Some("Espectral UHC · Mini"));
        let m = presence_for_server("24h.uhce.top");
        assert_eq!((m.details.as_deref(), m.image.as_deref()), (Some("Espectral Meetups 24/7"), Some("24h")));
        assert_eq!(presence_for_server("247.espectral.es").details.as_deref(), Some("Espectral Meetups 24/7"));
        let other = presence_for_server("play.hypixel.net");
        assert_eq!(other.state, "Jugando servidor multijugador");
        assert_eq!(other.details, None);
        assert!(!format!("{other:?}").contains("hypixel"));
    }

    // --- end-to-end with a stand-in JVM (unix) -----------------------------

    #[cfg(unix)]
    fn install_fake(e: &Engine, d: &std::path::Path, name: &str, stay: bool) {
        let java = test_util::fake_java(d, stay);
        let mut resolved = fake_resolved(e, name, &java.to_string_lossy());
        resolved.bridge_token = Some("c".repeat(64));
        e.set_launch_test_hooks(LaunchTestHooks {
            resolve_launch: Some(Arc::new(move |_, _| {
                let r = resolved.clone();
                async move { Ok(r) }.boxed()
            })),
            quiet_presence: true,
            ..Default::default()
        });
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_spawn_streams_logs_marker_and_exit() {
        let (d, e) = temp_engine();
        write_instance(&e, "e2e", json!({}));
        install_fake(&e, d.path(), "e2e", false);
        let mut rx = e.subscribe();
        let LaunchOutcome::Started(reply) = e.launch_instance("e2e", LaunchRequest::default()).await.unwrap() else {
            panic!()
        };
        let EngineEvent::LaunchReady(ready) = next_named(&mut rx, "launch-ready").await else { unreachable!() };
        assert_eq!((ready.key.as_str(), ready.instance.as_str()), (reply.key.as_str(), "e2e"));
        let s = crate::game_sessions::read_sessions(&e);
        assert!(s.get("e2e").is_some_and(|s| s.menu_at.is_some()));
        let EngineEvent::LaunchExit(x) = next_named(&mut rx, "launch-exit").await else { unreachable!() };
        assert_eq!(x.code, Some(3));
        assert_eq!(x.marker, Some(true));
        assert!(x.menu_ms.is_some() && x.played_ms.is_some() && x.spawn_ms.is_some() && x.boot_ms.is_some());
        assert!(x.phases.contains_key("loading_mc"), "{:?}", x.phases);
        let log = e.launch_log(&reply.key, 0).await.unwrap();
        assert!(!log.running);
        assert!(log.lines.iter().any(|l| l.contains("Loading Minecraft")));
        assert!(log.lines.iter().any(|l| l.starts_with("[espectral] Sound engine started (")));
        assert_eq!(log.lines.last().unwrap(), "[espectral] game exited (code 3)");
        assert!(!e.active_instances().contains("e2e"));
        assert!(crate::game_sessions::read_sessions(&e).is_empty(), "session removed on exit");
        let stats = e.launch_stats(None).await.unwrap().launches;
        assert_eq!(stats[0].key, reply.key);
        assert!(stats[0].phases.as_ref().is_some_and(|p| p.contains_key("loading_mc")));
        // argv: the game ran in its gameDir with the bridge token.
        let args = std::fs::read_to_string(e.paths().instance_dir("e2e").join("args.txt")).unwrap();
        assert!(args.contains(&format!("-Despectral.bridge.token={}", "c".repeat(64))));
        assert!(!args.contains("fake-java"), "argv[0] is dropped");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_running_game_and_records_session() {
        let (d, e) = temp_engine();
        write_instance(&e, "stopme", json!({}));
        install_fake(&e, d.path(), "stopme", true);
        let mut rx = e.subscribe();
        e.launch_instance("stopme", LaunchRequest::default()).await.unwrap();
        next_named(&mut rx, "launch-ready").await;
        let s = crate::game_sessions::read_sessions(&e)["stopme"].clone();
        assert!(s.pid > 0);
        assert_eq!(s.bridge_token.as_deref(), Some("c".repeat(64).as_str()));
        assert_eq!(s.game_dir.as_deref(), Some(e.paths().instance_dir("stopme").to_string_lossy().as_ref()));
        let rows = e.list_launches().await.unwrap().launches;
        assert!(rows[0].running);
        let stop = e.stop_instance("stopme").await.unwrap();
        assert_eq!(stop, StopReply { ok: true, instance: "stopme".into(), cancelled: None, already_stopped: None });
        let EngineEvent::LaunchExit(x) = next_named(&mut rx, "launch-exit").await else { unreachable!() };
        assert_eq!(x.signal.as_deref(), Some("SIGKILL"));
        assert!(!e.active_instances().contains("stopme"));
        assert!(!e.list_launches().await.unwrap().launches[0].running);
        assert!(crate::game_sessions::read_sessions(&e).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn reopened_engine_adopts_running_game_then_stop_kills_it() {
        let (_d, e) = temp_engine();
        e.set_launch_test_hooks(LaunchTestHooks { quiet_presence: true, ..Default::default() });
        write_instance(&e, "adopted", json!({ "version": "26.3", "loader": "fabric" }));
        let dir = e.paths().instance_dir("adopted");
        let mut game =
            std::process::Command::new("sh").args(["-c", "sleep 30; :", "--gameDir"]).arg(&dir).spawn().unwrap();
        let pid = game.id();
        // Reap in the background so a killed game does not linger as a zombie.
        let reaper = std::thread::spawn(move || {
            let _ = game.wait();
        });
        let now = crate::platform::now_ms();
        crate::game_sessions::record_session(
            &e,
            "adopted",
            json!({ "key": "lprevious", "pid": pid, "game_dir": dir.to_string_lossy(), "version": "26.3",
                    "started_at": now - 60_000, "menu_at": now - 50_000 }),
        );
        assert_eq!(e.adopt_running_games().await, vec!["adopted".to_string()]);
        let rows = e.list_launches().await.unwrap().launches;
        let row = rows.iter().find(|r| r.instance == "adopted").unwrap();
        assert!(row.running);
        assert_eq!(row.key, "lprevious");
        let err = e.launch_instance("adopted", LaunchRequest::default()).await.unwrap_err();
        assert_eq!((err.status(), err.envelope().code.as_str()), (409, "ALREADY_RUNNING"));
        let mut rx = e.subscribe();
        e.stop_instance("adopted").await.unwrap();
        reaper.join().unwrap();
        assert!(!crate::platform::is_pid_alive(pid), "Stop killed the adopted process");
        let EngineEvent::LaunchExit(x) = next_named(&mut rx, "launch-exit").await else { unreachable!() };
        assert_eq!(x.marker, Some(true));
        assert_eq!(x.menu_ms, Some(10_000));
        let rows = e.list_launches().await.unwrap().launches;
        assert!(!rows.iter().find(|r| r.instance == "adopted").unwrap().running);
        assert!(!crate::game_sessions::read_sessions(&e).contains_key("adopted"));
        // A second adoption pass finds nothing.
        assert!(e.adopt_running_games().await.is_empty());
    }
}
