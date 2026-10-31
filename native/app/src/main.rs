//! espectral-client — native launcher binary (Slint UI + in-process engine).
//!
//! Startup (replaces src-tauri/src/lib.rs `run()`):
//! 1. single instance: a second launch forwards its argv (.mrpack paths) to
//!    the running one and exits;
//! 2. tokio runtime + `Engine::start` (onboarding, cluster start hooks);
//! 3. the UI is built, every module wired, the window shown only once the
//!    engine is ready;
//! 4. on window close the event loop ends and the engine shuts down (games
//!    keep running).
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

slint::include_modules!();

mod backdrop;
mod bridge;
mod dialogs;
mod i18n;
mod pages;
mod palette;
mod prefs;
mod router;
mod screenshot;
mod session;
mod single_instance;
mod skin;
mod store;
mod theme;
mod toast;
mod util;

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

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("ESPECTRAL_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let shot = screenshot::ShotArgs::parse(&args);
    let data_dir = espectral_engine::paths::resolve_data_dir();

    // ---- single instance (not for screenshots) ----
    let guard = if shot.is_none() {
        match single_instance::acquire(&data_dir, &args) {
            Ok(single_instance::Acquire::Primary(g)) => Some(g),
            Ok(single_instance::Acquire::Forwarded) => {
                tracing::info!("another Espectral Client is running; arguments forwarded");
                return;
            }
            Err(e) => {
                tracing::warn!("single-instance lock unavailable: {e}");
                None
            }
        }
    } else {
        None
    };

    prefs::init(&data_dir);

    // ---- engine ----
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
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
                    &format!("El motor no pudo iniciarse / the engine failed to start:\n\n{e}"),
                );
            }
            std::process::exit(1);
        }
    };

    if let Some(s) = &shot {
        screenshot::install(s.width, s.height);
    }

    // ---- UI ----
    let ui = AppWindow::new().expect("failed to create the window");
    let ctx = Ctx::new(engine.clone(), rt.handle().clone(), ui.as_weak());

    toast::init(&ui);
    router::init(&ui);
    util::init(&ui);
    i18n::init(
        &ui,
        shot.as_ref().and_then(|s| s.lang.as_deref()).and_then(i18n::Lang::parse).unwrap_or_else(i18n::initial_lang),
    );
    theme::init(&ctx, &ui);
    backdrop::init(&ui);
    store::init(&ctx, &ui);
    session::init(&ctx, &ui, shot.as_ref().map(|s| !s.open.iter().any(|o| o == "gate")));
    palette::init(&ctx, &ui);
    skin::init(&ctx, &ui);
    ui.global::<Shell>().set_hide_hotkeys(prefs::get_bool("hide_hotkeys"));
    pages::init_all(&ctx, &ui);
    bridge::start_event_pump(&ctx);

    if let Some(s) = shot {
        run_screenshot(&ctx, &ui, s);
        return;
    }

    // .mrpack arguments of this (first) launch, and of later launches
    import_mrpacks(&ctx, args.iter().filter(|a| single_instance::is_mrpack_arg(a)).cloned().collect());
    if let Some(g) = guard.as_ref().and_then(|g| g.listener.try_clone().ok()) {
        let c = ctx.clone();
        single_instance::serve(g, move |argv| {
            let mrpacks: Vec<String> = argv.into_iter().filter(|a| single_instance::is_mrpack_arg(a)).collect();
            let c2 = c.clone();
            c.on_ui(move |ui| {
                let _ = ui.show(); // re-show / raise the window
                import_mrpacks(&c2, mrpacks);
            });
        });
    }

    router::navigate(&ui, "#/");
    ui.show().expect("failed to show the window");
    if let Ok(routes) = std::env::var("ESPECTRAL_TOUR") {
        let dwell = std::env::var("ESPECTRAL_TOUR_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
        let routes: Vec<String> = routes.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
        tour(ui.as_weak(), std::rc::Rc::new(routes), 0, Duration::from_millis(dwell));
    }
    if let Err(e) = slint::run_event_loop() {
        tracing::error!("event loop: {e}");
    }
    let _ = ui.hide();
    drop(ui);
    rt.block_on(async {
        let _ = tokio::time::timeout(Duration::from_secs(5), engine.shutdown()).await;
    });
    drop(guard);
}

/// Measurement hook (`ESPECTRAL_TOUR="#/servers,#/mods,..."`, `ESPECTRAL_TOUR_MS`):
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
