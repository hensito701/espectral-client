//! Single instance + .mrpack forwarding (replaces tauri-plugin-single-instance
//! and forward_mrpack_to_engine). FOUNDATION-OWNED.
//!
//! The first instance holds an exclusive lock on `<data_dir>/ui.lock` and
//! listens on a loopback TCP port written to `<data_dir>/ui.port`. A second
//! launch fails to take the lock, sends its argv (JSON array, one line) to
//! that port and exits; the first instance raises its window and imports any
//! `.mrpack` arguments.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

/// Held for the process lifetime (dropping it releases the lock).
pub struct InstanceGuard {
    _lock: File,
    pub listener: TcpListener,
}

pub enum Acquire {
    /// We are the primary instance.
    Primary(InstanceGuard),
    /// Another instance is running; our args were forwarded (or it did not answer).
    Forwarded,
}

/// `.mrpack` argument (case-insensitive suffix), e.g. a file-association double click.
pub fn is_mrpack_arg(arg: &str) -> bool {
    arg.len() >= 7 && arg[arg.len() - 7..].eq_ignore_ascii_case(".mrpack")
}

pub fn acquire(data_dir: &Path, args: &[String]) -> std::io::Result<Acquire> {
    std::fs::create_dir_all(data_dir)?;
    let lock_path = data_dir.join("ui.lock");
    let port_path = data_dir.join("ui.port");
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {
            let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
            std::fs::write(&port_path, listener.local_addr()?.port().to_string())?;
            Ok(Acquire::Primary(InstanceGuard { _lock: lock, listener }))
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            if let Ok(Ok(port)) = std::fs::read_to_string(&port_path).map(|s| s.trim().parse::<u16>()) {
                let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
                    let line = serde_json::to_string(args).unwrap_or_else(|_| "[]".into());
                    let _ = s.write_all(line.as_bytes());
                    let _ = s.write_all(b"\n");
                }
            }
            Ok(Acquire::Forwarded)
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Serve forwarded argv lines forever on a background thread; `on_args` is
/// called (on that thread) for each second launch.
pub fn serve(listener: TcpListener, on_args: impl Fn(Vec<String>) + Send + 'static) {
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok() {
                    let args: Vec<String> = serde_json::from_str(line.trim()).unwrap_or_default();
                    on_args(args);
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mrpack_suffix() {
        assert!(is_mrpack_arg("C:/x/Pack.MRPACK"));
        assert!(!is_mrpack_arg("pack.zip"));
        assert!(!is_mrpack_arg("x"));
    }

    #[test]
    fn second_instance_forwards() {
        let dir = tempfile::tempdir().unwrap();
        let Acquire::Primary(g) = acquire(dir.path(), &[]).unwrap() else { panic!("first must be primary") };
        let (tx, rx) = std::sync::mpsc::channel();
        serve(g.listener.try_clone().unwrap(), move |a| {
            let _ = tx.send(a);
        });
        // a second lock attempt from the same process uses a new handle -> WouldBlock
        assert!(matches!(acquire(dir.path(), &["a.mrpack".into()]).unwrap(), Acquire::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), vec!["a.mrpack".to_string()]);
    }
}
