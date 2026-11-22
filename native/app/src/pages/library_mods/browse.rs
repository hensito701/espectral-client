//! ModCatalog Modrinth tab (`#/mods`, tab 2): search with content-type pills,
//! sort, category chips and version/server filters; result cards with 1-click
//! installs; the project detail modal (gallery, plain-text description, versions);
//! and the install flows. Mods go through the compatibility preview, resource
//! packs and shaders through `install_modrinth_pack`, modpacks through
//! `install_modrinth_modpack`. Installed mods and the Mods page itself live in
//! `mods.rs`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use espectral_engine::api::content::ModrinthSearchQuery;
use espectral_engine::packs::PackKind;
use espectral_engine::types::{
    InstanceSummary, Loader, ModInstallPlan, ModInstallPreview, ModRef, ModVersionItem, ModrinthProject,
    ModrinthProjectDetail,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::{gallery, icons, mods, model, ss};
use crate::i18n::{self, Lang};
use crate::{
    router, store, toast, util, AppWindow, LibraryBridge, ModChangeSummary, ModRefRow, ModVersionOption, ModrinthCard,
    ModrinthChip,
};

const SEARCH_PAGE_SIZE: usize = 12;
const DEBOUNCE: Duration = Duration::from_millis(450);
/// Modrinth bodies can run to hundreds of KB; the modal shows the start, the rest is on Modrinth.
const DETAIL_BODY_MAX: usize = 20_000;
/// Content types in the order of the type pills (Modrinth `project_type` values).
const TYPES: [&str; 4] = ["mod", "resourcepack", "shader", "modpack"];
/// Sort options in the order of the dropdown; None = the engine's default for the query.
const SORTS: [Option<&str>; 5] = [None, Some("downloads"), Some("follows"), Some("updated"), Some("newest")];
/// Loader names Modrinth files next to the real categories; never shown as tags or chips.
const LOADER_TAGS: [&str; 8] = ["fabric", "forge", "neoforge", "quilt", "iris", "minecraft", "datapack", "vanilla"];

struct PendingInstall {
    project_id: String,
    instance: String,
    plan: Option<ModInstallPlan>,
}

#[derive(Default)]
struct State {
    /// Index into `TYPES` of the content type last applied (pills and bridge follow it).
    type_idx: i32,
    results: Option<Vec<ModrinthProject>>,
    total: Option<u64>,
    last_batch: usize,
    search_seq: u64,
    searching: bool,
    loading_more: bool,
    search_error: String,
    cat_seq: u64,
    installing_id: Option<String>,
    pending: Option<PendingInstall>,
    /// (project id, conflicting titles)
    conflict: Option<(String, String)>,
    optional: HashMap<String, Vec<ModRef>>,
    /// `instance::project_id` installs confirmed by a terminal event.
    confirmed: HashSet<String>,
    preview: Option<ModInstallPreview>,
    /// Project detail modal: the opened project, its request sequence and the shown gallery image.
    detail_seq: u64,
    detail: Option<ModrinthProjectDetail>,
    gallery_index: usize,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
    static CARDS: Rc<VecModel<ModrinthCard>> = Rc::new(VecModel::default());
    static CATS: Rc<VecModel<ModrinthChip>> = Rc::new(VecModel::default());
    static SINGLE_FALLBACK: Timer = Timer::default();
    static DEBOUNCE_TIMER: Timer = Timer::default();
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn win() -> Option<AppWindow> {
    super::ui_handle()
}

pub fn init(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    b.set_m_results(ModelRc::from(CARDS.with(|c| c.clone())));
    b.set_m_cat_list(ModelRc::from(CATS.with(|c| c.clone())));
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
    b.on_m_set_type(|idx| {
        let Some(ui) = win() else { return };
        if set_type(&ui, idx) {
            run_search(false);
        }
    });
    b.on_m_set_sort(|_| run_search(false));
    b.on_m_set_any_version(|on| {
        if let Some(ui) = win() {
            ui.global::<LibraryBridge>().set_m_search_any_version(on);
        }
        run_search(false);
    });
    b.on_m_set_server_only(|on| {
        if let Some(ui) = win() {
            ui.global::<LibraryBridge>().set_m_search_server_only(on);
        }
        run_search(false);
    });
    b.on_m_cat_toggle(|slug| toggle_category(slug.to_string()));
    b.on_m_install(|id, title, force| install_card(id.to_string(), title.to_string(), force));
    b.on_m_prev_confirm(confirm_preview);
    b.on_m_prev_cancel(cancel_preview);
    b.on_m_open_detail(|id| open_detail(id.to_string()));
    b.on_md_close(close_detail);
    b.on_md_install(|version| detail_install(version.to_string()));
    b.on_md_open_modrinth(open_in_browser);
    b.on_md_gallery_prev(|| gallery_step(-1));
    b.on_md_gallery_next(|| gallery_step(1));
}

/// The Modrinth tab was opened: categories of the current type, and a first search if none ran.
pub(super) fn enter(ui: &AppWindow) {
    load_categories(ui, true);
    let need = with(|s| s.results.is_none() && !s.searching);
    if need {
        run_search(false);
    }
}

/// The selected instance changed: its Modrinth results no longer apply.
pub(super) fn instance_switched(ui: &AppWindow) {
    with(|s| {
        reset_results(s);
        s.conflict = None;
        s.optional.clear();
    });
    rebuild_cards(ui);
    if ui.global::<LibraryBridge>().get_m_tab() == 2 {
        enter(ui);
    }
}

/// Open the Modrinth tab with `query` searched (the compat fix could not resolve a mod id).
pub(super) fn open_search(ui: &AppWindow, query: String) {
    let b = ui.global::<LibraryBridge>();
    b.set_m_search_query(ss(query));
    b.set_m_tab(2);
    load_categories(ui, true);
    run_search(false);
}

/// Screenshot fixture (`ESPECTRAL_SHOT_LIB=mods-fake`): sample results, no network.
pub(super) fn show_fake(ui: &AppWindow) {
    with(|s| {
        let r = super::debug::fake_projects();
        s.total = Some(40);
        s.last_batch = r.len();
        s.results = Some(r);
    });
    rebuild_cards(ui);
}

/// A single-mod install is in flight for `instance` (its terminal event is handled here).
pub(super) fn awaiting_single(instance: &str, has_file: bool) -> bool {
    has_file && with(|s| s.pending.as_ref().is_some_and(|pi| pi.instance == instance))
}

/// Terminal error event of a single-mod install.
pub(super) fn single_failed(ui: &AppWindow, message: String) {
    SINGLE_FALLBACK.with(|t| t.stop());
    let target = mods::target();
    with(|s| {
        if let Some(pi) = s.pending.take() {
            s.confirmed.remove(&format!("{target}::{}", pi.project_id));
            if let Some(r) = s.results.as_mut() {
                for pr in r.iter_mut().filter(|pr| pr.project_id == pi.project_id) {
                    pr.installed = Some(false);
                }
            }
        }
        s.installing_id = None;
    });
    refresh_cards(ui);
    mods::push_mods(ui);
    mods::load_mods();
    mods::load_preset();
    toast::err(message);
}

/// Terminal success event of a single-mod install.
pub(super) fn single_done(ui: &AppWindow, filename: &str) {
    SINGLE_FALLBACK.with(|t| t.stop());
    let target = mods::target();
    let plan = with(|s| {
        let pi = s.pending.take()?;
        s.installing_id = None;
        mark_installed(s, &target, &pi.project_id);
        if let Some(plan) = pi.plan.as_ref() {
            for dep in &plan.dependencies {
                mark_installed(s, &target, &dep.project_id);
            }
            if !plan.optional.is_empty() {
                s.optional.insert(pi.project_id.clone(), plan.optional.clone());
            }
        }
        Some(pi.plan)
    });
    refresh_cards(ui);
    mods::push_mods(ui);
    mods::load_mods();
    mods::load_preset();
    mods::load_compat();
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
}

// ---------------------------------------------------------------- search state

fn type_at(idx: i32) -> &'static str {
    usize::try_from(idx).ok().and_then(|i| TYPES.get(i)).copied().unwrap_or(TYPES[0])
}

fn type_index(project_type: &str) -> Option<i32> {
    TYPES.iter().position(|t| *t == project_type).map(|i| i as i32)
}

fn sort_index(idx: i32) -> usize {
    usize::try_from(idx).ok().filter(|i| *i < SORTS.len()).unwrap_or(0)
}

/// Mods need a mod loader to filter by: a Vanilla instance cannot search them.
fn mods_blocked(ui: &AppWindow) -> bool {
    type_at(ui.global::<LibraryBridge>().get_m_search_type()) == "mod"
        && mods::selected_instance(&mods::target()).is_some_and(|i| matches!(i.loader, Loader::Vanilla))
}

/// Apply a content type. Categories and results belong to one type, so both reset.
/// Returns false when the type did not change.
fn set_type(ui: &AppWindow, idx: i32) -> bool {
    let changed = with(|s| {
        let changed = s.type_idx != idx;
        s.type_idx = idx;
        if changed {
            reset_results(s);
        }
        changed
    });
    ui.global::<LibraryBridge>().set_m_search_type(idx);
    if changed {
        load_categories(ui, false);
        rebuild_cards(ui);
    }
    changed
}

fn reset_results(s: &mut State) {
    s.results = None;
    s.total = None;
    s.last_batch = 0;
    s.search_seq += 1;
    s.searching = false;
    s.loading_more = false;
    s.search_error.clear();
}

/// Fill the category chips of the current content type. `keep` carries the selection
/// over (the tab was opened again); a type switch starts clean.
fn load_categories(ui: &AppWindow, keep: bool) {
    let Some(c) = super::ctx() else { return };
    let kind = type_at(ui.global::<LibraryBridge>().get_m_search_type());
    let previous: HashSet<String> = if keep { selected_slugs() } else { HashSet::new() };
    let seq = with(|s| {
        s.cat_seq += 1;
        s.cat_seq
    });
    c.call_with(
        move |e| async move { e.modrinth_categories(kind).await },
        move |_ui, list| {
            if with(|s| s.cat_seq != seq) {
                return;
            }
            let rows: Vec<ModrinthChip> = list
                .iter()
                .map(|cat| ModrinthChip {
                    slug: ss(&cat.name),
                    label: ss(capitalise(&cat.name)),
                    selected: previous.contains(&cat.name),
                })
                .collect();
            CATS.with(|m| m.set_vec(rows));
        },
        |_ui, err| toast::engine_error(&err),
    );
}

/// Selected category slugs, in chip order.
fn selected_list() -> Vec<String> {
    CATS.with(|m| {
        (0..m.row_count())
            .filter_map(|i| m.row_data(i))
            .filter(|c| c.selected)
            .map(|c| c.slug.to_string())
            .collect()
    })
}

fn selected_slugs() -> HashSet<String> {
    selected_list().into_iter().collect()
}

fn toggle_category(slug: String) {
    CATS.with(|m| {
        for i in 0..m.row_count() {
            if let Some(mut row) = m.row_data(i).filter(|c| c.slug.as_str() == slug.as_str()) {
                row.selected = !row.selected;
                m.set_row_data(i, row);
                break;
            }
        }
    });
    run_search(false);
}

/// Every search goes through here: the bridge holds the query, type, sort, filters and chips.
fn build_query(ui: &AppWindow, sel: Option<&InstanceSummary>, offset: u64) -> ModrinthSearchQuery {
    let b = ui.global::<LibraryBridge>();
    let kind = type_at(b.get_m_search_type());
    let loader = sel
        .map(|i| i.loader)
        .filter(|l| !matches!(l, Loader::Vanilla))
        .filter(|_| matches!(kind, "mod" | "modpack"))
        .map(|l| l.content_loader().to_string());
    ModrinthSearchQuery {
        q: b.get_m_search_query().trim().to_string(),
        version: sel.map(|i| i.version.clone()),
        loader,
        instance: sel.map(|i| i.name.clone()),
        offset,
        sort: SORTS[sort_index(b.get_m_search_sort())].map(str::to_string),
        include_server_only: kind == "mod" && b.get_m_search_server_only(),
        project_type: Some(kind.to_string()),
        categories: selected_list(),
        any_version: b.get_m_search_any_version(),
    }
}

/// Run the Modrinth search for the bridge state (`append` = next page). A pasted
/// Modrinth project link opens its detail modal instead of searching.
pub(super) fn run_search(append: bool) {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    if !append {
        let raw = ui.global::<LibraryBridge>().get_m_search_query().trim().to_string();
        if let Some((kind, slug)) = c.engine().parse_modrinth_url(&raw) {
            if let Some(idx) = type_index(&kind) {
                set_type(&ui, idx);
            }
            open_detail(slug);
            return;
        }
    }
    if mods_blocked(&ui) {
        with(reset_results);
        rebuild_cards(&ui);
        return;
    }
    let sel = mods::selected_instance(&mods::target());
    let (seq, offset) = with(|s| {
        let offset = if append { s.results.as_ref().map_or(0, Vec::len) } else { 0 };
        s.search_seq += 1;
        if append {
            s.loading_more = true;
        } else {
            s.searching = true;
            s.search_error.clear();
        }
        (s.search_seq, offset as u64)
    });
    push_search(&ui);
    let query = build_query(&ui, sel.as_ref(), offset);
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
                let prefix = prefix();
                let new_cards: Vec<ModrinthCard> = with(|s| {
                    let r = s.results.as_deref().unwrap_or(&[]);
                    let start = CARDS.with(|m| m.row_count()).min(r.len());
                    r[start..].iter().map(|p| card(s, p, &prefix)).collect()
                });
                request_icons(&new_cards);
                CARDS.with(|m| {
                    for card in new_cards {
                        m.push(card);
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
                if append {
                    push_search(ui);
                } else {
                    rebuild_cards(ui);
                }
            }
        },
    );
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

/// `instance::` prefix of the confirmed-install keys.
fn prefix() -> String {
    format!("{}::", mods::target())
}

pub(super) fn push_search(ui: &AppWindow) {
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
    b.set_m_mods_blocked(mods_blocked(ui));
}

// ---------------------------------------------------------------- cards

/// Rebuild every card (keeps the model object so the grid stays bound).
fn rebuild_cards(ui: &AppWindow) {
    let prefix = prefix();
    let cards: Vec<ModrinthCard> = with(|s| {
        s.results.as_ref().map(|r| r.iter().map(|p| card(s, p, &prefix)).collect()).unwrap_or_default()
    });
    request_icons(&cards);
    CARDS.with(|m| m.set_vec(cards));
    push_search(ui);
}

/// Refresh the cards' dynamic fields in place (installed / conflict / optional).
fn refresh_cards(ui: &AppWindow) {
    let prefix = prefix();
    CARDS.with(|m| {
        with(|s| {
            let Some(results) = s.results.as_ref() else { return };
            for (i, p) in results.iter().enumerate() {
                if i >= m.row_count() {
                    break;
                }
                m.set_row_data(i, card(s, p, &prefix));
            }
        });
    });
    push_search(ui);
    refresh_detail(ui);
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
        author: ss(&p.author),
        downloads: ss(group_digits(p.downloads, i18n::lang())),
        updated: ss(updated_ago(&p.date_modified)),
        categories: model(tags(&p.categories, 3).iter().map(|c| ss(c)).collect()),
        side: ss(side_of(p.client_side.as_deref(), p.server_side.as_deref())),
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

/// Card side badge: "both" when the client and the server accept the mod, "client" or
/// "server" for one side only, "" when Modrinth does not say.
fn side_of(client: Option<&str>, server: Option<&str>) -> &'static str {
    let supports = |v: Option<&str>| matches!(v, Some("required" | "optional"));
    match (supports(client), supports(server)) {
        (true, true) => "both",
        (true, false) => "client",
        (false, true) => "server",
        (false, false) => "",
    }
}

/// Category chips of a project: loader names dropped, the first `max` kept.
fn tags(categories: &[String], max: usize) -> Vec<String> {
    categories.iter().filter(|c| !LOADER_TAGS.contains(&c.as_str())).take(max).map(|c| capitalise(c)).collect()
}

/// Category slug -> label ("optimization" -> "Optimization").
fn capitalise(slug: &str) -> String {
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Relative "updated" text, empty when Modrinth gave no date.
fn updated_ago(iso: &str) -> String {
    if iso.is_empty() {
        String::new()
    } else {
        util::time_ago(iso, i18n::lang())
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
fn group_digits(n: u64, lang: Lang) -> String {
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

// ---------------------------------------------------------------- installs

fn mark_installed(s: &mut State, target: &str, project_id: &str) {
    s.confirmed.insert(format!("{target}::{project_id}"));
    if let Some(r) = s.results.as_mut() {
        for p in r.iter_mut().filter(|p| p.project_id == project_id) {
            p.installed = Some(true);
        }
    }
}

/// Mark the card as busy and return the target instance; None when no instance is selected.
fn begin_install(project_id: &str) -> Option<String> {
    let target = mods::target();
    if target.is_empty() {
        return None;
    }
    with(|s| s.installing_id = Some(project_id.to_string()));
    if let Some(ui) = win() {
        push_search(&ui);
    }
    Some(target)
}

fn install_card(project_id: String, title: String, force: bool) {
    let kind = with(|s| s.results.iter().flatten().find(|p| p.project_id == project_id).map(|p| p.project_type.clone()))
        .unwrap_or_default();
    install_project(project_id, title, &kind, force, None);
}

/// Route an install by content type. `version_id` only applies to packs and modpacks:
/// the mod flow installs the build the compatibility preview picks.
fn install_project(project_id: String, title: String, kind: &str, force: bool, version_id: Option<String>) {
    match kind {
        "resourcepack" => install_pack(project_id, title, PackKind::ResourcePack, version_id),
        "shader" => install_pack(project_id, title, PackKind::Shader, version_id),
        "modpack" => install_modpack(project_id, version_id),
        _ => install_mod(project_id, title, force),
    }
}

fn install_pack(project_id: String, title: String, kind: PackKind, version_id: Option<String>) {
    let Some(c) = super::ctx() else { return };
    let Some(target) = begin_install(&project_id) else { return };
    let shader = kind == PackKind::Shader;
    let inst = target.clone();
    let pid = project_id.clone();
    c.call_with(
        move |e| async move { e.install_modrinth_pack(&target, &project_id, version_id, kind, true).await },
        move |ui, res| {
            with(|s| {
                s.installing_id = None;
                mark_installed(s, &inst, &pid);
            });
            refresh_cards(ui);
            toast::ok(i18n::tf("browse.packInstalled", &[("name", &title), ("instance", &inst)]));
            if res.note.as_deref() == Some("running") {
                toast::info(i18n::tf("browse.packRunning", &[("name", &title), ("instance", &inst)]));
            }
            if shader && res.needs_iris {
                toast::info(i18n::tf("mods.drop.needsIris", &[("instance", &inst)]));
            }
        },
        move |ui, err| {
            with(|s| s.installing_id = None);
            refresh_cards(ui);
            toast::engine_error(&err);
        },
    );
}

/// A modpack becomes a new instance: like the .mrpack import, navigate to it.
fn install_modpack(project_id: String, version_id: Option<String>) {
    let Some(c) = super::ctx() else { return };
    with(|s| s.installing_id = Some(project_id.clone()));
    if let Some(ui) = win() {
        push_search(&ui);
    }
    c.call_with(
        move |e| async move { e.install_modrinth_modpack(&project_id, version_id.as_deref(), None).await },
        move |ui, reply| {
            with(|s| s.installing_id = None);
            refresh_cards(ui);
            let name = reply.summary.name.clone();
            if reply.already_exists == Some(true) {
                toast::info(i18n::tf("browse.modpackExists", &[("name", &name)]));
            } else {
                toast::ok(i18n::tf("browse.modpackInstalled", &[("name", &name)]));
            }
            store::refresh_instances();
            router::navigate(ui, &router::instance_route(&name));
        },
        move |ui, err| {
            with(|s| s.installing_id = None);
            refresh_cards(ui);
            toast::engine_error(&err);
        },
    );
}

pub(super) fn install_mod(project_id: String, title: String, force: bool) {
    let Some(c) = super::ctx() else { return };
    let Some(target) = begin_install(&project_id) else { return };
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
                preview.compat_issues.iter().map(|i| ss(mods::format_issue_text(i))).collect();
            let has_errors = !preview.conflicts.is_empty()
                || preview.compat_issues.iter().any(|i| i.severity == espectral_engine::fabric_compat::CompatSeverity::Error);
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
    let target = mods::target();
    let pid = with(|s| s.preview.as_ref().map(|p| p.project_id.clone()).unwrap_or_default());
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
            // Terminal mod-progress event confirms the install; this fallback clears a stuck card.
            let (pending_id, pending_inst) = (pid.clone(), inst.clone());
            SINGLE_FALLBACK.with(|t| {
                t.start(TimerMode::SingleShot, mods::FALLBACK, move || {
                    let current = mods::target();
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
                        if pending_inst == current {
                            mark_installed(s, &pending_inst, &pending_id);
                            true
                        } else {
                            false
                        }
                    });
                    if let Some(ui) = win() {
                        refresh_cards(&ui);
                    }
                    if reload {
                        mods::load_mods();
                        mods::load_preset();
                        mods::load_compat();
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

// ---------------------------------------------------------------- detail modal

/// Open the project detail modal for a Modrinth id or slug.
fn open_detail(id_or_slug: String) {
    let (Some(c), Some(ui)) = (super::ctx(), win()) else { return };
    let seq = with(|s| {
        s.detail_seq += 1;
        s.detail = None;
        s.gallery_index = 0;
        s.detail_seq
    });
    let b = ui.global::<LibraryBridge>();
    b.set_md_open(true);
    b.set_md_loading(true);
    b.set_md_error(ss(""));
    b.set_md_tab(0);
    b.set_md_gallery_count(0);
    b.set_md_gallery_has_image(false);
    b.set_md_gallery_loading(false);
    b.set_md_versions(model(Vec::<ModVersionOption>::new()));
    b.set_md_installed(false);
    let target = mods::target();
    let instance = (!target.is_empty()).then_some(target);
    c.call_with(
        move |e| async move { e.modrinth_project_detail(&id_or_slug, instance.as_deref()).await },
        move |ui, d| {
            if with(|s| s.detail_seq == seq) {
                show_detail(ui, d);
            }
        },
        move |ui, err| {
            if with(|s| s.detail_seq != seq) {
                return;
            }
            let b = ui.global::<LibraryBridge>();
            b.set_md_loading(false);
            let msg = err.envelope().message;
            b.set_md_error(ss(if msg.is_empty() { i18n::t("browse.detailError") } else { msg }));
        },
    );
}

fn show_detail(ui: &AppWindow, d: ModrinthProjectDetail) {
    let b = ui.global::<LibraryBridge>();
    let lang = i18n::lang();
    b.set_md_loading(false);
    b.set_md_project_id(ss(&d.project_id));
    b.set_md_title(ss(&d.title));
    b.set_md_author(ss(&d.author));
    b.set_md_project_type(ss(&d.project_type));
    b.set_md_downloads(ss(group_digits(d.downloads, lang)));
    b.set_md_followers(ss(group_digits(d.followers, lang)));
    b.set_md_updated(ss(updated_ago(&d.updated)));
    b.set_md_license(ss(&d.license_id));
    b.set_md_side(ss(side_of(Some(d.client_side.as_str()), Some(d.server_side.as_str()))));
    b.set_md_categories(model(tags(&d.categories, usize::MAX).iter().map(|c| ss(c)).collect()));
    let source = if d.body.trim().is_empty() { &d.description } else { &d.body };
    b.set_md_body(ss(plain_text(source)));
    b.set_md_project_url(ss(&d.project_url));
    let installed = d.installed || with(|s| s.confirmed.contains(&format!("{}{}", prefix(), d.project_id)));
    b.set_md_installed(installed);
    b.set_md_versions(model(d.versions.iter().map(version_option).collect()));

    let icon_url = d.icon_url.clone().unwrap_or_default();
    let cached = if icon_url.is_empty() { None } else { icons::get(&icon_url) };
    let has_icon = cached.is_some();
    b.set_md_has_icon(has_icon);
    b.set_md_icon(cached.unwrap_or_default());
    if !has_icon && !icon_url.is_empty() {
        if let Some(c) = super::ctx() {
            icons::request(&c, &icon_url, detail_icon_ready);
        }
    }

    b.set_md_gallery_count(d.gallery.len() as i32);
    with(|s| s.detail = Some(d));
    show_gallery(ui, 0);
}

fn detail_icon_ready(ui: &AppWindow, url: &str) {
    let ours = with(|s| s.detail.as_ref().and_then(|d| d.icon_url.as_deref()) == Some(url));
    if !ours {
        return;
    }
    let Some(img) = icons::get(url) else { return };
    let b = ui.global::<LibraryBridge>();
    b.set_md_icon(img);
    b.set_md_has_icon(true);
}

/// Show gallery image `index` of the open project (loads it when it is not cached yet).
fn show_gallery(ui: &AppWindow, index: usize) {
    let item = with(|s| {
        s.gallery_index = index;
        s.detail.as_ref().and_then(|d| d.gallery.get(index).cloned())
    });
    let b = ui.global::<LibraryBridge>();
    b.set_md_gallery_index(index as i32);
    let Some(item) = item else {
        b.set_md_gallery_has_image(false);
        b.set_md_gallery_loading(false);
        b.set_md_gallery_title(ss(""));
        return;
    };
    b.set_md_gallery_title(ss(&item.title));
    match gallery::get(&item.url) {
        Some(img) => {
            b.set_md_gallery_image(img);
            b.set_md_gallery_has_image(true);
            b.set_md_gallery_loading(false);
        }
        None => {
            b.set_md_gallery_has_image(false);
            b.set_md_gallery_loading(true);
            if let Some(c) = super::ctx() {
                gallery::request(&c, &item.url, gallery_ready);
            }
        }
    }
}

fn gallery_ready(ui: &AppWindow, url: &str) {
    let shown = with(|s| {
        s.detail.as_ref().and_then(|d| d.gallery.get(s.gallery_index)).is_some_and(|g| g.url == url)
    });
    if !shown {
        return;
    }
    let Some(img) = gallery::get(url) else { return };
    let b = ui.global::<LibraryBridge>();
    b.set_md_gallery_image(img);
    b.set_md_gallery_has_image(true);
    b.set_md_gallery_loading(false);
}

fn gallery_step(delta: i32) {
    let Some(ui) = win() else { return };
    let (count, cur) = with(|s| (s.detail.as_ref().map_or(0, |d| d.gallery.len()), s.gallery_index));
    if count == 0 {
        return;
    }
    let next = (cur as i64 + i64::from(delta)).rem_euclid(count as i64) as usize;
    show_gallery(&ui, next);
}

fn version_option(v: &ModVersionItem) -> ModVersionOption {
    ModVersionOption {
        version_id: ss(&v.version_id),
        version_number: ss(&v.version_number),
        name: ss(&v.name),
        version_type: ss(&v.version_type),
        date: ss(updated_ago(&v.date_published)),
        installed: v.installed,
    }
}

/// Footer Install and the per-version buttons: `version_id` "" = the newest build.
fn detail_install(version_id: String) {
    let Some((pid, title, kind)) =
        with(|s| s.detail.as_ref().map(|d| (d.project_id.clone(), d.title.clone(), d.project_type.clone())))
    else {
        return;
    };
    let version = (!version_id.is_empty()).then_some(version_id);
    install_project(pid, title, &kind, false, version);
}

fn close_detail() {
    // a late answer for the closed modal is ignored
    with(|s| s.detail_seq += 1);
    if let Some(ui) = win() {
        ui.global::<LibraryBridge>().set_md_open(false);
    }
}

fn open_in_browser() {
    if let Some(url) = with(|s| s.detail.as_ref().map(|d| d.project_url.clone())) {
        util::open_url(&url);
    }
}

/// Refresh the detail modal's "installed" flag after an install changed the confirmed set.
fn refresh_detail(ui: &AppWindow) {
    let prefix = prefix();
    let installed = with(|s| {
        s.detail.as_ref().is_some_and(|d| d.installed || s.confirmed.contains(&format!("{prefix}{}", d.project_id)))
    });
    ui.global::<LibraryBridge>().set_md_installed(installed);
}

/// Modrinth markdown + HTML body -> plain text paragraphs for the modal. No HTML engine:
/// tags go, entities decode, markdown marks and images go, link labels stay, bullets
/// become "•", code fences keep their lines, and the result is capped at `DETAIL_BODY_MAX`.
fn plain_text(md: &str) -> String {
    let stripped = strip_html(md);
    let mut lines: Vec<String> = Vec::new();
    let mut fenced = false;
    for raw in stripped.lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            lines.push(line.to_string());
            continue;
        }
        let line = line.trim();
        if is_rule(line) {
            lines.push(String::new());
            continue;
        }
        let line = if line.starts_with('#') { line.trim_start_matches('#').trim_start() } else { line };
        let text = match line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ ")) {
            Some(rest) => format!("• {}", inline_plain(rest)),
            None => inline_plain(line),
        };
        lines.push(text.trim().to_string());
    }
    // collapse runs of blank lines and trim both ends
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let text = out.join("\n");
    if text.chars().count() > DETAIL_BODY_MAX {
        let mut cut: String = text.chars().take(DETAIL_BODY_MAX).collect();
        cut.push('…');
        cut
    } else {
        text
    }
}

fn is_rule(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

/// Remove markdown emphasis, code marks, images; keep link labels.
fn inline_plain(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some((_, after)) = link_parts(&chars, i + 1) {
                i = after;
                continue;
            }
        }
        if c == '[' {
            if let Some((label_end, after)) = link_parts(&chars, i) {
                out.extend(&chars[i + 1..label_end]);
                i = after;
                continue;
            }
        }
        if c == '*' || c == '`' {
            i += 1;
            continue;
        }
        if (c == '_' || c == '~') && chars.get(i + 1) == Some(&c) {
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// For `[label](url)` starting at `open` (the `[`): (index of `]`, index after `)`).
fn link_parts(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let close = (open + 1..chars.len()).find(|&j| chars[j] == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|&k| chars[k] == ')')?;
    Some((close, end + 1))
}

/// Drop tags (block-level ones keep a line break) and decode the common entities.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after = &rest[lt + 1..];
        let is_tag = after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        match after.find('>') {
            Some(gt) if is_tag => {
                let name = after[..gt]
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "br" | "p" | "div" | "li" | "tr" | "ul" | "ol" | "table" | "hr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                ) {
                    out.push('\n');
                }
                rest = &after[gt + 1..];
            }
            _ => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
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
    fn plain_text_strips_markdown_and_html() {
        let md = "# Title\n\n<p align=\"center\">Hi &amp; bye</p>\n\n- one **bold** [link](https://x.y)\n\
                  ![badge](https://img)\n\n---\n```\nlet a = 1;\n```";
        assert_eq!(plain_text(md), "Title\n\nHi & bye\n\n• one bold link\n\nlet a = 1;");
    }

    #[test]
    fn plain_text_keeps_lone_angle_brackets() {
        assert_eq!(plain_text("a < b and c > d"), "a < b and c > d");
    }

    #[test]
    fn side_badges() {
        assert_eq!(side_of(Some("required"), Some("required")), "both");
        assert_eq!(side_of(Some("optional"), Some("unsupported")), "client");
        assert_eq!(side_of(Some("unsupported"), Some("required")), "server");
        assert_eq!(side_of(Some("unknown"), Some("unknown")), "");
    }

    #[test]
    fn tags_skip_loaders_and_cap() {
        let cats: Vec<String> =
            ["fabric", "optimization", "utility", "social", "library"].iter().map(|s| s.to_string()).collect();
        assert_eq!(tags(&cats, 3), ["Optimization", "Utility", "Social"]);
    }

    #[test]
    fn type_pills_map_to_project_types() {
        assert_eq!(type_at(0), "mod");
        assert_eq!(type_at(3), "modpack");
        assert_eq!(type_at(9), "mod");
        assert_eq!(type_index("shader"), Some(2));
        assert_eq!(type_index("plugin"), None);
    }
}
