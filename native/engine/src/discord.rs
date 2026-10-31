//! Port of `src/engine/discord.mjs` — Discord Rich Presence over the local
//! IPC socket.
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! Frame format: `[opcode: u32 LE][length: u32 LE][JSON payload]`
//! (opcode 0 HANDSHAKE, 1 FRAME, 2 CLOSE, 3 PING, 4 PONG — opcode first;
//! any other layout is rejected by Discord with protocol error 1003).
//!
//! - Windows: named pipe `\\?\pipe\discord-ipc-N` (and the `\\.\` alias), N = 0..9.
//! - Linux/macOS: `$XDG_RUNTIME_DIR/discord-ipc-{0,1}` (the JS candidates),
//!   then — native additions for Flatpak/Snap Discord on Fedora/Ubuntu —
//!   `$XDG_RUNTIME_DIR/app/com.discordapp.Discord/discord-ipc-N`,
//!   `$XDG_RUNTIME_DIR/snap.discord/discord-ipc-N`, `$TMPDIR|/tmp/discord-ipc-N`;
//!   finally the Linux abstract socket `@discord-ipc-0`.
//!
//! Best-effort by design: any failure degrades to a quiet no-op — presence is
//! cosmetic and must never disturb the launcher. A PING is sent every 15 s
//! while connected (Discord drops idle connections).
//!
//! Deliberate difference: the JS matched replies with `pending.get(nonce)`
//! using a numeric key against the string nonce, so every SET_ACTIVITY reply
//! was ignored after the 2 s frame timeout. Here replies are matched by
//! nonce, so the "retry without images" path actually runs.
//!
//! CONTRACT (called by game-launch): set_presence, clear_presence.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::engine::Engine;
use crate::error::EngineResult;

/// Rich Presence application id (env ESPECTRAL_DISCORD_CLIENT_ID overrides,
/// like the JS). Art asset keys: `espectral_logo`, `uhc`, `24h`.
pub const DEFAULT_RPC_CLIENT_ID: &str = "1535246763856760874";

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
const FRAME_TIMEOUT: Duration = Duration::from_millis(2000);
const PING_EVERY: Duration = Duration::from_secs(15);
const MAX_FRAME: usize = 1 << 20;

/// `DISCORD_CLIENT_ID` of discord.mjs.
pub fn rpc_client_id() -> String {
    std::env::var("ESPECTRAL_DISCORD_CLIENT_ID").unwrap_or_else(|_| DEFAULT_RPC_CLIENT_ID.to_string())
}

/// setPresence() argument (JS object keys).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub details: Option<String>,
    pub state: Option<String>,
    /// ms epoch
    pub start_timestamp: Option<i64>,
    pub end_timestamp: Option<i64>,
    pub image: Option<String>,
    pub small_image: Option<String>,
    /// default "Espectral Client"
    pub large_text: Option<String>,
}

/// setPresence() result (`{ ok, error?, degraded? }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceOutcome {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<bool>,
}

trait IpcStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> IpcStream for T {}

struct Conn {
    stream: Box<dyn IpcStream>,
    ping: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        if let Some(p) = self.ping.take() {
            p.abort();
        }
    }
}

/// The IPC connection (JS module-level socket + timers).
#[derive(Default)]
pub struct DiscordIpcState {
    conn: Arc<tokio::sync::Mutex<Option<Conn>>>,
    sequence: AtomicU64,
    last_no_discord_warn: AtomicI64,
}

impl std::fmt::Debug for DiscordIpcState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordIpcState").finish_non_exhaustive()
    }
}

fn st(engine: &Engine) -> &DiscordIpcState {
    &engine.identity_discord_state().discord
}

/// Encode one frame: `[op u32 LE][len u32 LE][json]`.
pub fn encode_frame(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

async fn read_frame(s: &mut dyn IpcStream) -> std::io::Result<(u32, Value)> {
    let mut head = [0u8; 8];
    s.read_exact(&mut head).await?;
    let op = u32::from_le_bytes(head[0..4].try_into().expect("4 bytes"));
    let len = u32::from_le_bytes(head[4..8].try_into().expect("4 bytes")) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "discord frame too large"));
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).await?;
    Ok((op, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

/// Socket candidates for this platform (JS pickPipe + native additions).
pub fn ipc_candidates() -> Vec<PathBuf> {
    let mut out = vec![];
    if cfg!(windows) {
        for i in 0..10 {
            out.push(PathBuf::from(format!(r"\\?\pipe\discord-ipc-{i}")));
            out.push(PathBuf::from(format!(r"\\.\pipe\discord-ipc-{i}")));
        }
        return out;
    }
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok().filter(|s| !s.is_empty()).map(PathBuf::from);
    if let Some(base) = &xdg {
        out.push(base.join("discord-ipc-0"));
        out.push(base.join("discord-ipc-1"));
        for sub in ["app/com.discordapp.Discord", "app/com.discordapp.DiscordCanary", "snap.discord"] {
            for i in 0..2 {
                out.push(base.join(sub).join(format!("discord-ipc-{i}")));
            }
        }
    }
    let tmp =
        std::env::var("TMPDIR").ok().filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| "/tmp".into());
    for i in 0..2 {
        let p = tmp.join(format!("discord-ipc-{i}"));
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

#[cfg(unix)]
async fn open_path(p: &std::path::Path) -> Option<Box<dyn IpcStream>> {
    if !p.exists() {
        return None;
    }
    tokio::net::UnixStream::connect(p).await.ok().map(|s| Box::new(s) as Box<dyn IpcStream>)
}

#[cfg(windows)]
async fn open_path(p: &std::path::Path) -> Option<Box<dyn IpcStream>> {
    tokio::net::windows::named_pipe::ClientOptions::new()
        .open(p.as_os_str())
        .ok()
        .map(|s| Box::new(s) as Box<dyn IpcStream>)
}

#[cfg(target_os = "linux")]
fn open_abstract() -> Option<Box<dyn IpcStream>> {
    use std::os::linux::net::SocketAddrExt;
    let addr = std::os::unix::net::SocketAddr::from_abstract_name(b"discord-ipc-0").ok()?;
    let s = std::os::unix::net::UnixStream::connect_addr(&addr).ok()?;
    s.set_nonblocking(true).ok()?;
    tokio::net::UnixStream::from_std(s).ok().map(|s| Box::new(s) as Box<dyn IpcStream>)
}

#[cfg(not(target_os = "linux"))]
fn open_abstract() -> Option<Box<dyn IpcStream>> {
    None
}

async fn open_stream(engine: &Engine) -> Option<Box<dyn IpcStream>> {
    let (candidates, use_abstract) = match engine.identity_discord_state().overrides().ipc_candidates {
        Some(c) => (c, false),
        None => (ipc_candidates(), true),
    };
    for p in &candidates {
        if let Some(s) = open_path(p).await {
            return Some(s);
        }
    }
    if use_abstract {
        return open_abstract();
    }
    None
}

/// Connect + handshake (wait for the READY dispatch), bounded by the connect timeout.
async fn connect(engine: &Engine) -> Option<Conn> {
    let fut = async {
        let mut stream = open_stream(engine).await?;
        let hs = encode_frame(OP_HANDSHAKE, &json!({ "v": 1, "client_id": rpc_client_id() }));
        stream.write_all(&hs).await.ok()?;
        loop {
            let (op, payload) = read_frame(stream.as_mut()).await.ok()?;
            if op == OP_PING || op == OP_PONG {
                continue;
            }
            if payload.get("cmd").and_then(Value::as_str) == Some("DISPATCH")
                && payload.get("evt").and_then(Value::as_str) == Some("READY")
            {
                return Some(stream);
            }
        }
    };
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, fut).await.ok().flatten()?;
    Some(Conn { stream, ping: None })
}

fn spawn_ping(engine: &Engine) -> tokio::task::JoinHandle<()> {
    let conn = st(engine).conn.clone();
    let cancel = engine.cancel_token().clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(PING_EVERY) => {}
            }
            let mut g = conn.lock().await;
            let Some(c) = g.as_mut() else { return };
            if c.stream.write_all(&encode_frame(OP_PING, &json!({}))).await.is_err() {
                // Discord went away: drop the connection so the next call reconnects.
                if let Some(mut dead) = g.take() {
                    dead.ping = None; // this task; don't abort ourselves
                }
                return;
            }
        }
    })
}

/// Send one FRAME and wait for the reply carrying the same nonce (None on
/// timeout). A broken socket drops the connection.
async fn send(engine: &Engine, slot: &mut Option<Conn>, payload: Value) -> Option<Value> {
    let conn = slot.as_mut()?;
    let nonce = st(engine).sequence.fetch_add(1, Ordering::SeqCst).to_string();
    let mut p = payload;
    p["nonce"] = Value::String(nonce.clone());
    if conn.stream.write_all(&encode_frame(OP_FRAME, &p)).await.is_err() {
        *slot = None;
        return None;
    }
    let read = async {
        loop {
            let (op, reply) = read_frame(conn.stream.as_mut()).await?;
            if op == OP_PING || op == OP_PONG {
                continue;
            }
            let rn = reply.get("nonce").and_then(Value::as_str);
            if rn == Some(nonce.as_str()) {
                return Ok::<_, std::io::Error>(reply);
            }
        }
    };
    match tokio::time::timeout(FRAME_TIMEOUT, read).await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(_)) => {
            *slot = None;
            None
        }
        Err(_) => None,
    }
}

async fn ensure_connected<'a>(engine: &Engine, g: &mut tokio::sync::MutexGuard<'a, Option<Conn>>) -> bool {
    if g.is_some() {
        return true;
    }
    match connect(engine).await {
        Some(mut c) => {
            c.ping = Some(spawn_ping(engine));
            **g = Some(c);
            true
        }
        None => false,
    }
}

fn is_error(reply: &Option<Value>) -> bool {
    match reply {
        Some(r) => {
            r.get("evt").and_then(Value::as_str) == Some("ERROR")
                || r.get("data").and_then(|d| d.get("code")).is_some_and(Value::is_number)
        }
        None => false,
    }
}

fn err_parts(reply: &Option<Value>) -> (String, String) {
    let d = reply.as_ref().and_then(|r| r.get("data"));
    let code = d.and_then(|d| d.get("code")).map(|c| c.to_string()).unwrap_or_else(|| "?".into());
    let msg = d
        .and_then(|d| d.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".into());
    (code, msg)
}

/// Set the presence; never fails (best-effort), see [`set_presence_outcome`].
pub async fn set_presence(engine: &Engine, presence: Presence) -> EngineResult<()> {
    let _ = set_presence_outcome(engine, presence).await;
    Ok(())
}

/// setPresence() with its `{ ok, error?, degraded? }` outcome. Discord rejects
/// the whole activity when an asset key does not exist, so an error with
/// images requested retries once without images.
pub async fn set_presence_outcome(engine: &Engine, presence: Presence) -> PresenceOutcome {
    let state = st(engine);
    let conn = state.conn.clone();
    let mut g = conn.lock().await;
    if !ensure_connected(engine, &mut g).await {
        let now = crate::platform::now_ms();
        if now - state.last_no_discord_warn.load(Ordering::Relaxed) > 60_000 {
            state.last_no_discord_warn.store(now, Ordering::Relaxed);
            tracing::warn!("[discord] Discord desktop client not detected — Rich Presence unavailable");
        }
        return PresenceOutcome { ok: false, error: Some("no discord connection".into()), degraded: None };
    }

    let mut activity = Map::new();
    activity.insert("type".into(), json!(0)); // Playing
    if let Some(d) = presence.details.as_deref().filter(|s| !s.is_empty()) {
        activity.insert("details".into(), json!(d));
    }
    if let Some(s) = presence.state.as_deref().filter(|s| !s.is_empty()) {
        activity.insert("state".into(), json!(s));
    }
    let mut assets = Map::new();
    if let Some(i) = presence.image.as_deref().filter(|s| !s.is_empty()) {
        assets.insert("large_image".into(), json!(i));
    }
    if let Some(i) = presence.small_image.as_deref().filter(|s| !s.is_empty()) {
        assets.insert("small_image".into(), json!(i));
    }
    let has_assets = !assets.is_empty();
    if has_assets {
        assets.insert("large_text".into(), json!(presence.large_text.as_deref().unwrap_or("Espectral Client")));
        activity.insert("assets".into(), Value::Object(assets));
    }
    if let Some(t) = presence.start_timestamp.filter(|t| *t != 0) {
        activity.insert("timestamps".into(), json!({ "start": t }));
    } else if let Some(t) = presence.end_timestamp.filter(|t| *t != 0) {
        activity.insert("timestamps".into(), json!({ "end": t }));
    }
    let pid = std::process::id();
    let frame = |act: &Map<String, Value>| json!({ "cmd": "SET_ACTIVITY", "args": { "pid": pid, "activity": act } });

    let reply = send(engine, &mut g, frame(&activity)).await;
    if is_error(&reply) {
        let (code, message) = err_parts(&reply);
        if has_assets {
            tracing::warn!(
                "[discord] SET_ACTIVITY rejected ({code}: {message}) — retrying without images; upload the missing \
                 Rich Presence asset (exact key) and restart Discord"
            );
            activity.remove("assets");
            let reply = send(engine, &mut g, frame(&activity)).await;
            if is_error(&reply) {
                let (code, message) = err_parts(&reply);
                return PresenceOutcome {
                    ok: false,
                    error: Some(format!("SET_ACTIVITY rejected ({code}: {message})")),
                    degraded: None,
                };
            }
            return PresenceOutcome { ok: true, error: None, degraded: Some(true) };
        }
        return PresenceOutcome {
            ok: false,
            error: Some(format!("SET_ACTIVITY rejected ({code}: {message})")),
            degraded: None,
        };
    }
    PresenceOutcome { ok: true, error: None, degraded: None }
}

/// Clear the activity (best-effort).
pub async fn clear_presence(engine: &Engine) -> EngineResult<()> {
    let conn = st(engine).conn.clone();
    let mut g = conn.lock().await;
    if !ensure_connected(engine, &mut g).await {
        return Ok(());
    }
    let pid = std::process::id();
    let _ = send(engine, &mut g, json!({ "cmd": "SET_ACTIVITY", "args": { "pid": pid, "activity": null } })).await;
    Ok(())
}

/// Tear down the connection and ping timer (engine shutdown).
pub async fn shutdown(engine: &Engine) {
    let conn = st(engine).conn.clone();
    let mut g = conn.lock().await;
    *g = None;
}

/// True while an IPC connection is held (tests / diagnostics).
pub async fn is_connected(engine: &Engine) -> bool {
    st(engine).conn.lock().await.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    #[test]
    fn frame_layout_is_opcode_first() {
        let f = encode_frame(1, &json!({ "a": 1 }));
        assert_eq!(&f[0..4], &1u32.to_le_bytes());
        assert_eq!(&f[4..8], &7u32.to_le_bytes());
        assert_eq!(&f[8..], br#"{"a":1}"#);
    }

    #[cfg(unix)]
    #[test]
    fn unix_candidates_start_with_xdg_runtime_dir() {
        let c = ipc_candidates();
        if let Ok(x) = std::env::var("XDG_RUNTIME_DIR") {
            if !x.is_empty() {
                assert_eq!(c[0], PathBuf::from(&x).join("discord-ipc-0"));
                assert_eq!(c[1], PathBuf::from(&x).join("discord-ipc-1"));
            }
        }
        assert!(c.iter().all(|p| p.to_string_lossy().contains("discord-ipc-")));
    }

    #[tokio::test]
    async fn degrades_to_quiet_noop_without_discord() {
        let (d, e) = temp_engine();
        e.identity_discord_state().set_overrides(|o| o.ipc_candidates = Some(vec![d.path().join("no-such-socket")]));
        let t0 = std::time::Instant::now();
        let out = set_presence_outcome(
            &e,
            Presence { details: Some("test".into()), state: Some("test".into()), ..Default::default() },
        )
        .await;
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("no discord connection"));
        set_presence(&e, Presence::default()).await.unwrap();
        clear_presence(&e).await.unwrap();
        shutdown(&e).await;
        assert!(t0.elapsed() < Duration::from_secs(10));
    }

    /// A fake Discord client on a unix socket: READY on handshake, rejects
    /// activities with assets (unknown asset key), records frames.
    #[cfg(unix)]
    #[tokio::test]
    async fn talks_the_ipc_protocol_and_retries_without_images() {
        use tokio::net::UnixListener;
        let (d, e) = temp_engine();
        let sock = d.path().join("discord-ipc-0");
        let listener = UnixListener::bind(&sock).unwrap();
        let seen: Arc<std::sync::Mutex<Vec<(u32, Value)>>> = Arc::default();
        let seen2 = seen.clone();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            loop {
                let Ok((op, p)) = read_frame(&mut s).await else { return };
                seen2.lock().unwrap().push((op, p.clone()));
                let reply = if op == OP_HANDSHAKE {
                    json!({ "cmd": "DISPATCH", "evt": "READY", "data": { "v": 1 } })
                } else if p["args"]["activity"].get("assets").is_some() {
                    json!({ "cmd": "SET_ACTIVITY", "evt": "ERROR", "nonce": p["nonce"], "data": { "code": 4000, "message": "bad asset" } })
                } else {
                    json!({ "cmd": "SET_ACTIVITY", "nonce": p["nonce"], "data": {} })
                };
                // Unrelated frames first: a PONG and a reply for another nonce.
                s.write_all(&encode_frame(OP_PONG, &json!({}))).await.unwrap();
                s.write_all(&encode_frame(OP_FRAME, &json!({ "nonce": "other" }))).await.unwrap();
                s.write_all(&encode_frame(OP_FRAME, &reply)).await.unwrap();
            }
        });
        e.identity_discord_state().set_overrides(|o| o.ipc_candidates = Some(vec![sock.clone()]));
        let out = set_presence_outcome(
            &e,
            Presence {
                state: Some("En el launcher".into()),
                image: Some("espectral_logo".into()),
                start_timestamp: Some(1_700_000_000_000),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(out, PresenceOutcome { ok: true, error: None, degraded: Some(true) });
        assert!(is_connected(&e).await);
        clear_presence(&e).await.unwrap();
        let frames = seen.lock().unwrap().clone();
        assert_eq!(frames[0].0, OP_HANDSHAKE);
        assert_eq!(frames[0].1["v"], 1);
        assert_eq!(frames[0].1["client_id"], rpc_client_id());
        let act = &frames[1].1["args"]["activity"];
        assert_eq!(frames[1].1["cmd"], "SET_ACTIVITY");
        assert_eq!(act["type"], 0);
        assert_eq!(act["state"], "En el launcher");
        assert_eq!(act["assets"], json!({ "large_image": "espectral_logo", "large_text": "Espectral Client" }));
        assert_eq!(act["timestamps"], json!({ "start": 1_700_000_000_000i64 }));
        assert!(frames[2].1["args"]["activity"].get("assets").is_none(), "retry drops images");
        assert_eq!(frames[3].1["args"]["activity"], Value::Null, "clear sends a null activity");
        shutdown(&e).await;
        assert!(!is_connected(&e).await);
    }
}
