//! Platform identity + small OS helpers shared by every cluster.
//!
//! The JS engine keyed several on-disk artefacts on Node's `process.platform`
//! / `process.arch` strings (AOT cache keys, Mojang rule evaluation), so the
//! exact Node spellings are reproduced here.

use std::path::Path;

/// Mojang rule OS name (resolver.osName): 'windows' | 'osx' | 'linux'.
pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Node `process.arch` spelling: 'x64' | 'arm64' | 'ia32' | 'arm' | ...
/// AOT cache keys hash this string, so it MUST match what Node reported.
pub fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        "arm" => "arm",
        "riscv64" => "riscv64",
        "powerpc64" => "ppc64",
        "s390x" => "s390x",
        other => other,
    }
}

/// Node `process.platform` spelling: 'win32' | 'darwin' | 'linux'.
pub fn node_platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// Total physical memory in bytes (Node `os.totalmem()`); 0 when unknown.
pub fn total_memory_bytes() -> u64 {
    imp::total_memory_bytes()
}

/// True when a process with `pid` exists (JS `process.kill(pid, 0)` probe).
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    imp::is_pid_alive(pid)
}

/// Open a folder (or file) in the OS file manager, detached.
/// JS: `spawn({win32:'explorer', darwin:'open', linux:'xdg-open'}[platform], [target])`.
pub fn open_path(target: &Path) -> std::io::Result<()> {
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    spawn_detached(opener, &[target.as_os_str()])
}

/// Open a URL in the default browser, detached.
/// JS: `cmd /c start "" <url>` | `open <url>` | `xdg-open <url>`.
pub fn open_url(url: &str) -> std::io::Result<()> {
    if cfg!(windows) {
        spawn_detached("cmd", &["/c".as_ref(), "start".as_ref(), "".as_ref(), url.as_ref()])
    } else if cfg!(target_os = "macos") {
        spawn_detached("open", &[url.as_ref()])
    } else {
        spawn_detached("xdg-open", &[url.as_ref()])
    }
}

fn spawn_detached(cmd: &str, args: &[&std::ffi::OsStr]) -> std::io::Result<()> {
    let mut c = std::process::Command::new(cmd);
    c.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        c.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    let child = c.spawn()?;
    // Reap in the background so no zombie is left on unix.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// ISO-8601 timestamp with millisecond precision and `Z` (JS `new Date().toISOString()`).
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// ms since the epoch (JS `Date.now()`).
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// File mtime as fractional ms since the epoch (Node `stat.mtimeMs`).
pub fn mtime_ms(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[cfg(unix)]
mod imp {
    pub fn total_memory_bytes() -> u64 {
        #[cfg(target_os = "linux")]
        {
            if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
                for line in s.lines() {
                    if let Some(rest) = line.strip_prefix("MemTotal:") {
                        let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
                        return kb * 1024;
                    }
                }
            }
        }
        // SAFETY: sysconf is always safe to call.
        let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if pages > 0 && page > 0 {
            (pages as u64) * (page as u64)
        } else {
            0
        }
    }

    pub fn is_pid_alive(pid: u32) -> bool {
        // SAFETY: signal 0 performs only the existence/permission check.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if r == 0 {
            return true;
        }
        // EPERM = exists but owned by someone else.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    pub fn total_memory_bytes() -> u64 {
        // SAFETY: MEMORYSTATUSEX is plain data; dwLength must be set first.
        unsafe {
            let mut st: MEMORYSTATUSEX = std::mem::zeroed();
            st.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut st) != 0 {
                st.ullTotalPhys
            } else {
                0
            }
        }
    }

    pub fn is_pid_alive(pid: u32) -> bool {
        // SAFETY: handle is checked and closed.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE as u32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_spellings() {
        assert!(["windows", "osx", "linux"].contains(&os_name()));
        if cfg!(target_arch = "x86_64") {
            assert_eq!(node_arch(), "x64");
        }
    }

    #[test]
    fn this_process_is_alive() {
        assert!(is_pid_alive(std::process::id()));
        assert!(!is_pid_alive(0));
    }

    #[test]
    fn memory_is_known() {
        assert!(total_memory_bytes() > 0);
    }

    #[test]
    fn iso_has_millis_and_z() {
        let s = now_iso();
        assert!(s.ends_with('Z'));
        assert_eq!(s.len(), "2026-10-30T12:00:00.000Z".len());
    }
}
