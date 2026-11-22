//! ModCatalog (`#/mods`): instance chips, installed mods inventory (filters,
//! optimistic enable/disable), performance preset (engine install_mod_preset),
//! and live `mod-progress` tracking. The Modrinth tab lives in `browse.rs`; this file
//! keeps the installed-mods side and the page wiring.
//! Port of pages/ModCatalog.svelte (same terminal-event rules and fallbacks).

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use espectral_engine::events::{EngineEvent, ProgressEvent};
use espectral_engine::fabric_compat::{
    format_predicate_human, known_modrinth_for_id, CompatFix, CompatIssueKind, CompatSeverity, ModCompatIssue,
    ModCompatReport,
};
use espectral_engine::types::{
    InstanceSummary, ModEntry, ModPreset, ModPresetInfo, ModVersionItem, ModVersionSwitchPlan,
};
use slint::{ComponentHandle, SharedString, Timer, TimerMode};

use super::{browse, model, ss};
use crate::bridge::{self, Ctx};
use crate::i18n::{self, Lang};
use crate::router::Route;
use crate::{
    prefs, store, toast, util, AppWindow, LibraryBridge, ModChangeSummary, ModCompatNote, ModRow, ModVersionOption,
    PinnedPill,
};

const PREFS_INSTANCE: &str = "mods.instance";
pub(super) const FALLBACK: Duration = Duration::from_secs(90);

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
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
    static PRESET_FALLBACK: Timer = Timer::default();
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
    if let Some(saved) = prefs::get_str(PREFS_INSTANCE) {
        with(|s| s.target = saved);
    }

    b.on_m_select_instance(|name| select_instance(name.to_string()));
    b.on_m_set_tab(|tab| {
        let Some(ui) = win() else { return };
        ui.global::<LibraryBridge>().set_m_tab(tab);
        if tab == 2 {
            browse::enter(&ui);
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
        });
        push_all(ui);
        browse::show_fake(ui);
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

/// Instance the Mods page manages (the Modrinth tab in browse.rs reads it too).
pub(super) fn target() -> String {
    with(|s| s.target.clone())
}

pub(super) fn selected_instance(target: &str) -> Option<InstanceSummary> {
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
        with(|s| {
            s.mods.clear();
            s.compat = None;
            s.preset = None;
        });
        browse::instance_switched(&ui);
        ui.global::<LibraryBridge>().set_m_filter(0);
        push_mods(&ui);
        load_mods();
        load_preset();
        load_compat();
    }
}

pub(super) fn load_compat() {
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

pub(super) fn load_mods() {
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

pub(super) fn load_preset() {
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
    browse::push_search(ui);
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

pub(crate) fn format_issue_text(i: &ModCompatIssue) -> String {
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
        Some(CompatFix::UpdateFabricLoader { required }) => (
            i18n::tf("mods.fixLoader", &[("req", &format_predicate_human(required, i18n::lang() == Lang::Es))]),
            "loader".to_string(),
            String::new(),
            String::new(),
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

pub(super) fn push_mods(ui: &AppWindow) {
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
        "loader" => fix_safe_mods(),
        _ => {}
    }
}

/// "Fix" on a Fabric Loader issue: the batch fix re-pins the loader and applies the other safe fixes.
fn fix_safe_mods() {
    let Some(c) = super::ctx() else { return };
    let target = with(|s| s.target.clone());
    if target.is_empty() {
        return;
    }
    c.call_with(
        move |e| async move { e.apply_safe_compat_fixes(&target).await },
        |_ui, r| {
            let text = fix_summary(&r);
            if r.remaining_errors == 0 {
                toast::ok(text);
            } else {
                toast::err(text);
            }
            load_mods();
            load_compat();
        },
        |_ui, e| toast::engine_error(&e),
    );
}

/// The instance the Mods page shows; a dropped mod lands in it.
pub(crate) fn target_instance() -> String {
    with(|s| s.target.clone())
}

/// One-line summary of a batch fix for a toast; `remaining_errors` says what still blocks a launch.
pub(crate) fn fix_summary(r: &espectral_engine::types::CompatBatchFixResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !r.enabled.is_empty() {
        parts.push(i18n::tf("mods.batch.enabled", &[("count", &r.enabled.len().to_string())]));
    }
    if !r.disabled.is_empty() {
        parts.push(i18n::tf("mods.batch.disabled", &[("count", &r.disabled.len().to_string())]));
    }
    if !r.installed.is_empty() {
        parts.push(i18n::tf("mods.batch.installed", &[("count", &r.installed.len().to_string())]));
    }
    if !r.updated.is_empty() {
        parts.push(i18n::tf("mods.batch.updated", &[("count", &r.updated.len().to_string())]));
    }
    if r.loader_refreshed {
        parts.push(i18n::t("mods.batch.loader"));
    }
    if parts.is_empty() {
        parts.push(i18n::t("mods.batch.none"));
    }
    if r.remaining_errors > 0 {
        parts.push(i18n::tf("mods.batch.remaining", &[("count", &r.remaining_errors.to_string())]));
    }
    parts.join(" · ")
}

fn install_missing_mod_id(mod_id: String, display: String) {
    let Some(c) = super::ctx() else { return };
    let inst = target();
    if inst.is_empty() {
        return;
    }
    let id2 = mod_id.clone();
    let id_err = mod_id.clone();
    c.call_with(
        move |e| async move { e.resolve_modrinth_for_mod_id(&inst, &id2).await },
        move |ui, found| {
            if let Some(mref) = found {
                browse::install_mod(mref.project_id, mref.title, false);
            } else {
                // Fallback: switch to Explorar Modrinth pre-filled with the missing mod id.
                browse::open_search(ui, if display.is_empty() { mod_id } else { display });
            }
        },
        move |ui, _err| browse::open_search(ui, id_err),
    );
}

// ---------------------------------------------------------------- events

fn as_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
}

fn on_mod_progress(ui: &AppWindow, p: &ProgressEvent) {
    let target = target();
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
    let was_active = with(|s| s.installing_preset || s.progress.is_some());
    // A single-mod install reports through the same channel; its terminal events belong to browse.rs.
    let is_single = browse::awaiting_single(&target, !filename.is_empty() && p.preset.is_none());
    let clear_terminal = |s: &mut State| {
        s.progress = None;
        s.installing_preset = false;
    };

    if phase == "error" {
        PRESET_FALLBACK.with(|t| t.stop());
        with(clear_terminal);
        if is_single {
            browse::single_failed(ui, message);
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
            browse::single_done(ui, &filename);
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
