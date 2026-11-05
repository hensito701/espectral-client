//! Single instance + .mrpack forwarding (replaces tauri-plugin-single-instance
//! and forward_mrpack_to_engine). FOUNDATION-OWNED.
//!
//! The first instance holds an exclusive lock on `<data_dir>/ui.lock` and
//! listens on a loopback TCP port from the moment it takes the lock. It writes
//! `<port> <token> <pid>` to `<data_dir>/ui.port` (owner-only file in the
//! owner-only data dir). A second launch fails to take the lock, sends
//! `{"token":…,"args":[…]}` (one JSON line) to that port and reads one reply
//! line:
//!
//! * `ok` — the first instance took the launch. While it is still starting,
//!   the args are queued (its window opens once it is ready); once its UI
//!   runs, its window was raised on the UI thread. The second launch exits.
//! * `closing` — the first instance's window is gone and it is shutting down:
//!   the second launch keeps trying the lock and becomes the new first
//!   instance once it is free.
//! * `busy`, or no answer — the first instance's UI thread did not take the
//!   launch in time.
//!
//! A launch is never dropped silently: when the lock holder has not taken it
//! within [`WAIT_FOR_PRIMARY`], [`acquire`] returns [`Acquire::Unresponsive`]
//! and main.rs offers to close the stuck process ([`kill_stuck_instance`]).
//!
//! The token keeps other local users (who can reach any loopback port but not
//! this user's data dir) from injecting imports. Frames are capped in size and
//! in total time, and handled off the accept thread, so a slow or hostile
//! client cannot starve real second launches.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Largest accepted frame (a few hundred paths).
const MAX_FRAME: usize = 64 * 1024;
/// Whole-frame deadline, however the bytes trickle in.
const FRAME_DEADLINE: Duration = Duration::from_secs(2);
/// Connections handled at once; extra ones are dropped.
const MAX_CONCURRENT: usize = 8;
/// Connect budget of one forward attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// Reply budget of one forward attempt: the UI thread's answer plus slack.
const REPLY_TIMEOUT: Duration = Duration::from_secs(4);
/// How long a second launch keeps trying before it reports the lock holder
/// as unresponsive. A closing first instance lets go within ~5 s.
pub const WAIT_FOR_PRIMARY: Duration = Duration::from_secs(10);
/// Pause between two attempts of a second launch.
const RETRY_INTERVAL: Duration = Duration::from_millis(200);

const REPLY_OK: &str = "ok";
const REPLY_CLOSING: &str = "closing";
const REPLY_BUSY: &str = "busy";

/// Takes an authenticated second launch's argv once the UI runs (called on a
/// connection thread); `true` = the UI thread took it.
type Handler = Box<dyn FnMut(Vec<String>) -> bool + Send>;

enum State {
    /// Args of second launches that arrived before the UI was ready.
    Starting(Vec<Vec<String>>),
    Running(Arc<Mutex<Handler>>),
    Closing,
}

struct Shared {
    token: String,
    state: Mutex<State>,
}

impl Shared {
    /// The reply line for an authenticated second launch.
    fn take(&self, args: Vec<String>) -> &'static str {
        let handler = match &mut *lock(&self.state) {
            State::Starting(queued) => {
                queued.push(args);
                return REPLY_OK;
            }
            State::Closing => return REPLY_CLOSING,
            State::Running(handler) => handler.clone(),
        };
        let took = {
            let mut handler = lock(&handler);
            (**handler)(args)
        };
        if took {
            REPLY_OK
        } else {
            REPLY_BUSY
        }
    }
}

/// Held for the process lifetime (dropping it releases the lock).
pub struct InstanceGuard {
    shared: Arc<Shared>,
    port_path: PathBuf,
    #[cfg(test)]
    addr: SocketAddr,
    _lock: File,
}

impl InstanceGuard {
    /// The UI is up: later second launches go to `handler`. Returns the args
    /// of second launches queued while this instance was starting.
    pub fn ready(&self, handler: impl FnMut(Vec<String>) -> bool + Send + 'static) -> Vec<Vec<String>> {
        let mut state = lock(&self.shared.state);
        match std::mem::replace(&mut *state, State::Running(Arc::new(Mutex::new(Box::new(handler))))) {
            State::Starting(queued) => queued,
            other => {
                *state = other;
                Vec::new()
            }
        }
    }

    /// The window is gone: second launches wait for the lock instead.
    pub fn closing(&self) {
        *lock(&self.shared.state) = State::Closing;
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // Before the lock goes (fields drop after this): a launch that reads
        // the port file now finds none and retries the lock.
        self.closing();
        let _ = std::fs::remove_file(&self.port_path);
    }
}

pub enum Acquire {
    /// We are the primary instance.
    Primary(InstanceGuard),
    /// Another instance is running and took our args.
    Forwarded,
    /// Another instance holds the lock but never took the launch (its UI
    /// thread is stuck). `pid` comes from `ui.port`.
    Unresponsive { pid: Option<u32> },
}

/// `.mrpack` argument (ASCII case-insensitive suffix), e.g. a file-association
/// double click. Compares bytes: any UTF-8 path is safe to test.
pub fn is_mrpack_arg(arg: &str) -> bool {
    let b = arg.as_bytes();
    b.len() >= 7 && b[b.len() - 7..].eq_ignore_ascii_case(b".mrpack")
}

/// Become the first instance, or hand `args` to the running one. Blocks for
/// up to [`WAIT_FOR_PRIMARY`] while the lock holder is closing or stuck.
pub fn acquire(data_dir: &Path, args: &[String]) -> std::io::Result<Acquire> {
    acquire_within(data_dir, args, WAIT_FOR_PRIMARY)
}

fn acquire_within(data_dir: &Path, args: &[String], wait: Duration) -> std::io::Result<Acquire> {
    espectral_engine::paths::ensure_private_dir(data_dir)?;
    let port_path = data_dir.join("ui.port");
    let lock_file =
        OpenOptions::new().create(true).truncate(false).read(true).write(true).open(data_dir.join("ui.lock"))?;
    let deadline = Instant::now() + wait;
    let mut pid = None;
    // Outer None: nothing logged yet (an unanswered first attempt is logged too).
    let mut last_reply: Option<Option<String>> = None;
    loop {
        match lock_file.try_lock() {
            Ok(()) => return become_primary(lock_file, port_path).map(Acquire::Primary),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
        if let Some(primary) = std::fs::read_to_string(&port_path).ok().as_deref().and_then(parse_port_file) {
            pid = primary.pid.or(pid);
            let reply = forward(&primary, args);
            if reply.as_deref() == Some(REPLY_OK) {
                return Ok(Acquire::Forwarded);
            }
            if last_reply.as_ref() != Some(&reply) {
                tracing::info!(
                    "[single-instance] running instance (pid {:?}) answered {:?}; waiting for it",
                    primary.pid,
                    reply.as_deref().unwrap_or("nothing")
                );
                last_reply = Some(reply);
            }
        }
        if Instant::now() >= deadline {
            return Ok(Acquire::Unresponsive { pid });
        }
        std::thread::sleep(RETRY_INTERVAL);
    }
}

fn become_primary(lock_file: File, port_path: PathBuf) -> std::io::Result<InstanceGuard> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    let addr = listener.local_addr()?;
    let shared = Arc::new(Shared {
        token: espectral_engine::bridge::mint_token(),
        state: Mutex::new(State::Starting(Vec::new())),
    });
    espectral_engine::paths::write_private(
        &port_path,
        format!("{} {} {}", addr.port(), shared.token, std::process::id()).as_bytes(),
    )?;
    serve(listener, shared.clone());
    Ok(InstanceGuard {
        shared,
        port_path,
        #[cfg(test)]
        addr,
        _lock: lock_file,
    })
}

/// `ui.port`: `<port> <token> [<pid>]` (older launchers wrote no pid).
struct PortFile {
    port: u16,
    token: String,
    pid: Option<u32>,
}

fn parse_port_file(s: &str) -> Option<PortFile> {
    let mut it = s.split_whitespace();
    let port = it.next()?.parse().ok()?;
    let token = it.next()?.to_string();
    let pid = it.next().and_then(|p| p.parse().ok());
    Some(PortFile { port, token, pid })
}

/// Send our args to the lock holder; its reply line (None: unreachable or
/// closed without answering).
fn forward(primary: &PortFile, args: &[String]) -> Option<String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, primary.port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    let line = json!({ "token": primary.token, "args": args }).to_string();
    stream.write_all(line.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    stream.set_read_timeout(Some(REPLY_TIMEOUT)).ok()?;
    let mut reply = String::new();
    BufReader::new(stream.take(64)).read_line(&mut reply).ok()?;
    let reply = reply.trim();
    (!reply.is_empty()).then(|| reply.to_string())
}

/// Serve second launches forever on a background thread.
fn serve(listener: TcpListener, shared: Arc<Shared>) {
    let busy = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if busy.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT {
                    busy.fetch_sub(1, Ordering::SeqCst);
                    continue; // dropped
                }
                let (shared, held) = (shared.clone(), busy.clone());
                let spawned = std::thread::Builder::new().name("single-instance-conn".into()).spawn(move || {
                    let mut stream = stream;
                    if let Some(args) = read_frame(&mut stream).and_then(|f| authenticated_args(&f, &shared.token)) {
                        let reply = shared.take(args);
                        let _ = stream.write_all(format!("{reply}\n").as_bytes());
                    }
                    held.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    busy.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })
        .ok();
}

/// One newline-terminated frame, at most [`MAX_FRAME`] bytes, received within
/// [`FRAME_DEADLINE`] in total. None otherwise.
fn read_frame(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let deadline = Instant::now() + FRAME_DEADLINE;
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 4096];
    loop {
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
        stream.set_read_timeout(Some(left)).ok()?;
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        if let Some(i) = chunk[..n].iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&chunk[..i]);
            return (buf.len() <= MAX_FRAME).then_some(buf);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_FRAME {
            return None;
        }
    }
}

/// The `args` of a frame whose `token` matches (constant-time).
fn authenticated_args(frame: &[u8], token: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_slice(frame).ok()?;
    let presented = v.get("token")?.as_str()?;
    if !ct_eq(presented.as_bytes(), token.as_bytes()) {
        return None;
    }
    Some(v.get("args")?.as_array()?.iter().filter_map(|a| a.as_str().map(str::to_string)).collect())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Kill the stuck lock holder `pid` (from `ui.port`) when it really is this
/// launcher's executable — a pid can be reused. A running game is a separate
/// process and keeps running. True when it was killed.
pub fn kill_stuck_instance(pid: u32) -> bool {
    if pid == std::process::id() || !is_launcher_process(pid) {
        return false;
    }
    espectral_engine::launch::process::kill_hard(pid)
}

fn is_launcher_process(pid: u32) -> bool {
    let Some(exe_name) =
        std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
    else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        // `"espectral-client.exe","1234",…`
        out.is_ok_and(|o| {
            String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&format!("\"{exe_name}\",\"{pid}\""))
        })
    }
    #[cfg(unix)]
    {
        std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            .is_some_and(|n| n.trim_end_matches(" (deleted)") == exe_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const SHORT_WAIT: Duration = Duration::from_millis(1500);

    fn primary(dir: &Path) -> InstanceGuard {
        let Acquire::Primary(g) = acquire_within(dir, &[], SHORT_WAIT).unwrap() else {
            panic!("first must be primary")
        };
        g
    }

    /// A primary whose UI is up and takes every launch.
    fn running(dir: &Path) -> (InstanceGuard, mpsc::Receiver<Vec<String>>) {
        let g = primary(dir);
        let (tx, rx) = mpsc::channel();
        assert!(g.ready(move |a| tx.send(a).is_ok()).is_empty());
        (g, rx)
    }

    fn second(dir: &Path, args: &[&str]) -> Acquire {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        acquire_within(dir, &args, SHORT_WAIT).unwrap()
    }

    fn send_raw(g: &InstanceGuard, bytes: &[u8]) -> TcpStream {
        let mut s = TcpStream::connect(g.addr).unwrap();
        let _ = s.write_all(bytes);
        s
    }

    #[test]
    fn mrpack_suffix() {
        assert!(is_mrpack_arg("C:/x/Pack.MRPACK"));
        assert!(!is_mrpack_arg("pack.zip"));
        assert!(!is_mrpack_arg("x"));
        // Multi-byte paths used to panic (byte index inside 'é').
        for s in ["éééé", "ééééé.mrpack", "日本語", "é"] {
            let _ = is_mrpack_arg(s);
        }
        assert!(is_mrpack_arg("ééééé.mrpack"));
    }

    #[test]
    fn second_instance_forwards() {
        let dir = tempfile::tempdir().unwrap();
        let (_g, rx) = running(dir.path());
        // a second lock attempt from the same process uses a new handle -> WouldBlock
        assert!(matches!(second(dir.path(), &["a.mrpack"]), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["a.mrpack".to_string()]);
    }

    /// A launch that arrives while the first instance is still starting is
    /// accepted and handed over once its UI is ready.
    #[test]
    fn launches_during_startup_are_queued() {
        let dir = tempfile::tempdir().unwrap();
        let g = primary(dir.path());
        assert!(matches!(second(dir.path(), &["early.mrpack"]), Acquire::Forwarded));
        let (tx, rx) = mpsc::channel();
        assert_eq!(g.ready(move |a| tx.send(a).is_ok()), vec![vec!["early.mrpack".to_string()]]);
        assert!(rx.try_recv().is_err(), "queued args are returned, not replayed");
    }

    /// A closing first instance does not swallow the launch: the second one
    /// waits and takes over once the lock is free.
    #[test]
    fn closing_instance_hands_over_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (g, rx) = running(dir.path());
        g.closing();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            drop(g);
        });
        assert!(matches!(second(dir.path(), &["x.mrpack"]), Acquire::Primary(_)));
        releaser.join().unwrap();
        assert!(rx.try_recv().is_err(), "a closing instance never takes launches");
    }

    /// A first instance whose UI thread never takes the launch is reported,
    /// with its pid, instead of the launch vanishing.
    #[test]
    fn stuck_instance_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let g = primary(dir.path());
        g.ready(|_| false);
        let t = Instant::now();
        match second(dir.path(), &[]) {
            Acquire::Unresponsive { pid } => assert_eq!(pid, Some(std::process::id())),
            _ => panic!("expected Unresponsive"),
        }
        assert!(t.elapsed() >= SHORT_WAIT);
    }

    /// The port file belongs to the lock holder: it goes with the guard.
    #[test]
    fn port_file_is_removed_with_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let g = primary(dir.path());
        let port = std::fs::read_to_string(dir.path().join("ui.port")).unwrap();
        let parsed = parse_port_file(&port).unwrap();
        assert_eq!(parsed.pid, Some(std::process::id()));
        drop(g);
        assert!(!dir.path().join("ui.port").exists());
        assert!(matches!(second(dir.path(), &[]), Acquire::Primary(_)));
    }

    /// Another local user can reach the port but not the token: ignored.
    #[test]
    fn frames_without_the_token_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (g, rx) = running(dir.path());
        send_raw(&g, b"[\"/tmp/shared/evil.mrpack\"]\n");
        send_raw(&g, b"{\"token\":\"nope\",\"args\":[\"/tmp/shared/evil.mrpack\"]}\n");
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
        assert!(matches!(second(dir.path(), &["ok.mrpack"]), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["ok.mrpack".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("ui.port")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    /// Oversized or trickling frames are cut off and never block a real launch.
    #[test]
    fn oversized_and_slow_frames_do_not_block_others() {
        let dir = tempfile::tempdir().unwrap();
        let (g, rx) = running(dir.path());
        let big = vec![b'x'; MAX_FRAME + 10];
        let _big = send_raw(&g, &big);
        let mut slow = send_raw(&g, b"{");
        let t = Instant::now();
        assert!(matches!(second(dir.path(), &["next.mrpack"]), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["next.mrpack".to_string()]);
        assert!(t.elapsed() < FRAME_DEADLINE, "served while the slow client is still open");
        let _ = slow.write_all(b" ");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "oversized frame ignored");
    }

    #[test]
    fn unicode_arguments_keep_the_server_alive() {
        let dir = tempfile::tempdir().unwrap();
        let (_g, rx) = running(dir.path());
        assert!(matches!(second(dir.path(), &["éééé"]), Acquire::Forwarded));
        let got = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(!got.iter().any(|a| is_mrpack_arg(a)));
        assert!(matches!(second(dir.path(), &["b.mrpack"]), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["b.mrpack".to_string()]);
    }
}
