//! InstanceHub (#/instances/:name): detail load, config tab (version change,
//! memory, game dir, capture tools, JVM, AOT, appearance, personalization,
//! options import, client suite), servers tab, mods tab, danger zone, rename.

use std::cell::RefCell;
use std::time::Duration;

use espectral_engine::api::accounts_skins::{ImportFramesPackRequest, ImportImagePackRequest};
use espectral_engine::events::{EngineEvent, ProgressEvent};
use espectral_engine::types::{
    ClientInfo, ClientPatch, InstanceDetail, InstancePatch, InstanceSummary, JvmInfo, ModPreset, PersonalizationPatch,
    ServerEntry, VersionChangePlan, VersionTarget,
};
use espectral_engine::EngineError;
use serde_json::{json, Value};
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::home::{loader_str, parse_loader, with_manifest};
use super::launchlog;
use crate::bridge::Ctx;
use crate::router::{self, Route};
use crate::{dialogs, i18n, store, toast, util};
use crate::{
    AppState, AppWindow, HiBgPack, HiFeature, HiOption, HiServer, HiServerDraft, HiVcMod, HomeBridge, InstanceRow,
    Page, SelectOption, Shell,
};

const TAGLINE_MAX: usize = 40;
const CAPTURE_VERSIONS: &[&str] = &["1.21.11", "26.2", "26.3"];
const LOADERS: [&str; 3] = ["fabric", "vanilla", "neoforge"];

#[derive(Default)]
struct Hub {
    name: String,
    summary: Option<InstanceSummary>,
    servers: Vec<ServerEntry>,
    client: Option<ClientInfo>,
    plan: Option<VersionChangePlan>,
    vc_ids: Vec<String>,
    draft: Vec<(String, String)>,
    preset_fallback: Option<slint::Timer>,
    prev_page: Option<Page>,
}

thread_local! {
    static HUB: RefCell<Hub> = RefCell::new(Hub::default());
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn cx() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

fn hub_name() -> String {
    HUB.with(|h| h.borrow().name.clone())
}

fn msg(e: &EngineError) -> String {
    e.envelope().message
}

/// engine InstanceSummary -> InstanceRow (icon decoded by the caller).
pub fn instance_row(i: &InstanceSummary, icon: Option<Image>) -> InstanceRow {
    InstanceRow {
        name: i.name.as_str().into(),
        version: i.version.as_str().into(),
        loader: loader_str(i.loader).into(),
        loader_version: i.loader_version.as_deref().unwrap_or("").into(),
        modpack: i.modpack.as_deref().unwrap_or("").into(),
        modpack_version: i.modpack_version.as_deref().unwrap_or("").into(),
        memory_mb: i.memory_mb as i32,
        memory_auto: i.memory_auto,
        hue: i.hue.map(|h| h.rem_euclid(360) as i32).unwrap_or(-1),
        game_dir: i.game_dir.as_deref().unwrap_or("").into(),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        mod_count: i.mod_count as i32,
        enabled_mod_count: i.enabled_mod_count as i32,
        aot_key: i.aot_key.as_deref().unwrap_or("").into(),
        aot_cache_exists: i.aot_cache_exists,
        imported_from: i.imported_from.as_deref().unwrap_or("").into(),
        capture_tools: i.capture_tools,
        created_at: i.created_at.as_deref().unwrap_or("").into(),
        running: i.running.unwrap_or(false),
    }
}

fn hub_hue(s: &InstanceSummary) -> i32 {
    match s.hue {
        Some(h) => h.rem_euclid(360) as i32,
        None => match s.loader {
            espectral_engine::types::Loader::Fabric => 195,
            espectral_engine::types::Loader::Neoforge => 25,
            espectral_engine::types::Loader::Vanilla => 160,
        },
    }
}

/// Apply a fresh summary (keeps the current icon unless `icon` is given).
fn set_summary(ui: &AppWindow, s: InstanceSummary, icon: Option<Option<Image>>) {
    let hb = ui.global::<HomeBridge>();
    let icon = match icon {
        Some(i) => i,
        None => {
            let cur = hb.get_hub();
            (cur.has_icon && s.has_icon).then_some(cur.icon)
        }
    };
    let row = instance_row(&s, icon);
    hb.set_hub(row);
    hb.set_capture_tools(s.capture_tools);
    hb.set_capture_supported(
        s.loader == espectral_engine::types::Loader::Fabric && CAPTURE_VERSIONS.contains(&s.version.as_str()),
    );
    if ui.global::<crate::Router>().get_page() == Page::Instance {
        ui.global::<Shell>().set_ambient_hue(hub_hue(&s));
    }
    HUB.with(|h| h.borrow_mut().summary = Some(s));
    update_running(ui);
}

fn update_running(ui: &AppWindow) {
    let name = hub_name();
    let hb = ui.global::<HomeBridge>();
    let running = !name.is_empty()
        && (HUB.with(|h| h.borrow().summary.as_ref().and_then(|s| s.running).unwrap_or(false))
            || ui.global::<AppState>().get_instances().iter().any(|r| r.name == name.as_str() && r.running)
            || store::launches().iter().any(|l| l.instance == name && l.running));
    hb.set_hub_running(running);
}

/// Shared data changed (instances / launches / servers).
pub fn shared_changed(ui: &AppWindow) {
    if hub_name().is_empty() {
        return;
    }
    // running flag comes from the instance list (the summary snapshot is stale)
    let name = hub_name();
    if let Some(r) = ui.global::<AppState>().get_instances().iter().find(|r| r.name == name.as_str()) {
        HUB.with(|h| {
            if let Some(s) = h.borrow_mut().summary.as_mut() {
                s.running = Some(r.running);
                s.mod_count = r.mod_count as u32;
                s.enabled_mod_count = r.enabled_mod_count as u32;
            }
        });
        let hb = ui.global::<HomeBridge>();
        let mut row = hb.get_hub();
        row.running = r.running;
        row.mod_count = r.mod_count;
        row.enabled_mod_count = r.enabled_mod_count;
        row.aot_cache_exists = r.aot_cache_exists;
        hb.set_hub(row);
    }
    update_running(ui);
    update_servers(ui);
    update_options_sources(ui);
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));

    router::on_route(|ui, route: &Route| {
        let prev = HUB.with(|h| h.borrow_mut().prev_page.replace(route.page));
        if route.page == Page::Instance {
            let tab = match route.param("tab").as_deref() {
                Some("servers") => 1,
                Some("mods") => 2,
                Some("danger") => 3,
                _ => 0,
            };
            let hb = ui.global::<HomeBridge>();
            hb.set_hub_tab(tab);
            hb.set_rename_open(false);
            load(ui, &route.instance);
        } else {
            HUB.with(|h| {
                let mut h = h.borrow_mut();
                h.name.clear();
                h.summary = None;
                h.preset_fallback = None;
            });
            launchlog::set_hub_instance(ui, "");
            // InstanceHub onDestroy removed --ambient-hue (Home sets its own)
            if prev == Some(Page::Instance) && route.page != Page::Home {
                ui.global::<Shell>().set_ambient_hue(160);
            }
        }
    });

    let hb = ui.global::<HomeBridge>();

    hb.on_hub_open_folder(|| {
        let name = hub_name();
        if let Some(c) = cx() {
            c.fire(move |e| async move { e.open_folder(&format!("instances/{name}")).await });
        }
    });

    // ---- rename ----
    hb.on_rename_save(|draft| rename(draft.trim().to_string()));

    // ---- version change ----
    let w = ui.as_weak();
    hb.on_vc_select(move |vi, li| {
        let Some(ui) = w.upgrade() else { return };
        let hb = ui.global::<HomeBridge>();
        hb.set_vc_version_index(vi);
        hb.set_vc_loader_index(li);
        clear_plan(&ui);
        update_vc_changed(&ui);
    });
    let w = ui.as_weak();
    hb.on_vc_toggle_all(move |all| {
        let Some(ui) = w.upgrade() else { return };
        ui.global::<HomeBridge>().set_vc_show_all(all);
        rebuild_vc_versions(&ui, None);
    });
    hb.on_vc_plan(vc_plan);
    hb.on_vc_apply(vc_apply);
    let w = ui.as_weak();
    hb.on_vc_cancel(move || {
        if let Some(ui) = w.upgrade() {
            clear_plan(&ui);
        }
    });

    // ---- memory ----
    hb.on_save_memory(|mb| {
        patch(InstancePatch { memory_mb: Some(Some(mb as u32)), ..Default::default() }, true, move |_ui, s| {
            toast::ok(i18n::tf("instance.memorySaved", &[("mem", &util::mem_label(s.memory_mb as f64))]))
        })
    });
    hb.on_auto_memory(|| {
        patch(InstancePatch { memory_mb: Some(None), ..Default::default() }, true, |ui, s| {
            ui.global::<HomeBridge>().set_hub_memory(s.memory_mb as i32);
            toast::ok(i18n::tf("instance.memoryAutoSaved", &[("mem", &util::mem_label(s.memory_mb as f64))]));
        })
    });

    // ---- game dir ----
    hb.on_pick_game_dir(|| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_picking_game_dir() {
            return;
        }
        hb.set_picking_game_dir(true);
        let title = i18n::t("instance.gameDirTitle");
        c.call(
            move |_e| async move { Ok(dialogs::pick_folder(&title).await) },
            |ui, p| {
                let hb = ui.global::<HomeBridge>();
                hb.set_picking_game_dir(false);
                if let Some(p) = p {
                    hb.set_game_dir(p.to_string_lossy().as_ref().into());
                }
            },
        );
    });
    hb.on_save_game_dir(|dir| {
        let d = dir.trim().to_string();
        set_flag(|hb, v| hb.set_saving_game_dir(v), true);
        patch_with(
            InstancePatch { game_dir: Some((!d.is_empty()).then_some(d)), ..Default::default() },
            |ui, s| {
                let hb = ui.global::<HomeBridge>();
                hb.set_saving_game_dir(false);
                hb.set_game_dir(s.game_dir.as_deref().unwrap_or("").into());
                toast::ok(i18n::t("instance.gameDirSaved"));
            },
            |ui| ui.global::<HomeBridge>().set_saving_game_dir(false),
        );
    });
    hb.on_clear_game_dir(|| {
        set_flag(|hb, v| hb.set_saving_game_dir(v), true);
        patch_with(
            InstancePatch { game_dir: Some(None), ..Default::default() },
            |ui, _s| {
                let hb = ui.global::<HomeBridge>();
                hb.set_saving_game_dir(false);
                hb.set_game_dir("".into());
                toast::ok(i18n::t("instance.gameDirSaved"));
            },
            |ui| ui.global::<HomeBridge>().set_saving_game_dir(false),
        );
    });

    // ---- capture tools (engine: toggling not implemented, see native/PORTING.md) ----
    let w = ui.as_weak();
    hb.on_toggle_capture(move || {
        let Some(ui) = w.upgrade() else { return };
        let hb = ui.global::<HomeBridge>();
        let cur = hb.get_hub().capture_tools;
        hb.set_capture_tools(cur);
        toast::err(format!(
            "{}: {}",
            i18n::t("instance.captureError"),
            "capture_tools no está disponible en el motor nativo"
        ));
    });

    // ---- JVM ----
    hb.on_save_jdk(|path| {
        let p = path.trim().to_string();
        set_flag(|hb, v| hb.set_saving_config(v), true);
        patch_with(
            InstancePatch { jdk_path_override: Some((!p.is_empty()).then_some(p)), ..Default::default() },
            |ui, _| {
                ui.global::<HomeBridge>().set_saving_config(false);
                toast::ok(i18n::t("instance.jdkSaved"));
            },
            |ui| ui.global::<HomeBridge>().set_saving_config(false),
        );
    });

    // ---- AOT ----
    let w = ui.as_weak();
    hb.on_toggle_auto_train(move || {
        let Some(ui) = w.upgrade() else { return };
        let hb = ui.global::<HomeBridge>();
        let next = !hb.get_aot_auto_train();
        hb.set_aot_auto_train(next);
        patch_with(
            InstancePatch { aot_auto_train: Some(next), ..Default::default() },
            |_ui, _| {},
            move |ui| ui.global::<HomeBridge>().set_aot_auto_train(!next),
        );
    });
    hb.on_train_aot(|| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_training_aot() {
            return;
        }
        hb.set_training_aot(true);
        hb.set_train_progress_visible(true);
        hb.set_train_done(0.0);
        hb.set_train_total(100.0);
        toast::info(i18n::t("instance.trainAotStarted"));
        let name = hub_name();
        c.call_with(
            move |e| async move { e.train_instance(&name).await },
            |_ui, _| {},
            |ui, e| {
                let hb = ui.global::<HomeBridge>();
                hb.set_training_aot(false);
                hb.set_train_progress_visible(false);
                toast::engine_error(&e);
            },
        );
    });

    // ---- appearance ----
    hb.on_save_hue(|hue| {
        set_flag(|hb, v| hb.set_saving_hue(v), true);
        patch_with(
            InstancePatch { hue: Some(Some(hue as i64)), ..Default::default() },
            |ui, _| {
                let hb = ui.global::<HomeBridge>();
                hb.set_saving_hue(false);
                hb.set_hue_dirty(false);
                toast::ok(i18n::t("instance.hueSaved"));
            },
            |ui| ui.global::<HomeBridge>().set_saving_hue(false),
        );
    });
    hb.on_upload_icon(upload_icon);
    hb.on_remove_icon(|| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_uploading_icon() {
            return;
        }
        hb.set_uploading_icon(true);
        hb.set_icon_error("".into());
        let name = hub_name();
        c.call_with(
            move |e| async move {
                e.remove_instance_icon(&name).await?;
                e.get_instance_detail(&name).await
            },
            |ui, d| {
                ui.global::<HomeBridge>().set_uploading_icon(false);
                set_summary(ui, d.summary, Some(None));
                store::refresh_instances();
                toast::ok(i18n::t("instance.iconRemoved"));
            },
            |ui, e| {
                let hb = ui.global::<HomeBridge>();
                hb.set_uploading_icon(false);
                hb.set_icon_error(msg(&e).into());
                toast::engine_error(&e);
            },
        );
    });
    hb.on_save_tagline(|text| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_saving_tagline() {
            return;
        }
        let next: String = text.trim().chars().take(TAGLINE_MAX).collect();
        hb.set_saving_tagline(true);
        let name = hub_name();
        let n2 = next.clone();
        c.call_with(
            move |e| async move {
                e.patch_personalization(
                    &name,
                    PersonalizationPatch { tagline: Some((!n2.is_empty()).then_some(n2)), ..Default::default() },
                )
                .await
            },
            move |ui, info| {
                let hb = ui.global::<HomeBridge>();
                hb.set_saving_tagline(false);
                hb.set_tagline(next.as_str().into());
                apply_client(ui, info);
                toast::ok(i18n::t("perso.taglineSaveOk"));
            },
            |ui, e| {
                ui.global::<HomeBridge>().set_saving_tagline(false);
                toast::err(format!("{}: {}", i18n::t("perso.taglineSaveError"), msg(&e)));
            },
        );
    });

    // ---- background packs ----
    hb.on_bg_select(|id| select_bg(id.to_string()));
    hb.on_bg_delete(|id, name| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        let id = id.to_string();
        if hb.get_bg_confirm_delete() != id.as_str() {
            hb.set_bg_confirm_delete(id.as_str().into());
            toast::info(i18n::tf("perso.bgDeleteConfirm", &[("name", &name)]));
            return;
        }
        hb.set_bg_confirm_delete("".into());
        hb.set_bg_error("".into());
        let was_selected = hb.get_bg_selected() == id.as_str();
        let i2 = id.clone();
        c.call_with(
            move |e| async move { e.delete_background(&i2).await },
            move |_ui, _| {
                if was_selected {
                    select_bg(String::new());
                }
                refresh_bg_packs();
                toast::ok(i18n::t("perso.bgDeleteOk"));
            },
            |ui, e| {
                ui.global::<HomeBridge>().set_bg_error(msg(&e).into());
                toast::err(i18n::t("perso.bgDeleteError"));
            },
        );
    });
    hb.on_bg_import(|kind, name| bg_import(kind.to_string(), name.trim().to_string()));

    // ---- options import ----
    hb.on_import_options(|source| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        let source = source.to_string();
        if source.is_empty() || hb.get_importing_options() {
            return;
        }
        hb.set_importing_options(true);
        hb.set_options_note("".into());
        let name = hub_name();
        let s2 = source.clone();
        c.call_with(
            move |e| async move { e.import_instance_options(&name, &s2).await },
            move |ui, r| {
                let hb = ui.global::<HomeBridge>();
                hb.set_importing_options(false);
                set_options(ui, &r.options);
                if r.copied {
                    let note =
                        i18n::tf("instance.optionsImported", &[("source", &source), ("count", &r.count.to_string())]);
                    hb.set_options_note(note.as_str().into());
                    toast::ok(note);
                } else {
                    hb.set_options_note(i18n::t("instance.optionsImportNone").into());
                }
            },
            |ui, e| {
                let hb = ui.global::<HomeBridge>();
                hb.set_importing_options(false);
                hb.set_options_note(msg(&e).into());
                toast::engine_error(&e);
            },
        );
    });

    // ---- client suite ----
    hb.on_toggle_feature(|id, next| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if !hb.get_toggling_feature().is_empty() {
            return;
        }
        hb.set_toggling_feature(id.clone());
        let name = hub_name();
        let id = id.to_string();
        c.call_with(
            move |e| async move {
                let mut features = serde_json::Map::new();
                features.insert(id, json!({ "enabled": next }));
                e.patch_client(&name, ClientPatch { features: Some(Value::Object(features)), ..Default::default() })
                    .await
            },
            |ui, info| {
                ui.global::<HomeBridge>().set_toggling_feature("".into());
                apply_client(ui, info);
            },
            |ui, e| {
                ui.global::<HomeBridge>().set_toggling_feature("".into());
                toast::engine_error(&e);
            },
        );
    });

    // ---- servers ----
    let w = ui.as_weak();
    hb.on_edit_servers(move || {
        let Some(ui) = w.upgrade() else { return };
        HUB.with(|h| {
            let mut h = h.borrow_mut();
            h.draft = h.servers.iter().map(|s| (s.name.clone(), s.ip.clone())).collect();
        });
        let hb = ui.global::<HomeBridge>();
        hb.set_server_err("".into());
        hb.set_editing_servers(true);
        push_draft(&ui);
    });
    let w = ui.as_weak();
    hb.on_add_server_row(move || {
        HUB.with(|h| h.borrow_mut().draft.push((String::new(), String::new())));
        if let Some(ui) = w.upgrade() {
            push_draft(&ui);
        }
    });
    let w = ui.as_weak();
    hb.on_remove_server_row(move |i| {
        HUB.with(|h| {
            let mut h = h.borrow_mut();
            if (i as usize) < h.draft.len() {
                h.draft.remove(i as usize);
            }
        });
        if let Some(ui) = w.upgrade() {
            push_draft(&ui);
        }
    });
    hb.on_set_server_row(|i, name, ip| {
        HUB.with(|h| {
            if let Some(r) = h.borrow_mut().draft.get_mut(i as usize) {
                *r = (name.to_string(), ip.to_string());
            }
        });
    });
    hb.on_save_servers(save_servers);

    // ---- mods ----
    hb.on_install_preset(install_preset);
    hb.on_open_mods_folder(|| {
        let Some(c) = cx() else { return };
        let name = hub_name();
        c.fire(move |e| async move {
            let p = e.mods_dir(&name).await?;
            e.open_folder(&p.path).await
        });
    });

    // ---- danger ----
    hb.on_delete_instance(|| {
        let Some(c) = cx() else { return };
        let Some(ui) = c.ui() else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_deleting() {
            return;
        }
        hb.set_deleting(true);
        let name = hub_name();
        let n2 = name.clone();
        c.call_with(
            move |e| async move { e.delete_instance(&n2).await },
            move |ui, _| {
                ui.global::<HomeBridge>().set_deleting(false);
                store::refresh_instances();
                toast::ok(i18n::tf("instance.deleteSuccess", &[("name", &name)]));
                router::navigate(ui, "#/");
            },
            |ui, e| {
                ui.global::<HomeBridge>().set_deleting(false);
                toast::engine_error(&e);
            },
        );
    });
}

fn set_flag(f: impl FnOnce(&HomeBridge, bool), v: bool) {
    if let Some(ui) = cx().and_then(|c| c.ui()) {
        f(&ui.global::<HomeBridge>(), v);
    }
}

/// PATCH the hub instance; on success apply the summary, refresh the shared list
/// (optional) and run `then`. `saving-config` guards the call when `guard`.
fn patch(p: InstancePatch, guard: bool, then: impl FnOnce(&AppWindow, &InstanceSummary) + Send + 'static) {
    if guard {
        let Some(ui) = cx().and_then(|c| c.ui()) else { return };
        let hb = ui.global::<HomeBridge>();
        if hb.get_saving_config() {
            return;
        }
        hb.set_saving_config(true);
    }
    patch_with(
        p,
        move |ui, s| {
            if guard {
                ui.global::<HomeBridge>().set_saving_config(false);
            }
            then(ui, s);
        },
        move |ui| {
            if guard {
                ui.global::<HomeBridge>().set_saving_config(false);
            }
        },
    );
}

fn patch_with(
    p: InstancePatch,
    then: impl FnOnce(&AppWindow, &InstanceSummary) + Send + 'static,
    on_err: impl FnOnce(&AppWindow) + Send + 'static,
) {
    let Some(c) = cx() else { return };
    let name = hub_name();
    c.call_with(
        move |e| async move { e.patch_instance(&name, p).await },
        move |ui, s| {
            set_summary(ui, s.clone(), None);
            store::refresh_instances();
            then(ui, &s);
        },
        move |ui, e| {
            on_err(ui);
            toast::engine_error(&e);
        },
    );
}

// ---------------------------------------------------------------------------
// load
// ---------------------------------------------------------------------------

fn load(ui: &AppWindow, name: &str) {
    let Some(c) = cx() else { return };
    HUB.with(|h| {
        let mut h = h.borrow_mut();
        h.name = name.to_string();
        h.plan = None;
        h.summary = None;
        h.preset_fallback = None;
    });
    launchlog::set_hub_instance(ui, name);
    let hb = ui.global::<HomeBridge>();
    hb.set_hub_loading(true);
    hb.set_hub_loaded(false);
    hb.set_hub_error("".into());
    hb.set_options_note("".into());
    hb.set_server_err("".into());
    hb.set_editing_servers(false);
    hb.set_vc_error("".into());
    hb.set_vc_has_plan(false);
    hb.set_icon_error("".into());
    hb.set_bg_error("".into());
    hb.set_bg_confirm_delete("".into());
    hb.set_bg_name("".into());
    hb.set_options_source_index(-1);
    hb.set_install_progress_visible(false);
    hb.set_installing_preset(false);
    hb.set_training_aot(false);
    hb.set_train_progress_visible(false);
    hb.set_jdk_override("".into());
    hb.set_aot_auto_train(false);
    hb.set_hue_dirty(false);
    hb.set_client_visible(false);
    hb.set_preset_supported(false);
    hb.set_deleting(false);
    let mut row = hb.get_hub();
    row.name = name.into();
    hb.set_hub(row);

    let n = name.to_string();
    c.call_with(
        move |e| async move {
            let (detail, jvm, client) = tokio::join!(e.get_instance_detail(&n), e.jvm_info(), e.get_client_info(&n));
            let detail = detail?;
            let icon = if detail.summary.has_icon {
                e.instance_icon(&n).await.ok().and_then(|b| util::decode_png(&b))
            } else {
                None
            };
            Ok((detail, jvm.ok(), client.ok(), icon))
        },
        |ui, (detail, jvm, client, icon)| apply_detail(ui, detail, jvm, client, icon),
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_hub_loading(false);
            hb.set_hub_error(msg(&e).into());
        },
    );
    refresh_bg_packs();
    let n = name.to_string();
    c.call_with(
        move |e| async move { e.mod_preset_info(&n).await },
        |ui, p| ui.global::<HomeBridge>().set_preset_supported(p.supported),
        |_ui, _e| {},
    );
}

fn apply_detail(
    ui: &AppWindow,
    d: InstanceDetail,
    jvm: Option<JvmInfo>,
    client: Option<ClientInfo>,
    icon: Option<SharedPixelBuffer<Rgba8Pixel>>,
) {
    if d.summary.name != hub_name() && hub_name() != d.summary.name {
        // a stale reply for an instance we already left
        if HUB.with(|h| h.borrow().name.is_empty()) {
            return;
        }
    }
    let hb = ui.global::<HomeBridge>();
    let s = d.summary.clone();
    hb.set_hub_memory(s.memory_mb as i32);
    hb.set_game_dir(s.game_dir.as_deref().unwrap_or("").into());
    hb.set_hue_value(hub_hue_raw(&s));
    hb.set_hue_dirty(false);
    set_summary(ui, s.clone(), Some(icon.map(util::to_image)));

    // version change selection
    let li = LOADERS.iter().position(|l| *l == loader_str(s.loader)).unwrap_or(0) as i32;
    hb.set_vc_loader_index(li);
    rebuild_vc_versions(ui, Some(s.version.clone()));

    // AOT
    hb.set_aot_cache_exists(d.aot.cache_exists);
    hb.set_aot_size(format_bytes_kb(d.aot.cache_size_bytes).into());
    hb.set_aot_proof_ok(d.aot.proof.as_ref().is_some_and(|p| p.using_aot_linked_classes));

    // JVM
    match jvm {
        Some(j) => {
            let src = serde_json::to_value(j.runtime.source)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "Bundled".into());
            hb.set_jvm_source(src.into());
            hb.set_jvm_version(format!("{} ({})", j.runtime.version, j.runtime.vendor).into());
            hb.set_jvm_path(j.runtime.path.as_str().into());
        }
        None => {
            hb.set_jvm_source("Bundled".into());
            hb.set_jvm_version("—".into());
            hb.set_jvm_path("".into());
        }
    }

    // options / servers
    set_options(ui, &d.options);
    HUB.with(|h| h.borrow_mut().servers = d.servers.clone());
    update_servers(ui);
    update_options_sources(ui);

    // client suite + personalization
    match client {
        Some(info) => apply_client(ui, info),
        None => {
            hb.set_client_visible(false);
            hb.set_tagline("".into());
            hb.set_saved_tagline("".into());
            hb.set_bg_selected("".into());
        }
    }
    let tagline = hb.get_saved_tagline();
    hb.set_tagline(tagline);

    hb.set_hub_loading(false);
    hb.set_hub_loaded(true);
}

fn hub_hue_raw(s: &InstanceSummary) -> i32 {
    s.hue.map(|h| h.rem_euclid(360) as i32).unwrap_or(160)
}

/// InstanceHub formatBytes: 0 KB / x.y KB / x.yy MB.
fn format_bytes_kb(b: u64) -> String {
    if b == 0 {
        "0 KB".into()
    } else if b >= 1024 * 1024 {
        format!("{:.2} MB", b as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", b as f64 / 1024.0)
    }
}

fn set_options(ui: &AppWindow, opts: &[(String, String)]) {
    let hb = ui.global::<HomeBridge>();
    hb.set_options_count(opts.len() as i32);
    let rows: Vec<HiOption> =
        opts.iter().take(30).map(|(k, v)| HiOption { key: k.as_str().into(), value: v.as_str().into() }).collect();
    hb.set_options_rows(ModelRc::new(VecModel::from(rows)));
}

fn update_options_sources(ui: &AppWindow) {
    let name = hub_name();
    let hb = ui.global::<HomeBridge>();
    let cur = {
        let m = hb.get_options_sources();
        let i = hb.get_options_source_index();
        (i >= 0).then(|| m.row_data(i as usize)).flatten().map(|o| o.value.to_string())
    };
    let rows: Vec<SelectOption> = ui
        .global::<AppState>()
        .get_instances()
        .iter()
        .filter(|r| r.name != name.as_str())
        .map(|r| SelectOption { value: r.name.clone(), label: format!("{} ({})", r.name, r.version).into() })
        .collect();
    let idx = cur.and_then(|c| rows.iter().position(|r| r.value == c.as_str())).map(|i| i as i32).unwrap_or(-1);
    hb.set_options_sources(ModelRc::new(VecModel::from(rows)));
    hb.set_options_source_index(idx);
}

fn update_servers(ui: &AppWindow) {
    let net: Vec<crate::ServerRow> = ui.global::<AppState>().get_servers().iter().collect();
    let rows: Vec<HiServer> = HUB.with(|h| {
        h.borrow()
            .servers
            .iter()
            .map(|s| {
                let m = net
                    .iter()
                    .find(|n| n.ip == s.ip.as_str() || n.hostname == s.ip.as_str() || n.host == s.ip.as_str());
                let status = match m {
                    Some(n) if n.online => i18n::tf(
                        "instance.serverPlayers",
                        &[("online", &n.players_online.to_string()), ("max", &n.players_max.to_string())],
                    ),
                    Some(_) => i18n::t("instance.serverOffline"),
                    None => String::new(),
                };
                let motd = m
                    .filter(|n| n.online)
                    .map(|n| n.motd_clean.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                HiServer {
                    name: if s.name.is_empty() { s.ip.as_str().into() } else { s.name.as_str().into() },
                    ip: s.ip.as_str().into(),
                    matched: m.is_some(),
                    online: m.is_some_and(|n| n.online),
                    status: status.into(),
                    motd: motd.into(),
                }
            })
            .collect()
    });
    ui.global::<HomeBridge>().set_servers(ModelRc::new(VecModel::from(rows)));
}

fn apply_client(ui: &AppWindow, info: ClientInfo) {
    let hb = ui.global::<HomeBridge>();
    let perso = info.config.personalization.clone().unwrap_or_default();
    let tagline = perso.get("tagline").and_then(|v| v.as_str()).unwrap_or("").to_string();
    hb.set_saved_tagline(tagline.as_str().into());
    hb.set_bg_selected(perso.get("background").and_then(|v| v.as_str()).unwrap_or("").into());
    let suite = info.config.suite.as_ref().and_then(|s| s.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(true);
    hb.set_suite_enabled(suite);
    let feats: Vec<HiFeature> = info
        .registry
        .iter()
        .map(|f| {
            let enabled = info
                .config
                .features
                .get(&f.id)
                .and_then(|v| v.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(f.default_enabled);
            HiFeature {
                id: f.id.as_str().into(),
                name: f.name.as_str().into(),
                desc: f.description.as_str().into(),
                keybind: f.keybind.as_deref().unwrap_or("").into(),
                enabled,
            }
        })
        .collect();
    hb.set_client_visible(info.supported && !info.registry.is_empty());
    hb.set_features(ModelRc::new(VecModel::from(feats)));
    HUB.with(|h| h.borrow_mut().client = Some(info));
}

// ---------------------------------------------------------------------------
// version change
// ---------------------------------------------------------------------------

fn rebuild_vc_versions(ui: &AppWindow, select: Option<String>) {
    let Some(c) = cx() else { return };
    let current = HUB.with(|h| h.borrow().summary.as_ref().map(|s| s.version.clone())).unwrap_or_default();
    let want = select.unwrap_or_else(|| {
        let hb = ui.global::<HomeBridge>();
        HUB.with(|h| h.borrow().vc_ids.get(hb.get_vc_version_index().max(0) as usize).cloned()).unwrap_or_default()
    });
    // show the current version right away, the full list once the manifest is in
    apply_vc_ids(ui, vec![current.clone()], &current, &want);
    with_manifest(&c, move |ui, m| {
        let all = ui.global::<HomeBridge>().get_vc_show_all();
        let mut ids: Vec<String> =
            m.versions.iter().filter(|v| all || v.kind == "release").map(|v| v.id.clone()).collect();
        if !current.is_empty() && !ids.contains(&current) {
            ids.insert(0, current.clone());
        }
        apply_vc_ids(ui, ids, &current, &want);
    });
}

fn apply_vc_ids(ui: &AppWindow, ids: Vec<String>, current: &str, want: &str) {
    let hb = ui.global::<HomeBridge>();
    let suffix = i18n::t("instance.vcCurrent");
    let opts: Vec<SelectOption> = ids
        .iter()
        .map(|id| SelectOption {
            value: id.as_str().into(),
            label: if id == current { format!("{id} · {suffix}").into() } else { id.as_str().into() },
        })
        .collect();
    let idx = ids.iter().position(|i| i == want).or_else(|| ids.iter().position(|i| i == current)).unwrap_or(0);
    HUB.with(|h| h.borrow_mut().vc_ids = ids);
    hb.set_vc_versions(ModelRc::new(VecModel::from(opts)));
    hb.set_vc_version_index(idx as i32);
    update_vc_changed(ui);
}

fn vc_target(ui: &AppWindow) -> (String, String) {
    let hb = ui.global::<HomeBridge>();
    let v = HUB.with(|h| h.borrow().vc_ids.get(hb.get_vc_version_index().max(0) as usize).cloned()).unwrap_or_default();
    let l = LOADERS.get(hb.get_vc_loader_index().max(0) as usize).copied().unwrap_or("fabric").to_string();
    (v, l)
}

fn update_vc_changed(ui: &AppWindow) {
    let (v, l) = vc_target(ui);
    let changed = HUB.with(|h| {
        h.borrow().summary.as_ref().is_some_and(|s| !v.is_empty() && (v != s.version || l != loader_str(s.loader)))
    });
    ui.global::<HomeBridge>().set_vc_changed(changed);
}

fn clear_plan(ui: &AppWindow) {
    HUB.with(|h| h.borrow_mut().plan = None);
    let hb = ui.global::<HomeBridge>();
    hb.set_vc_has_plan(false);
    hb.set_vc_error("".into());
}

fn vc_plan() {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    if !hb.get_vc_changed() || hb.get_vc_planning() {
        return;
    }
    let (v, l) = vc_target(&ui);
    hb.set_vc_planning(true);
    hb.set_vc_error("".into());
    clear_plan(&ui);
    let name = hub_name();
    c.call_with(
        move |e| async move {
            e.plan_version_change(&name, VersionTarget { version: Some(v), loader: Some(parse_loader(&l)) }).await
        },
        |ui, plan| {
            ui.global::<HomeBridge>().set_vc_planning(false);
            show_plan(ui, plan);
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_vc_planning(false);
            hb.set_vc_error(msg(&e).into());
        },
    );
}

fn show_plan(ui: &AppWindow, plan: VersionChangePlan) {
    let hb = ui.global::<HomeBridge>();
    hb.set_vc_plan_from(format!("{} · {} → ", plan.from.version, loader_str(plan.from.loader)).into());
    hb.set_vc_plan_to(format!("{} · {}", plan.to.version, loader_str(plan.to.loader)).into());
    hb.set_vc_warn_downgrade(
        if plan.downgrade {
            i18n::t(if plan.backup_worlds { "instance.vcDowngradeBackup" } else { "instance.vcDowngrade" })
        } else {
            String::new()
        }
        .into(),
    );
    let vanilla = plan.to.loader == espectral_engine::types::Loader::Vanilla;
    hb.set_vc_warn_menu(
        if !plan.espectral_menu && !vanilla { i18n::t("instance.vcNoMenu") } else { String::new() }.into(),
    );
    let mut mods = Vec::new();
    let summary = if vanilla && !plan.mods.is_empty() {
        i18n::tf("instance.vcVanillaMods", &[("count", &plan.mods.len().to_string())])
    } else if !plan.mods.is_empty() {
        let count = |a: &str| plan.mods.iter().filter(|m| m.action == a).count();
        for m in plan.mods.iter().filter(|m| m.action != "idle") {
            let to_ver =
                m.to.as_ref()
                    .and_then(|t| serde_json::to_value(t).ok())
                    .and_then(|v| v.get("version_number").and_then(|x| x.as_str()).map(str::to_string));
            let what = match m.action.as_str() {
                "update" => format!("{} {}", i18n::t("instance.vcUpdate"), to_ver.unwrap_or_default()),
                "add" => format!("{} {}", i18n::t("instance.vcAdd"), to_ver.unwrap_or_default()),
                "keep" => i18n::t("instance.vcKeep"),
                _ => i18n::t(if m.reason.as_deref() == Some("unknown") {
                    "instance.vcDisableUnknown"
                } else {
                    "instance.vcDisableNoBuild"
                }),
            };
            mods.push(HiVcMod { title: m.title.as_str().into(), what: what.into(), kind: m.action.as_str().into() });
        }
        i18n::tf(
            "instance.vcSummary",
            &[
                ("update", &(count("update") + count("add")).to_string()),
                ("keep", &count("keep").to_string()),
                ("disable", &count("disable").to_string()),
            ],
        )
    } else {
        String::new()
    };
    hb.set_vc_summary(summary.into());
    hb.set_vc_mods(ModelRc::new(VecModel::from(mods)));
    hb.set_vc_has_plan(true);
    HUB.with(|h| h.borrow_mut().plan = Some(plan));
}

fn vc_apply() {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    let Some(plan) = HUB.with(|h| h.borrow().plan.clone()) else { return };
    if hb.get_vc_applying() {
        return;
    }
    hb.set_vc_applying(true);
    hb.set_vc_error("".into());
    let name = hub_name();
    c.call_with(
        move |e| async move {
            e.apply_version_change(
                &name,
                VersionTarget { version: Some(plan.to.version), loader: Some(plan.to.loader) },
            )
            .await
        },
        |ui, res| {
            let hb = ui.global::<HomeBridge>();
            hb.set_vc_applying(false);
            clear_plan(ui);
            let failed = res.mods.errors.len();
            let mut text = i18n::tf(
                "instance.vcDone",
                &[("version", &res.summary.version), ("loader", loader_str(res.summary.loader))],
            );
            if res.backup.is_some() {
                text.push(' ');
                text.push_str(&i18n::t("instance.vcBackedUp"));
            }
            if failed > 0 {
                text.push(' ');
                text.push_str(&i18n::tf("instance.vcFailedMods", &[("count", &failed.to_string())]));
                toast::err(text);
            } else {
                toast::ok(text);
            }
            store::refresh_instances();
            let n = hub_name();
            load(ui, &n);
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_vc_applying(false);
            hb.set_vc_error(msg(&e).into());
        },
    );
}

// ---------------------------------------------------------------------------
// rename
// ---------------------------------------------------------------------------

fn rename(next: String) {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    let cur = hub_name();
    if hb.get_rename_saving() {
        return;
    }
    if next.is_empty() || next == cur {
        hb.set_rename_open(false);
        return;
    }
    hb.set_rename_saving(true);
    c.call_with(
        move |e| async move { e.patch_instance(&cur, InstancePatch { name: Some(next), ..Default::default() }).await },
        |ui, s| {
            let hb = ui.global::<HomeBridge>();
            hb.set_rename_saving(false);
            hb.set_rename_open(false);
            store::refresh_instances();
            toast::ok(i18n::tf("instance.renameSuccess", &[("name", &s.name)]));
            if hb.get_selected_instance() == hub_name().as_str() {
                hb.set_selected_instance(s.name.as_str().into());
            }
            router::navigate(ui, &router::instance_route(&s.name));
        },
        |ui, e| {
            ui.global::<HomeBridge>().set_rename_saving(false);
            let env = e.envelope();
            let text = match env.code.as_str() {
                "ALREADY_EXISTS" => i18n::t("instance.renameConflict"),
                "ALREADY_RUNNING" => i18n::t("instance.renameRunning"),
                "INVALID_NAME" => i18n::t("instance.renameInvalid"),
                _ => env.message,
            };
            toast::err(text);
        },
    );
}

// ---------------------------------------------------------------------------
// icon / backgrounds
// ---------------------------------------------------------------------------

/// Downscale (box filter) to fit `max` px and encode as a PNG data URL.
fn png_data_url(buf: &SharedPixelBuffer<Rgba8Pixel>, max: u32) -> Option<String> {
    use base64::Engine as _;
    let (w, h) = (buf.width(), buf.height());
    let scale = (max as f64 / w.max(h) as f64).min(1.0);
    let (nw, nh) = (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1));
    let src = buf.as_bytes();
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let (y0, y1) = ((y * h / nh), (((y + 1) * h / nh).max(y * h / nh + 1)).min(h));
        for x in 0..nw {
            let (x0, x1) = ((x * w / nw), (((x + 1) * w / nw).max(x * w / nw + 1)).min(w));
            let mut acc = [0u64; 4];
            let mut n = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * w + sx) * 4) as usize;
                    for k in 0..4 {
                        acc[k] += src[i + k] as u64;
                    }
                    n += 1;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            for k in 0..4 {
                out[o + k] = (acc[k] / n.max(1)) as u8;
            }
        }
    }
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, nw, nh);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(&out).ok()?;
    }
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn upload_icon() {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    if hb.get_uploading_icon() {
        return;
    }
    hb.set_uploading_icon(true);
    hb.set_icon_error("".into());
    let name = hub_name();
    let title = i18n::t("instance.iconTitle");
    c.call_with(
        move |e| async move {
            let Some(path) = dialogs::pick_file(&title, &[("PNG", &["png"])]).await else { return Ok(None) };
            let bytes = std::fs::read(&path).map_err(|err| EngineError::bad_request("BAD_IMAGE", err.to_string()))?;
            let fname = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let Some(buf) = util::decode_png(&bytes) else {
                return Err(EngineError::bad_request(
                    "BAD_IMAGE",
                    i18n::tf("instance.iconUnsupported", &[("label", "PNG"), ("name", &fname)]),
                ));
            };
            let url = png_data_url(&buf, 256)
                .ok_or_else(|| EngineError::bad_request("BAD_IMAGE", "PNG encode failed".to_string()))?;
            e.upload_instance_icon(&name, &url).await?;
            let d = e.get_instance_detail(&name).await?;
            let icon = e.instance_icon(&name).await.ok().and_then(|b| util::decode_png(&b));
            Ok(Some((d, icon)))
        },
        |ui, r| {
            ui.global::<HomeBridge>().set_uploading_icon(false);
            if let Some((d, icon)) = r {
                set_summary(ui, d.summary, Some(icon.map(util::to_image)));
                store::refresh_instances();
                toast::ok(i18n::t("instance.iconSaved"));
            }
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_uploading_icon(false);
            hb.set_icon_error(msg(&e).into());
            toast::engine_error(&e);
        },
    );
}

fn refresh_bg_packs() {
    let Some(c) = cx() else { return };
    if let Some(ui) = c.ui() {
        let hb = ui.global::<HomeBridge>();
        hb.set_bg_loading(true);
        hb.set_bg_error("".into());
    }
    c.call_with(
        |e| async move {
            let reply = e.list_backgrounds().await?;
            let mut out = Vec::with_capacity(reply.packs.len());
            for p in reply.packs {
                let thumb = e
                    .background_image(&p.id)
                    .await
                    .ok()
                    .and_then(|b| util::decode_png(&b))
                    .map(|b| shrink(&b, 240).unwrap_or(b)); // thumbnails only: keep RAM low
                out.push((p, thumb));
            }
            Ok(out)
        },
        |ui, packs| {
            let hb = ui.global::<HomeBridge>();
            let rows: Vec<HiBgPack> = packs
                .into_iter()
                .map(|(p, thumb)| HiBgPack {
                    id: p.id.as_str().into(),
                    name: p.name.as_str().into(),
                    frames: if p.kind == "frames" {
                        i18n::tf("perso.bgFrames", &[("count", &p.frame_count.to_string())]).into()
                    } else {
                        SharedString::new()
                    },
                    has_thumb: thumb.is_some(),
                    thumb: thumb.map(util::to_image).unwrap_or_default(),
                })
                .collect();
            hb.set_bg_packs(ModelRc::new(VecModel::from(rows)));
            hb.set_bg_loading(false);
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_bg_loading(false);
            hb.set_bg_error(msg(&e).into());
        },
    );
}

/// Nearest-neighbour downscale of a pixel buffer to fit `max` px.
fn shrink(b: &SharedPixelBuffer<Rgba8Pixel>, max: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let (w, h) = (b.width(), b.height());
    if w.max(h) <= max {
        return Some(b.clone());
    }
    let scale = max as f64 / w.max(h) as f64;
    let (nw, nh) = (((w as f64 * scale) as u32).max(1), ((h as f64 * scale) as u32).max(1));
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(nw, nh);
    let src = b.as_slice();
    let dst = out.make_mut_slice();
    for y in 0..nh {
        for x in 0..nw {
            dst[(y * nw + x) as usize] = src[((y * h / nh) * w + x * w / nw) as usize];
        }
    }
    Some(out)
}

fn select_bg(id: String) {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    if hb.get_bg_importing() {
        return;
    }
    hb.set_bg_error("".into());
    let name = hub_name();
    let i2 = id.clone();
    c.call_with(
        move |e| async move {
            e.patch_personalization(
                &name,
                PersonalizationPatch { background: Some((!i2.is_empty()).then_some(i2)), ..Default::default() },
            )
            .await
        },
        move |ui, info| {
            apply_client(ui, info);
            ui.global::<HomeBridge>().set_bg_selected(id.as_str().into());
            toast::ok(i18n::t("perso.bgSaveOk"));
        },
        |ui, e| {
            ui.global::<HomeBridge>().set_bg_error(msg(&e).into());
            toast::err(i18n::t("perso.bgSaveError"));
        },
    );
}

fn bg_import(kind: String, pack_name: String) {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    if pack_name.is_empty() {
        let t = i18n::t("perso.bgNameRequired");
        hb.set_bg_error(t.as_str().into());
        toast::err(t);
        return;
    }
    if hb.get_bg_importing() {
        return;
    }
    hb.set_bg_importing(true);
    hb.set_bg_error("".into());
    let title = i18n::t(match kind.as_str() {
        "image" => "perso.bgImportImage",
        "frames" => "perso.bgImportFrames",
        _ => "perso.bgImportFolder",
    });
    c.call_with(
        move |e| async move {
            let reply = match kind.as_str() {
                "image" => {
                    let Some(p) = dialogs::pick_file(&title, &[("PNG", &["png"])]).await else { return Ok(None) };
                    e.import_image_background(ImportImagePackRequest {
                        name: pack_name,
                        source_path: Some(p.to_string_lossy().to_string()),
                        ..Default::default()
                    })
                    .await?
                }
                "frames" => {
                    let ps = dialogs::pick_files(&title, &[("PNG", &["png"])]).await;
                    if ps.is_empty() {
                        return Ok(None);
                    }
                    e.import_frames_background(ImportFramesPackRequest {
                        name: pack_name,
                        frame_paths: Some(ps.iter().map(|p| p.to_string_lossy().to_string()).collect()),
                        ..Default::default()
                    })
                    .await?
                }
                _ => {
                    let Some(p) = dialogs::pick_folder(&title).await else { return Ok(None) };
                    e.import_frames_background(ImportFramesPackRequest {
                        name: pack_name,
                        source_path: Some(p.to_string_lossy().to_string()),
                        ..Default::default()
                    })
                    .await?
                }
            };
            Ok(Some(reply.pack))
        },
        |ui, pack| {
            let hb = ui.global::<HomeBridge>();
            hb.set_bg_importing(false);
            if let Some(p) = pack {
                hb.set_bg_name("".into());
                refresh_bg_packs();
                toast::ok(i18n::tf("perso.bgImportOk", &[("name", &p.name)]));
            }
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_bg_importing(false);
            hb.set_bg_error(msg(&e).into());
            toast::engine_error(&e);
        },
    );
}

// ---------------------------------------------------------------------------
// servers
// ---------------------------------------------------------------------------

fn push_draft(ui: &AppWindow) {
    let rows: Vec<HiServerDraft> = HUB.with(|h| {
        h.borrow().draft.iter().map(|(n, i)| HiServerDraft { name: n.as_str().into(), ip: i.as_str().into() }).collect()
    });
    ui.global::<HomeBridge>().set_server_draft(ModelRc::new(VecModel::from(rows)));
}

/// `^[^:]+(:\d+)?$`
fn valid_ip(ip: &str) -> bool {
    match ip.split_once(':') {
        None => !ip.is_empty(),
        Some((host, port)) => !host.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()),
    }
}

fn save_servers() {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    if hb.get_saving_servers() {
        return;
    }
    hb.set_server_err("".into());
    let (cleaned, existing): (Vec<(String, String)>, Vec<ServerEntry>) = HUB.with(|h| {
        let h = h.borrow();
        (
            h.draft
                .iter()
                .map(|(n, i)| (n.trim().to_string(), i.trim().to_string()))
                .filter(|(n, i)| !n.is_empty() || !i.is_empty())
                .collect(),
            h.servers.clone(),
        )
    });
    for (_, ip) in &cleaned {
        if !ip.is_empty() && !valid_ip(ip) {
            hb.set_server_err(i18n::tf("instance.invalidIp", &[("ip", ip)]).into());
            return;
        }
    }
    // keep the flags of entries that survive the edit (Svelte spread `...s`)
    let entries: Vec<ServerEntry> = cleaned
        .into_iter()
        .map(|(name, ip)| {
            let prev = existing.iter().find(|s| s.ip == ip && s.name == name).cloned();
            ServerEntry {
                hidden: prev.as_ref().is_some_and(|p| p.hidden),
                accept_textures: prev.as_ref().map(|p| p.accept_textures).unwrap_or(true),
                has_icon: prev.as_ref().is_some_and(|p| p.has_icon),
                name,
                ip,
            }
        })
        .collect();
    hb.set_saving_servers(true);
    let name = hub_name();
    c.call_with(
        move |e| async move {
            e.put_instance_servers(&name, entries).await?;
            e.instance_servers(&name).await
        },
        |ui, r| {
            let hb = ui.global::<HomeBridge>();
            hb.set_saving_servers(false);
            hb.set_editing_servers(false);
            HUB.with(|h| h.borrow_mut().servers = r.servers);
            update_servers(ui);
            toast::ok(i18n::t("instance.serversSaved"));
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_saving_servers(false);
            hb.set_server_err(msg(&e).into());
        },
    );
}

// ---------------------------------------------------------------------------
// mods preset
// ---------------------------------------------------------------------------

fn install_preset() {
    let Some(c) = cx() else { return };
    let Some(ui) = c.ui() else { return };
    let hb = ui.global::<HomeBridge>();
    if hb.get_installing_preset() {
        return;
    }
    hb.set_installing_preset(true);
    hb.set_install_progress_visible(true);
    hb.set_install_done(0.0);
    hb.set_install_total(0.0);
    hb.set_install_file("".into());
    let name = hub_name();
    c.call_with(
        move |e| async move { e.install_mod_preset(&name, ModPreset::Performance).await },
        |ui, _| {
            // fallback only: clears a bar whose terminal event never arrives
            let t = slint::Timer::default();
            let w = ui.as_weak();
            t.start(slint::TimerMode::SingleShot, Duration::from_secs(90), move || {
                let Some(ui) = w.upgrade() else { return };
                let hb = ui.global::<HomeBridge>();
                if hb.get_installing_preset() {
                    hb.set_installing_preset(false);
                    hb.set_install_progress_visible(false);
                    let n = hub_name();
                    load(&ui, &n);
                }
            });
            HUB.with(|h| h.borrow_mut().preset_fallback = Some(t));
        },
        |ui, e| {
            let hb = ui.global::<HomeBridge>();
            hb.set_installing_preset(false);
            hb.set_install_progress_visible(false);
            toast::engine_error(&e);
        },
    );
}

fn on_mod_progress(ui: &AppWindow, p: &ProgressEvent) {
    let name = hub_name();
    let hb = ui.global::<HomeBridge>();
    if p.instance.as_deref() != Some(name.as_str()) || p.kind.is_some() {
        return;
    }
    let installing = hb.get_installing_preset();
    let visible = hb.get_install_progress_visible();
    match p.preset.as_deref() {
        Some(pr) if pr != "performance" => return,
        None if !installing && !visible => return,
        _ => {}
    }
    let phase = p.phase.as_deref().unwrap_or("");
    let filename = p.filename.clone().unwrap_or_default();
    let index = p.index.as_ref().and_then(|v| v.as_u64());
    let total = p.total;
    let message = p.message.clone().filter(|m| !m.is_empty()).or_else(|| p.error.clone().filter(|m| !m.is_empty()));
    let was_active = installing || visible;
    let clear = |hb: &HomeBridge| {
        HUB.with(|h| h.borrow_mut().preset_fallback = None);
        hb.set_install_progress_visible(false);
        hb.set_installing_preset(false);
    };
    let completed = phase == "done" && matches!((index, total), (Some(i), Some(t)) if i >= t);
    if phase == "error" {
        clear(&hb);
        if was_active {
            toast::err(message.unwrap_or_else(|| "preset install failed".into()));
            load(ui, &name);
        }
    } else if (completed && filename.is_empty()) || (phase == "done" && filename.is_empty() && index.is_none()) {
        clear(&hb);
        if was_active {
            toast::ok(i18n::t("instance.presetInstalled"));
            load(ui, &name);
        }
    } else if completed {
        hb.set_install_progress_visible(false);
        hb.set_installing_preset(false);
        load(ui, &name);
    } else if let (Some(i), Some(t)) = (index, total.filter(|t| *t > 0)) {
        let done = if phase == "done" { i + 1 } else { i };
        let prev = if visible { hb.get_install_done() as u64 } else { 0 };
        hb.set_install_file(filename.into());
        hb.set_install_done(prev.max(done.min(t)) as f32);
        hb.set_install_total(t as f32);
        hb.set_install_progress_visible(true);
        hb.set_installing_preset(true);
    } else if !filename.is_empty() || phase == "start" {
        hb.set_install_file(filename.into());
        hb.set_install_progress_visible(true);
        hb.set_installing_preset(true);
    }
}

// ---------------------------------------------------------------------------
// events
// ---------------------------------------------------------------------------

pub fn on_event(ui: &AppWindow, ev: &EngineEvent) {
    let name = hub_name();
    if name.is_empty() {
        return;
    }
    let hb = ui.global::<HomeBridge>();
    match ev {
        EngineEvent::TrainProgress(p) if p.instance == name => {
            hb.set_training_aot(true);
            hb.set_train_progress_visible(true);
        }
        EngineEvent::TrainDone(d) if d.instance == name => {
            hb.set_training_aot(false);
            hb.set_train_progress_visible(false);
            if !d.ok {
                toast::err(i18n::tf(
                    "instance.trainAotError",
                    &[("error", d.error.as_deref().unwrap_or("Unknown error"))],
                ));
            } else {
                toast::ok(i18n::t("instance.trainAotSuccess"));
                let Some(c) = cx() else { return };
                c.call_with(
                    move |e| async move { e.instance_aot_status(&name).await },
                    |ui, st| {
                        let hb = ui.global::<HomeBridge>();
                        hb.set_aot_cache_exists(st.cache_exists);
                        hb.set_aot_size(format_bytes_kb(st.cache_size_bytes).into());
                        hb.set_aot_proof_ok(st.proof.as_ref().is_some_and(|p| p.using_aot_linked_classes));
                    },
                    |_ui, _e| {},
                );
                store::refresh_instances();
            }
        }
        EngineEvent::ModProgress(p) => on_mod_progress(ui, p),
        EngineEvent::LaunchExit(x) if x.instance == name || x.instance.is_empty() => {
            let Some(c) = cx() else { return };
            c.call_with(move |e| async move { e.get_client_info(&name).await }, apply_client, |_ui, _e| {});
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_ip_validation() {
        assert!(valid_ip("mc.example.org"));
        assert!(valid_ip("1.2.3.4:25565"));
        assert!(!valid_ip("host:"));
        assert!(!valid_ip(":25565"));
        assert!(!valid_ip("host:abc"));
    }

    #[test]
    fn aot_size_format() {
        assert_eq!(format_bytes_kb(0), "0 KB");
        assert_eq!(format_bytes_kb(2048), "2.0 KB");
        assert_eq!(format_bytes_kb(3 * 1024 * 1024), "3.00 MB");
    }
}
