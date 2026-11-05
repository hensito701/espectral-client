//! Launcher log: tracing to stderr (dev consoles) and to
//! `<data_dir>/logs/launcher.log`, plus a panic hook. FOUNDATION-OWNED.
//!
//! The Windows release build has no console (`windows_subsystem = "windows"`),
//! so without the file a launcher that fails to open leaves no trace at all.
//! The file is moved to `launcher.1.log` at start once it passes
//! [`MAX_LOG_BYTES`], so it never grows without bound.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Last panic on the main (UI) thread, for the error dialog.
static MAIN_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// Install the subscriber (`ESPECTRAL_LOG` filter, default `info`) and the
/// panic hook. `data_dir: None` (headless screenshots) logs to stderr only.
pub fn init(data_dir: Option<&Path>) {
    let filter = EnvFilter::try_from_env("ESPECTRAL_LOG").unwrap_or_else(|_| "info".into());
    let file_layer = data_dir.and_then(open_log_file).map(|(file, path)| {
        let _ = LOG_PATH.set(path);
        fmt::layer().with_ansi(false).with_writer(Mutex::new(file))
    });
    tracing_subscriber::registry().with(filter).with(fmt::layer().with_writer(std::io::stderr)).with(file_layer).init();
    install_panic_hook();
}

/// `<data_dir>/logs/launcher.log`, when the file log is active.
pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().map(PathBuf::as_path)
}

/// The message of the last panic on the main thread (taken).
pub fn take_main_panic() -> Option<String> {
    MAIN_PANIC.lock().unwrap_or_else(|p| p.into_inner()).take()
}

fn open_log_file(data_dir: &Path) -> Option<(File, PathBuf)> {
    espectral_engine::paths::ensure_private_dir(data_dir).ok()?;
    let dir = data_dir.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("launcher.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, dir.join("launcher.1.log"));
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path).ok()?;
    Some((file, path))
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        tracing::error!("panic on thread '{name}'{at}: {message}");
        if name == "main" {
            *MAIN_PANIC.lock().unwrap_or_else(|p| p.into_inner()) = Some(format!("{message}{at}"));
        }
        default_hook(info);
    }));
}
