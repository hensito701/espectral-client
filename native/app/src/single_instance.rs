//! Single instance + .mrpack forwarding (replaces tauri-plugin-single-instance
//! and forward_mrpack_to_engine). FOUNDATION-OWNED.
//!
//! The first instance holds an exclusive lock on `<data_dir>/ui.lock` and
//! listens on a loopback TCP port. It writes `<port> <token>` to
//! `<data_dir>/ui.port` (owner-only file in the owner-only data dir). A second
//! launch fails to take the lock, sends `{"token":…,"args":[…]}` (one JSON
//! line) to that port and exits; the first instance raises its window and
//! imports any `.mrpack` arguments.
//!
//! The token keeps other local users (who can reach any loopback port but not
//! this user's data dir) from injecting imports. Frames are capped in size and
//! in total time, and handled off the accept thread, so a slow or hostile
//! client cannot starve real second launches.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Largest accepted frame (a few hundred paths).
const MAX_FRAME: usize = 64 * 1024;
/// Whole-frame deadline, however the bytes trickle in.
const FRAME_DEADLINE: Duration = Duration::from_secs(2);
/// Connections handled at once; extra ones are dropped.
const MAX_CONCURRENT: usize = 8;

/// Held for the process lifetime (dropping it releases the lock).
pub struct InstanceGuard {
    _lock: File,
    pub listener: TcpListener,
    token: String,
}

pub enum Acquire {
    /// We are the primary instance.
    Primary(InstanceGuard),
    /// Another instance is running; our args were forwarded (or it did not answer).
    Forwarded,
}

/// `.mrpack` argument (ASCII case-insensitive suffix), e.g. a file-association
/// double click. Compares bytes: any UTF-8 path is safe to test.
pub fn is_mrpack_arg(arg: &str) -> bool {
    let b = arg.as_bytes();
    b.len() >= 7 && b[b.len() - 7..].eq_ignore_ascii_case(b".mrpack")
}

pub fn acquire(data_dir: &Path, args: &[String]) -> std::io::Result<Acquire> {
    espectral_engine::paths::ensure_private_dir(data_dir)?;
    let lock_path = data_dir.join("ui.lock");
    let port_path = data_dir.join("ui.port");
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {
            let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
            let token = espectral_engine::bridge::mint_token();
            let port = listener.local_addr()?.port();
            espectral_engine::paths::write_private(&port_path, format!("{port} {token}").as_bytes())?;
            Ok(Acquire::Primary(InstanceGuard { _lock: lock, listener, token }))
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            if let Some((port, token)) = std::fs::read_to_string(&port_path).ok().as_deref().and_then(parse_port_file) {
                let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
                    let line = json!({ "token": token, "args": args }).to_string();
                    let _ = s.write_all(line.as_bytes());
                    let _ = s.write_all(b"\n");
                }
            }
            Ok(Acquire::Forwarded)
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// `<port> <token>` from `ui.port`.
fn parse_port_file(s: &str) -> Option<(u16, String)> {
    let mut it = s.split_whitespace();
    let port = it.next()?.parse().ok()?;
    let token = it.next()?.to_string();
    Some((port, token))
}

/// Serve forwarded argv frames forever on a background thread; `on_args` is
/// called (on a worker thread) for each authenticated second launch.
pub fn serve(guard: &InstanceGuard, on_args: impl Fn(Vec<String>) + Send + 'static) {
    let Ok(listener) = guard.listener.try_clone() else { return };
    let token = Arc::new(guard.token.clone());
    let on_args = Arc::new(std::sync::Mutex::new(on_args));
    let busy = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if busy.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT {
                    busy.fetch_sub(1, Ordering::SeqCst);
                    continue; // dropped
                }
                let (token, on_args, held) = (token.clone(), on_args.clone(), busy.clone());
                let spawned = std::thread::Builder::new().name("single-instance-conn".into()).spawn(move || {
                    if let Some(args) = read_frame(stream).and_then(|f| authenticated_args(&f, &token)) {
                        (on_args.lock().unwrap_or_else(|p| p.into_inner()))(args);
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
fn read_frame(mut stream: TcpStream) -> Option<Vec<u8>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn primary(dir: &Path) -> (InstanceGuard, mpsc::Receiver<Vec<String>>) {
        let Acquire::Primary(g) = acquire(dir, &[]).unwrap() else { panic!("first must be primary") };
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        serve(&g, move |a| {
            let _ = tx.lock().unwrap().send(a);
        });
        (g, rx)
    }

    fn send_raw(g: &InstanceGuard, bytes: &[u8]) -> TcpStream {
        let mut s = TcpStream::connect(g.listener.local_addr().unwrap()).unwrap();
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
        let (_g, rx) = primary(dir.path());
        // a second lock attempt from the same process uses a new handle -> WouldBlock
        assert!(matches!(acquire(dir.path(), &["a.mrpack".into()]).unwrap(), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["a.mrpack".to_string()]);
    }

    /// Another local user can reach the port but not the token: ignored.
    #[test]
    fn frames_without_the_token_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (g, rx) = primary(dir.path());
        send_raw(&g, b"[\"/tmp/shared/evil.mrpack\"]\n");
        send_raw(&g, b"{\"token\":\"nope\",\"args\":[\"/tmp/shared/evil.mrpack\"]}\n");
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
        assert!(matches!(acquire(dir.path(), &["ok.mrpack".into()]).unwrap(), Acquire::Forwarded));
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
        let (g, rx) = primary(dir.path());
        let big = vec![b'x'; MAX_FRAME + 10];
        let _big = send_raw(&g, &big);
        let mut slow = send_raw(&g, b"{");
        let t = Instant::now();
        assert!(matches!(acquire(dir.path(), &["next.mrpack".into()]).unwrap(), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["next.mrpack".to_string()]);
        assert!(t.elapsed() < FRAME_DEADLINE, "served while the slow client is still open");
        let _ = slow.write_all(b" ");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "oversized frame ignored");
    }

    #[test]
    fn unicode_arguments_keep_the_server_alive() {
        let dir = tempfile::tempdir().unwrap();
        let (_g, rx) = primary(dir.path());
        assert!(matches!(acquire(dir.path(), &["éééé".into()]).unwrap(), Acquire::Forwarded));
        let got = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(!got.iter().any(|a| is_mrpack_arg(a)));
        assert!(matches!(acquire(dir.path(), &["b.mrpack".into()]).unwrap(), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["b.mrpack".to_string()]);
    }
}
