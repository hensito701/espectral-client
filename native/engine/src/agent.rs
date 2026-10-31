//! Agent access (port of src/engine/agent.mjs, upstream c22767f): the
//! `agent.enabled` / `agent.allowed_servers` config, the rotatable
//! `<data_dir>/agent-token`, Bearer authentication for `/api/agent/*`, the
//! launch-option validation and the per-launch command registry the in-game
//! mod relays through.
//!
//! Everything is inert unless `agent.enabled` is set: the agent routes answer
//! 403 `AGENT_DISABLED` and launches are byte-identical to plain ones.
//!
//! Sections, in the JS order: launch options (`normalize_*`, `game_arg_fragment`),
//! the launch registry + command queue ([`AgentState`]), and the file-result
//! rewriting ([`rewrite_file_results`]). HTTP wiring lives in [`crate::loopback`].

use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use rand::RngCore;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::oneshot;

use crate::bridge::BridgeLaunch;
use crate::config::ConfigStore;
use crate::error::{EngineError, EngineResult};

// ---------------------------------------------------------------------------
// Policy constants
// ---------------------------------------------------------------------------

/// Public Espectral hosts where agent *control* must never work, whatever the
/// allowlist says. Domain suffixes — the mod mirrors this list and also blocks
/// the IPs they resolve to.
pub const BLOCKED_HOSTS: [&str; 2] = ["uhce.top", "espectral.es"];

/// Token file name under the data dir.
pub const TOKEN_FILE: &str = "agent-token";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// `config.agent`, normalized (JS `agentConfig()`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    pub enabled: bool,
    pub allowed_servers: Vec<String>,
}

/// Current config view; defaults on a missing / malformed `agent` object.
pub fn agent_config(config: &ConfigStore) -> AgentConfig {
    config.with(|doc| {
        let raw = doc.get("agent").and_then(Value::as_object);
        AgentConfig {
            enabled: raw.and_then(|a| a.get("enabled")) == Some(&Value::Bool(true)),
            allowed_servers: normalize_allowed_servers(raw.and_then(|a| a.get("allowed_servers"))),
        }
    })
}

static ALLOWED_SERVER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\[[0-9a-f:.]+\]|[a-z0-9._-]+)(:\d{1,5})?$").expect("valid regex"));

/// `host[:port]` entries, lowercased, deduped, order preserved. Invalid
/// entries are dropped rather than failing the whole save (a typo in one row
/// must not discard the rest of the allowlist). Non-arrays -> `[]`.
pub fn normalize_allowed_servers(list: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = list else { return Vec::new() };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let Value::String(s) = item else { continue };
        let lower = s.trim().to_lowercase();
        let v = lower.strip_suffix('.').unwrap_or(&lower);
        if v.is_empty() || v.len() > 255 || !ALLOWED_SERVER_RE.is_match(v) {
            continue;
        }
        if seen.insert(v.to_string()) {
            out.push(v.to_string());
        }
    }
    out
}

/// UI patch for the agent config (`PUT /api/agent-access`); absent = keep.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentAccessPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_servers: Option<Vec<String>>,
}

/// Merge a patch into `config.agent` and persist it (JS `saveAgentConfig`).
/// Unknown keys inside `config.agent` are preserved.
pub fn save_agent_config(config: &ConfigStore, patch: &AgentAccessPatch) -> AgentConfig {
    config.update(|doc| {
        let mut agent: Map<String, Value> = Map::new();
        agent.insert("enabled".into(), Value::Bool(false));
        agent.insert("allowed_servers".into(), Value::Array(Vec::new()));
        if let Some(Value::Object(cur)) = doc.get("agent") {
            for (k, v) in cur {
                agent.insert(k.clone(), v.clone());
            }
        }
        if let Some(enabled) = patch.enabled {
            agent.insert("enabled".into(), Value::Bool(enabled));
        }
        if let Some(list) = &patch.allowed_servers {
            let raw = Value::Array(list.iter().cloned().map(Value::String).collect());
            let norm = normalize_allowed_servers(Some(&raw));
            agent.insert("allowed_servers".into(), Value::Array(norm.into_iter().map(Value::String).collect()));
        }
        doc.insert("agent".into(), Value::Object(agent));
    });
    agent_config(config)
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

/// `<data_dir>/agent-token`.
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

fn is_token(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The stored token; `None` when missing, unreadable or malformed. Never
/// logged, never returned to the UI.
pub fn read_token(data_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(token_path(data_dir)).ok()?;
    let t = raw.trim();
    is_token(t).then(|| t.to_string())
}

/// Write a fresh token (`64 hex + "\n"`, 0600 on Unix). Atomic replace: the
/// MCP shim re-reads the file on every call, so a truncate-in-place rotate
/// could be observed as a blank token. Rotating is how the user revokes access.
pub fn write_token(data_dir: &Path) -> EngineResult<String> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    let file = token_path(data_dir);
    std::fs::create_dir_all(data_dir)?;
    let mut suffix = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut suffix);
    let tmp = data_dir.join(format!("{TOKEN_FILE}.{}.{}.tmp", std::process::id(), hex::encode(suffix)));
    let result = (|| -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(token.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // `mode` only applies on creation and may be masked by the umask;
            // a no-op failure on FAT/exFAT is fine (the token still works).
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &file)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(token)
}

/// The token, creating it on first use (enabling agent access creates it).
pub fn ensure_token(data_dir: &Path) -> EngineResult<String> {
    match read_token(data_dir) {
        Some(t) => Ok(t),
        None => write_token(data_dir),
    }
}

/// Constant-time compare of a presented token against the stored one.
pub fn token_matches(data_dir: &Path, presented: &str) -> bool {
    if !is_token(presented) {
        return false;
    }
    let Some(stored) = read_token(data_dir) else { return false };
    let (a, b) = (stored.as_bytes(), presented.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 403 `AGENT_DISABLED` (the agent API and the mod relay answer it while off).
pub fn disabled_error() -> EngineError {
    EngineError::api(403, "AGENT_DISABLED", "agent access is disabled (enable it in Settings → Agent access)")
}

/// Bearer auth for every `/api/agent/*` route (JS `authenticate`).
/// `AGENT_DISABLED` comes before auth so a disabled engine does not confirm
/// a token's validity. `authorization` is the raw header value, if any.
pub fn authenticate(config: &ConfigStore, data_dir: &Path, authorization: Option<&str>) -> EngineResult<()> {
    if !agent_config(config).enabled {
        return Err(disabled_error());
    }
    let presented = authorization.and_then(|h| {
        let h = h.trim_start();
        let (scheme, rest) = h.split_at(h.find(char::is_whitespace)?);
        scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
    });
    match presented {
        Some(t) if !t.is_empty() && token_matches(data_dir, t) => Ok(()),
        _ => Err(EngineError::api(401, "AGENT_AUTH", "missing or invalid Authorization: Bearer <agent-token>")),
    }
}

// ---------------------------------------------------------------------------
// Launch options (agent / join / window)
// ---------------------------------------------------------------------------
// Validation lives here (not in the launch route) so the UI route and the
// agent route share one implementation and one set of error codes.

pub const MIN_WINDOW: u32 = 64;
pub const MAX_WINDOW_W: u32 = 7680;
pub const MAX_WINDOW_H: u32 = 4320;

/// Save-folder names: a single path component under `saves/`.
static WORLD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^[^/\\:*?"<>|\x00-\x1F]{1,255}$"#).expect("valid regex"));
/// `host[:port]` with optional `[IPv6]`. Deliberately strict: the value reaches
/// a JVM argument, and a stray space/quote would split the command line.
static SERVER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9]([A-Za-z0-9.-]{0,253}))(:\d{1,5})?$").expect("valid regex")
});

/// Validated `join` option: exactly one of `world` / `server`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinOption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

/// Validated `window` option. `hidden` is agent-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowOption {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub hidden: bool,
}

/// The whole validated launch-option bundle (`normalizeLaunchOptions`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchOptions {
    pub agent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<JoinOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowOption>,
}

/// JS `Number(v)` for the shapes a JSON body can carry (`undefined` → NaN).
pub fn js_number(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Some(Value::Bool(b)) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Some(Value::Null) => 0.0,
        _ => f64::NAN,
    }
}

/// `Number.isInteger(v) && v >= min && v <= max`.
fn is_integer_in(v: f64, min: u32, max: u32) -> bool {
    v.is_finite() && v.fract() == 0.0 && v >= f64::from(min) && v <= f64::from(max)
}

/// Normalize `join` to `{world}` or `{server}`. Both/neither → 400. `None`
/// when absent (the common case).
pub fn normalize_join(join: Option<&Value>) -> EngineResult<Option<JoinOption>> {
    let Some(join) = join.filter(|v| !v.is_null()) else { return Ok(None) };
    let Value::Object(obj) = join else {
        return Err(EngineError::bad_request("BAD_JOIN", "join must be an object: {world} or {server}"));
    };
    let has_world = obj.get("world").is_some_and(|v| !v.is_null());
    let has_server = obj.get("server").is_some_and(|v| !v.is_null());
    if has_world && has_server {
        return Err(EngineError::bad_request("BAD_JOIN", "join takes either {world} or {server}, not both"));
    }
    if has_world {
        let Value::String(world) = &obj["world"] else {
            return Err(EngineError::bad_request(
                "BAD_JOIN",
                r#"join.world must be an existing save folder name (no slashes or "..")"#,
            ));
        };
        if !WORLD_RE.is_match(world) || world == "." || world == ".." {
            return Err(EngineError::bad_request(
                "BAD_JOIN",
                r#"join.world must be an existing save folder name (no slashes or "..")"#,
            ));
        }
        // A value starting with '-' would be read as the NEXT OPTION by the
        // game's arg parser instead of as the --quickPlaySingleplayer value.
        if world.contains("..") || world.starts_with('-') {
            return Err(EngineError::bad_request("BAD_JOIN", r#"join.world may not contain ".." or start with "-""#));
        }
        return Ok(Some(JoinOption { world: Some(world.clone()), server: None }));
    }
    if has_server {
        let server = match &obj["server"] {
            Value::String(s) => s.trim().to_string(),
            _ => String::new(),
        };
        if !SERVER_RE.is_match(&server) {
            return Err(EngineError::bad_request(
                "BAD_JOIN",
                "join.server must be host[:port] (hostname, IPv4 or [IPv6])",
            ));
        }
        if server.contains("..") {
            return Err(EngineError::bad_request("BAD_JOIN", "join.server is not a valid host"));
        }
        return Ok(Some(JoinOption { world: None, server: Some(server) }));
    }
    Err(EngineError::bad_request("BAD_JOIN", "join must carry {world} or {server}"))
}

/// Normalize `window` to `{width, height, hidden}`. `hidden` is agent-only —
/// a plain UI launch with hidden:true is a 400 rather than a silent no-op.
pub fn normalize_window(win: Option<&Value>, agent: bool) -> EngineResult<Option<WindowOption>> {
    let Some(win) = win.filter(|v| !v.is_null()) else { return Ok(None) };
    let Value::Object(obj) = win else {
        return Err(EngineError::bad_request("BAD_WINDOW", "window must be an object: {width, height, hidden?}"));
    };
    let width = js_number(obj.get("width"));
    let height = js_number(obj.get("height"));
    if !is_integer_in(width, MIN_WINDOW, MAX_WINDOW_W) {
        return Err(EngineError::bad_request(
            "BAD_WINDOW",
            format!("window.width must be an integer {MIN_WINDOW}…{MAX_WINDOW_W}"),
        ));
    }
    if !is_integer_in(height, MIN_WINDOW, MAX_WINDOW_H) {
        return Err(EngineError::bad_request(
            "BAD_WINDOW",
            format!("window.height must be an integer {MIN_WINDOW}…{MAX_WINDOW_H}"),
        ));
    }
    let hidden = obj.get("hidden") == Some(&Value::Bool(true));
    if hidden && !agent {
        return Err(EngineError::bad_request("BAD_WINDOW", "window.hidden is only available on agent launches"));
    }
    Ok(Some(WindowOption { width: width as u32, height: height as u32, hidden }))
}

/// `agent` must be a boolean when present.
pub fn normalize_agent(value: Option<&Value>) -> EngineResult<bool> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(EngineError::bad_request("BAD_AGENT", "agent must be true or false")),
    }
}

/// Validate the whole launch-option bundle at once. `agent` in the body is
/// honored for agent launches too (the agent route forces it).
pub fn normalize_launch_options(body: &Value, force_agent: bool) -> EngineResult<LaunchOptions> {
    let b = body.as_object();
    let agent = force_agent || normalize_agent(b.and_then(|o| o.get("agent")))?;
    let join = normalize_join(b.and_then(|o| o.get("join")))?;
    let window = normalize_window(b.and_then(|o| o.get("window")), agent)?;
    Ok(LaunchOptions { agent, join, window })
}

/// Game-arg fragment for the validated options (order asserted by tests):
/// `--quickPlaySingleplayer <world>` | `--quickPlayMultiplayer <server>`,
/// then `--width W --height H`.
pub fn game_arg_fragment(options: &LaunchOptions) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(join) = &options.join {
        if let Some(world) = &join.world {
            out.push("--quickPlaySingleplayer".into());
            out.push(world.clone());
        } else if let Some(server) = &join.server {
            out.push("--quickPlayMultiplayer".into());
            out.push(server.clone());
        }
    }
    if let Some(w) = &options.window {
        out.push("--width".into());
        out.push(w.width.to_string());
        out.push("--height".into());
        out.push(w.height.to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Launch registry + command queue
// ---------------------------------------------------------------------------

/// Long-poll budget for the mod's command channel (contract: ≤25 s).
pub const POLL_TIMEOUT_MS: u64 = 25_000;
/// Default `call` timeout when the caller does not pass one (contract: 30 s).
pub const CALL_TIMEOUT_S: u64 = 30;
pub const MAX_CALL_TIMEOUT_S: u64 = 300;
/// Commands queued for a game that stopped polling — bounded so a dead launch
/// cannot grow a queue forever.
pub const MAX_QUEUE: usize = 100;

/// One queued mod command — the wire shape the mod correlates by `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCommand {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    pub timeout_ms: u64,
}

/// The mod's latest state report (`POST /api/bridge/agent/state`), as the mod
/// sent it, plus `updated_at` when read back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentReport {
    pub phase: Option<String>,
    pub screen: Option<String>,
    pub server: Option<String>,
    pub world: Option<String>,
    pub context: Option<String>,
    pub paused: bool,
    pub recording: bool,
    #[serde(default = "no_updated_at")]
    pub updated_at: Option<i64>,
}

fn no_updated_at() -> Option<i64> {
    None
}

impl AgentReport {
    /// Normalize what the mod posted (`setState`): non-strings → null,
    /// booleans only when strictly `true`.
    fn from_wire(state: &Value) -> AgentReport {
        let s = |k: &str| state.get(k).and_then(Value::as_str).map(str::to_string);
        AgentReport {
            phase: s("phase"),
            screen: s("screen"),
            server: s("server"),
            world: s("world"),
            context: s("context"),
            paused: state.get("paused") == Some(&Value::Bool(true)),
            recording: state.get("recording") == Some(&Value::Bool(true)),
            updated_at: None,
        }
    }
}

/// A parked mod poll. The batch is parked in `slot` until the waiting handler
/// takes it; a handler dropped before that (client hang-up) puts it back.
#[derive(Debug, Default)]
struct PollWaiter {
    slot: Mutex<Option<Vec<AgentCommand>>>,
    notify: tokio::sync::Notify,
}

impl PollWaiter {
    /// Hand the batch over. `None` when a batch is already parked (never
    /// replayed to a second waiter), returning the batch untouched.
    fn deliver(&self, batch: Vec<AgentCommand>) -> Option<Vec<AgentCommand>> {
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_some() {
            return Some(batch);
        }
        *slot = Some(batch);
        drop(slot);
        self.notify.notify_one();
        None
    }

    fn take(&self) -> Vec<AgentCommand> {
        self.slot.lock().unwrap_or_else(|p| p.into_inner()).take().unwrap_or_default()
    }
}

struct Inflight {
    tool: String,
    tx: oneshot::Sender<EngineResult<Value>>,
}

#[derive(Default)]
struct LaunchInner {
    queue: std::collections::VecDeque<AgentCommand>,
    /// Parked mod polls, oldest first (exactly one gets each batch).
    waiters: Vec<Arc<PollWaiter>>,
    inflight: HashMap<String, Inflight>,
    state: Option<AgentReport>,
    state_at: i64,
}

/// One agent-capable launch (JS `launches.get(key)` entry).
pub struct AgentLaunch {
    key: String,
    instance: String,
    agent: bool,
    running: std::sync::atomic::AtomicBool,
    game_dir: Mutex<Option<PathBuf>>,
    inner: Mutex<LaunchInner>,
}

impl std::fmt::Debug for AgentLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = lock_launch(self);
        f.debug_struct("AgentLaunch")
            .field("key", &self.key)
            .field("instance", &self.instance)
            .field("agent", &self.agent)
            .field("running", &self.is_running())
            .field("game_dir", &self.game_dir())
            .field("queued", &inner.queue.len())
            .field("waiters", &inner.waiters.len())
            .field("inflight", &inner.inflight.len())
            .finish()
    }
}

impl AgentLaunch {
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn is_agent(&self) -> bool {
        self.agent
    }
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    pub fn game_dir(&self) -> Option<PathBuf> {
        self.game_dir.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[derive(Default)]
struct AgentInner {
    launches: HashMap<String, Arc<AgentLaunch>>,
    seq: u64,
}

/// Per-engine launch registry + command queue (JS module state).
#[derive(Default)]
pub struct AgentState {
    inner: Mutex<AgentInner>,
}

impl std::fmt::Debug for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = lock(self);
        f.debug_struct("AgentState").field("launches", &inner.launches.len()).field("seq", &inner.seq).finish()
    }
}

fn lock(st: &AgentState) -> MutexGuard<'_, AgentInner> {
    st.inner.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_launch(entry: &AgentLaunch) -> MutexGuard<'_, LaunchInner> {
    entry.inner.lock().unwrap_or_else(|p| p.into_inner())
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Fresh command id (unique per process; the mod correlates by it).
fn next_command_id(st: &AgentState) -> String {
    let seq = {
        let mut inner = lock(st);
        inner.seq += 1;
        inner.seq
    };
    format!("c{}{}", base36(crate::platform::now_ms().max(0) as u64), base36(seq))
}

/// Register an agent-capable launch. Called twice per launch by design: once
/// when the key is minted (so a call that races the spawn still finds the key)
/// and again on spawn with the resolved gameDir (per-account profile dirs are
/// only known then). The second call MERGES into the existing entry — replacing
/// it would orphan everything queued in between.
pub fn register_launch(st: &AgentState, key: &str, instance: &str, game_dir: Option<PathBuf>) -> Arc<AgentLaunch> {
    let mut inner = lock(st);
    if let Some(existing) = inner.launches.get(key) {
        // `instance` is fixed at registration; a spawn re-register only fills
        // in the resolved gameDir.
        if let Some(dir) = game_dir {
            *existing.game_dir.lock().unwrap_or_else(|p| p.into_inner()) = Some(dir);
        }
        existing.running.store(true, Ordering::SeqCst);
        return existing.clone();
    }
    let entry = Arc::new(AgentLaunch {
        key: key.to_string(),
        instance: instance.to_string(),
        agent: true,
        running: std::sync::atomic::AtomicBool::new(true),
        game_dir: Mutex::new(game_dir),
        inner: Mutex::new(LaunchInner::default()),
    });
    inner.launches.insert(key.to_string(), entry.clone());
    entry
}

/// Mark a launch gone: fail every pending call (GAME_GONE by default), release
/// parked polls with an empty reply (so the mod's thread unblocks instead of
/// polling a dead key), and drop the queue. Idempotent.
pub fn release_launch(st: &AgentState, key: &str) -> bool {
    release_launch_with(st, key, "GAME_GONE", "game exited")
}

pub fn release_launch_with(st: &AgentState, key: &str, code: &str, message: &str) -> bool {
    let entry = lock(st).launches.remove(key);
    let Some(entry) = entry else { return false };
    entry.running.store(false, Ordering::SeqCst);
    let mut inner = lock_launch(&entry);
    for (_, waiter) in inner.inflight.drain() {
        let _ = waiter.tx.send(Err(EngineError::api(502, code, message)));
    }
    inner.queue.clear();
    let waiters = std::mem::take(&mut inner.waiters);
    for waiter in waiters {
        // An empty reply unblocks the mod's relay thread.
        let _ = waiter.deliver(Vec::new());
    }
    true
}

/// True when `key` is a live agent launch.
pub fn is_agent_launch(st: &AgentState, key: &str) -> bool {
    get_launch(st, key).is_some_and(|e| e.is_agent() && e.is_running())
}

/// The agent registry entry for a key (or `None`).
pub fn get_launch(st: &AgentState, key: &str) -> Option<Arc<AgentLaunch>> {
    lock(st).launches.get(key).cloned()
}

/// All live agent launches (`GET /api/agent/launches`).
pub fn all_launches(st: &AgentState) -> Vec<Arc<AgentLaunch>> {
    let inner = lock(st);
    let mut out: Vec<_> = inner.launches.values().cloned().collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// Resolve a bridge-authenticated launch to its agent registry entry.
/// Non-agent launches → 409 NOT_AGENT_LAUNCH (the mod token is valid, but that
/// game was never started in agent mode).
pub fn launch_for_bridge(
    st: &AgentState,
    bridge_entry: Option<&BridgeLaunch>,
    key: Option<&str>,
) -> EngineResult<Arc<AgentLaunch>> {
    let Some(bridge_entry) = bridge_entry else {
        return Err(EngineError::api(403, "BRIDGE_AUTH", "missing or invalid x-espectral-bridge token"));
    };
    let key = key.filter(|k| !k.is_empty());
    let entry = match key {
        Some(k) => get_launch(st, k),
        None => find_by_bridge(st, bridge_entry),
    };
    let Some(entry) = entry else {
        // Distinguish "bridge token valid but this launch is not an agent
        // launch" from "unknown launch": the former is the contract's 409.
        return Err(EngineError::api(409, "NOT_AGENT_LAUNCH", "this launch was not started with agent: true"));
    };
    // A key names a launch, and the bridge token names a game: they must agree.
    // Without this check a mod running under ANY launch (a plain player game
    // included, it holds a valid bridge token too) could poll another agent
    // launch's commands or post forged results for it — keys are not secret
    // (they come back from /api/launches).
    if key.is_some() && bridge_entry.instance != entry.instance {
        return Err(EngineError::api(403, "BRIDGE_AUTH", "this bridge token does not own that launch key"));
    }
    if !entry.is_agent() {
        return Err(EngineError::api(409, "NOT_AGENT_LAUNCH", "this launch was not started with agent: true"));
    }
    if !entry.is_running() {
        return Err(EngineError::api(409, "NOT_AGENT_LAUNCH", "this launch is no longer running"));
    }
    Ok(entry)
}

/// The mod does not know its launch key (it only holds the bridge token), so
/// the mapping is by instance: the launch route registers both.
fn find_by_bridge(st: &AgentState, bridge_entry: &BridgeLaunch) -> Option<Arc<AgentLaunch>> {
    lock(st).launches.values().find(|e| e.instance == bridge_entry.instance).cloned()
}

/// Hand queued commands to the parked polls. With NO waiter the queue must
/// stay put: draining it here would silently discard every command issued
/// between two polls. Exactly ONE waiter gets the batch (two parked polls would
/// otherwise execute the same commands twice).
fn release_pollers_locked(inner: &mut LaunchInner) {
    while !inner.queue.is_empty() && !inner.waiters.is_empty() {
        let waiter = inner.waiters.remove(0);
        let batch: Vec<AgentCommand> = inner.queue.drain(..).collect();
        if let Some(back) = waiter.deliver(batch) {
            // A waiter that already holds a batch: keep the commands and try
            // the next one. (Never replayed to a second live poll otherwise.)
            for c in back.into_iter().rev() {
                inner.queue.push_front(c);
            }
        } else {
            return;
        }
    }
}

/// A call awaiting the mod's result. Dropping it (or timing out) drops the
/// command from the queue so the mod cannot execute it late.
pub struct PendingCall {
    command: AgentCommand,
    entry: Arc<AgentLaunch>,
    rx: Option<oneshot::Receiver<EngineResult<Value>>>,
    deadline: tokio::time::Instant,
    finished: bool,
}

impl PendingCall {
    pub fn command(&self) -> &AgentCommand {
        &self.command
    }

    /// Wait for the mod's result (`AGENT_TIMEOUT` / `GAME_GONE` on failure).
    pub async fn wait(mut self) -> EngineResult<Value> {
        let Some(mut rx) = self.rx.take() else { return Err(game_gone()) };
        let res = tokio::time::timeout_at(self.deadline, &mut rx).await;
        self.finished = true;
        match res {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => Err(game_gone()),
            Err(_) => {
                abandon(&self.entry, &self.command.id);
                // The caller has given up, so the command must not sit in the
                // queue: the mod would pick it up on its next poll (a
                // `record_start` minutes late) and post a result nobody waits for.
                let secs = (self.command.timeout_ms as f64 / 1000.0).round() as u64;
                Err(EngineError::api(
                    504,
                    "AGENT_TIMEOUT",
                    format!("no result for '{}' within {secs} s", self.command.tool),
                ))
            }
        }
    }
}

impl Drop for PendingCall {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        abandon(&self.entry, &self.command.id);
    }
}

fn game_gone() -> EngineError {
    EngineError::api(502, "GAME_GONE", "the game is no longer running")
}

/// Drop an in-flight call and any still-queued copy of its command.
fn abandon(entry: &AgentLaunch, id: &str) {
    let mut inner = lock_launch(entry);
    inner.inflight.remove(id);
    if let Some(pos) = inner.queue.iter().position(|c| c.id == id) {
        inner.queue.remove(pos);
    }
}

/// Enqueue a command for a launch. The returned [`PendingCall`] settles when
/// the mod posts the result, or fails with AGENT_TIMEOUT / GAME_GONE.
pub fn call_tool(
    st: &AgentState,
    key: &str,
    tool: &str,
    args: Option<&Value>,
    timeout_ms: u64,
) -> EngineResult<PendingCall> {
    let entry =
        get_launch(st, key).ok_or_else(|| EngineError::not_found("NO_LAUNCH", format!("unknown launch key: {key}")))?;
    if !entry.is_agent() {
        return Err(EngineError::api(409, "NOT_AGENT_LAUNCH", "this launch was not started with agent: true"));
    }
    if !entry.is_running() {
        return Err(game_gone());
    }
    if tool.is_empty() {
        return Err(EngineError::bad_request("BAD_REQUEST", "tool is required"));
    }
    let timeout_ms = timeout_ms.clamp(1000, MAX_CALL_TIMEOUT_S * 1000);
    let (tx, rx) = oneshot::channel();
    let id = next_command_id(st);
    let command = AgentCommand {
        id: id.clone(),
        tool: tool.to_string(),
        args: match args {
            Some(v @ Value::Object(_)) => v.clone(),
            _ => json!({}),
        },
        timeout_ms,
    };
    {
        let mut inner = lock_launch(&entry);
        if inner.queue.len() >= MAX_QUEUE {
            return Err(EngineError::api(
                429,
                "QUEUE_FULL",
                "too many commands queued for this launch (is the mod polling?)",
            ));
        }
        inner.inflight.insert(id, Inflight { tool: tool.to_string(), tx });
        inner.queue.push_back(command.clone());
        release_pollers_locked(&mut inner);
    }
    Ok(PendingCall {
        command,
        entry,
        rx: Some(rx),
        deadline: tokio::time::Instant::now() + Duration::from_millis(timeout_ms),
        finished: false,
    })
}

/// Puts a handed-over batch back if the poll was dropped before taking it
/// (client hang-up, engine shutdown) — a dropped poll must never swallow
/// commands.
struct PollGuard {
    entry: Arc<AgentLaunch>,
    waiter: Arc<PollWaiter>,
}

impl Drop for PollGuard {
    fn drop(&mut self) {
        let mut inner = lock_launch(&self.entry);
        inner.waiters.retain(|w| !Arc::ptr_eq(w, &self.waiter));
        let leftovers = self.waiter.take();
        if !leftovers.is_empty() {
            for c in leftovers.into_iter().rev() {
                inner.queue.push_front(c);
            }
            release_pollers_locked(&mut inner);
        }
    }
}

/// Long-poll: the queued commands, or `[]` after `timeout` / when the game
/// exits. Dropping the returned future (client hang-up) releases the poll.
pub async fn await_commands(entry: &Arc<AgentLaunch>, timeout: Duration) -> Vec<AgentCommand> {
    if !entry.is_running() {
        return Vec::new();
    }
    let waiter = Arc::new(PollWaiter::default());
    let immediate = {
        let mut inner = lock_launch(entry);
        if !inner.queue.is_empty() {
            Some(inner.queue.drain(..).collect::<Vec<_>>())
        } else {
            inner.waiters.push(waiter.clone());
            None
        }
    };
    if let Some(batch) = immediate {
        return batch;
    }
    let guard = PollGuard { entry: entry.clone(), waiter: waiter.clone() };
    let _ = tokio::time::timeout(timeout, waiter.notify.notified()).await;
    // Take whatever landed (the guard puts nothing back once it is taken).
    let out = waiter.take();
    drop(guard);
    out
}

/// Resolve an in-flight call with the mod's result payload.
pub fn complete_command(entry: &AgentLaunch, id: &str, ok: bool, data: Option<&Value>, error: Option<&Value>) -> bool {
    let waiter = {
        let mut inner = lock_launch(entry);
        inner.inflight.remove(id)
    };
    let Some(waiter) = waiter else { return false };
    if ok {
        let _ = waiter.tx.send(Ok(data.filter(|d| !d.is_null()).cloned().unwrap_or_else(|| json!({}))));
    } else {
        let code = error
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .filter(|c| is_upper_snake(c))
            .unwrap_or("FAILED")
            .to_string();
        let message = error
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("'{}' failed", waiter.tool));
        let _ = waiter.tx.send(Err(EngineError::api(502, code, message)));
    }
    true
}

fn is_upper_snake(code: &str) -> bool {
    let mut chars = code.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Store the mod's latest state report. Returns the view the `agent-state`
/// event carries (`updated_at` included).
pub fn set_state(entry: &AgentLaunch, state: &Value) -> AgentReport {
    let mut next = AgentReport::from_wire(state);
    let at = crate::platform::now_ms();
    {
        let mut inner = lock_launch(entry);
        inner.state = Some(next.clone());
        inner.state_at = at;
    }
    next.updated_at = Some(at);
    next
}

/// The `state` view for a launch: the last report plus `updated_at`, or None.
pub fn state_of(entry: &AgentLaunch) -> Option<AgentReport> {
    let inner = lock_launch(entry);
    let mut state = inner.state.clone()?;
    state.updated_at = Some(inner.state_at);
    Some(state)
}

/// Policy block handed to the mod on every poll (tiers enforced mod-side).
pub fn policy(config: &ConfigStore) -> Value {
    json!({
        "allowed_servers": agent_config(config).allowed_servers,
        "blocked_hosts": BLOCKED_HOSTS,
    })
}

// ---------------------------------------------------------------------------
// File results
// ---------------------------------------------------------------------------

pub const CAPTURES_DIR: &str = "espectral-captures";

/// Inline preview for an image file (bounded so a 4K PNG cannot bloat a reply).
pub const MAX_PREVIEW_BYTES: u64 = 4 * 1024 * 1024;

/// Rewrite every `*_path` field of a mod result from gameDir-relative to
/// absolute, refusing anything that escapes `<gameDir>/espectral-captures/`.
///
/// Containment is checked on the RESOLVED path (symlinks/`..` cannot win) and
/// must be inside the captures directory — the mod writes there, and a result
/// naming `../options.txt` is either a bug or a compromised game process; both
/// must not hand the agent a launcher file. `preview_path` additionally gets an
/// inline `preview: {mime, base64}` so MCP image content needs no second request.
pub fn rewrite_file_results(data: &Value, game_dir: Option<&Path>) -> Value {
    let Some(game_dir) = game_dir else { return data.clone() };
    match data {
        Value::Object(map) => {
            let mut out = Map::new();
            let mut preview: Option<Value> = None;
            for (k, v) in map {
                if let Value::String(rel) = v {
                    if k.ends_with("_path") {
                        let abs = resolve_capture_path(rel, game_dir, None);
                        if k == "preview_path" {
                            preview = abs.as_deref().and_then(read_preview);
                        }
                        out.insert(
                            k.clone(),
                            abs.map(|p| Value::String(p.to_string_lossy().into_owned())).unwrap_or(Value::Null),
                        );
                        continue;
                    }
                }
                out.insert(k.clone(), v.clone());
            }
            if let Some(p) = preview {
                out.insert("preview".into(), p);
            }
            Value::Object(out)
        }
        // JS copies an array's entries index-by-index (no index ends in
        // `_path`), so arrays pass through unchanged.
        other => other.clone(),
    }
}

/// Absolute path of `rel` (relative to `game_dir`) when it stays inside
/// `allowed_root` (default `<game_dir>/espectral-captures`); `None` otherwise.
/// A `None` (rather than an error) keeps one bad field from failing the whole
/// result — the agent sees `path: null` instead of a launcher file.
pub fn resolve_capture_path(rel: &str, game_dir: &Path, allowed_root: Option<&Path>) -> Option<PathBuf> {
    if rel.is_empty() {
        return None;
    }
    let base = lexical_resolve(allowed_root.unwrap_or(&game_dir.join(CAPTURES_DIR)));
    let abs = lexical_resolve(&game_dir.join(rel));
    let cmp = cfg!(windows);
    let fold = |p: &Path| {
        let s = p.to_string_lossy().into_owned();
        if cmp {
            s.to_lowercase()
        } else {
            s
        }
    };
    let (a, b) = (fold(&abs), fold(&base));
    let sep = std::path::MAIN_SEPARATOR;
    if a != b && !a.starts_with(&format!("{b}{sep}")) {
        return None;
    }
    // Lexical containment is not enough: a SYMLINK inside the captures dir (or
    // a captured path that traverses one) resolves to a real file elsewhere.
    if let Ok(real_base) = std::fs::canonicalize(&base) {
        if let Ok(real_abs) = std::fs::canonicalize(&abs) {
            let (ra, rb) = (fold(&real_abs), fold(&real_base));
            if ra != rb && !ra.starts_with(&format!("{rb}{sep}")) {
                return None;
            }
        }
    }
    Some(abs)
}

/// JS `path.resolve` without touching the filesystem: one absolute, normalized
/// path built from `p` (`..` pops, `.` is skipped, absolute input wins).
fn lexical_resolve(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Prefix(pfx) => out.push(pfx.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(n) => out.push(n),
        }
    }
    out
}

fn read_preview(file: &Path) -> Option<Value> {
    let f = std::fs::File::open(file).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_PREVIEW_BYTES {
        return None;
    }
    // Read at most MAX + 1: the stat above is a hint and the file can grow
    // between it and the read (the game writes these).
    let mut buf = Vec::with_capacity(meta.len().min(MAX_PREVIEW_BYTES) as usize + 1);
    f.take(MAX_PREVIEW_BYTES + 1).read_to_end(&mut buf).ok()?;
    if buf.is_empty() || buf.len() as u64 > MAX_PREVIEW_BYTES {
        return None;
    }
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    Some(json!({
        "mime": mime,
        "base64": base64::engine::general_purpose::STANDARD.encode(&buf),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allowlist_is_normalized_deduped_and_drops_invalid_rows() {
        let raw =
            json!([" Play.Example.NET. ", "play.example.net", "http://x", "a b", "[::1]:25565", 7, "10.0.0.2:25566"]);
        assert_eq!(normalize_allowed_servers(Some(&raw)), vec!["play.example.net", "[::1]:25565", "10.0.0.2:25566"]);
        assert!(normalize_allowed_servers(Some(&json!("x"))).is_empty());
    }

    #[test]
    fn token_rotation_replaces_the_file_and_old_token_stops_matching() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_token(dir.path()), None);
        let first = ensure_token(dir.path()).unwrap();
        assert_eq!(ensure_token(dir.path()).unwrap(), first);
        let raw = std::fs::read_to_string(token_path(dir.path())).unwrap();
        assert_eq!(raw, format!("{first}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(dir.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let second = write_token(dir.path()).unwrap();
        assert_ne!(first, second);
        assert!(!token_matches(dir.path(), &first));
        assert!(token_matches(dir.path(), &second));
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 1, "no tmp files left: {leftovers:?}");
    }
}
