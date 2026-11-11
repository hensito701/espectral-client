//! espectral-client — native launcher binary (Slint UI + in-process engine).
//!
//! Startup (replaces src-tauri/src/lib.rs `run()`):
//! 1. log file (`<data>/logs/launcher.log`, see logging.rs);
//! 2. single instance: a second launch hands its argv (.mrpack paths) to the
//!    running one and exits — or waits for a closing one, or offers to close
//!    a stuck one (single_instance.rs);
//! 3. renderer: OpenGL, or the software renderer on a low-memory PC or where
//!    OpenGL failed (renderer.rs);
//! 4. tokio runtime + `Engine::start` (onboarding, cluster start hooks);
//! 5. the UI is built, every module wired, the window shown only once the
//!    engine is ready;
//! 6. on window close the event loop ends and the engine shuts down (games
//!    keep running); after an in-app update the new binary is started last.
//!    A window that fails restarts the launcher on the software renderer; a
//!    panic on the UI thread ends in an error dialog naming the log file.
//!
//! `--screenshot <route> <out.png> [opts]` renders one route headless and
//! exits (see screenshot.rs, scripts/screenshot.sh).
//!
//! `--mcp` runs the stdio MCP server instead of the UI (agent access, see
//! `espectral_engine::mcp`): it is dispatched before single-instance, tokio
//! and Slint so the MCP client talks to a bare JSON-RPC process. The release
//! build's `windows_subsystem = "windows"` only hides a console; the client's
//! stdin/stdout pipes are inherited and carry the frames.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// The UI (AppWindow, globals, structs) comes from the espectral-client-ui
// crate, kept separate so edits here do not recompile the generated module.
pub use espectral_client_ui::*;

mod bridge;
mod dialogs;
mod i18n;
mod logging;
mod pages;
mod palette;
mod prefs;
mod renderer;
mod router;
mod screenshot;
mod session;
mod single_instance;
mod skin;
mod store;
mod theme;
mod toast;
mod updater;
mod util;

use std::path::Path;
use std::time::Duration;

use espectral_engine::api::content::ImportMrpackRequest;
use espectral_engine::{Engine, EngineOptions};
use slint::ComponentHandle;

use crate::bridge::Ctx;

fn main() {
    // MCP stdio server (`espectral-client --mcp`): before single-instance,
    // tokio and Slint — it must own stdin/stdout and exit with its own status.
    if std::env::args_os().skip(1).any(|a| a == espectral_engine::api::agent::MCP_ARG) {
        std::process::exit(espectral_engine::mcp::run_stdio());
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let shot = screenshot::ShotArgs::parse(&args);
    let data_dir = espectral_engine::paths::resolve_data_dir();
    logging::init(shot.is_none().then_some(data_dir.as_path()));

    // ---- single instance (not for screenshots) ----
    let guard = if shot.is_none() {
        tracing::info!("Espectral Client {} starting (pid {})", env!("CARGO_PKG_VERSION"), std::process::id());
        match claim_instance(&data_dir, &args) {
            Claim::Primary(g) => Some(g),
            Claim::Unguarded => None,
            Claim::Exit => return,
        }
    } else {
        None
    };

    prefs::init(&data_dir);
    let renderer = shot.is_none().then(renderer::select);

    // ---- engine ----
    // Size the runtime to the machine: two workers on a dual-core, four on a
    // modern CPU; more threads would only contend with the game for cores.
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(2, 4);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .thread_name("espectral-rt")
        .enable_all()
        .build()
        .expect("tokio runtime");
    let opts = EngineOptions {
        data_dir: Some(data_dir.clone()),
        start_background_tasks: shot.is_none(),
        ..EngineOptions::default()
    };
    let engine = match rt.block_on(Engine::start(opts)) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("engine failed to start: {e}");
            if shot.is_none() {
                dialogs::fatal(
                    "Espectral Client",
                    &format!("El motor no pudo iniciarse / the engine failed to start:\n\n{e}{}", log_hint()),
                );
            }
            std::process::exit(1);
        }
    };

    if let Some(s) = &shot {
        screenshot::install(s.width, s.height);
    }

    // ---- UI ----
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_ui(&engine, &rt, &args, shot, guard.as_ref(), renderer)
    }))
    .unwrap_or_else(|_| {
        Some(UiFailure::Panic(logging::take_main_panic().unwrap_or_else(|| "unknown error".to_string())))
    });
    // From here on a second launch waits for the lock instead of handing
    // itself to a window that is gone.
    if let Some(g) = &guard {
        g.closing();
    }
    rt.block_on(async {
        let _ = tokio::time::timeout(Duration::from_secs(5), engine.shutdown()).await;
    });
    drop(guard);
    match failure {
        None => updater::relaunch_if_requested(),
        Some(failure) => ui_failed(failure, renderer),
    }
}

/// How long the UI thread may take to raise the window for a second launch
/// (below the second launch's own reply timeout in single_instance.rs).
const SECOND_LAUNCH_TIMEOUT: Duration = Duration::from_secs(3);

const STUCK_QUESTION: &str = "Espectral Client ya está abierto, pero no responde.\n\
¿Cerrarlo y volver a abrirlo? Si estás jugando, Minecraft sigue abierto.\n\n\
Espectral Client is already open but not responding.\n\
Close it and open it again? A running Minecraft keeps running.";

const STUCK_MANUAL: &str = "Espectral Client ya está abierto, pero no responde. \
Ciérralo en el Administrador de tareas (espectral-client.exe) y vuelve a abrirlo.\n\n\
Espectral Client is already open but not responding. \
End espectral-client.exe in Task Manager and open it again.";

enum Claim {
    Primary(single_instance::InstanceGuard),
    /// The lock file is unusable: run without single-instance protection.
    Unguarded,
    /// The running launcher took this launch, or the user said no.
    Exit,
}

/// Become the first instance or hand this launch to the running one. A
/// launcher that holds the lock but never answers is offered to be closed
/// (once) instead of swallowing every launch.
fn claim_instance(data_dir: &Path, args: &[String]) -> Claim {
    let mut offered = false;
    loop {
        match single_instance::acquire(data_dir, args) {
            Ok(single_instance::Acquire::Primary(g)) => return Claim::Primary(g),
            Ok(single_instance::Acquire::Forwarded) => {
                tracing::info!("another Espectral Client is running; this launch was handed to it");
                return Claim::Exit;
            }
            Ok(single_instance::Acquire::Unresponsive { pid }) => {
                tracing::warn!("the running Espectral Client (pid {pid:?}) is not responding");
                if let Some(pid) = pid.filter(|_| !offered) {
                    offered = true;
                    if !dialogs::confirm("Espectral Client", STUCK_QUESTION) {
                        return Claim::Exit;
                    }
                    if single_instance::kill_stuck_instance(pid) {
                        tracing::warn!("closed the unresponsive Espectral Client (pid {pid})");
                        continue;
                    }
                    tracing::error!("could not close the unresponsive Espectral Client (pid {pid})");
                }
                dialogs::fatal("Espectral Client", STUCK_MANUAL);
                return Claim::Exit;
            }
            Err(e) => {
                tracing::warn!("single-instance lock unavailable: {e}");
                return Claim::Unguarded;
            }
        }
    }
}

/// Why the window could not run.
enum UiFailure {
    /// The window system / renderer failed (no window, lost context, …).
    Window(String),
    /// A panic on the UI thread.
    Panic(String),
}

/// Build the UI, show the window and run the event loop until it ends.
/// `Some` = the window failed.
fn run_ui(
    engine: &Engine,
    rt: &tokio::runtime::Runtime,
    args: &[String],
    shot: Option<screenshot::ShotArgs>,
    guard: Option<&single_instance::InstanceGuard>,
    renderer: Option<renderer::Renderer>,
) -> Option<UiFailure> {
    let ui = match AppWindow::new() {
        Ok(ui) => ui,
        Err(e) => return Some(UiFailure::Window(format!("creating the window: {e}"))),
    };
    let ctx = Ctx::new(engine.clone(), rt.handle().clone(), ui.as_weak());

    toast::init(&ui);
    router::init(&ui);
    util::init(&ui);
    i18n::init(
        &ui,
        shot.as_ref().and_then(|s| s.lang.as_deref()).and_then(i18n::Lang::parse).unwrap_or_else(i18n::initial_lang),
    );
    theme::init(&ctx, &ui);
    store::init(&ctx, &ui);
    session::init(&ctx, &ui, shot.as_ref().map(|s| !s.open.iter().any(|o| o == "gate")));
    palette::init(&ctx, &ui);
    skin::init(&ctx, &ui);
    ui.global::<Shell>().set_hide_hotkeys(prefs::get_bool("hide_hotkeys"));
    pages::init_all(&ctx, &ui);
    bridge::start_event_pump(&ctx);

    if let Some(s) = shot {
        run_screenshot(&ctx, &ui, s);
        return None;
    }

    // .mrpack arguments of this (first) launch, of launches that arrived
    // while it was starting, and of later launches.
    let mut mrpacks: Vec<String> = args.iter().filter(|a| single_instance::is_mrpack_arg(a)).cloned().collect();
    if let Some(g) = guard {
        let queued = g.ready(take_second_launch(&ctx));
        mrpacks.extend(queued.into_iter().flatten().filter(|a| single_instance::is_mrpack_arg(a)));
    }
    import_mrpacks(&ctx, mrpacks);
    updater::init(&ctx, &ui);

    router::navigate(&ui, "#/");
    if renderer.is_some_and(|r| r.can_fall_back) {
        renderer::watch_first_frame(&ui);
    }
    if let Err(e) = ui.show() {
        return Some(UiFailure::Window(format!("showing the window: {e}")));
    }
    if let Ok(routes) = std::env::var("ESPECTRAL_TOUR") {
        let dwell = std::env::var("ESPECTRAL_TOUR_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
        let routes: Vec<String> = routes.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
        tour(ui.as_weak(), std::rc::Rc::new(routes), 0, Duration::from_millis(dwell));
    }
    let result = slint::run_event_loop();
    let _ = ui.hide();
    result.err().map(|e| UiFailure::Window(e.to_string()))
}

/// A second launch while the window runs: raise the window and import its
/// .mrpack arguments on the UI thread. False when the UI thread does not
/// answer in time, so the second launch can report a stuck launcher.
fn take_second_launch(ctx: &Ctx) -> impl FnMut(Vec<String>) -> bool + Send + 'static {
    let ctx = ctx.clone();
    move |argv| {
        let mrpacks: Vec<String> = argv.into_iter().filter(|a| single_instance::is_mrpack_arg(a)).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        let (c, weak) = (ctx.clone(), ctx.weak());
        let posted = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                raise_window(&ui);
                import_mrpacks(&c, mrpacks);
                let _ = tx.send(());
            }
        });
        posted.is_ok() && rx.recv_timeout(SECOND_LAUNCH_TIMEOUT).is_ok()
    }
}

/// Show the window again, restore it when minimized and bring it forward.
fn raise_window(ui: &AppWindow) {
    use slint::winit_030::WinitWindowAccessor;
    let _ = ui.show();
    let window = ui.window();
    if window.is_minimized() {
        window.set_minimized(false);
    }
    window.with_winit_window(|w| w.focus_window());
}

/// The window failed: on OpenGL, switch this PC to the software renderer and
/// start again; otherwise say what happened and where the log is.
fn ui_failed(failure: UiFailure, renderer: Option<renderer::Renderer>) {
    let message = match failure {
        UiFailure::Window(err) => {
            tracing::error!("the window failed: {err}");
            if renderer.is_some_and(|r| r.can_fall_back) {
                renderer::remember_software(&err);
                match renderer::relaunch_software() {
                    Ok(()) => {
                        tracing::info!("restarting on the software renderer");
                        return;
                    }
                    Err(e) => tracing::error!("could not restart the launcher: {e}"),
                }
            }
            format!(
                "No se pudo abrir la ventana de Espectral Client.\nThe Espectral Client window could not be opened.\n\n{err}{}",
                log_hint()
            )
        }
        UiFailure::Panic(msg) => format!(
            "Espectral Client se cerró por un error inesperado.\nEspectral Client closed after an unexpected error.\n\n{msg}{}",
            log_hint()
        ),
    };
    dialogs::fatal("Espectral Client", &message);
    std::process::exit(1);
}

/// `\n\nRegistro / log: <path>` when the log file is active.
fn log_hint() -> String {
    logging::log_path().map(|p| format!("\n\nRegistro / log: {}", p.display())).unwrap_or_default()
}

/// Measurement hook (`ESPECTRAL_TOUR="#/mods,#/account,..."`, `ESPECTRAL_TOUR_MS`):
/// visit each route in turn on the real windowed app (memory / CPU probes,
/// UI-PORTING.md "Measurements"). Stays on the last route.
fn tour(weak: slint::Weak<AppWindow>, routes: std::rc::Rc<Vec<String>>, i: usize, dwell: Duration) {
    if i >= routes.len() {
        return;
    }
    slint::Timer::single_shot(dwell, move || {
        let Some(ui) = weak.upgrade() else { return };
        router::navigate(&ui, &routes[i]);
        tour(weak, routes, i + 1, dwell);
    });
}

fn import_mrpacks(ctx: &Ctx, paths: Vec<String>) {
    for path in paths {
        // Progress + outcome arrive as import-progress / import-done events (store.rs toasts them).
        ctx.fire(move |e| async move { e.import_mrpack(ImportMrpackRequest { path, memory_mb: None }).await });
    }
}

fn run_screenshot(ctx: &Ctx, ui: &AppWindow, s: screenshot::ShotArgs) {
    if let Some(t) = &s.theme {
        let accent = s.accent.clone().unwrap_or_else(|| ctx.engine().config().accent());
        theme::apply(ui, t, &accent);
    } else if let Some(a) = &s.accent {
        ui.global::<Theme>().set_accent_id(a.as_str().into());
    }
    ui.show().expect("show");
    router::navigate(ui, &s.route);
    let shell = ui.global::<Shell>();
    for o in &s.open {
        match o.as_str() {
            "palette" => shell.set_palette_open(true),
            "wizard" => shell.set_wizard_open(true),
            "account" => shell.set_account_popover_open(true),
            "pip" => shell.set_pip_log_open(true),
            _ => {}
        }
    }
    screenshot::pump(Duration::from_millis(s.wait_ms / 2));
    if let Some(q) = &s.query {
        ui.global::<PaletteState>().set_query(q.as_str().into());
        ui.global::<PaletteState>().invoke_query_edited(q.as_str().into());
    }
    for a in &s.actions {
        if let screenshot::Action::Nav(route) = a {
            router::navigate(ui, route);
        }
        screenshot::dispatch(a);
    }
    if s.scroll > 0.0 {
        shell.set_debug_scroll(s.scroll);
    }
    screenshot::pump(Duration::from_millis(s.wait_ms - s.wait_ms / 2));
    match screenshot::save_png(&s.out) {
        Ok(()) => println!("{}", s.out.display()),
        Err(e) => {
            eprintln!("screenshot failed: {e}");
            std::process::exit(2);
        }
    }
    // background tasks are off in screenshot mode; skip the graceful shutdown
    std::process::exit(0);
}
