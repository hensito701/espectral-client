//! ModCatalog (`#/mods`): instance chips, installed mods inventory (filters,
//! optimistic enable/disable), performance preset (engine install_mod_preset),
//! Modrinth search/explore (12 per page, offset paging, infinite scroll) with
//! single-mod installs + dependency plans, and live `mod-progress` tracking.
//! Port of pages/ModCatalog.svelte (same terminal-event rules and fallbacks).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use espectral_engine::api::content::{InstallModrinthRequest, ModrinthSearchQuery};
use espectral_engine::events::{EngineEvent, ProgressEvent};
use espectral_engine::types::{
    InstanceSummary, Loader, ModEntry, ModInstallPlan, ModPreset, ModPresetInfo, ModRef, ModrinthProject,
};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use super::{icons, model, ss};
use crate::bridge::{self, Ctx};
use crate::i18n::{self, Lang};
use crate::{prefs, store, toast, util, AppWindow, LibraryBridge, ModRefRow, ModRow, ModrinthCard, PinnedPill};

const PREFS_INSTANCE: &str = "mods.instance";
const SEARCH_PAGE_SIZE: usize = 12;
const FALLBACK: Duration = Duration::from_secs(90);
const DEBOUNCE: Duration = Duration::from_millis(450);

/// Pinned performance set: (slug, requires, required by).
const PINNED_SET: [(&str, &[&str], &[&str]); 7] = [
    ("sodium", &[], &["iris"]),
    ("iris", &["sodium"], &[]),
    ("lithium", &[], &[]),
    ("ferrite-core", &[], &[]),
    ("krypton", &[], &[]),
    ("modmenu", &[], &[]),
    ("fabric-api", &[], &[]),
];

struct PendingInstall {
    project_id: String,
    instance: String,
    plan: Option<ModInstallPlan>,
}

#[derive(Default)]
struct State {
    target: String,
    mods: Vec<ModEntry>,
    mods_seq: u64,
    loading: bool,
    error: String,
    preset: Option<ModPresetInfo>,
    installing_preset: bool,
    /// (filename, done, total)
    progress: Option<(String, u64, u64)>,
    // Modrinth
    results: Option<Vec<ModrinthProject>>,
    total: Option<u64>,
    last_batch: usize,
    search_seq: u64,
    searching: bool,
    loading_more: bool,
    search_error: String,
    installing_id: Option<String>,
    pending: Option<PendingInstall>,
    /// (project id, conflicting titles)
    conflict: Option<(String, String)>,
    optional: HashMap<String, Vec<ModRef>>,
    /// `instance::project_id` installs confirmed by a terminal event.
    confirmed: HashSet<String>,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
    static CARDS: Rc<VecModel<ModrinthCard>> = Rc::new(VecModel::default());
    static PRESET_FALLBACK: Timer = Timer::default();
    static SINGLE_FALLBACK: Timer = Timer::default();
    static DEBOUNCE_TIMER: Timer = Timer::default();
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn win() -> Option<AppWindow> {
    super::ui_handle()
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    super::set_ctx(ctx);
    let b = ui.global::<LibraryBridge>();
    b.set_m_results(ModelRc::from(CARDS.with(|c| c.clone())));
    if let Some(saved) = prefs::get_str(PREFS_INSTANCE) {
        with(|s| s.target = saved);
    }

    b.on_m_select_instance(|name| select_instance(name.to_string()));
    b.on_m_set_tab(|tab| {
        let Some(ui) = win() else { return };
        ui.global::<LibraryBridge>().set_m_tab(tab);
        if tab == 2 {
            let need = with(|s| s.results.is_none() && !s.searching);
            if need {
                run_search(false);
            }
        }
    });
    b.on_m_set_filter(|f| {
        let Some(ui) = win() else { return };
        ui.global::<LibraryBridge>().set_m_filter(f);
        push_mods(&ui);
    });
    b.on_m_set_filter_query(|_q| {
        if let Some(ui) = win() {
            push_mods(&ui);
        }
    });
    b.on_m_toggle(|filename| toggle(filename.to_string()));
    b.on_m_install_preset(install_preset);
    b.on_m_open_folder(open_folder);
    b.on_m_search_edited(|_q| {
        DEBOUNCE_TIMER.with(|t| t.start(TimerMode::SingleShot, DEBOUNCE, || run_search(false)));
    });
    b.on_m_search(|| {
        DEBOUNCE_TIMER.with(|t| t.stop());
        run_search(false);
    });
    b.on_m_load_more(|| {
        let ok = with(|s| !s.loading_more && !s.searching && has_more(s));
        if ok {
            run_search(true);
        }
    });
    b.on_m_install(|id, title, force| install_mod(id.to_string(), title.to_string(), force));
    b.on_m_reload(|| {
        load_mods();
        load_preset();
    });

    bridge::on_event(|ui, ev| {
        if let EngineEvent::ModProgress(p) = ev {
            on_mod_progress(ui, p);
        }
    });
}

/// Route entered: Svelte remounts the page (tab back to "installed", reload).
pub fn on_enter(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    b.set_m_tab(super::debug::mods_tab().unwrap_or(0));
    ensure_target(ui);
    if super::debug::has("mods-fake") {
        with(|s| {
            s.mods = super::debug::fake_mods();
            s.loading = false;
            let r = super::debug::fake_projects();
            s.total = Some(40);
            s.last_batch = r.len();
            s.results = Some(r);
        });
        push_all(ui);
        rebuild_cards(ui);
        return;
    }
    push_all(ui);
    load_mods();
    load_preset();
}

pub fn on_instances_changed(ui: &AppWindow) {
    let before = with(|s| s.target.clone());
    ensure_target(ui);
    let after = with(|s| s.target.clone());
    push_target(ui);
    if before != after {
        load_mods();
        load_preset();
    }
}

/// Default to the first instance when the remembered one is gone.
fn ensure_target(_ui: &AppWindow) {
    let list = store::instances();
    if list.is_empty() {
        return;
    }
    with(|s| {
        if s.target.is_empty() || !list.iter().any(|i| i.name == s.target) {
            s.target = list[0].name.clone();
            prefs::set(PREFS_INSTANCE, serde_json::Value::String(s.target.clone()));
        }
    });
}

fn selected_instance(target: &str) -> Option<InstanceSummary> {
    store::instances().into_iter().find(|i| i.name == target)
}

fn select_instance(name: String) {
    let changed = with(|s| {
        let changed = s.target != name;
        s.target = name.clone();
        changed
    });
    prefs::set(PREFS_INSTANCE, serde_json::Value::String(name));
    let Some(ui) = win() else { return };
    push_target(&ui);
    if changed {
        with(|s| {
            s.mods.clear();
            s.preset = None;
        });
        push_mods(&ui);
        load_mods();
        load_preset();
    }
}

// ---------------------------------------------------------------- loading

fn load_mods() {
    if super::debug::has("mods-fake") {
        return;
    }
    let Some(c) = super::ctx() else { return };
    let (target, seq) = with(|s| {
        s.mods_seq += 1;
        s.loading = true;
        s.error.clear();
        (s.target.clone(), s.mods_seq)
    });
    if target.is_empty() {
        with(|s| s.loading = false);
        return;
    }
    if let Some(ui) = win() {
        push_mods(&ui);
    }
    c.call_with(
        move |e| async move { e.list_mods(&target).await },
        move |ui, mods| {
            let current = with(|s| {
                if s.mods_seq != seq {
                    return false;
                }
                s.mods = mods;
                s.loading = false;
                true
            });
            if current {
                push_mods(ui);
            }
        },
        move |ui, err| {
            let current = with(|s| {
                if s.mods_seq != seq {
                    return false;
                }
                s.error = err.envelope().message;
                s.loading = false;
                true
            });
            if current {
                push_mods(ui);
            }
        },
    );
}

fn load_preset() {
    let Some(c) = super::ctx() else { return };
    let target = with(|s| {
        s.preset = None;
        s.target.clone()
    });
    if target.is_empty() {
        return;
    }
    let t2 = target.clone();
    c.call_with(
        move |e| async move { e.mod_preset_info(&target).await },
        move |ui, info| {
            let current = with(|s| {
                if s.target != t2 {
                    return false;
                }
                s.preset = Some(info);
                true
            });
            if current {
                push_mods(ui);
            }
        },
        |_ui, _err| {},
    );
}

// ---------------------------------------------------------------- bridge sync

fn push_all(ui: &AppWindow) {
    push_target(ui);
    push_mods(ui);
    push_search(ui);
}

fn push_target(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    let target = with(|s| s.target.clone());
    let sel = selected_instance(&target);
    b.set_m_target(ss(&target));
    b.set_m_has_target(!target.is_empty());
    b.set_m_sel_loader(ss(sel.as_ref().map(|i| i.loader.as_str()).unwrap_or("")));
    b.set_m_sel_version(ss(sel.as_ref().map(|i| i.version.as_str()).unwrap_or("")));
}

pub fn display_name(filename: &str, slug: Option<&str>) -> String {
    let base = strip_jar(filename);
    if base.is_empty() {
        slug.unwrap_or("").to_string()
    } else {
        base.to_string()
    }
}

fn strip_jar(filename: &str) -> &str {
    if filename.len() >= 4 && filename[filename.len() - 4..].eq_ignore_ascii_case(".jar") {
        &filename[..filename.len() - 4]
    } else {
        filename
    }
}

fn push_mods(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    let filter = b.get_m_filter();
    let q = b.get_m_filter_query().trim().to_lowercase();
    with(|s| {
        let enabled = s.mods.iter().filter(|m| m.enabled).count();
        let rows: Vec<ModRow> = s
            .mods
            .iter()
            .filter(|m| match filter {
                1 => m.enabled,
                2 => !m.enabled,
                _ => true,
            })
            .filter(|m| {
                q.is_empty()
                    || m.filename.to_lowercase().contains(&q)
                    || m.project_slug.as_deref().is_some_and(|p| p.to_lowercase().contains(&q))
            })
            .map(mod_row)
            .collect();
        let pinned: Vec<PinnedPill> = PINNED_SET
            .iter()
            .map(|(slug, req, _)| PinnedPill {
                slug: ss(slug),
                requires: ss(if req.is_empty() { String::new() } else { format!("({})", req.join(", ")) }),
                installed: s.mods.iter().any(|m| m.project_slug.as_deref() == Some(*slug) && m.installed),
            })
            .collect();
        let perf_missing = s
            .mods
            .iter()
            .filter(|m| PINNED_SET.iter().any(|(slug, _, _)| m.project_slug.as_deref() == Some(*slug)) && !m.installed)
            .count();
        b.set_m_rows(model(rows));
        b.set_m_count(s.mods.len() as i32);
        b.set_m_enabled_count(enabled as i32);
        b.set_m_disabled_count((s.mods.len() - enabled) as i32);
        b.set_m_loading(s.loading);
        b.set_m_error(ss(&s.error));
        b.set_m_pinned(model(pinned));
        b.set_m_perf_missing(perf_missing as i32);
        b.set_m_preset_available(s.preset.as_ref().map(|p| p.supported).unwrap_or(true));
        b.set_m_preset_note(ss(s
            .preset
            .as_ref()
            .filter(|p| !p.supported)
            .and_then(|p| p.note.clone())
            .unwrap_or_default()));
        b.set_m_installing_preset(s.installing_preset);
        match &s.progress {
            Some((file, done, total)) => {
                b.set_m_progress_active(true);
                b.set_m_progress_file(ss(file));
                b.set_m_progress_done(*done as i32);
                b.set_m_progress_total(*total as i32);
            }
            None => b.set_m_progress_active(false),
        }
    });
}

fn mod_row(m: &ModEntry) -> ModRow {
    let pin = m.project_slug.as_deref().and_then(|slug| PINNED_SET.iter().find(|p| p.0 == slug));
    let mut meta = String::new();
    if let Some(size) = m.size.filter(|s| *s > 0) {
        meta.push_str(&format!("· {}", util::format_bytes(size as f64)));
    }
    if let Some(sha) = m.sha1.as_deref().filter(|s| !s.is_empty()) {
        if !meta.is_empty() {
            meta.push(' ');
        }
        meta.push_str(&format!("· {}", &sha[..sha.len().min(8)]));
    }
    ModRow {
        filename: ss(&m.filename),
        name: ss(display_name(&m.filename, m.project_slug.as_deref())),
        slug: ss(m.project_slug.as_deref().unwrap_or("")),
        version: ss(m.version_number.as_deref().unwrap_or("")),
        meta: ss(meta),
        enabled: m.enabled,
        requires: ss(pin
            .filter(|p| !p.1.is_empty())
            .map(|p| i18n::tf("mods.requires", &[("mods", &p.1.join(", "))]))
            .unwrap_or_default()),
        required_by: ss(pin
            .filter(|p| !p.2.is_empty())
            .map(|p| i18n::tf("mods.requiredBy", &[("mods", &p.2.join(", "))]))
            .unwrap_or_default()),
    }
}

fn has_more(s: &State) -> bool {
    match &s.results {
        Some(r) if !r.is_empty() => match s.total {
            Some(t) => (r.len() as u64) < t,
            None => s.last_batch == SEARCH_PAGE_SIZE,
        },
        _ => false,
    }
}

fn push_search(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    with(|s| {
        b.set_m_searching(s.searching);
        b.set_m_loading_more(s.loading_more);
        b.set_m_search_error(ss(&s.search_error));
        b.set_m_has_results(s.results.is_some());
        b.set_m_has_more(has_more(s));
        b.set_m_installing_id(ss(s.installing_id.as_deref().unwrap_or("")));
        b.set_m_any_optional(s.optional.values().any(|v| !v.is_empty()));
    });
}

/// Rebuild every card (keeps the model object so the grid stays bound).
fn rebuild_cards(ui: &AppWindow) {
    let cards: Vec<ModrinthCard> = with(|s| {
        let prefix = format!("{}::", s.target);
        s.results.as_ref().map(|r| r.iter().map(|p| card(s, p, &prefix)).collect()).unwrap_or_default()
    });
    request_icons(&cards);
    CARDS.with(|m| m.set_vec(cards));
    push_search(ui);
}

/// Refresh the cards' dynamic fields in place (installed / conflict / optional).
fn refresh_cards(ui: &AppWindow) {
    CARDS.with(|m| {
        with(|s| {
            let prefix = format!("{}::", s.target);
            let Some(results) = s.results.as_ref() else { return };
            for (i, p) in results.iter().enumerate() {
                if i >= m.row_count() {
                    break;
                }
                let c = card(s, p, &prefix);
                m.set_row_data(i, c);
            }
        });
    });
    push_search(ui);
}

fn card(s: &State, p: &ModrinthProject, prefix: &str) -> ModrinthCard {
    let url = p.icon_url.clone().unwrap_or_default();
    let icon = if url.is_empty() { None } else { icons::get(&url) };
    let installed = p.installed == Some(true) || s.confirmed.contains(&format!("{prefix}{}", p.project_id));
    let optional: Vec<ModRefRow> = s
        .optional
        .get(&p.project_id)
        .map(|v| v.iter().map(|o| ModRefRow { project_id: ss(&o.project_id), title: ss(&o.title) }).collect())
        .unwrap_or_default();
    ModrinthCard {
        project_id: ss(&p.project_id),
        title: ss(&p.title),
        description: ss(&p.description),
        downloads: ss(group_digits(p.downloads, i18n::lang())),
        icon_url: ss(&url),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        installed,
        conflict: ss(s
            .conflict
            .as_ref()
            .filter(|(id, _)| *id == p.project_id)
            .map(|(_, m)| m.clone())
            .unwrap_or_default()),
        optional: model(optional),
    }
}

fn request_icons(cards: &[ModrinthCard]) {
    let Some(c) = super::ctx() else { return };
    for card in cards.iter().filter(|c| !c.has_icon && !c.icon_url.is_empty()) {
        icons::request(&c, &card.icon_url, icon_ready);
    }
}

fn icon_ready(_ui: &AppWindow, url: &str) {
    let Some(img) = icons::get(url) else { return };
    CARDS.with(|m| {
        for i in 0..m.row_count() {
            if let Some(mut c) = m.row_data(i) {
                if c.icon_url.as_str() == url && !c.has_icon {
                    c.icon = img.clone();
                    c.has_icon = true;
                    m.set_row_data(i, c);
                }
            }
        }
    });
}

/// `Number.toLocaleString()` for es (grouping from 5 digits, '.') / en (',').
pub fn group_digits(n: u64, lang: Lang) -> String {
    let digits = n.to_string();
    let sep = match lang {
        Lang::Es => '.',
        Lang::En => ',',
    };
    if lang == Lang::Es && digits.len() <= 4 {
        return digits;
    }
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------- actions

fn toggle(filename: String) {
    let Some(c) = super::ctx() else { return };
    let (target, next, name) = with(|s| {
        let target = s.target.clone();
        let Some(m) = s.mods.iter_mut().find(|m| m.filename == filename) else {
            return (target, None, String::new());
        };
        m.enabled = !m.enabled;
        (target, Some(m.enabled), display_name(&m.filename, m.project_slug.as_deref()))
    });
    let Some(next) = next else { return };
    if let Some(ui) = win() {
        push_mods(&ui);
    }
    let f2 = filename.clone();
    let t2 = target.clone();
    c.call_with(
        move |e| async move { e.set_mod_enabled(&target, &filename, next).await },
        move |_ui, _r| {
            toast::ok(format!("{name} {}", i18n::t(if next { "mods.enabled" } else { "mods.disabled" })));
        },
        move |ui, err| {
            with(|s| {
                if s.target == t2 {
                    if let Some(m) = s.mods.iter_mut().find(|m| m.filename == f2) {
                        m.enabled = !next;
                    }
                }
            });
            push_mods(ui);
            let msg = err.envelope().message;
            toast::err(if msg.is_empty() { i18n::t("mods.toggleError") } else { msg });
        },
    );
}

fn install_preset() {
    let Some(c) = super::ctx() else { return };
    let target = with(|s| {
        if s.target.is_empty() || s.installing_preset {
            return None;
        }
        s.installing_preset = true;
        s.progress = Some((String::new(), 0, 0));
        Some(s.target.clone())
    });
    let Some(target) = target else { return };
    if let Some(ui) = win() {
        push_mods(&ui);
    }
    c.call_with(
        move |e| async move { e.install_mod_preset(&target, ModPreset::Performance).await },
        |_ui, _queued| {
            // completion arrives via the terminal mod-progress event; this is a
            // fallback that clears a banner whose terminal event never arrives
            PRESET_FALLBACK.with(|t| {
                t.start(TimerMode::SingleShot, FALLBACK, || {
                    let live = with(|s| {
                        let live = s.installing_preset;
                        if live {
                            s.installing_preset = false;
                            s.progress = None;
                        }
                        live
                    });
                    if live {
                        load_mods();
                        load_preset();
                    }
                })
            });
        },
        |ui, err| {
            PRESET_FALLBACK.with(|t| t.stop());
            let msg = err.envelope().message;
            with(|s| {
                s.error = msg.clone();
                s.progress = None;
                s.installing_preset = false;
            });
            push_mods(ui);
            toast::err(if msg.is_empty() { "Error installing preset".to_string() } else { msg });
        },
    );
}

fn open_folder() {
    let Some(c) = super::ctx() else { return };
    let target = with(|s| s.target.clone());
    if target.is_empty() {
        return;
    }
    c.call_with(
        move |e| async move {
            let dir = e.mods_dir(&target).await?;
            e.open_folder(&dir.path).await
        },
        |_ui, _| {
            let ok = i18n::t("mods.openFolderSuccess");
            toast::ok(if ok.is_empty() { "Carpeta abierta".to_string() } else { ok });
        },
        |_ui, _err| toast::err(i18n::t("mods.openFolderError")),
    );
}

fn run_search(append: bool) {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    let q = ui.global::<LibraryBridge>().get_m_search_query().trim().to_string();
    let (seq, offset, target) = with(|s| {
        let offset = if append { s.results.as_ref().map(|r| r.len()).unwrap_or(0) } else { 0 };
        s.search_seq += 1;
        if append {
            s.loading_more = true;
        } else {
            s.searching = true;
            s.search_error.clear();
        }
        (s.search_seq, offset as u64, s.target.clone())
    });
    push_search(&ui);
    let sel = selected_instance(&target);
    let query = ModrinthSearchQuery {
        q,
        version: sel.as_ref().map(|i| i.version.clone()),
        loader: sel.as_ref().map(|i| if i.loader == Loader::Neoforge { "neoforge" } else { "fabric" }.to_string()),
        instance: sel.as_ref().map(|i| i.name.clone()),
        offset,
    };
    c.call_with(
        move |e| async move { e.search_modrinth(query).await },
        move |ui, data| {
            let current = with(|s| {
                if s.search_seq != seq {
                    return false;
                }
                s.last_batch = data.results.len();
                s.total = Some(data.total);
                s.searching = false;
                s.loading_more = false;
                match (append, s.results.as_mut()) {
                    (true, Some(r)) => r.extend(data.results),
                    _ => s.results = Some(data.results),
                }
                true
            });
            if !current {
                return;
            }
            if append {
                // append the new cards only (keeps the scroll position)
                let new_cards: Vec<ModrinthCard> = with(|s| {
                    let prefix = format!("{}::", s.target);
                    let r = s.results.as_deref().unwrap_or(&[]);
                    let start = CARDS.with(|m| m.row_count()).min(r.len());
                    r[start..].iter().map(|p| card(s, p, &prefix)).collect()
                });
                request_icons(&new_cards);
                CARDS.with(|m| {
                    for c in new_cards {
                        m.push(c);
                    }
                });
                push_search(ui);
            } else {
                rebuild_cards(ui);
            }
        },
        move |ui, _err| {
            let current = with(|s| {
                if s.search_seq != seq {
                    return false;
                }
                s.search_error = i18n::t("mods.searchError");
                if !append {
                    s.results = None;
                    s.total = None;
                    s.last_batch = 0;
                }
                s.searching = false;
                s.loading_more = false;
                true
            });
            if current {
                if !append {
                    rebuild_cards(ui);
                } else {
                    push_search(ui);
                }
            }
        },
    );
}

fn mark_installed(s: &mut State, project_id: &str) {
    s.confirmed.insert(format!("{}::{project_id}", s.target));
    if let Some(r) = s.results.as_mut() {
        for p in r.iter_mut().filter(|p| p.project_id == project_id) {
            p.installed = Some(true);
        }
    }
}

fn install_mod(project_id: String, _title: String, force: bool) {
    let Some(c) = super::ctx() else { return };
    let target = with(|s| {
        if s.target.is_empty() {
            return None;
        }
        s.installing_id = Some(project_id.clone());
        s.pending = Some(PendingInstall { project_id: project_id.clone(), instance: s.target.clone(), plan: None });
        Some(s.target.clone())
    });
    let Some(target) = target else { return };
    if let Some(ui) = win() {
        push_search(&ui);
    }
    let (pid, pid_err, inst) = (project_id.clone(), project_id.clone(), target.clone());
    c.call_with(
        move |e| async move { e.install_modrinth_mod(&target, InstallModrinthRequest { project_id, force }).await },
        move |ui, plan| {
            let text = if plan.dependencies.is_empty() {
                format!("{} {}…", i18n::t("mods.installingMod"), plan.project.title)
            } else {
                i18n::tf(
                    "mods.installingWithDeps",
                    &[("name", &plan.project.title), ("count", &plan.dependencies.len().to_string())],
                )
            };
            with(|s| {
                if let Some(p) = s.pending.as_mut().filter(|p| p.project_id == pid) {
                    p.plan = Some(plan);
                }
                if s.conflict.as_ref().is_some_and(|(id, _)| *id == pid) {
                    s.conflict = None;
                }
            });
            refresh_cards(ui);
            toast::info(text);
            let (pending_id, pending_inst) = (pid.clone(), inst.clone());
            SINGLE_FALLBACK.with(|t| {
                t.start(TimerMode::SingleShot, FALLBACK, move || {
                    let reload = with(|s| {
                        let same = s
                            .pending
                            .as_ref()
                            .is_some_and(|p| p.project_id == pending_id && p.instance == pending_inst);
                        if !same {
                            return false;
                        }
                        s.pending = None;
                        s.installing_id = None;
                        if pending_inst == s.target {
                            mark_installed(s, &pending_id);
                            true
                        } else {
                            false
                        }
                    });
                    if let Some(ui) = win() {
                        refresh_cards(&ui);
                    }
                    if reload {
                        load_mods();
                        load_preset();
                    }
                })
            });
        },
        move |ui, err| {
            SINGLE_FALLBACK.with(|t| t.stop());
            let conflict = err.raw_code() == Some("mod_conflicts");
            let msg = err.envelope().message;
            with(|s| {
                s.pending = None;
                s.installing_id = None;
                if conflict {
                    s.conflict = Some((pid_err, msg.clone()));
                }
            });
            refresh_cards(ui);
            if !conflict {
                toast::err(if msg.is_empty() { "Error installing mod".to_string() } else { msg });
            }
        },
    );
}

// ---------------------------------------------------------------- events

fn as_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
}

fn on_mod_progress(ui: &AppWindow, p: &ProgressEvent) {
    let target = with(|s| s.target.clone());
    if p.instance.as_deref().is_some_and(|i| i != target) {
        return;
    }
    if p.kind.is_some() {
        return;
    }
    let phase = p.phase.as_deref().unwrap_or("");
    let filename = p.filename.clone().unwrap_or_default();
    let index = p.index.as_ref().and_then(as_u64);
    let total = p.total;
    let message = p
        .message
        .clone()
        .filter(|m| !m.is_empty())
        .or_else(|| p.error.clone().filter(|m| !m.is_empty()))
        .unwrap_or_else(|| "Error installing preset".to_string());
    let completed = phase == "done" && matches!((index, total), (Some(i), Some(t)) if i >= t);
    let (was_active, is_single) = with(|s| {
        (
            s.installing_preset || s.progress.is_some(),
            !filename.is_empty() && p.preset.is_none() && s.pending.as_ref().is_some_and(|pi| pi.instance == s.target),
        )
    });
    let clear_terminal = |s: &mut State| {
        s.progress = None;
        s.installing_preset = false;
    };

    if phase == "error" {
        PRESET_FALLBACK.with(|t| t.stop());
        with(clear_terminal);
        if is_single {
            SINGLE_FALLBACK.with(|t| t.stop());
            with(|s| {
                if let Some(pi) = s.pending.take() {
                    let key = format!("{}::{}", s.target, pi.project_id);
                    s.confirmed.remove(&key);
                    if let Some(r) = s.results.as_mut() {
                        for pr in r.iter_mut().filter(|pr| pr.project_id == pi.project_id) {
                            pr.installed = Some(false);
                        }
                    }
                }
                s.installing_id = None;
            });
            refresh_cards(ui);
            push_mods(ui);
            load_mods();
            load_preset();
            toast::err(message);
            return;
        }
        push_mods(ui);
        if was_active {
            load_mods();
            load_preset();
            toast::err(message);
        }
        return;
    }

    if completed {
        PRESET_FALLBACK.with(|t| t.stop());
        with(clear_terminal);
        if is_single {
            SINGLE_FALLBACK.with(|t| t.stop());
            let plan = with(|s| {
                let pi = s.pending.take()?;
                s.installing_id = None;
                mark_installed(s, &pi.project_id);
                if let Some(plan) = pi.plan.as_ref() {
                    for dep in &plan.dependencies {
                        mark_installed(s, &dep.project_id);
                    }
                    if !plan.optional.is_empty() {
                        s.optional.insert(pi.project_id.clone(), plan.optional.clone());
                    }
                }
                Some(pi.plan)
            });
            refresh_cards(ui);
            push_mods(ui);
            load_mods();
            load_preset();
            let plan = plan.flatten();
            let text = match plan.as_ref() {
                Some(pl) if !pl.dependencies.is_empty() => {
                    let deps: Vec<&str> = pl.dependencies.iter().map(|d| d.title.as_str()).collect();
                    i18n::tf("mods.installedWithDeps", &[("name", &pl.project.title), ("deps", &deps.join(", "))])
                }
                Some(pl) => format!("{} {}", pl.project.title, i18n::t("mods.installed")),
                None => format!("{filename} {}", i18n::t("mods.installed")),
            };
            toast::ok(text);
            if let Some(pl) = plan.as_ref().filter(|pl| !pl.missing.is_empty()) {
                let deps: Vec<&str> = pl.missing.iter().map(|d| d.title.as_str()).collect();
                toast::err(i18n::tf("mods.depsMissing", &[("deps", &deps.join(", "))]));
            }
            return;
        }
        push_mods(ui);
        if was_active {
            load_mods();
            load_preset();
            toast::ok(if filename.is_empty() {
                i18n::t("mods.installed")
            } else {
                format!("{filename} {}", i18n::t("mods.installed"))
            });
        }
        return;
    }

    if let (Some(index), Some(total)) = (index, total) {
        if total > 0 {
            let done = if phase == "done" { index + 1 } else { index };
            with(|s| {
                let prev = s.progress.as_ref().map(|p| p.1).unwrap_or(0);
                s.progress = Some((filename, prev.max(done.min(total)), total));
                s.installing_preset = true;
            });
            push_mods(ui);
            return;
        }
    }
    if !filename.is_empty() || phase == "start" {
        with(|s| {
            let (done, total) = s.progress.as_ref().map(|p| (p.1, p.2)).unwrap_or((0, 0));
            s.progress = Some((filename, done, total));
            s.installing_preset = true;
        });
        push_mods(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_digits() {
        assert_eq!(group_digits(1234, Lang::Es), "1234");
        assert_eq!(group_digits(12345, Lang::Es), "12.345");
        assert_eq!(group_digits(1234567, Lang::Es), "1.234.567");
        assert_eq!(group_digits(1234, Lang::En), "1,234");
        assert_eq!(group_digits(999, Lang::En), "999");
    }

    #[test]
    fn names() {
        assert_eq!(display_name("sodium-0.6.jar", Some("sodium")), "sodium-0.6");
        assert_eq!(display_name(".jar", Some("x")), "x");
        assert_eq!(display_name("A.JAR", None), "A");
    }

    #[test]
    fn meta_tail() {
        let m = ModEntry {
            filename: "a.jar".into(),
            project_slug: Some("iris".into()),
            version_number: None,
            version_id: None,
            sha1: Some("0123456789abcdef".into()),
            size: Some(1536),
            enabled: true,
            installed: true,
        };
        let r = mod_row(&m);
        assert_eq!(r.meta.as_str(), "· 1.5 KB · 01234567");
        assert!(!r.requires.is_empty());
        assert!(r.required_by.is_empty());
    }
}
