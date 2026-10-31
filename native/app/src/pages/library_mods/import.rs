//! ImportCenter (`#/library/import`).
//!
//! Lane A: `.mrpack` installer (native picker, OS drag & drop via `drop.rs`,
//! memory presets, `import-progress` / `import-done` tracking + a 15 s
//! watchdog). Lane B: launcher profile migrator (engine `import_sources`,
//! `import_profile`). Bottom: session-local import history.

use std::cell::RefCell;
use std::time::Duration;

use espectral_engine::api::content::{ImportMrpackRequest, ImportProfileRequest};
use espectral_engine::events::EngineEvent;
use espectral_engine::types::{ImportResult, ImportSource, ImportSourceKind, OverwritePolicy};
use slint::{ComponentHandle, Timer, TimerMode};

use super::{file_name_of, model, ss};
use crate::bridge::{self, Ctx};
use crate::i18n;
use crate::{
    dialogs, router, store, toast, AppWindow, ImportSourceRow, LibraryBridge, ParsedServerRow, RecentImportRow,
    SelectOption, Shell, SkippedRow, ToastKind,
};

/// Lane A phases (shared with MrpackModal): idle post files overrides done error.
pub const IDLE: i32 = 0;
pub const POST: i32 = 1;
pub const FILES: i32 = 2;
pub const OVERRIDES: i32 = 3;
pub const DONE: i32 = 4;
pub const ERROR: i32 = 5;

const WATCHDOG: Duration = Duration::from_secs(15);

#[derive(Default)]
struct State {
    // lane A
    path: String,
    phase: i32,
    done: i32,
    total: i32,
    error: String,
    result_instance: String,
    already_exists: bool,
    busy: bool,
    browsing: bool,
    // lane B
    sources: Vec<ImportSource>,
    sources_loading: bool,
    sources_error: String,
    result: Option<ImportResult>,
    // history (newest first)
    recent: Vec<RecentImportRow>,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State { sources_loading: true, ..Default::default() });
    static WATCHDOG_TIMER: Timer = Timer::default();
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn working(s: &State) -> bool {
    s.busy || (POST..=OVERRIDES).contains(&s.phase)
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    super::set_ctx(ctx);
    let b = ui.global::<LibraryBridge>();

    b.on_i_browse(browse);
    b.on_i_clear(|| {
        let ok = with(|s| {
            if working(s) {
                return false;
            }
            s.path.clear();
            s.phase = IDLE;
            s.error.clear();
            s.done = 0;
            s.total = 0;
            true
        });
        if ok {
            push_lane_a();
        }
    });
    b.on_i_install(install);
    b.on_i_reset(|| {
        with(|s| {
            s.path.clear();
            s.phase = IDLE;
            s.done = 0;
            s.total = 0;
            s.error.clear();
            s.result_instance.clear();
            s.already_exists = false;
        });
        push_lane_a();
    });
    b.on_i_set_memory_text(|text| {
        let Some(ui) = super::ui_handle() else { return };
        let mb = text.trim().parse::<i32>().ok().filter(|v| *v > 0).unwrap_or(0);
        ui.global::<LibraryBridge>().set_i_memory_mb(mb);
    });
    b.on_i_open_instance(|name| {
        if let Some(ui) = super::ui_handle() {
            router::navigate(&ui, &router::instance_route(&name));
        }
    });
    b.on_i_open_wizard(|| {
        if let Some(ui) = super::ui_handle() {
            router::navigate(&ui, "#/");
            ui.global::<Shell>().set_wizard_open(true);
        }
    });
    b.on_i_load_sources(load_sources);
    b.on_i_import_profile(import_profile);
    b.on_i_reset_result(|| {
        with(|s| s.result = None);
        if let Some(ui) = super::ui_handle() {
            ui.global::<LibraryBridge>().set_i_profile_error(ss(""));
        }
        push_lane_b();
    });

    bridge::on_event(|_ui, ev| match ev {
        EngineEvent::ImportProgress(p) => {
            let relevant = with(|s| {
                if !working(s) || (!s.result_instance.is_empty() && p.instance != s.result_instance) {
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
            });
            if relevant {
                WATCHDOG_TIMER.with(|t| t.stop());
                push_lane_a();
            }
        }
        EngineEvent::ImportDone(d) => {
            let relevant = with(|s| {
                if !working(s) || (!s.result_instance.is_empty() && d.instance != s.result_instance) {
                    return false;
                }
                if d.ok {
                    s.phase = DONE;
                    s.already_exists = d.already_exists == Some(true);
                    s.result_instance = d.instance.clone();
                    let summary = if s.already_exists {
                        i18n::tf("import.mrpack.alreadyExists", &[("name", &d.instance)])
                    } else {
                        i18n::t("import.mrpack.phaseDone")
                    };
                    s.recent.insert(0, recent_row(true, "", &d.instance, summary));
                } else {
                    s.phase = ERROR;
                    s.error = d.error.clone().unwrap_or_else(|| "Error desconocido durante la instalación".into());
                }
                true
            });
            if relevant {
                WATCHDOG_TIMER.with(|t| t.stop());
                push_lane_a();
                push_recent();
            }
        }
        _ => {}
    });
}

pub fn on_enter(ui: &AppWindow) {
    super::debug::open_mrpack(ui);
    if super::debug::has("import-done") {
        with(|s| {
            s.phase = DONE;
            s.result_instance = "Fabulously Optimized".into();
        });
    }
    ensure_target(ui);
    push_lane_a();
    push_lane_b();
    push_recent();
    load_sources();
}

pub fn on_instances_changed(ui: &AppWindow) {
    ensure_target(ui);
}

/// Target instance select: options from the shared instance list, default first.
fn ensure_target(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    let list = store::instances();
    let options: Vec<SelectOption> = list
        .iter()
        .map(|i| SelectOption {
            value: ss(&i.name),
            label: ss(format!("{} ({} • {})", i.name, i.version, i.loader.as_str())),
        })
        .collect();
    let mut target = b.get_i_target().to_string();
    if !list.iter().any(|i| i.name == target) {
        target = list.first().map(|i| i.name.clone()).unwrap_or_default();
    }
    let index = list.iter().position(|i| i.name == target).map(|i| i as i32).unwrap_or(-1);
    b.set_i_target_options(model(options));
    b.set_i_target(ss(&target));
    b.set_i_target_index(index);
}

/// Select a `.mrpack` for lane A (picker, drag & drop).
pub fn set_file(path: &str) {
    let ok = with(|s| {
        if working(s) {
            return false;
        }
        s.path = path.to_string();
        s.error.clear();
        if s.phase == ERROR || s.phase == DONE {
            s.phase = IDLE;
        }
        true
    });
    if ok {
        push_lane_a();
    }
}

fn push_lane_a() {
    let Some(ui) = super::ui_handle() else { return };
    let b = ui.global::<LibraryBridge>();
    with(|s| {
        b.set_i_path(ss(&s.path));
        b.set_i_file_name(ss(if s.path.is_empty() { String::new() } else { file_name_of(&s.path) }));
        b.set_i_phase(s.phase);
        b.set_i_done(s.done);
        b.set_i_total(s.total);
        b.set_i_error(ss(&s.error));
        b.set_i_result_instance(ss(&s.result_instance));
        b.set_i_already_exists(s.already_exists);
        b.set_i_busy(s.busy);
        b.set_i_browsing(s.browsing);
    });
}

fn push_lane_b() {
    let Some(ui) = super::ui_handle() else { return };
    let b = ui.global::<LibraryBridge>();
    with(|s| {
        b.set_i_sources_loading(s.sources_loading);
        b.set_i_sources_error(ss(&s.sources_error));
        b.set_i_sources(model(s.sources.iter().map(source_row).collect()));
        if !s.sources.iter().any(|x| x.id.as_str() == b.get_i_source_id().as_str()) {
            b.set_i_source_id(ss(s.sources.first().map(|x| x.id.as_str()).unwrap_or("")));
        }
        match &s.result {
            Some(r) => {
                b.set_i_has_result(true);
                b.set_i_res_copied(model(r.copied.iter().map(ss).collect()));
                b.set_i_res_skipped(model(
                    r.skipped.iter().map(|k| SkippedRow { file: ss(&k.file), reason: ss(&k.reason) }).collect(),
                ));
                b.set_i_res_servers(model(
                    r.servers_parsed
                        .iter()
                        .map(|v| ParsedServerRow { name: ss(&v.name), ip: ss(&v.ip), has_icon: v.has_icon })
                        .collect(),
                ));
                b.set_i_res_options(r.options_keys as i32);
            }
            None => b.set_i_has_result(false),
        }
    });
}

fn push_recent() {
    let Some(ui) = super::ui_handle() else { return };
    let rows = with(|s| s.recent.clone());
    ui.global::<LibraryBridge>().set_i_recent(model(rows));
}

fn kind_str(k: ImportSourceKind) -> &'static str {
    match k {
        ImportSourceKind::Vanilla => "vanilla",
        ImportSourceKind::Fastclient => "fastclient",
        ImportSourceKind::Lunar => "lunar",
    }
}

fn source_row(s: &ImportSource) -> ImportSourceRow {
    ImportSourceRow {
        id: ss(&s.id),
        kind: ss(kind_str(s.kind)),
        label: ss(&s.label),
        path: ss(&s.path),
        options_count: s.options_key_count as i32,
        servers_count: s.servers_entry_count as i32,
        lunar_ram: s.lunar.as_ref().and_then(|l| l.allocated_memory).unwrap_or(0) as i32,
    }
}

fn recent_row(is_mrpack: bool, kind: &str, instance: &str, summary: String) -> RecentImportRow {
    RecentImportRow {
        time: ss(chrono::Local::now().format("%H:%M:%S").to_string()),
        is_mrpack,
        kind: ss(kind),
        instance: ss(instance),
        summary: ss(summary),
    }
}

// ---------------------------------------------------------------- lane A

fn browse() {
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
    push_lane_a();
    let title = i18n::t("import.mrpack.browseButton");
    c.call(
        move |_e| async move {
            Ok::<_, espectral_engine::EngineError>(
                dialogs::pick_file(&title, &[("Modrinth modpack", &["mrpack"])]).await,
            )
        },
        |_ui, picked| {
            with(|s| s.browsing = false);
            match picked {
                Some(p) => set_file(&p.display().to_string()),
                None => push_lane_a(),
            }
        },
    );
}

fn install() {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = super::ui_handle() else { return };
    let mb = ui.global::<LibraryBridge>().get_i_memory_mb();
    let path = with(|s| {
        let path = s.path.trim().to_string();
        if path.is_empty() || working(s) {
            return None;
        }
        s.busy = true;
        s.error.clear();
        s.phase = POST;
        s.done = 0;
        s.total = 0;
        s.result_instance.clear();
        Some(path)
    });
    let Some(path) = path else { return };
    push_lane_a();
    let memory_mb = (mb > 0).then_some(mb as u32);
    c.call_with(
        move |e| async move { e.import_mrpack(ImportMrpackRequest { path, memory_mb }).await },
        |_ui, reply| {
            with(|s| {
                s.busy = false;
                s.result_instance = reply.summary.name.clone();
                s.already_exists = reply.already_exists == Some(true);
            });
            push_lane_a();
            WATCHDOG_TIMER.with(|t| t.start(TimerMode::SingleShot, WATCHDOG, check_stuck));
        },
        |_ui, err| {
            let msg = err.envelope().message;
            with(|s| {
                s.busy = false;
                s.phase = ERROR;
                s.error = msg.clone();
            });
            push_lane_a();
            toast::err(i18n::tf("import.mrpack.phaseError", &[("error", &msg)]));
        },
    );
}

/// No progress event 15 s after the POST: if the instance exists, it is done.
fn check_stuck() {
    let Some(c) = super::ctx() else { return };
    let name = with(|s| (s.phase == POST && !s.result_instance.is_empty()).then(|| s.result_instance.clone()));
    let Some(name) = name else { return };
    c.call_with(
        move |e| async move { e.get_instance_detail(&name).await },
        |_ui, _detail| {
            let changed = with(|s| {
                if s.phase == POST {
                    s.phase = DONE;
                    true
                } else {
                    false
                }
            });
            if changed {
                push_lane_a();
                store::refresh_instances();
            }
        },
        |_ui, _err| {},
    );
}

// ---------------------------------------------------------------- lane B

fn load_sources() {
    let Some(c) = super::ctx() else { return };
    with(|s| {
        s.sources_loading = true;
        s.sources_error.clear();
    });
    push_lane_b();
    c.call_with(
        |e| async move { e.import_sources().await },
        |_ui, sources| {
            with(|s| {
                s.sources = sources;
                s.sources_loading = false;
            });
            push_lane_b();
        },
        |_ui, err| {
            with(|s| {
                s.sources_error = err.envelope().message;
                s.sources_loading = false;
            });
            push_lane_b();
        },
    );
}

fn import_profile() {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = super::ui_handle() else { return };
    let b = ui.global::<LibraryBridge>();
    if b.get_i_profile_importing() {
        return;
    }
    let target = b.get_i_target().to_string();
    let source_id = b.get_i_source_id().to_string();
    let source = with(|s| s.sources.iter().find(|x| x.id == source_id).cloned());
    let (Some(source), false) = (source, target.is_empty()) else { return };
    let policy = if b.get_i_overwrite_if_older() { OverwritePolicy::IfOlder } else { OverwritePolicy::Never };
    b.set_i_profile_importing(true);
    b.set_i_profile_error(ss(""));
    with(|s| s.result = None);
    push_lane_b();
    let t2 = target.clone();
    c.call_with(
        move |e| async move {
            e.import_profile(&target, ImportProfileRequest { source_id, overwrite_policy: policy }).await
        },
        move |ui, res| {
            let label = if source.label.is_empty() { source.id.clone() } else { source.label.clone() };
            let summary = i18n::tf(
                "import.recent.copiedSummary",
                &[("copied", &res.copied.len().to_string()), ("servers", &res.servers_parsed.len().to_string())],
            );
            with(|s| {
                s.recent.insert(0, recent_row(false, kind_str(source.kind), &t2, summary));
                s.result = Some(res);
            });
            ui.global::<LibraryBridge>().set_i_profile_importing(false);
            push_lane_b();
            push_recent();
            toast::push(ToastKind::Ok, format!("Ajustes de {label} importados en {t2}"), router::instance_route(&t2));
        },
        |ui, err| {
            let msg = err.envelope().message;
            let b = ui.global::<LibraryBridge>();
            b.set_i_profile_importing(false);
            b.set_i_profile_error(ss(&msg));
            toast::err(msg);
        },
    );
}
