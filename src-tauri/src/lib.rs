// EspectralClient Tauri shell — library crate (Tauri v2 convention).
//
// The Node engine (`node src/engine/cli.mjs`) is spawned as a child process;
// the Tauri window loads the UI (Vite dev server in dev, bundled dist in
// prod). The UI talks to the engine over http://127.0.0.1:<port> (default
// 4199, ESPECTRAL_PORT-overridable) exactly as it
// does in the browser flow — the Tauri layer only owns the window and the
// engine process lifecycle.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::{Child, Command};

use parking_lot::Mutex;
use tauri::Manager;

/// Engine process handle (single instance; child is killed on app exit).
#[derive(Default)]
struct EngineState(Mutex<Option<Child>>);

/// Resolve the Node executable to spawn the engine with.
/// Order: ESPECTRAL_NODE env → bundled node.exe (resource dir) → `node` on
/// PATH → common Windows install dirs.
fn resolve_node(app: &tauri::AppHandle) -> String {
    if let Ok(n) = std::env::var("ESPECTRAL_NODE") {
        if !n.is_empty() {
            return n;
        }
    }
    // Bundled node.exe ships next to the engine in the resource dir (staged
    // by scripts/stage-tauri-resources.mjs). The portable exe keeps resources
    // beside the exe, so this probe covers both installed and portable
    // layouts; in dev the resource dir has no node.exe and we fall through.
    if let Some(res) = app.path().resource_dir().ok() {
        let bundled = res.join("node.exe");
        if bundled.exists() {
            return bundled.to_string_lossy().into_owned();
        }
    }
    let candidates = [
        r"C:\Program Files\nodejs\node.exe",
        r"C:\Program Files (x86)\nodejs\node.exe",
        r"%LOCALAPPDATA%\Programs\nodejs\node.exe",
    ];
    // Prefer `node` from PATH (Command's own lookup resolves it). Only when it
    // is NOT on PATH do the known install-dir candidates matter — previously
    // `|| c == "node"` short-circuited the loop on the first iteration, making
    // every hardcoded fallback unreachable dead code.
    let node_on_path = std::env::var_os("PATH").map_or(false, |p| {
        std::env::split_paths(&p).any(|d| d.join("node.exe").exists() || d.join("node").exists())
    });
    if node_on_path {
        return "node".into();
    }
    for c in candidates {
        let expanded = c.replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default());
        if std::path::Path::new(&expanded).exists() {
            return expanded;
        }
    }
    "node".into()
}

/// Spawn the engine process (no state access — callable from setup). Returns
/// the Child so the caller can stash it in EngineState.
fn spawn_engine(app: &tauri::AppHandle) -> Result<Child, String> {
    let node = resolve_node(app);
    let cwd = std::env::var("ESPECTRAL_ENGINE_CWD")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| {
            // Bundled app: the engine lives at <resource_dir>/engine/cli.mjs.
            app.path().resource_dir().ok().map(|d| d.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| ".".into());
    // Script path: dev layout is src/engine/cli.mjs from the repo root; the
    // bundle copies engine/cli.mjs directly into the resource dir.
    let script = if std::path::Path::new(&cwd).join("engine").join("cli.mjs").exists() {
        "engine/cli.mjs".into()
    } else {
        std::env::var("ESPECTRAL_ENGINE_SCRIPT").unwrap_or_else(|_| "src/engine/cli.mjs".into())
    };
    // Data dir: portable app writes next to the exe; installed apps use the
    // per-user local app data dir (Program Files is read-only).
    let data_dir = std::env::var("ESPECTRAL_DATA_DIR").ok().filter(|d| !d.is_empty()).or_else(|| {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_default();
        if !base.is_empty() {
            let d = std::path::Path::new(&base).join("espectral-client");
            let _ = std::fs::create_dir_all(&d);
            Some(d.to_string_lossy().into_owned())
        } else {
            None
        }
    });
    let port = resolve_port();

    let mut cmd = Command::new(node);
    cmd.arg(&script)
        .current_dir(&cwd)
        .env("ESPECTRAL_PORT", port.to_string());
    if let Some(d) = data_dir {
        cmd.env("ESPECTRAL_DATA_DIR", d);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // node.exe is a console-subsystem program. Spawned from a GUI-subsystem
        // process (this exe) it would otherwise get its own visible cmd-style
        // console window for every engine process — two such windows on launch
        // (setup + the UI's start_engine fallback) looked like a hack. Suppress
        // them: the engine's stdout/stderr are diagnostics only.
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.spawn().map_err(|e| format!("engine spawn failed: {e}"))
}

/// Resolve the engine port. Defaults to 4199; honors ESPECTRAL_PORT exactly
/// like the engine's cli.mjs (`Number(process.env.ESPECTRAL_PORT) || 4199`).
fn resolve_port() -> u16 {
    std::env::var("ESPECTRAL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(4199)
}

/// Poll the engine's /api/health until it answers (or timeout) so the UI's
/// first fetch never races the engine boot. Uses a raw TCP probe — the engine
/// is plain HTTP, so no extra HTTP client dependency is needed.
fn wait_for_engine(port: u16, timeout_ms: u64) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let addr = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if let Ok(mut sock) = TcpStream::connect_timeout(
            &addr.parse().unwrap(),
            std::time::Duration::from_millis(400),
        ) {
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(400)));
            let req = "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
            if sock.write_all(req.as_bytes()).is_ok() {
                let mut buf = [0u8; 64];
                if sock.read(&mut buf).is_ok() {
                    let head = String::from_utf8_lossy(&buf);
                    if head.contains("200") {
                        return true;
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    false
}

/// Reclaim the engine port: if some process is squatting on it without
/// answering /api/health (accepts TCP but never responds — e.g. an orphaned
/// engine from a killed/crashed app session, or a second app instance), kill
/// it so a fresh engine can bind. A healthy engine that answers health is
/// left alone (the caller reuses it). Best-effort: every failure is swallowed
/// — the spawn that follows will surface EADDRINUSE if reclaim failed.
fn reclaim_engine_port(port: u16) {
    // Healthy engine already answering? Nothing to reclaim.
    if wait_for_engine(port, 600) {
        return;
    }
    #[cfg(windows)]
    {
        // Find the PID LISTENING on the engine port via netstat -ano (last
        // column of the LISTENING row), then force-kill it ONLY if it is
        // verifiably our engine (node.exe running a cli.mjs script). Match any
        // local address that ends with :{port} — 127.0.0.1, 0.0.0.0 and [::]
        // all cover the loopback the engine binds.
        let port_suffix = format!(":{port}");
        let Ok(out) = Command::new("netstat").args(["-ano"]).output() else {
            return;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // TCP <local> <foreign> <state> <pid>
            if fields.len() < 5 || !fields[3].eq_ignore_ascii_case("LISTENING") {
                continue;
            }
            if !fields[1].ends_with(&port_suffix) {
                continue;
            }
            if let Ok(pid) = fields[4].parse::<u32>() {
                if pid == std::process::id() {
                    continue; // never kill ourselves
                }
                if !is_our_engine_process(pid) {
                    continue; // foreign process squatting the port — leave it
                }
                let _ = Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status();
            }
        }
    }
}

/// True when `pid` is verifiably one of our engine processes: a node.exe
/// whose command line runs an `engine/cli.mjs` (bundled layout) or
/// `src/engine/cli.mjs` (dev layout) script. Uses PowerShell/CIM — already
/// present on every Windows target, so no new dependency. Anything we cannot
/// verify (query failed, empty output) is reported as NOT ours: the caller
/// then leaves it alone and the following spawn surfaces EADDRINUSE.
#[cfg(windows)]
fn is_our_engine_process(pid: u32) -> bool {
    let query = format!(
        "(Get-CimInstance Win32_Process -Filter 'ProcessId={pid}') | ForEach-Object {{ $_.Name + '|' + $_.CommandLine }}"
    );
    let Ok(out) = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &query])
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let lower = line.to_ascii_lowercase();
    lower.starts_with("node.exe|") && lower.contains("cli.mjs")
}

/// A CLI argument worth forwarding to the engine as a modpack import: ends
/// with `.mrpack` (case-insensitive), e.g. a Windows file-association
/// double-click on a modpack file.
fn is_mrpack_arg(arg: &str) -> bool {
    const SUFFIX: &str = ".mrpack";
    arg.len() >= SUFFIX.len()
        && arg
            .get(arg.len() - SUFFIX.len()..)
            .map_or(false, |tail| tail.eq_ignore_ascii_case(SUFFIX))
}

/// Escape a string for embedding in a JSON string literal — quotes,
/// backslashes and control characters (enough for a file path).
fn json_escape(s: &str) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Best-effort HTTP POST of a .mrpack path to the running engine so a
/// double-click on a modpack file imports it into the launcher. Raw std
/// TcpStream — the engine is plain HTTP, so no HTTP client dependency is
/// needed. Every failure is swallowed: this is a convenience path and nothing
/// depends on it.
fn forward_mrpack_to_engine(path: &str) {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let port = resolve_port();
    let body = format!("{{\"path\":\"{}\"}}", json_escape(path));
    let req = format!(
        "POST /api/instances/import-mrpack HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body
    );
    let addr = format!("127.0.0.1:{port}");
    let Ok(mut sock) = TcpStream::connect_timeout(
        &addr.parse().unwrap(),
        std::time::Duration::from_millis(400),
    ) else {
        return;
    };
    let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(2000)));
    if sock.write_all(req.as_bytes()).is_err() {
        return;
    }
    // Read and discard the response; the socket closes on drop.
    let mut buf = [0u8; 512];
    loop {
        match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

/// Spawn `node src/engine/cli.mjs` (idempotent; used by the UI invoke path).
#[tauri::command]
fn start_engine(app: tauri::AppHandle) -> Result<(), String> {
    // Idempotent: never spawn a second engine. The setup path records the child
    // in EngineState before the window loads, so this is normally a no-op; the
    // health probe also guards a stale slot (e.g. a zombie engine from a
    // previous session still holding the engine port).
    if wait_for_engine(resolve_port(), 250) {
        return Ok(());
    }
    let state = app.state::<EngineState>();
    let mut guard = state.0.lock();
    if let Some(child) = guard.as_mut() {
        if child.try_wait().map_err(|e| format!("engine wait failed: {e}"))?.is_none() {
            return Ok(()); // already running
        }
    }
    // A stale process (orphaned engine from a killed/crashed session) may be
    // squatting on the port without answering health — kill it so the fresh
    // engine can actually bind, instead of dying on EADDRINUSE.
    reclaim_engine_port(resolve_port());
    let child = spawn_engine(&app)?;
    *guard = Some(child);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, args, _cwd| {
            // A second launch focuses the existing window; forward any .mrpack
            // arguments so the modpack import still happens.
            for arg in &args {
                if is_mrpack_arg(arg) {
                    forward_mrpack_to_engine(arg);
                }
            }
        }))
        // Shell plugin: ONLY for its `open` command — the Discord OAuth
        // authorize URL must open in the system browser (the WebView blocks
        // window.open popups). The capability file grants shell:default
        // (open with an http(s):// scope). Without this registration the
        // permission exists but the plugin:shell|open command does not.
        .plugin(tauri_plugin_shell::init())
        // Updater: signed auto-update against the latest.json endpoint in
        // tauri.conf.json (plugins.updater). The UI drives check/download via
        // @tauri-apps/plugin-updater; process plugin provides the relaunch
        // after an update installs.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(EngineState::default())
        .invoke_handler(tauri::generate_handler![start_engine])
        .setup(|app| {
            // Start the engine BEFORE showing the window: the UI's first
            // fetches hit the engine API, so the child must be up first (the
            // window is `visible: false` until health is OK). The window
            // content itself is the bundled dist served via Tauri's custom
            // protocol — not the engine.
            //
            // Spawn synchronously and record the child in EngineState before
            // setup returns: the webview starts loading immediately (even while
            // hidden) and App.svelte calls start_engine() on boot, so a delayed
            // spawn on a thread let the UI race it and spawn a SECOND engine —
            // the second visible console window on launch.
            let handle = app.handle().clone();
            // Reclaim the port first: a stale process from a killed/crashed
            // session that accepts TCP but never answers /api/health would
            // otherwise make the fresh engine die on EADDRINUSE, and every
            // UI request would hang forever ("failed to fetch everywhere").
            let port = resolve_port();
            reclaim_engine_port(port);
            match spawn_engine(&handle) {
                Ok(child) => {
                    if let Some(state) = handle.try_state::<EngineState>() {
                        *state.0.lock() = Some(child);
                    }
                }
                Err(e) => eprintln!("[espectral] engine spawn failed: {e}"),
            }
            // Health poll on a background thread; show the window once ready.
            std::thread::spawn(move || {
                let ready = wait_for_engine(port, 10_000);
                // First launch may carry .mrpack args (the file association can
                // start the app directly). Forward them now the engine is up —
                // best-effort, failures are ignored.
                for arg in std::env::args().skip(1) {
                    if is_mrpack_arg(&arg) {
                        forward_mrpack_to_engine(&arg);
                    }
                }
                if let Some(win) = handle.get_webview_window("main") {
                    let _ = win.show();
                    if !ready {
                        // Engine failed to boot — surface it in the window
                        // (the UI will show its own offline state).
                        eprintln!("[espectral] engine did not become ready within 10s");
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let app = window.app_handle();
                let state = app.state::<EngineState>();
                let mut guard = state.0.lock();
                if let Some(mut child) = guard.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
