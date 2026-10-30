//! Child-process plumbing shared by game launches (launch.rs) and AOT
//! training runs (aot.rs): spawning the JVM, draining + relaying its output,
//! exit observation, and the OS-level stop / close helpers.
//!
//! JS equivalents: `spawnJava` (argv[0] dropped), Node's `detached: true` +
//! `windowsHide: true`, `child.kill('SIGTERM'|'SIGKILL')`, `taskkill` calls
//! in routes/launch.mjs (`forceKillProcess`) and aot.mjs
//! (`closeWindowGracefully`).

use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;

/// `CREATE_NO_WINDOW` (JS `windowsHide: true` for helpers / the trainer).
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// How a JVM process ended (Node `exit` event arguments + `error`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExitStatusInfo {
    pub code: Option<i32>,
    /// Node-style signal name (`SIGKILL`, …) when killed by a signal (unix).
    pub signal: Option<String>,
    /// Waiting on the process failed (JS `error` event message).
    pub error: Option<String>,
}

/// A spawned child: pid + an observable exit status. Cheap to clone.
#[derive(Debug, Clone)]
pub struct ProcessHandle {
    pid: u32,
    rx: watch::Receiver<Option<ExitStatusInfo>>,
}

impl ProcessHandle {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Exit status once the process has ended (JS `exitCode` / `signalCode`).
    pub fn exit_status(&self) -> Option<ExitStatusInfo> {
        self.rx.borrow().clone()
    }

    pub fn has_exited(&self) -> bool {
        self.rx.borrow().is_some()
    }

    /// Wait up to `timeout` for the process to end; true when it did.
    pub async fn wait_timeout(&self, timeout: Duration) -> bool {
        if self.has_exited() {
            return true;
        }
        let mut rx = self.rx.clone();
        // A dropped sender (cannot happen: the waiter always sends) counts as
        // ended so callers never hang.
        let r = tokio::time::timeout(timeout, async {
            let _ = rx.wait_for(|s| s.is_some()).await;
        })
        .await;
        r.is_ok()
    }
}

/// Line sink for the child's stdout/stderr (each call = one line, newline kept).
pub type LineSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Build a command for `program` with the JS spawn options.
fn base_command(program: &str, args: &[String], cwd: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd
}

/// Spawn java with the FULL argv as built by `build_argv` (argv[0] = the java
/// path). The OS prepends the executable itself, so argv[0] is dropped here
/// on every platform (JS spawnJava, H5).
///
/// `detached`: the game must outlive the launcher. Unix: new session
/// (`setsid`, Node `detached`). Windows: `DETACHED_PROCESS |
/// CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`, first trying to break away
/// from a kill-on-close job the launcher may run in. Non-detached children
/// (the AOT trainer) are killed when their monitor is dropped (the JS trainer
/// died with the launcher too).
pub fn spawn_java(java_path: &str, argv: &[String], cwd: &Path, detached: bool) -> std::io::Result<Child> {
    let args = argv.get(1..).unwrap_or(&[]);
    #[cfg(windows)]
    {
        let flags =
            if detached { DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW } else { CREATE_NO_WINDOW };
        if detached {
            let mut cmd = base_command(java_path, args, cwd);
            cmd.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB).kill_on_drop(false);
            if let Ok(child) = cmd.spawn() {
                return Ok(child);
            }
        }
        let mut cmd = base_command(java_path, args, cwd);
        cmd.creation_flags(flags).kill_on_drop(!detached);
        cmd.spawn()
    }
    #[cfg(unix)]
    {
        let mut cmd = base_command(java_path, args, cwd);
        cmd.kill_on_drop(!detached);
        if detached {
            // SAFETY: setsid() is async-signal-safe and touches no Rust state.
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        cmd.spawn()
    }
}

async fn pump<R: AsyncRead + Unpin>(reader: R, sink: Option<LineSink>) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::with_capacity(1024);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                // Relay to the launcher's stdout (JS process.stdout.write); a
                // closed stdout (GUI build) is ignored.
                let _ = std::io::stdout().write_all(&buf);
                if let Some(s) = &sink {
                    s(&String::from_utf8_lossy(&buf));
                }
            }
        }
    }
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    match sig {
        1 => "SIGHUP".into(),
        2 => "SIGINT".into(),
        3 => "SIGQUIT".into(),
        4 => "SIGILL".into(),
        6 => "SIGABRT".into(),
        7 => "SIGBUS".into(),
        8 => "SIGFPE".into(),
        9 => "SIGKILL".into(),
        11 => "SIGSEGV".into(),
        13 => "SIGPIPE".into(),
        14 => "SIGALRM".into(),
        15 => "SIGTERM".into(),
        n => format!("SIG{n}"),
    }
}

fn exit_info(status: std::io::Result<std::process::ExitStatus>) -> ExitStatusInfo {
    match status {
        Ok(st) => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                ExitStatusInfo { code: st.code(), signal: st.signal().map(signal_name), error: None }
            }
            #[cfg(not(unix))]
            {
                ExitStatusInfo { code: st.code(), signal: None, error: None }
            }
        }
        Err(e) => ExitStatusInfo { code: None, signal: None, error: Some(e.to_string()) },
    }
}

/// Drain the child's stdout/stderr (relayed + `on_line`), wait for it, then
/// publish the exit status and call `on_exit` exactly once. The returned
/// handle observes the same exit status.
pub fn monitor(
    mut child: Child,
    on_line: Option<LineSink>,
    on_exit: impl FnOnce(ExitStatusInfo) + Send + 'static,
) -> ProcessHandle {
    let pid = child.id().unwrap_or(0);
    let (tx, rx) = watch::channel(None);
    let out = child.stdout.take().map(|s| tokio::spawn(pump(s, on_line.clone())));
    let err = child.stderr.take().map(|s| tokio::spawn(pump(s, on_line.clone())));
    tokio::spawn(async move {
        let status = child.wait().await;
        // 'close' semantics: let the pipes drain so the last lines precede the
        // exit summary — bounded, a grandchild may keep the pipe open.
        for h in [out, err].into_iter().flatten() {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }
        let info = exit_info(status);
        let _ = tx.send(Some(info.clone()));
        on_exit(info);
    });
    ProcessHandle { pid, rx }
}

/// A hidden helper command (taskkill, powershell): `CREATE_NO_WINDOW` on Windows.
pub fn hidden_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Node `child.kill('SIGTERM')`: SIGTERM on unix, TerminateProcess on Windows.
pub fn terminate(pid: u32) -> bool {
    imp::terminate(pid)
}

/// Node `child.kill('SIGKILL')` / `process.kill(pid, 'SIGKILL')`.
pub fn kill_hard(pid: u32) -> bool {
    imp::kill_hard(pid)
}

/// routes/launch.mjs forceKillProcess: Windows `taskkill /pid <pid> /T /F`
/// (whole tree), elsewhere SIGKILL.
pub async fn force_kill_tree(pid: u32) {
    if cfg!(windows) {
        let _ = hidden_command("taskkill").args(["/pid", &pid.to_string(), "/T", "/F"]).status().await;
    } else {
        kill_hard(pid);
    }
}

/// aot.mjs closeWindowGracefully: ask the game to close (WM_CLOSE to every
/// top-level window via `taskkill /PID` WITHOUT /F on Windows; SIGTERM
/// elsewhere). Never force-kills: the AOT cache is written on normal exit only.
pub async fn close_gracefully(pid: u32) -> bool {
    if cfg!(windows) {
        matches!(hidden_command("taskkill").args(["/PID", &pid.to_string()]).status().await, Ok(s) if s.success())
    } else {
        imp::terminate(pid)
    }
}

#[cfg(unix)]
mod imp {
    fn signal(pid: u32, sig: i32) -> bool {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: plain kill(2) on a positive pid.
        unsafe { libc::kill(pid as libc::pid_t, sig) == 0 }
    }
    pub fn terminate(pid: u32) -> bool {
        signal(pid, libc::SIGTERM)
    }
    pub fn kill_hard(pid: u32) -> bool {
        signal(pid, libc::SIGKILL)
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    pub fn terminate(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        // SAFETY: the handle is checked and closed.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 1) != 0;
            CloseHandle(h);
            ok
        }
    }
    pub fn kill_hard(pid: u32) -> bool {
        terminate(pid)
    }
}
