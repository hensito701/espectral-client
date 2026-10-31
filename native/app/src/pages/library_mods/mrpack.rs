//! MrpackModal (components/MrpackModal.svelte): pick a `.mrpack`, choose the
//! memory (automatic until the slider moves), import with live progress.
//! Hosted by any page that places `MrpackModal {}`; opened through
//! `LibraryBridge.mrpack-open`.

use std::cell::RefCell;
use std::time::Duration;

use espectral_engine::api::content::ImportMrpackRequest;
use espectral_engine::events::EngineEvent;
use slint::{ComponentHandle, Timer, TimerMode};

use super::import::{DONE, ERROR, FILES, OVERRIDES, POST};
use super::ss;
use crate::bridge::{self, Ctx};
use crate::i18n;
use crate::{dialogs, router, store, toast, AppWindow, LibraryBridge};

#[derive(Default)]
struct State {
    phase: i32,
    done: i32,
    total: i32,
    error: String,
    instance: String,
    already_exists: bool,
    busy: bool,
    browsing: bool,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
    static WATCHDOG: Timer = Timer::default();
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn working(s: &State) -> bool {
    s.busy || (POST..=OVERRIDES).contains(&s.phase)
}

fn push() {
    let Some(ui) = super::ui_handle() else { return };
    let b = ui.global::<LibraryBridge>();
    with(|s| {
        b.set_mp_phase(s.phase);
        b.set_mp_done(s.done);
        b.set_mp_total(s.total);
        b.set_mp_error(ss(&s.error));
        b.set_mp_instance(ss(&s.instance));
        b.set_mp_already_exists(s.already_exists);
        b.set_mp_busy(s.busy);
        b.set_mp_browsing(s.browsing);
    });
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    super::set_ctx(ctx);
    let b = ui.global::<LibraryBridge>();

    // Opening: reset a finished/failed flow, memory back to the configured default.
    b.on_mp_opened(|| {
        let reset = with(|s| {
            if working(s) {
                return false;
            }
            *s = State::default();
            true
        });
        let Some(ui) = super::ui_handle() else { return };
        let b = ui.global::<LibraryBridge>();
        if reset {
            b.set_mp_memory_touched(false);
        }
        push();
        let Some(c) = super::ctx() else { return };
        c.call_with(
            |e| async move { e.get_config().await },
            |ui, cfg| {
                let b = ui.global::<LibraryBridge>();
                if !b.get_mp_memory_touched() {
                    b.set_mp_memory(cfg.default_memory_mb as f32);
                }
            },
            |_ui, _e| {},
        );
    });
    b.on_mp_browse(|| {
        let Some(c) = super::ctx() else { return };
        let go = with(|s| {
            if s.browsing || working(s) {
                return false;
            }
            s.browsing = true;
            true
        });
        if !go {
            return;
        }
        push();
        let title = i18n::t("home.mrpackBrowse");
        c.call(
            move |_e| async move {
                Ok::<_, espectral_engine::EngineError>(
                    dialogs::pick_file(&title, &[("Modrinth modpack", &["mrpack"])]).await,
                )
            },
            |ui, picked| {
                with(|s| s.browsing = false);
                if let Some(p) = picked {
                    ui.global::<LibraryBridge>().set_mp_path(ss(p.display().to_string()));
                }
                push();
            },
        );
    });
    b.on_mp_start(start);
    b.on_mp_close(|| {
        if with(|s| working(s)) {
            return;
        }
        WATCHDOG.with(|t| t.stop());
        if let Some(ui) = super::ui_handle() {
            ui.global::<LibraryBridge>().set_mrpack_open(false);
        }
    });
    b.on_mp_goto(|| {
        let name = with(|s| s.instance.clone());
        let Some(ui) = super::ui_handle() else { return };
        WATCHDOG.with(|t| t.stop());
        ui.global::<LibraryBridge>().set_mrpack_open(false);
        if !name.is_empty() {
            router::navigate(&ui, &router::instance_route(&name));
        }
    });

    bridge::on_event(|_ui, ev| {
        let relevant = match ev {
            EngineEvent::ImportProgress(p) => with(|s| {
                if !working(s) || (!s.instance.is_empty() && p.instance != s.instance) {
                    return false;
                }
                if p.phase == "files" || p.phase == "overrides" {
                    s.phase = if p.phase == "files" { FILES } else { OVERRIDES };
                    if let Some(i) = p.index {
                        s.done = i as i32 + 1;
                    }
                    s.total = p.total as i32;
                }
                true
            }),
            EngineEvent::ImportDone(d) => {
                let hit = with(|s| {
                    if !working(s) || (!s.instance.is_empty() && d.instance != s.instance) {
                        return false;
                    }
                    if d.ok {
                        s.phase = DONE;
                        s.already_exists = d.already_exists == Some(true);
                        if s.instance.is_empty() {
                            s.instance = d.instance.clone();
                        }
                    } else {
                        s.phase = ERROR;
                        s.error = d.error.clone().unwrap_or_else(|| d.ok.to_string());
                    }
                    true
                });
                if hit && d.ok {
                    toast::ok(i18n::t("home.mrpackPhaseDone"));
                }
                hit
            }
            _ => false,
        };
        if relevant {
            WATCHDOG.with(|t| t.stop());
            push();
        }
    });
}

fn start() {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = super::ui_handle() else { return };
    let b = ui.global::<LibraryBridge>();
    let path = b.get_mp_path().trim().to_string();
    let memory_mb = b.get_mp_memory_touched().then(|| b.get_mp_memory().round() as u32);
    let go = with(|s| {
        if path.is_empty() || working(s) {
            return false;
        }
        *s = State { busy: true, phase: POST, ..State::default() };
        true
    });
    if !go {
        return;
    }
    push();
    c.call_with(
        move |e| async move { e.import_mrpack(ImportMrpackRequest { path, memory_mb }).await },
        |_ui, reply| {
            with(|s| {
                s.busy = false;
                s.instance = reply.summary.name.clone();
                s.already_exists = reply.already_exists == Some(true);
            });
            push();
            WATCHDOG.with(|t| t.start(TimerMode::SingleShot, Duration::from_secs(15), check_watchdog));
        },
        |_ui, err| {
            with(|s| {
                s.busy = false;
                s.phase = ERROR;
                s.error = err.envelope().message;
            });
            push();
        },
    );
}

fn check_watchdog() {
    let Some(c) = super::ctx() else { return };
    let open = super::ui_handle().is_some_and(|ui| ui.global::<LibraryBridge>().get_mrpack_open());
    let name = with(|s| (open && s.phase == POST && !s.instance.is_empty()).then(|| s.instance.clone()));
    let Some(name) = name else { return };
    c.call_with(
        move |e| async move { e.get_instance_detail(&name).await },
        |_ui, _d| {
            let changed = with(|s| {
                let c = s.phase == POST;
                if c {
                    s.phase = DONE;
                }
                c
            });
            if changed {
                push();
                store::refresh_instances();
            }
        },
        |_ui, _e| {},
    );
}
