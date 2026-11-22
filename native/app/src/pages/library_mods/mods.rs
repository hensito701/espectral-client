//! ModCatalog (`#/mods`): instance chips, installed mods inventory (filters,
//! optimistic enable/disable), performance preset (engine install_mod_preset),
//! Modrinth search/explore (12 per page, offset paging, infinite scroll) with
//! single-mod installs + dependency plans, and live `mod-progress` tracking.
//! Port of pages/ModCatalog.svelte (same terminal-event rules and fallbacks).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use espectral_engine::api::content::ModrinthSearchQuery;
use espectral_engine::events::{EngineEvent, ProgressEvent};
use espectral_engine::fabric_compat::{
    format_predicate_human, known_modrinth_for_id, CompatFix, CompatIssueKind, CompatSeverity, ModCompatIssue,
    ModCompatReport,
};
use espectral_engine::types::{
    InstanceSummary, ModEntry, ModInstallPlan, ModInstallPreview, ModPreset, ModPresetInfo, ModRef, ModVersionItem,
    ModVersionSwitchPlan, ModrinthProject,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::{icons, model, ss};
use crate::bridge::{self, Ctx};
use crate::i18n::{self, Lang};
use crate::router::Route;
use crate::{
    prefs, store, toast, util, AppWindow, LibraryBridge, ModChangeSummary, ModCompatNote, ModRefRow, ModRow,
    ModVersionOption, ModrinthCard, PinnedPill,
};

const PREFS_INSTANCE: &str = "mods.instance";
const SEARCH_PAGE_SIZE: usize = 12;
const FALLBACK: Duration = Duration::from_secs(90);
const DEBOUNCE: Duration = Duration::from_millis(450);

/// Pinned performance set: (slug, requires, required by).
const PINNED_SET: [(&str, &[&str], &[&str]); 8] = [
    ("sodium", &[], &["iris"]),
    ("iris", &["sodium"], &[]),
    ("immediatelyfast", &[], &[]),
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
    compat: Option<ModCompatReport>,
    compat_seq: u64,
    loading: bool,
    error: String,
    preset: Option<ModPresetInfo>,
    installing_preset: bool,
    /// (filename, done, total)
    progress: Option<(String, u64, u64)>,
    // Delete modal
    delete_filename: String,
    // Version switcher modal
    ver_filename: String,
    ver_title: String,
    ver_selected: String,
    ver_options: Vec<ModVersionItem>,
    ver_plan: Option<ModVersionSwitchPlan>,
    ver_seq: u64,
    ver_plan_seq: u64,
    // Modrinth install preview modal
    preview: Option<ModInstallPreview>,
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
        load_compat();
    });
    b.on_m_ask_delete(|filename| ask_delete(filename.to_string()));
    b.on_m_confirm_delete(confirm_delete);
    b.on_m_cancel_delete(cancel_delete);
    b.on_m_open_versions(|filename| open_versions(filename.to_string(), None));
    b.on_m_select_version(|vid| select_version(vid.to_string()));
    b.on_m_apply_version(apply_version);
    b.on_m_close_versions(close_versions);
    b.on_m_prev_confirm(confirm_preview);
    b.on_m_prev_cancel(cancel_preview);
    b.on_m_apply_fix(|kind, target, arg| apply_fix(kind.to_string(), target.to_string(), arg.to_string()));

    bridge::on_event(|ui, ev| {
        if let EngineEvent::ModProgress(p) = ev {
            on_mod_progress(ui, p);
        }
    });
}

/// Route entered: select `?instance=<name>` if provided, reset tab to installed, reload.
pub fn on_enter(ui: &AppWindow, route: &Route) {
    let b = ui.global::<LibraryBridge>();
    b.set_m_tab(super::debug::mods_tab().unwrap_or(0));
    b.set_m_filter(0);
    if let Some(inst) = route.param("instance").filter(|s| !s.is_empty()) {
        with(|s| s.target = inst.clone());
        prefs::set(PREFS_INSTANCE, serde_json::Value::String(inst));
    }
    ensure_target(ui);
    if super::debug::has("mods-fake") {
        with(|s| {
            s.mods = super::debug::fake_mods();
            s.compat = None;
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
    load_compat();
}

pub fn on_instances_changed(ui: &AppWindow) {
    let before = with(|s| s.target.clone());
    ensure_target(ui);
    let after = with(|s| s.target.clone());
    push_target(ui);
    if before != after {
        load_mods();
        load_preset();
        load_compat();
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
        // Modrinth results belong to the previous instance: drop them and the
        // cards so nothing stale flashes, and re-search if the tab is open.
        let on_modrinth_tab = ui.global::<LibraryBridge>().get_m_tab() == 2;
        with(|s| {
            s.mods.clear();
            s.compat = None;
            s.preset = None;
            s.results = None;
            s.total = None;
            s.last_batch = 0;
            s.conflict = None;
            s.optional.clear();
            s.search_seq += 1;
            s.searching = false;
            s.loading_more = false;
        });
        ui.global::<LibraryBridge>().set_m_filter(0);
        push_mods(&ui);
        rebuild_cards(&ui);
        load_mods();
        load_preset();
        load_compat();
        if on_modrinth_tab {
            run_search(false);
        }
    }
}

fn load_compat() {
    if super::debug::has("mods-fake") {
        return;
    }
    let Some(c) = super::ctx() else { return };
    let (target, seq) = with(|s| {
        s.compat_seq += 1;
        (s.target.clone(), s.compat_seq)
    });
    if target.is_empty() {
        return;
    }
    c.call_with(
        move |e| async move { e.check_mods_compat(&target).await },
        move |ui, report| {
            let current = with(|s| {
                if s.compat_seq != seq {
                    return false;
                }
                s.compat = Some(report);
                true
            });
            if current {
                push_mods(ui);
            }
        },
        |_ui, _err| {},
    );
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
    // `get`: the byte 4 from the end may sit inside a multi-byte character.
    let cut = filename.len().saturating_sub(4);
    match filename.get(cut..) {
        Some(ext) if ext.eq_ignore_ascii_case(".jar") => &filename[..cut],
        _ => filename,
    }
}

fn is_branding_jar(filename: &str) -> bool {
    let l = filename.to_ascii_lowercase();
    l.starts_with("espectral-menu-") || l.starts_with("espectral-brand-")
}

fn format_issue_text(i: &ModCompatIssue) -> String {
    let es = i18n::lang() == Lang::Es;
    let req = format_predicate_human(&i.required, es);
    let actual = i.actual.as_deref().unwrap_or("?");
    match i.kind {
        CompatIssueKind::MinecraftVersion => i18n::tf("mods.compatMc", &[("req", &req), ("actual", actual)]),
        CompatIssueKind::LoaderVersion => i18n::tf("mods.compatLoader", &[("req", &req), ("actual", actual)]),
        CompatIssueKind::JavaVersion => i18n::tf("mods.compatJava", &[("req", &req), ("actual", actual)]),
        CompatIssueKind::MissingDep => {
            if i.required.is_empty() || i.required == "*" {
                i18n::tf("mods.compatMissing", &[("mod", &i.target_name)])
            } else {
                i18n::tf("mods.compatMissingVer", &[("mod", &i.target_name), ("req", &req)])
            }
        }
        CompatIssueKind::DisabledDep => i18n::tf("mods.compatDisabled", &[("mod", &i.target_name)]),
        CompatIssueKind::IncompatibleDep => {
            i18n::tf("mods.compatIncompat", &[("mod", &i.target_name), ("req", &req), ("actual", actual)])
        }
        CompatIssueKind::BreaksMod => i18n::tf("mods.compatBreaks", &[("mod", &i.target_name)]),
        CompatIssueKind::ConflictsMod => i18n::tf("mods.compatConflicts", &[("mod", &i.target_name)]),
        CompatIssueKind::DuplicateMod => {
            i18n::tf("mods.compatDuplicate", &[("mod", &i.target_name), ("actual", actual)])
        }
        CompatIssueKind::ServerOnly => i18n::t("mods.compatServerOnly"),
        CompatIssueKind::EspectralConflict => {
            let key = format!("mods.espectralConflict.{}", i.reason.as_deref().unwrap_or("unknown"));
            i18n::tf(&key, &[("mod", &i.mod_name)])
        }
        CompatIssueKind::WrongLoader => {
            i18n::tf("mods.compatWrongLoader", &[("req", &i.required), ("actual", actual)])
        }
    }
}

fn format_compat_note(i: &ModCompatIssue, can_switch_self: bool, iris_enabled: bool) -> ModCompatNote {
    let text = format_issue_text(i);
    let severity = match i.severity {
        CompatSeverity::Error => "error",
        CompatSeverity::Warning => "warn",
    };
    let (fix_label, fix_kind, fix_target, fix_arg) = match &i.fix {
        Some(CompatFix::Disable { filename }) => (
            i18n::t("mods.fixDisable"),
            "disable".to_string(),
            filename.clone(),
            String::new(),
        ),
        Some(CompatFix::Enable { filename, name }) => (
            i18n::tf("mods.fixEnable", &[("mod", name)]),
            "enable".to_string(),
            filename.clone(),
            String::new(),
        ),
        Some(CompatFix::UpdateMod { filename, target_id, target_name, required }) => {
            let label = if target_id == "sodium" && iris_enabled {
                format!("{} (+ Iris)", i18n::tf("mods.fixUpdate", &[("mod", target_name)]))
            } else {
                i18n::tf("mods.fixUpdate", &[("mod", target_name)])
            };
            (label, "update".to_string(), filename.clone(), required.clone())
        }
        Some(CompatFix::ChangeVersion { filename }) => {
            if can_switch_self {
                (i18n::t("mods.fixChangeVersion"), "change_version".to_string(), filename.clone(), String::new())
            } else {
                (i18n::t("mods.fixDisable"), "disable".to_string(), filename.clone(), String::new())
            }
        }
        Some(CompatFix::InstallMod { mod_id, name }) => (
            i18n::tf("mods.fixInstall", &[("mod", name)]),
            "install".to_string(),
            mod_id.clone(),
            name.clone(),
        ),
        None => (String::new(), String::new(), String::new(), String::new()),
    };
    ModCompatNote {
        text: ss(text),
        severity: ss(severity),
        fix_label: ss(fix_label),
        fix_kind: ss(fix_kind),
        fix_target: ss(fix_target),
        fix_arg: ss(fix_arg),
    }
}

fn push_mods(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    let mut filter = b.get_m_filter();
    let q = b.get_m_filter_query().trim().to_lowercase();
    with(|s| {
        let enabled = s.mods.iter().filter(|m| m.enabled).count();
        let iris_enabled = s
            .mods
            .iter()
            .any(|m| m.enabled && (m.mod_id.as_deref() == Some("iris") || m.project_slug.as_deref() == Some("iris")));
        let mut issues_by_file: HashMap<&str, Vec<&ModCompatIssue>> = HashMap::new();
        let (compat_errors, compat_warnings) = match &s.compat {
            Some(rep) => {
                for iss in &rep.issues {
                    issues_by_file.entry(iss.filename.as_str()).or_default().push(iss);
                }
                (rep.error_count, rep.warning_count)
            }
            None => (0, 0),
        };
        let problem_count = s.mods.iter().filter(|m| issues_by_file.contains_key(m.filename.as_str())).count();
        if filter == 3 && problem_count == 0 {
            filter = 0;
            b.set_m_filter(0);
        }
        let rows: Vec<ModRow> = s
            .mods
            .iter()
            .filter(|m| match filter {
                1 => m.enabled,
                2 => !m.enabled,
                3 => issues_by_file.contains_key(m.filename.as_str()),
                _ => true,
            })
            .filter(|m| {
                q.is_empty()
                    || m.filename.to_lowercase().contains(&q)
                    || m.project_slug.as_deref().is_some_and(|p| p.to_lowercase().contains(&q))
                    || m.display_name.as_deref().is_some_and(|n| n.to_lowercase().contains(&q))
            })
            .map(|m| mod_row(m, issues_by_file.get(m.filename.as_str()).map(Vec::as_slice).unwrap_or(&[]), iris_enabled))
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
        b.set_m_problem_count(problem_count as i32);
        b.set_m_compat_errors(compat_errors as i32);
        b.set_m_compat_warnings(compat_warnings as i32);
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

fn mod_row(m: &ModEntry, raw_issues: &[&ModCompatIssue], iris_enabled: bool) -> ModRow {
    let pin = m.project_slug.as_deref().and_then(|slug| PINNED_SET.iter().find(|p| p.0 == slug));
    let mut meta = String::new();
    if let Some(size) = m.size.filter(|s| *s > 0) {
        meta.push_str(&format!("· {}", util::format_bytes(size as f64)));
    }
    if let Some(sha) = m.sha1.as_deref().filter(|s| !s.is_empty()) {
        if !meta.is_empty() {
            meta.push(' ');
        }
        meta.push_str(&format!("· {}", sha.get(..8).unwrap_or(sha)));
    }
    let branding = is_branding_jar(&m.filename);
    let can_delete = m.installed && !branding;
    let can_switch_version = m.installed
        && !branding
        && (m.project_id.is_some()
            || m.mod_id.as_deref().and_then(known_modrinth_for_id).is_some());
    let has_error = raw_issues.iter().any(|i| i.severity == CompatSeverity::Error);
    let has_warn = !has_error && raw_issues.iter().any(|i| i.severity == CompatSeverity::Warning);
    let issues: Vec<ModCompatNote> =
        raw_issues.iter().map(|i| format_compat_note(i, can_switch_version, iris_enabled)).collect();
    let nice_name = m
        .display_name
        .as_deref()
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| display_name(&m.filename, m.project_slug.as_deref()));
    ModRow {
        filename: ss(&m.filename),
        name: ss(nice_name),
        slug: ss(m.project_slug.as_deref().or(m.mod_id.as_deref()).unwrap_or("")),
        version: ss(m.version_number.as_deref().unwrap_or("")),
        meta: ss(meta),
        enabled: m.enabled,
        installed: m.installed,
        can_delete,
        can_switch_version,
        requires: ss(pin
            .filter(|p| !p.1.is_empty())
            .map(|p| i18n::tf("mods.requires", &[("mods", &p.1.join(", "))]))
            .unwrap_or_default()),
        required_by: ss(pin
            .filter(|p| !p.2.is_empty())
            .map(|p| i18n::tf("mods.requiredBy", &[("mods", &p.2.join(", "))]))
            .unwrap_or_default()),
        has_error,
        has_warn,
        issues: model(issues),
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
    let (target, next, name, installed) = with(|s| {
        let target = s.target.clone();
        let Some(m) = s.mods.iter_mut().find(|m| m.filename == filename) else {
            return (target, None, String::new(), true);
        };
        m.enabled = !m.enabled;
        (target, Some(m.enabled), display_name(&m.filename, m.project_slug.as_deref()), m.installed)
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
            // A pinned mod that wasn't on disk was just downloaded: re-list it.
            if !installed {
                load_mods();
            }
            load_compat();
            store::refresh_instances();
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
        loader: sel.as_ref().map(|i| i.loader.content_loader().to_string()),
        instance: sel.as_ref().map(|i| i.name.clone()),
        offset,
        sort: None,
        include_server_only: false,
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

fn ask_delete(filename: String) {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    let (target, name) = with(|s| {
        s.delete_filename = filename.clone();
        let name = s
            .mods
            .iter()
            .find(|m| m.filename == filename)
            .and_then(|m| m.display_name.clone())
            .unwrap_or_else(|| display_name(&filename, None));
        (s.target.clone(), name)
    });
    let b = ui.global::<LibraryBridge>();
    b.set_m_delete_filename(ss(&filename));
    b.set_m_delete_name(ss(&name));
    b.set_m_delete_dependents(ss(""));
    b.set_m_deleting(false);
    b.set_m_delete_open(true);

    // Check locally if any enabled mod depends on this jar so the confirm dialog can warn!
    let f2 = filename.clone();
    c.call_with(
        move |e| async move {
            let inst = espectral_engine::resolver::load_instance_meta(&e, &target)?;
            let dir = espectral_engine::instances::effective_mods_dir(&e, &inst);
            let deps = tokio::task::spawn_blocking(move || {
                let jars = espectral_engine::fabric_compat::scan_mods_dir(&dir);
                espectral_engine::fabric_compat::dependents_of_jar(&jars, &f2)
            })
            .await
            .unwrap_or_default();
            Ok(deps)
        },
        move |ui, deps| {
            let still_same = with(|s| s.delete_filename == filename);
            if still_same && !deps.is_empty() {
                let names: Vec<String> = deps.into_iter().map(|(_, n)| n).collect();
                ui.global::<LibraryBridge>().set_m_delete_dependents(ss(names.join(", ")));
            }
        },
        |_ui, _err| {},
    );
}

fn cancel_delete() {
    let Some(ui) = win() else { return };
    let b = ui.global::<LibraryBridge>();
    if b.get_m_deleting() {
        return;
    }
    b.set_m_delete_open(false);
    with(|s| s.delete_filename.clear());
}

fn confirm_delete() {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    let b = ui.global::<LibraryBridge>();
    if b.get_m_deleting() {
        return;
    }
    let (target, filename, name) = with(|s| {
        let f = s.delete_filename.clone();
        let name = s
            .mods
            .iter()
            .find(|m| m.filename == f)
            .and_then(|m| m.display_name.clone())
            .unwrap_or_else(|| display_name(&f, None));
        (s.target.clone(), f, name)
    });
    if target.is_empty() || filename.is_empty() {
        return;
    }
    b.set_m_deleting(true);
    let f2 = filename.clone();
    c.call_with(
        move |e| async move { e.delete_mod(&target, &filename).await },
        move |ui, _| {
            let b = ui.global::<LibraryBridge>();
            b.set_m_deleting(false);
            b.set_m_delete_open(false);
            with(|s| {
                s.delete_filename.clear();
                s.mods.retain(|m| m.filename != f2);
            });
            push_mods(ui);
            load_mods();
            load_preset();
            load_compat();
            store::refresh_instances();
            toast::ok(i18n::tf("mods.deleted", &[("name", &name)]));
        },
        |ui, err| {
            ui.global::<LibraryBridge>().set_m_deleting(false);
            let msg = err.envelope().message;
            toast::err(if msg.is_empty() { i18n::t("mods.deleteError") } else { msg });
        },
    );
}

fn short_date(iso: &str) -> String {
    iso.get(..10).unwrap_or(iso).to_string()
}

fn open_versions(filename: String, auto_predicate: Option<String>) {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    let (target, seq, fallback_name, current_ver) = with(|s| {
        s.ver_seq += 1;
        s.ver_filename = filename.clone();
        s.ver_selected.clear();
        s.ver_options.clear();
        s.ver_plan = None;
        let m = s.mods.iter().find(|m| m.filename == filename);
        let name = m
            .and_then(|m| m.display_name.clone())
            .unwrap_or_else(|| display_name(&filename, m.and_then(|m| m.project_slug.as_deref())));
        let cur = m.and_then(|m| m.version_number.clone()).unwrap_or_default();
        s.ver_title = name.clone();
        (s.target.clone(), s.ver_seq, name, cur)
    });
    if target.is_empty() {
        return;
    }
    let b = ui.global::<LibraryBridge>();
    b.set_m_ver_filename(ss(&filename));
    b.set_m_ver_title(ss(&fallback_name));
    b.set_m_ver_current(ss(&current_ver));
    b.set_m_ver_loading(true);
    b.set_m_ver_error(ss(""));
    b.set_m_ver_options(model(vec![]));
    b.set_m_ver_selected(ss(""));
    b.set_m_ver_planning(false);
    b.set_m_ver_applying(false);
    b.set_m_ver_companions(model(vec![]));
    b.set_m_ver_warnings(model(vec![]));
    b.set_m_ver_open(true);

    let f2 = filename.clone();
    c.call_with(
        move |e| async move { e.list_mod_versions(&target, &f2).await },
        move |ui, reply| {
            let mut auto_pick: Option<String> = None;
            if let Some(pred_str) = auto_predicate.as_deref() {
                let pred = espectral_engine::fabric_compat::VersionPredicate::parse_str(pred_str);
                'outer: for tier in ["release", "beta", "alpha"] {
                    for v in &reply.versions {
                        if v.version_type != tier || v.installed {
                            continue;
                        }
                        if let Some(sv) = espectral_engine::fabric_compat::extract_mod_semver(&v.version_number) {
                            if pred.matches(&sv) {
                                auto_pick = Some(v.version_id.clone());
                                break 'outer;
                            }
                        }
                    }
                }
            }
            let current = with(|s| {
                if s.ver_seq != seq {
                    return false;
                }
                s.ver_title = reply.title.clone();
                s.ver_options = reply.versions.clone();
                true
            });
            if !current {
                return;
            }
            let opts: Vec<ModVersionOption> = reply
                .versions
                .iter()
                .map(|v| ModVersionOption {
                    version_id: ss(&v.version_id),
                    version_number: ss(&v.version_number),
                    name: ss(&v.name),
                    version_type: ss(&v.version_type),
                    date: ss(short_date(&v.date_published)),
                    installed: v.installed,
                })
                .collect();
            let b = ui.global::<LibraryBridge>();
            b.set_m_ver_loading(false);
            b.set_m_ver_title(ss(&reply.title));
            if let Some(cur) = reply.current_version_number {
                b.set_m_ver_current(ss(cur));
            }
            b.set_m_ver_options(model(opts));
            if let Some(vid) = auto_pick {
                select_version(vid);
            }
        },
        move |ui, err| {
            let current = with(|s| s.ver_seq == seq);
            if !current {
                return;
            }
            let b = ui.global::<LibraryBridge>();
            b.set_m_ver_loading(false);
            let msg = if err.raw_code() == Some("mod_not_on_modrinth") {
                i18n::t("mods.versionNotOnModrinth")
            } else {
                err.envelope().message
            };
            b.set_m_ver_error(ss(msg));
        },
    );
}

fn select_version(version_id: String) {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    let (target, filename, pseq) = with(|s| {
        s.ver_selected = version_id.clone();
        s.ver_plan = None;
        s.ver_plan_seq += 1;
        (s.target.clone(), s.ver_filename.clone(), s.ver_plan_seq)
    });
    let b = ui.global::<LibraryBridge>();
    b.set_m_ver_selected(ss(&version_id));
    b.set_m_ver_planning(true);
    b.set_m_ver_companions(model(vec![]));
    b.set_m_ver_warnings(model(vec![]));

    c.call_with(
        move |e| async move { e.plan_mod_version_switch(&target, &filename, &version_id).await },
        move |ui, plan| {
            let current = with(|s| {
                if s.ver_plan_seq != pseq {
                    return false;
                }
                s.ver_plan = Some(plan.clone());
                true
            });
            if !current {
                return;
            }
            let companions: Vec<ModChangeSummary> = plan
                .companions
                .iter()
                .map(|c| ModChangeSummary {
                    title: ss(&c.title),
                    from_ver: ss(&c.from_version),
                    to_ver: ss(&c.to_version_number),
                })
                .collect();
            let warnings: Vec<SharedString> = plan
                .warnings
                .iter()
                .map(|w| ss(i18n::tf("mods.versionWarnCompanion", &[("mods", w)])))
                .collect();
            let b = ui.global::<LibraryBridge>();
            b.set_m_ver_planning(false);
            b.set_m_ver_companions(model(companions));
            b.set_m_ver_warnings(model(warnings));
        },
        move |ui, err| {
            let current = with(|s| s.ver_plan_seq == pseq);
            if !current {
                return;
            }
            let b = ui.global::<LibraryBridge>();
            b.set_m_ver_planning(false);
            b.set_m_ver_error(ss(err.envelope().message));
        },
    );
}

fn close_versions() {
    let Some(ui) = win() else { return };
    let b = ui.global::<LibraryBridge>();
    if b.get_m_ver_applying() {
        return;
    }
    b.set_m_ver_open(false);
    with(|s| {
        s.ver_selected.clear();
        s.ver_plan = None;
    });
}

fn apply_version() {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    let b = ui.global::<LibraryBridge>();
    if b.get_m_ver_applying() {
        return;
    }
    let (target, filename, vid, title, to_ver) = with(|s| {
        let to_ver = s
            .ver_plan
            .as_ref()
            .map(|p| p.to_version_number.clone())
            .or_else(|| s.ver_options.iter().find(|v| v.version_id == s.ver_selected).map(|v| v.version_number.clone()))
            .unwrap_or_default();
        (s.target.clone(), s.ver_filename.clone(), s.ver_selected.clone(), s.ver_title.clone(), to_ver)
    });
    if target.is_empty() || filename.is_empty() || vid.is_empty() {
        return;
    }
    b.set_m_ver_applying(true);
    c.call_with(
        move |e| async move { e.apply_mod_version_switch(&target, &filename, &vid, true).await },
        move |ui, res| {
            let b = ui.global::<LibraryBridge>();
            b.set_m_ver_applying(false);
            b.set_m_ver_open(false);
            load_mods();
            load_preset();
            load_compat();
            store::refresh_instances();
            let msg = if res.companions_changed.is_empty() {
                i18n::tf("mods.versionSwitched", &[("name", &title), ("ver", &to_ver)])
            } else {
                i18n::tf(
                    "mods.versionSwitchedWith",
                    &[("name", &title), ("ver", &to_ver), ("companions", &res.companions_changed.join(", "))],
                )
            };
            toast::ok(msg);
        },
        |ui, err| {
            ui.global::<LibraryBridge>().set_m_ver_applying(false);
            let msg = err.envelope().message;
            toast::err(if msg.is_empty() { i18n::t("mods.versionSwitchError") } else { msg });
        },
    );
}

fn apply_fix(kind: String, target: String, arg: String) {
    match kind.as_str() {
        "disable" => {
            let enabled = with(|s| s.mods.iter().find(|m| m.filename == target).map(|m| m.enabled));
            if enabled == Some(true) {
                toggle(target);
            }
        }
        "enable" => {
            let enabled = with(|s| s.mods.iter().find(|m| m.filename == target).map(|m| m.enabled));
            if enabled == Some(false) {
                toggle(target);
            }
        }
        "change_version" => open_versions(target, None),
        "update" => open_versions(target, (!arg.is_empty()).then_some(arg)),
        "install" => install_missing_mod_id(target, arg),
        _ => {}
    }
}

fn install_missing_mod_id(mod_id: String, display: String) {
    let Some(c) = super::ctx() else { return };
    let inst = with(|s| s.target.clone());
    if inst.is_empty() {
        return;
    }
    let id2 = mod_id.clone();
    let id_err = mod_id.clone();
    c.call_with(
        move |e| async move { e.resolve_modrinth_for_mod_id(&inst, &id2).await },
        move |ui, found| {
            if let Some(mref) = found {
                install_mod(mref.project_id, mref.title, false);
            } else {
                // Fallback: switch to Explorar Modrinth pre-filled with the missing mod id.
                let b = ui.global::<LibraryBridge>();
                b.set_m_tab(2);
                b.set_m_search_query(ss(if display.is_empty() { mod_id } else { display }));
                run_search(false);
            }
        },
        move |ui, _err| {
            let b = ui.global::<LibraryBridge>();
            b.set_m_tab(2);
            b.set_m_search_query(ss(id_err));
            run_search(false);
        },
    );
}

fn install_mod(project_id: String, title: String, force: bool) {
    let Some(c) = super::ctx() else { return };
    let target = with(|s| {
        if s.target.is_empty() {
            return None;
        }
        s.installing_id = Some(project_id.clone());
        Some(s.target.clone())
    });
    let Some(target) = target else { return };
    if let Some(ui) = win() {
        push_search(&ui);
    }
    if force {
        commit_install(target, project_id, true, true);
        return;
    }
    // Pre-install compatibility & upgrade check before touching mods/
    let (pid_ok, t_ok) = (project_id.clone(), target.clone());
    c.call_with(
        move |e| async move { e.preview_modrinth_install(&target, &project_id).await },
        move |ui, preview| {
            if !preview.needs_confirmation {
                commit_install(t_ok, pid_ok, false, false);
                return;
            }
            // Show pre-install confirmation modal with upgrades / conflicts / compat warnings!
            let upgrades: Vec<ModChangeSummary> = preview
                .upgrades
                .iter()
                .map(|u| ModChangeSummary {
                    title: ss(&u.title),
                    from_ver: ss(&u.from_version),
                    to_ver: ss(&u.to_version_number),
                })
                .collect();
            let enables: Vec<SharedString> = preview.enable_disabled.iter().map(ss).collect();
            let conflicts = preview.conflicts.iter().map(|c| c.title.as_str()).collect::<Vec<_>>().join(", ");
            let missing = preview.missing.iter().map(|m| m.title.as_str()).collect::<Vec<_>>().join(", ");
            let issues: Vec<SharedString> =
                preview.compat_issues.iter().map(|i| ss(format_issue_text(i))).collect();
            let has_errors = !preview.conflicts.is_empty()
                || preview.compat_issues.iter().any(|i| i.severity == CompatSeverity::Error);
            let modal_title = if preview.title.is_empty() { title } else { preview.title.clone() };
            with(|s| {
                s.installing_id = None;
                s.preview = Some(preview);
            });
            let b = ui.global::<LibraryBridge>();
            b.set_m_prev_project_id(ss(&pid_ok));
            b.set_m_prev_title(ss(modal_title));
            b.set_m_prev_upgrades(model(upgrades));
            b.set_m_prev_enables(model(enables));
            b.set_m_prev_conflicts(ss(conflicts));
            b.set_m_prev_missing(ss(missing));
            b.set_m_prev_issues(model(issues));
            b.set_m_prev_has_errors(has_errors);
            b.set_m_prev_committing(false);
            b.set_m_prev_open(true);
            refresh_cards(ui);
        },
        move |ui, err| {
            with(|s| {
                s.installing_id = None;
                s.pending = None;
            });
            refresh_cards(ui);
            let msg = err.envelope().message;
            toast::err(if msg.is_empty() { "Error installing mod".to_string() } else { msg });
        },
    );
}

fn cancel_preview() {
    let Some(ui) = win() else { return };
    let b = ui.global::<LibraryBridge>();
    if b.get_m_prev_committing() {
        return;
    }
    b.set_m_prev_open(false);
    with(|s| s.preview = None);
}

fn confirm_preview(apply_upgrades: bool) {
    let Some(ui) = win() else { return };
    let b = ui.global::<LibraryBridge>();
    if b.get_m_prev_committing() {
        return;
    }
    let (target, pid) = with(|s| {
        let pid = s.preview.as_ref().map(|p| p.project_id.clone()).unwrap_or_default();
        (s.target.clone(), pid)
    });
    if target.is_empty() || pid.is_empty() {
        b.set_m_prev_open(false);
        return;
    }
    b.set_m_prev_committing(true);
    commit_install(target, pid, apply_upgrades, true);
}

fn commit_install(target: String, project_id: String, apply_upgrades: bool, force: bool) {
    let Some(c) = super::ctx() else { return };
    with(|s| {
        s.installing_id = Some(project_id.clone());
        s.pending = Some(PendingInstall { project_id: project_id.clone(), instance: target.clone(), plan: None });
    });
    if let Some(ui) = win() {
        push_search(&ui);
    }
    let (pid, pid_err, inst) = (project_id.clone(), project_id.clone(), target.clone());
    c.call_with(
        move |e| async move { e.commit_modrinth_install(&target, &project_id, apply_upgrades, force).await },
        move |ui, plan| {
            let b = ui.global::<LibraryBridge>();
            b.set_m_prev_committing(false);
            b.set_m_prev_open(false);
            let text = if plan.dependencies.is_empty() {
                format!("{} {}…", i18n::t("mods.installingMod"), plan.project.title)
            } else {
                i18n::tf(
                    "mods.installingWithDeps",
                    &[("name", &plan.project.title), ("count", &plan.dependencies.len().to_string())],
                )
            };
            with(|s| {
                s.preview = None;
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
                        load_compat();
                        store::refresh_instances();
                    }
                })
            });
        },
        move |ui, err| {
            SINGLE_FALLBACK.with(|t| t.stop());
            let b = ui.global::<LibraryBridge>();
            b.set_m_prev_committing(false);
            b.set_m_prev_open(false);
            let conflict = err.raw_code() == Some("mod_conflicts");
            let msg = err.envelope().message;
            with(|s| {
                s.preview = None;
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
            load_compat();
            store::refresh_instances();
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
            load_compat();
            store::refresh_instances();
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
        // A multi-byte character across the 4th byte from the end used to panic.
        assert_eq!(display_name("ñabc", None), "ñabc");
        assert_eq!(display_name("😀a", None), "😀a");
        assert_eq!(display_name("mod-ñ.jar", None), "mod-ñ");
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
            project_id: Some("YL57xq9U".into()),
            mod_id: Some("iris".into()),
            display_name: Some("Iris".into()),
        };
        let r = mod_row(&m, &[], true);
        assert_eq!(r.meta.as_str(), "· 1.5 KB · 01234567");
        assert!(r.can_delete && r.can_switch_version);
        assert!(!r.requires.is_empty());
        assert!(r.required_by.is_empty());
    }
}
