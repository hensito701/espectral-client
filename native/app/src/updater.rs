//! In-app updater UI (port of lib/updater.svelte.ts) on top of
//! `espectral_engine::updater`.
//!
//! Flow: first check 30 s after start (off the boot path), then a forced
//! re-check every 6 h so a release that drops mid-session is noticed. When the
//! feed offers a newer signed version the top bar shows the CTA
//! (`Shell.update-*`); on click: download with progress → signature check →
//! install. Windows hands over to the NSIS installer (it closes this process
//! if needed, replaces the exe and relaunches it); Linux installs the .deb with
//! pkexec, then the app exits and starts the new binary
//! ([`relaunch_if_requested`], called by main after the engine shut down).
//! Every failure lands back in a usable state — the updater never blocks the
//! launcher. Copies that were not installed by the installer / .deb (dev
//! builds, portable exes) never check, unless `ESPECTRAL_UPDATE_ENDPOINT`
//! points at a test feed.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use espectral_engine::updater::{self, Installed, Update, Updater};
use slint::ComponentHandle;

use crate::bridge::Ctx;
use crate::{i18n, toast, AppWindow, Shell};

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const RECHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Visible states (the `Shell.update-status` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Idle,
    Available,
    Downloading,
    Installing,
    Ready,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Available => "available",
            Status::Downloading => "downloading",
            Status::Installing => "installing",
            Status::Ready => "ready",
        }
    }
}

struct State {
    status: Status,
    update: Option<Update>,
    checking: bool,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { status: Status::Idle, update: None, checking: false }) };
    static RECHECK: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}

/// Set by a successful Linux install: main starts this binary after exiting.
static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn set_status(ui: &AppWindow, status: Status) {
    STATE.with(|s| s.borrow_mut().status = status);
    ui.global::<Shell>().set_update_status(status.as_str().into());
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let up = Updater::new(ctx.engine().http());
    if up.installer().is_none() && !up.is_test_feed() {
        tracing::info!("updater: not an installed copy, update checks are off");
        return;
    }
    tracing::info!(endpoint = up.endpoint(), installer = ?up.installer(), "updater armed");
    let up = Arc::new(up);

    let (c, u) = (ctx.clone(), up.clone());
    ui.global::<Shell>().on_install_update(move || install(&c, &u));

    let (c, u) = (ctx.clone(), up.clone());
    slint::Timer::single_shot(FIRST_CHECK_DELAY, move || check(&c, &u));
    let timer = slint::Timer::default();
    let (c, u) = (ctx.clone(), up);
    timer.start(slint::TimerMode::Repeated, RECHECK_INTERVAL, move || check(&c, &u));
    RECHECK.with(|t| *t.borrow_mut() = Some(timer));
}

/// Ask the feed. Runs from idle or available (a re-check never interrupts a
/// download/install); offline / endpoint down are non-events.
fn check(ctx: &Ctx, up: &Arc<Updater>) {
    let go = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let ok = !s.checking && matches!(s.status, Status::Idle | Status::Available);
        s.checking |= ok;
        ok
    });
    if !go {
        return;
    }
    let up = up.clone();
    let c = ctx.clone();
    ctx.spawn(async move {
        let res = up.check().await;
        c.on_ui(move |ui| {
            STATE.with(|s| s.borrow_mut().checking = false);
            // a click may have started an install meanwhile
            if !matches!(STATE.with(|s| s.borrow().status), Status::Idle | Status::Available) {
                return;
            }
            match res {
                Ok(Some(update)) => {
                    tracing::info!(version = %update.version, "update available");
                    ui.global::<Shell>().set_update_version(update.version.as_str().into());
                    STATE.with(|s| s.borrow_mut().update = Some(update));
                    set_status(ui, Status::Available);
                }
                Ok(None) => {
                    STATE.with(|s| s.borrow_mut().update = None);
                    set_status(ui, Status::Idle);
                }
                Err(e) => tracing::info!(error = %e, "update check failed"),
            }
        });
    });
}

/// Download, verify, install — exactly one installer run.
fn install(ctx: &Ctx, up: &Arc<Updater>) {
    let Some(update) = STATE.with(|s| {
        let s = s.borrow();
        (s.status == Status::Available).then(|| s.update.clone()).flatten()
    }) else {
        return;
    };
    let Some(ui) = ctx.ui() else { return };
    let Some(installer) = up.installer() else {
        toast::err(i18n::tf("update.failed", &[("error", &i18n::t("update.notInstalled"))]));
        return;
    };
    ui.global::<Shell>().set_update_progress(0.0);
    set_status(&ui, Status::Downloading);

    let up = up.clone();
    let c = ctx.clone();
    ctx.spawn(async move {
        let mut last_pct = 0u64;
        let progress = c.clone();
        let bytes = up
            .download(&update, move |received, total| {
                let Some(total) = total.filter(|t| *t > 0) else { return };
                let pct = (received * 100 / total).min(100);
                if pct != last_pct {
                    last_pct = pct;
                    progress.on_ui(move |ui| ui.global::<Shell>().set_update_progress(pct as f32 / 100.0));
                }
            })
            .await;
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => return failed(&c, e.to_string()),
        };
        c.on_ui(|ui| {
            ui.global::<Shell>().set_update_progress(1.0);
            set_status(ui, Status::Installing);
        });
        let version = update.version.clone();
        let res = tokio::task::spawn_blocking(move || updater::install(&update, installer, &bytes)).await;
        match res {
            Ok(Ok(done)) => {
                tracing::info!(%version, ?done, "update installed; exiting");
                if let Installed::RelaunchAfterExit(exe) = done {
                    *RELAUNCH.lock().unwrap_or_else(|p| p.into_inner()) = Some(exe);
                }
                c.on_ui(|ui| {
                    set_status(ui, Status::Ready);
                    let _ = slint::quit_event_loop();
                });
            }
            Ok(Err(e)) => failed(&c, e.to_string()),
            Err(e) => failed(&c, e.to_string()),
        }
    });
}

fn failed(ctx: &Ctx, error: String) {
    tracing::warn!(%error, "update failed");
    ctx.on_ui(move |ui| {
        set_status(ui, Status::Available);
        toast::err(i18n::tf("update.failed", &[("error", &error)]));
    });
}

/// After a Linux .deb install: start the new binary once this process let go
/// of the single-instance lock (call it last in main).
pub fn relaunch_if_requested() {
    let Some(exe) = RELAUNCH.lock().unwrap_or_else(|p| p.into_inner()).take() else { return };
    if let Err(e) = std::process::Command::new(&exe).spawn() {
        tracing::error!(exe = %exe.display(), error = %e, "relaunch after update failed");
    }
}
