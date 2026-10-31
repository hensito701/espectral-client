//! Home (rail selection, server strip, MOTD), the telemetry capsule (launch /
//! stop / account / AOT chip), the dry-run inspector, the instance wizard and
//! the .mrpack import modal.

use std::cell::RefCell;
use std::collections::HashSet;

use espectral_engine::api::content::ImportMrpackRequest;
use espectral_engine::api::game_launch::{LaunchOutcome, LaunchRequest};
use espectral_engine::events::EngineEvent;
use espectral_engine::types::{AotStatus, CreateInstanceRequest, Loader, VersionManifest};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::launchlog;
use crate::bridge::Ctx;
use crate::router::{self, Route};
use crate::{dialogs, i18n, store, toast, util};
use crate::{AppState, AppWindow, HiVersion, HomeBridge, Page, SelectOption, ServerRow, Shell};

thread_local! {
    static MANIFEST: RefCell<Option<VersionManifest>> = const { RefCell::new(None) };
    /// Instances with an AOT train in flight (train-progress seen, no train-done yet).
    static TRAINING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static AOT: RefCell<Option<(String, Option<AotStatus>)>> = const { RefCell::new(None) };
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn cx() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

pub fn loader_str(l: Loader) -> &'static str {
    match l {
        Loader::Vanilla => "vanilla",
        Loader::Fabric => "fabric",
        Loader::Neoforge => "neoforge",
    }
}

pub fn parse_loader(s: &str) -> Loader {
    match s {
        "fabric" => Loader::Fabric,
        "neoforge" | "forge" => Loader::Neoforge,
        _ => Loader::Vanilla,
    }
}

/// HorizonRail ambient hue (loader palette only).
fn rail_hue(loader: &str) -> i32 {
    match loader {
        "fabric" => 190,
        "neoforge" | "forge" => 24,
        _ => 160,
    }
}

pub fn active_username(ui: &AppWindow) -> String {
    let s = ui.global::<AppState>();
    if s.get_has_active_account() {
        s.get_active_account().username.to_string()
    } else {
        String::new()
    }
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    let hb = ui.global::<HomeBridge>();

    // ---- rail ----
    let w = ui.as_weak();
    hb.on_select_instance(move |name| {
        if let Some(ui) = w.upgrade() {
            ui.global::<HomeBridge>().set_selected_instance(name.clone());
            apply_home_hue(&ui);
        }
    });
    let w = ui.as_weak();
    hb.on_open_instance(move |name| {
        if let Some(ui) = w.upgrade() {
            router::navigate(&ui, &router::instance_route(&name));
        }
    });

    // ---- capsule ----
    let w = ui.as_weak();
    hb.on_capsule_target(move |name| {
        let Some(ui) = w.upgrade() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_capsule_instance() != name {
            hb.set_capsule_instance(name.clone());
            hb.set_aot_chip_kind("".into());
        }
        launchlog::set_capsule_instance(&ui, &name);
        update_capsule_running(&ui);
        refresh_aot(name.as_str());
    });
    let c = ctx.clone();
    hb.on_launch(move |name| launch(&c, name.to_string()));
    let c = ctx.clone();
    hb.on_stop(move |name| {
        let name = name.to_string();
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if !hb.get_stopping().is_empty() {
            return;
        }
        hb.set_stopping(name.as_str().into());
        let n2 = name.clone();
        c.call_with(
            move |e| async move { e.stop_instance(&n2).await },
            move |ui, _| {
                ui.global::<HomeBridge>().set_stopping("".into());
                store::refresh_launches();
                store::refresh_instances();
                toast::info(format!("Instancia {name} detenida."));
            },
            |ui, e| {
                ui.global::<HomeBridge>().set_stopping("".into());
                toast::engine_error(&e);
            },
        );
    });
    let c = ctx.clone();
    hb.on_select_account(move |username| {
        let u = username.to_string();
        c.call(move |e| async move { e.set_active_account(&u).await }, |_ui, _| store::refresh_accounts());
    });

    // ---- dry run ----
    let c = ctx.clone();
    hb.on_dryrun(move |name| dryrun(&c, name.to_string()));

    // ---- wizard ----
    let c = ctx.clone();
    hb.on_wiz_opened(move || wizard_opened(&c));
    let w = ui.as_weak();
    hb.on_wiz_filter(move |q, f| {
        if let Some(ui) = w.upgrade() {
            apply_version_filter(&ui, &q, &f);
        }
    });
    let c = ctx.clone();
    hb.on_wiz_load_sources(move || {
        let Some(ui) = c.ui() else { return };
        if ui.global::<HomeBridge>().get_wiz_sources_loaded() {
            return;
        }
        c.call_with(
            |e| async move { e.import_sources().await },
            |ui, list| {
                let rows: Vec<SelectOption> = list
                    .iter()
                    .map(|s| {
                        let kind = serde_json::to_value(s.kind)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_default();
                        SelectOption { value: s.id.as_str().into(), label: format!("{} ({kind})", s.label).into() }
                    })
                    .collect();
                let hb = ui.global::<HomeBridge>();
                hb.set_wiz_sources(ModelRc::new(VecModel::from(rows)));
                hb.set_wiz_sources_loaded(true);
            },
            |_ui, e| tracing::warn!(error = %e, "import sources failed"),
        );
    });
    let c = ctx.clone();
    hb.on_wiz_pick_dir(move || {
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_wiz_picking() {
            return;
        }
        hb.set_wiz_picking(true);
        let title = i18n::t("home.wizardGameDirTitle");
        c.call(
            move |_e| async move { Ok(dialogs::pick_folder(&title).await) },
            |ui, picked| {
                let hb = ui.global::<HomeBridge>();
                hb.set_wiz_picking(false);
                if let Some(p) = picked {
                    hb.set_wiz_game_dir(p.to_string_lossy().as_ref().into());
                }
            },
        );
    });
    let c = ctx.clone();
    hb.on_wiz_create(move |name, version, loader, memory, game_dir, hue, source| {
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        let name = name.trim().to_string();
        if name.is_empty() {
            hb.set_wiz_error(i18n::t("home.wizardErrorName").into());
            return;
        }
        if version.is_empty() {
            hb.set_wiz_error(i18n::t("home.wizardErrorVersion").into());
            return;
        }
        hb.set_wiz_creating(true);
        hb.set_wiz_error("".into());
        let src = source.to_string();
        let gd = game_dir.trim().to_string();
        let req = CreateInstanceRequest {
            name,
            version: version.to_string(),
            loader: Some(parse_loader(&loader)),
            memory_mb: (memory > 0).then_some(memory as u32),
            import_from: (!src.is_empty()).then(|| src.clone()),
            merge_optionslc: !src.is_empty(),
            hue: Some(hue as i64),
            game_dir: (!gd.is_empty()).then_some(gd),
            ..Default::default()
        };
        c.call_with(
            move |e| async move { e.create_instance(req).await },
            |ui, summary| {
                let hb = ui.global::<HomeBridge>();
                hb.set_wiz_creating(false);
                store::refresh_instances();
                toast::ok(i18n::tf("home.wizardSuccess", &[("name", &summary.name)]));
                ui.global::<Shell>().set_wizard_open(false);
                hb.set_selected_instance(summary.name.as_str().into());
                router::navigate(ui, &router::instance_route(&summary.name));
            },
            |ui, e| {
                let hb = ui.global::<HomeBridge>();
                hb.set_wiz_creating(false);
                hb.set_wiz_error(e.envelope().message.into());
            },
        );
    });

    // ---- mrpack ----
    let c = ctx.clone();
    hb.on_mrpack_opened(move || {
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        hb.set_mrpack_memory_touched(false);
        if hb.get_mrpack_phase() == "done" || hb.get_mrpack_phase() == "error" {
            hb.set_mrpack_phase("idle".into());
            hb.set_mrpack_error("".into());
        }
        c.call_with(
            |e| async move { e.get_config().await },
            |ui, cfg| {
                let hb = ui.global::<HomeBridge>();
                if !hb.get_mrpack_memory_touched() {
                    hb.set_mrpack_memory(cfg.default_memory_mb as i32);
                }
            },
            |_ui, _e| {},
        );
    });
    let c = ctx.clone();
    hb.on_mrpack_browse(move || {
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_mrpack_browsing() {
            return;
        }
        hb.set_mrpack_browsing(true);
        c.call(
            |_e| async move { Ok(dialogs::pick_file("Modrinth .mrpack", &[("Modrinth modpack", &["mrpack"])]).await) },
            |ui, picked| {
                let hb = ui.global::<HomeBridge>();
                hb.set_mrpack_browsing(false);
                if let Some(p) = picked {
                    hb.set_mrpack_path(p.to_string_lossy().as_ref().into());
                }
            },
        );
    });
    let c = ctx.clone();
    hb.on_mrpack_start(move || {
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        let path = hb.get_mrpack_path().trim().to_string();
        if path.is_empty() || hb.get_mrpack_importing() {
            return;
        }
        hb.set_mrpack_importing(true);
        hb.set_mrpack_error("".into());
        hb.set_mrpack_phase("post".into());
        hb.set_mrpack_done(0);
        hb.set_mrpack_total(0);
        hb.set_mrpack_created("".into());
        hb.set_mrpack_already(false);
        let memory_mb = hb.get_mrpack_memory_touched().then_some(hb.get_mrpack_memory() as u32);
        c.call_with(
            move |e| async move { e.import_mrpack(ImportMrpackRequest { path, memory_mb }).await },
            |ui, reply| {
                let hb = ui.global::<HomeBridge>();
                hb.set_mrpack_importing(false);
                hb.set_mrpack_created(reply.summary.name.as_str().into());
                hb.set_mrpack_already(reply.already_exists == Some(true));
                // watchdog: a lost import-done must not leave the modal spinning
                let name = reply.summary.name.clone();
                let w = ui.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_secs(15), move || {
                    let Some(ui) = w.upgrade() else { return };
                    let hb = ui.global::<HomeBridge>();
                    if hb.get_mrpack_phase() == "post" && hb.get_mrpack_created() == name.as_str() {
                        if let Some(c) = cx() {
                            let n = name.clone();
                            c.call_with(
                                move |e| async move { e.get_instance_detail(&n).await },
                                |ui, _| {
                                    let hb = ui.global::<HomeBridge>();
                                    if hb.get_mrpack_phase() == "post" {
                                        hb.set_mrpack_phase("done".into());
                                        store::refresh_instances();
                                    }
                                },
                                |_ui, _e| {},
                            );
                        }
                    }
                });
            },
            |ui, e| {
                let hb = ui.global::<HomeBridge>();
                hb.set_mrpack_importing(false);
                hb.set_mrpack_phase("error".into());
                hb.set_mrpack_error(e.envelope().message.into());
            },
        );
    });

    // ---- route enter ----
    router::on_route(|ui, route: &Route| {
        if route.page == Page::Home {
            let hb = ui.global::<HomeBridge>();
            hb.set_greeting(greeting_bucket());
            ensure_selection(ui);
            apply_home_hue(ui);
            update_servers(ui);
        }
    });
}

/// Called by HomeBridge.shared-changed (instances / launches / servers / accounts changed).
pub fn shared_changed(ui: &AppWindow) {
    ensure_selection(ui);
    if ui.global::<crate::Router>().get_page() == Page::Home {
        apply_home_hue(ui);
    }
    update_servers(ui);
    update_capsule_running(ui);
}

pub fn on_event(ui: &AppWindow, ev: &EngineEvent) {
    let hb = ui.global::<HomeBridge>();
    let cap = hb.get_capsule_instance().to_string();
    match ev {
        EngineEvent::TrainProgress(p) => {
            TRAINING.with(|t| t.borrow_mut().insert(p.instance.clone()));
            if p.instance == cap {
                update_aot_chip(ui);
            }
        }
        EngineEvent::TrainDone(d) => {
            TRAINING.with(|t| t.borrow_mut().remove(&d.instance));
            if d.instance == cap {
                refresh_aot(&cap);
            }
        }
        EngineEvent::LaunchExit(x) => {
            if x.instance == cap {
                refresh_aot(&cap);
            }
        }
        EngineEvent::ImportProgress(p) => {
            let created = hb.get_mrpack_created();
            if !created.is_empty() && p.instance != created.as_str() {
                return;
            }
            if p.phase == "files" || p.phase == "overrides" {
                hb.set_mrpack_phase(p.phase.as_str().into());
                if let Some(i) = p.index {
                    hb.set_mrpack_done(i as i32 + 1);
                }
                hb.set_mrpack_total(p.total as i32);
            }
        }
        EngineEvent::ImportDone(d) => {
            let created = hb.get_mrpack_created();
            let phase = hb.get_mrpack_phase();
            let tracking = hb.get_mrpack_importing() || matches!(phase.as_str(), "post" | "files" | "overrides");
            if !tracking || (!created.is_empty() && d.instance != created.as_str()) {
                return;
            }
            if d.ok {
                hb.set_mrpack_phase("done".into());
                hb.set_mrpack_created(d.instance.as_str().into());
                hb.set_mrpack_already(d.already_exists == Some(true));
            } else {
                hb.set_mrpack_phase("error".into());
                hb.set_mrpack_error(d.error.clone().unwrap_or_default().into());
            }
        }
        _ => {}
    }
}

fn greeting_bucket() -> i32 {
    use chrono::Timelike;
    let h = chrono::Local::now().hour();
    if (6..13).contains(&h) {
        0
    } else if (13..21).contains(&h) {
        1
    } else {
        2
    }
}

/// HorizonRail: select the first instance when nothing (valid) is selected.
fn ensure_selection(ui: &AppWindow) {
    let hb = ui.global::<HomeBridge>();
    let sel = hb.get_selected_instance();
    let list = ui.global::<AppState>().get_instances();
    if list.row_count() == 0 {
        return;
    }
    if sel.is_empty() || !list.iter().any(|r| r.name == sel) {
        if let Some(first) = list.row_data(0) {
            hb.set_selected_instance(first.name);
        }
    }
}

fn apply_home_hue(ui: &AppWindow) {
    let sel = ui.global::<HomeBridge>().get_selected_instance();
    let loader = ui
        .global::<AppState>()
        .get_instances()
        .iter()
        .find(|r| r.name == sel)
        .map(|r| r.loader.to_string())
        .unwrap_or_else(|| "vanilla".into());
    ui.global::<Shell>().set_ambient_hue(rail_hue(&loader));
}

fn update_servers(ui: &AppWindow) {
    let s = ui.global::<AppState>();
    let mut list: Vec<ServerRow> = s.get_servers().iter().collect();
    list.sort_by(|a, b| b.online.cmp(&a.online).then(b.players_online.cmp(&a.players_online)));
    let motd = list.iter().find(|s| s.online && (s.motd_raw.row_count() > 0 || s.motd_clean.row_count() > 0));
    let hb = ui.global::<HomeBridge>();
    match motd {
        Some(m) => {
            let line = m.motd_clean.row_data(0).or_else(|| m.motd_raw.row_data(0)).unwrap_or_default();
            hb.set_motd_text(strip_section(&line).into());
            hb.set_motd_server(if m.hostname.is_empty() { m.host.clone() } else { m.hostname.clone() });
        }
        None => {
            hb.set_motd_text("".into());
            hb.set_motd_server("Espectral Network".into());
        }
    }
    list.truncate(4);
    hb.set_top_servers(ModelRc::new(VecModel::from(list)));
}

/// Drop `§x` formatting codes (lib/motd.ts renders them as colors).
fn strip_section(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '§' {
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn update_capsule_running(ui: &AppWindow) {
    let hb = ui.global::<HomeBridge>();
    let name = hb.get_capsule_instance();
    let running = !name.is_empty()
        && (ui.global::<AppState>().get_instances().iter().any(|r| r.name == name && r.running)
            || store::launches().iter().any(|l| l.instance == name.as_str() && l.running));
    hb.set_capsule_running(running);
}

/// (Re)load the AOT status of the capsule instance.
pub fn refresh_aot(name: &str) {
    let Some(c) = cx() else { return };
    if name.is_empty() {
        AOT.with(|a| *a.borrow_mut() = None);
        if let Some(ui) = c.ui() {
            update_aot_chip(&ui);
        }
        return;
    }
    let n = name.to_string();
    c.call_with(
        move |e| async move { Ok((n.clone(), e.instance_aot_status(&n).await.ok())) },
        |ui, (n, st)| {
            if ui.global::<HomeBridge>().get_capsule_instance() != n.as_str() {
                return;
            }
            AOT.with(|a| *a.borrow_mut() = Some((n, st)));
            update_aot_chip(ui);
        },
        |_ui, _e| {},
    );
}

fn update_aot_chip(ui: &AppWindow) {
    let hb = ui.global::<HomeBridge>();
    let cap = hb.get_capsule_instance().to_string();
    let (kind, key, title) = AOT.with(|a| {
        let a = a.borrow();
        let Some((n, Some(st))) = a.as_ref() else { return ("", "", String::new()) };
        if *n != cap || st.ready_to_train == Some(false) {
            return ("", "", String::new());
        }
        if TRAINING.with(|t| t.borrow().contains(&cap)) {
            return ("warn", "home.aotChipTraining", i18n::t("home.aotChipTrainingTitle"));
        }
        let path = st.cache_path.clone().unwrap_or_default();
        match &st.proof {
            Some(p) if st.cache_exists && p.using_aot_linked_classes => {
                let t = match &p.log_path {
                    Some(l) => format!("{path} — {l}"),
                    None => path,
                };
                ("ok", "home.aotChipActive", t)
            }
            Some(p) if st.cache_exists && p.refusal.is_some() => {
                ("warn", "home.aotChipRefused", p.refusal.clone().unwrap_or_default())
            }
            _ if st.cache_exists && st.stale == Some(true) => {
                ("warn", "home.aotChipStale", i18n::t("home.aotChipStaleTitle"))
            }
            _ if st.cache_exists => ("ok", "home.aotChipTrained", path),
            _ => ("off", "home.aotChipMissing", i18n::t("home.aotChipMissingTitle")),
        }
    });
    hb.set_aot_chip_kind(kind.into());
    hb.set_aot_chip_key(key.into());
    hb.set_aot_chip_title(title.into());
}

fn launch(c: &Ctx, name: String) {
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    if name.is_empty()
        || !hb.get_launching().is_empty()
        || hb.get_capsule_running() && hb.get_capsule_instance() == name.as_str()
    {
        return;
    }
    hb.set_launching(name.as_str().into());
    let account = active_username(&ui);
    let acc = account.clone();
    let n = name.clone();
    c.call_with(
        move |e| async move {
            e.launch_instance(&n, LaunchRequest { dry_run: false, account: (!acc.is_empty()).then_some(acc) }).await
        },
        move |ui, outcome| {
            ui.global::<HomeBridge>().set_launching("".into());
            if let LaunchOutcome::Started(reply) = outcome {
                launchlog::start(ui, &reply.key, &name, &account);
            }
            store::refresh_launches();
            store::refresh_instances();
            toast::ok(format!("Lanzando {name}..."));
        },
        |ui, e| {
            ui.global::<HomeBridge>().set_launching("".into());
            toast::engine_error(&e);
        },
    );
}

fn dryrun(c: &Ctx, name: String) {
    let Some(ui) = c.ui() else { return };
    if name.is_empty() {
        return;
    }
    let hb = ui.global::<HomeBridge>();
    hb.set_dryrun_instance(name.as_str().into());
    hb.set_dryrun_open(true);
    hb.set_dryrun_loading(true);
    hb.set_dryrun_error("".into());
    hb.set_dryrun_has_result(false);
    c.call_with(
        move |e| async move { e.launch_instance(&name, LaunchRequest { dry_run: true, account: None }).await },
        |ui, outcome| {
            let hb = ui.global::<HomeBridge>();
            hb.set_dryrun_loading(false);
            let LaunchOutcome::DryRun(r) = outcome else { return };
            let strs = |v: &[String]| {
                ModelRc::new(VecModel::from(v.iter().map(|s| SharedString::from(s.as_str())).collect::<Vec<_>>()))
            };
            let java = if r.java_path.is_empty() { "javaw.exe".to_string() } else { r.java_path.clone() };
            let args: Vec<String> =
                r.argv.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect();
            hb.set_dryrun_command(format!("{java} {}", args.join(" ")).into());
            hb.set_dryrun_classpath_joined(r.classpath.join(";").into());
            hb.set_dryrun_argv(strs(&r.argv));
            hb.set_dryrun_classpath(strs(&r.classpath));
            hb.set_dryrun_missing(strs(&r.classpath_missing));
            hb.set_dryrun_warnings(strs(&r.warnings));
            hb.set_dryrun_java(r.java_path.as_str().into());
            hb.set_dryrun_natives(r.natives_dir.as_str().into());
            hb.set_dryrun_assets_ok(r.assets_ok);
            hb.set_dryrun_has_result(true);
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_dryrun_loading(false);
            hb.set_dryrun_error(e.envelope().message.into());
        },
    );
}

fn wizard_opened(c: &Ctx) {
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    // random accent hue for the new instance
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    hb.set_wiz_hue((nanos % 360) as i32);
    hb.set_wiz_game_dir("".into());
    hb.set_wiz_error("".into());
    apply_version_filter(&ui, "", "release");
    c.call_with(
        |e| async move { e.get_config().await },
        |ui, cfg| {
            let hb = ui.global::<HomeBridge>();
            hb.set_wiz_default_memory(cfg.default_memory_mb as i32);
            hb.set_wiz_manifest_serial(hb.get_wiz_manifest_serial() + 1);
        },
        |_ui, _e| {},
    );
    if MANIFEST.with(|m| m.borrow().is_some()) {
        return;
    }
    c.call_with(
        |e| async move { e.list_versions().await },
        |ui, manifest| {
            let hb = ui.global::<HomeBridge>();
            hb.set_wiz_latest_release(manifest.latest_release.as_str().into());
            hb.set_wiz_latest_snapshot(manifest.latest_snapshot.as_str().into());
            MANIFEST.with(|m| *m.borrow_mut() = Some(manifest));
            apply_version_filter(ui, "", "release");
            hb.set_wiz_manifest_serial(hb.get_wiz_manifest_serial() + 1);
        },
        |_ui, e| tracing::warn!(error = %e, "version manifest unavailable"),
    );
}

fn apply_version_filter(ui: &AppWindow, query: &str, filter: &str) {
    let q = query.trim().to_lowercase();
    let rows: Vec<HiVersion> = MANIFEST.with(|m| {
        let m = m.borrow();
        let Some(m) = m.as_ref() else { return Vec::new() };
        m.versions
            .iter()
            .filter(|v| match filter {
                "release" => v.kind == "release",
                "snapshot" => v.kind == "snapshot",
                "old" => v.kind == "old_beta" || v.kind == "old_alpha",
                _ => true,
            })
            .filter(|v| q.is_empty() || v.id.to_lowercase().contains(&q))
            .take(50)
            .map(|v| HiVersion { id: v.id.as_str().into(), kind: v.kind.as_str().into() })
            .collect()
    });
    ui.global::<HomeBridge>().set_wiz_versions(ModelRc::new(VecModel::from(rows)));
}

/// Cached manifest (InstanceHub's version-change select shares it).
pub fn manifest() -> Option<VersionManifest> {
    MANIFEST.with(|m| m.borrow().clone())
}

/// Load the manifest if needed, then run `then` on the UI thread.
pub fn with_manifest(c: &Ctx, then: impl FnOnce(&AppWindow, &VersionManifest) + Send + 'static) {
    if let Some(m) = manifest() {
        if let Some(ui) = c.ui() {
            then(&ui, &m);
        }
        return;
    }
    c.call_with(
        |e| async move { e.list_versions().await },
        move |ui, m| {
            let hb = ui.global::<HomeBridge>();
            hb.set_wiz_latest_release(m.latest_release.as_str().into());
            hb.set_wiz_latest_snapshot(m.latest_snapshot.as_str().into());
            MANIFEST.with(|x| *x.borrow_mut() = Some(m.clone()));
            then(ui, &m);
        },
        |_ui, e| tracing::warn!(error = %e, "version manifest unavailable"),
    );
}

#[allow(dead_code)]
fn _mem(mb: u32) -> String {
    util::mem_label(mb as f64)
}
