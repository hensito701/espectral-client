//! Resource Packs and Shaders tabs of ModCatalog (`#/mods?tab=packs|shaders`):
//! the options.txt priority stack (move, toggle, nested-zip unwrap, delete),
//! compat and texture-overlap rows, pack.png thumbnails decoded off the UI
//! thread, Modrinth update badges, cross-instance copy, the server-pack cache
//! and the Iris shader list with its master toggle (smart-mods plan 4.2).
//!
//! The target instance is the ModCatalog one (`mods::target_instance`). The
//! panes load lazily when their tab is shown, and mods.rs calls
//! [`target_changed`] when the instance switches.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use espectral_engine::packs::{
    PackCompatStatus, PackFormatRange, PackKind, PackUpdate, ResourcePackEntry, ServerPackCacheInfo, ShaderPackEntry,
    ShadersState,
};
use espectral_engine::EngineError;
use slint::{ComponentHandle, Image};

use super::{icons, model, ss};
use crate::{i18n, store, toast, util, AppWindow, LibraryBridge, RpCopyInstanceRow, RpCopyPackRow, RpRow, ShRow};

/// Show the server-pack cache pill once `downloads/` grows past this.
const SERVER_CACHE_WARN: u64 = 50 * 1024 * 1024;
/// Upper bound of decoded pack thumbnails kept in memory.
const ICON_CAP: usize = 256;

#[derive(Default)]
struct State {
    packs_seq: u64,
    shaders_seq: u64,
    packs: Vec<ResourcePackEntry>,
    cache: ServerPackCacheInfo,
    rp_updates: HashMap<String, PackUpdate>,
    /// The ModCatalog target the resource-pack data belongs to.
    last_target: String,
    loading: bool,
    error: String,
    shaders: Option<ShadersState>,
    sh_loading: bool,
    sh_error: String,
    sh_updates: HashMap<String, PackUpdate>,
    /// A pack mutation (order, toggle, unwrap, delete, shader pick) is in flight.
    busy: bool,
    checking: bool,
    installing_iris: bool,
    // Copy-from-instance modal
    copy_source: String,
    copy_rows: Vec<RpCopyPackRow>,
    copy_loading: bool,
    copy_busy: bool,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
    /// Decoded pack.png thumbnails keyed by content (see [`icon_key`]).
    static ICONS: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
    static ICON_PENDING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn win() -> Option<AppWindow> {
    super::ui_handle()
}

/// The instance the ModCatalog works on.
fn target() -> String {
    super::mods::target_instance()
}

/// Engine error text for a toast; a running instance gets a fixed message.
fn err_text(e: &EngineError) -> String {
    let env = e.envelope();
    if env.code == "ALREADY_RUNNING" {
        i18n::t("packs.alreadyRunning")
    } else if env.message.is_empty() {
        i18n::t("packs.opError")
    } else {
        env.message
    }
}

fn compat_str(c: PackCompatStatus) -> &'static str {
    match c {
        PackCompatStatus::Compatible => "compatible",
        PackCompatStatus::TooOld => "too_old",
        PackCompatStatus::TooNew => "too_new",
        PackCompatStatus::InvalidMeta => "invalid_meta",
        PackCompatStatus::NestedArchive => "nested_archive",
        PackCompatStatus::Unknown => "unknown",
    }
}

/// Packs that will not load as-is (the filter chip "Issues").
fn has_issue(c: PackCompatStatus) -> bool {
    !matches!(c, PackCompatStatus::Compatible | PackCompatStatus::Unknown)
}

/// "34", "34.1", "34–46" or "34+" when the pack declares no upper bound.
fn format_range(r: &PackFormatRange) -> String {
    let major = |(maj, min): (u32, u32)| if min == 0 { maj.to_string() } else { format!("{maj}.{min}") };
    if r.min == r.max {
        major(r.min)
    } else if r.max.1 == u32::MAX {
        format!("{}+", major(r.min))
    } else {
        format!("{}–{}", major(r.min), major(r.max))
    }
}

/// Thumbnail cache key: the file's content hash, so a replaced pack refreshes.
fn icon_key(target: &str, p: &ResourcePackEntry) -> String {
    match p.sha1.as_deref().filter(|s| !s.is_empty()) {
        Some(sha) => format!("{target}::{sha}"),
        None => format!("{target}::{}#{}", p.filename, p.size),
    }
}

fn size_text(bytes: u64) -> String {
    if bytes > 0 {
        util::format_bytes(bytes as f64)
    } else {
        String::new()
    }
}

fn rp_row(p: &ResourcePackEntry, target: &str, update: Option<&PackUpdate>) -> RpRow {
    let icon = if p.has_icon { ICONS.with(|c| c.borrow().get(&icon_key(target, p)).cloned()) } else { None };
    let name = if p.display_name.is_empty() { p.filename.clone() } else { p.display_name.clone() };
    RpRow {
        filename: ss(&p.filename),
        name: ss(name),
        version: ss(p.version_number.as_deref().unwrap_or("")),
        description: ss(&p.description),
        size: ss(size_text(p.size)),
        enabled: p.enabled,
        priority: p.priority.map(|i| i as i32).unwrap_or(-1),
        compat: ss(compat_str(p.compat)),
        format_range: ss(p.format_range.as_ref().map(format_range).unwrap_or_default()),
        nested: p.compat == PackCompatStatus::NestedArchive,
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        overrides_count: p.overrides_count as i32,
        overridden_by: ss(p.overridden_by_names.join(", ")),
        overridden_count: p.overridden_by_count as i32,
        tags: model(p.texture_tags.iter().map(ss).collect()),
        has_update: update.is_some(),
        update_ver: ss(update.and_then(|u| u.to_version_number.clone()).unwrap_or_default()),
    }
}

fn sh_row(p: &ShaderPackEntry, update: Option<&PackUpdate>) -> ShRow {
    let name = p.filename.strip_suffix(".zip").unwrap_or(p.filename.as_str());
    ShRow {
        filename: ss(&p.filename),
        name: ss(name),
        version: ss(p.version_number.as_deref().unwrap_or("")),
        size: ss(size_text(p.size)),
        active: p.active,
        has_update: update.is_some(),
        update_ver: ss(update.and_then(|u| u.to_version_number.clone()).unwrap_or_default()),
    }
}

// ---------------------------------------------------------------- bridge sync

fn push_packs(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    let filter = b.get_rp_filter();
    let query = b.get_rp_query().trim().to_lowercase();
    with(|s| {
        let matches = |p: &ResourcePackEntry| {
            query.is_empty()
                || p.filename.to_lowercase().contains(&query)
                || p.display_name.to_lowercase().contains(&query)
                || p.description.to_lowercase().contains(&query)
        };
        // filter: 0 all, 1 active only, 2 inactive only, 3 packs with issues
        let shown = |p: &ResourcePackEntry| {
            matches(p)
                && (filter != 3 || has_issue(p.compat))
                && (filter != 1 || p.enabled)
                && (filter != 2 || !p.enabled)
        };
        let row = |p: &ResourcePackEntry| rp_row(p, &s.last_target, s.rp_updates.get(&p.filename));
        let active: Vec<RpRow> = s.packs.iter().filter(|p| p.enabled).filter(|p| shown(p)).map(row).collect();
        let inactive: Vec<RpRow> = s.packs.iter().filter(|p| !p.enabled).filter(|p| shown(p)).map(row).collect();
        let active_total = s.packs.iter().filter(|p| p.enabled).count();
        let issues = s.packs.iter().filter(|p| has_issue(p.compat)).count();
        b.set_rp_active(model(active));
        b.set_rp_inactive(model(inactive));
        b.set_rp_count_active(active_total as i32);
        b.set_rp_count_inactive((s.packs.len() - active_total) as i32);
        b.set_rp_count_issues(issues as i32);
        b.set_rp_loading(s.loading);
        b.set_rp_error(ss(&s.error));
        b.set_rp_busy(s.busy);
        b.set_rp_update_count(s.rp_updates.len() as i32);
        b.set_rp_checking_updates(s.checking);
        b.set_rp_server_cache_files(s.cache.file_count as i32);
        b.set_rp_server_cache_size(ss(if s.cache.total_bytes > SERVER_CACHE_WARN {
            util::format_bytes(s.cache.total_bytes as f64)
        } else {
            String::new()
        }));
    });
}

fn push_shaders(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    with(|s| {
        let rows: Vec<ShRow> = s
            .shaders
            .as_ref()
            .map(|st| st.packs.iter().map(|p| sh_row(p, s.sh_updates.get(&p.filename))).collect())
            .unwrap_or_default();
        b.set_sh_iris_installed(s.shaders.as_ref().is_some_and(|st| st.iris_installed));
        b.set_sh_enabled(s.shaders.as_ref().is_some_and(|st| st.shaders_enabled));
        b.set_sh_rows(model(rows));
        b.set_sh_loading(s.sh_loading);
        b.set_sh_error(ss(&s.sh_error));
        b.set_sh_busy(s.busy);
        b.set_sh_installing_iris(s.installing_iris);
    });
}

fn push_copy(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    with(|s| {
        b.set_rp_copy_source(ss(&s.copy_source));
        b.set_rp_copy_packs(model(s.copy_rows.clone()));
        b.set_rp_copy_loading(s.copy_loading);
        b.set_rp_copy_busy(s.copy_busy);
    });
}

/// Decode the pack.png thumbnails of rows that declare one and are not cached yet.
fn request_icons(packs: &[ResourcePackEntry], target: &str) {
    let Some(c) = super::ctx() else { return };
    for p in packs.iter().filter(|p| p.has_icon) {
        let key = icon_key(target, p);
        if ICONS.with(|i| i.borrow().contains_key(&key)) {
            continue;
        }
        if !ICON_PENDING.with(|i| i.borrow_mut().insert(key.clone())) {
            continue;
        }
        let (target, file) = (target.to_string(), p.filename.clone());
        let c2 = c.clone();
        c.spawn(async move {
            let engine = c2.engine().clone();
            let bytes = engine.resource_pack_icon(&target, &file).await.ok().flatten();
            let buf = match bytes {
                Some(b) => tokio::task::spawn_blocking(move || icons::decode(&b)).await.ok().flatten(),
                None => None,
            };
            c2.on_ui(move |ui| {
                ICON_PENDING.with(|i| i.borrow_mut().remove(&key));
                let Some(buf) = buf else { return };
                ICONS.with(|i| {
                    let mut i = i.borrow_mut();
                    if i.len() >= ICON_CAP {
                        i.clear();
                    }
                    i.insert(key, Image::from_rgba8(buf));
                });
                push_packs(ui);
            });
        });
    }
}

// ---------------------------------------------------------------- loading

fn load_packs() {
    let Some(c) = super::ctx() else { return };
    let target = target();
    let seq = with(|s| {
        s.packs_seq += 1;
        s.loading = true;
        s.error.clear();
        s.packs_seq
    });
    if target.is_empty() {
        with(|s| {
            s.packs.clear();
            s.loading = false;
        });
        if let Some(ui) = win() {
            push_packs(&ui);
        }
        return;
    }
    if let Some(ui) = win() {
        push_packs(&ui);
    }
    let (t_call, t_ok, t_err) = (target.clone(), target.clone(), target);
    c.call_with(
        move |e| async move {
            let packs = e.list_resource_packs(&t_call).await?;
            // A failed cache probe only hides the pill; the list still matters.
            let cache = e
                .server_pack_cache_info(&t_call)
                .await
                .unwrap_or(ServerPackCacheInfo { file_count: 0, total_bytes: 0 });
            Ok((packs, cache))
        },
        move |ui, (packs, cache)| {
            let current = with(|s| {
                if s.packs_seq != seq {
                    return false;
                }
                s.packs = packs.clone();
                s.cache = cache;
                s.last_target = t_ok.clone();
                s.loading = false;
                true
            });
            if current {
                request_icons(&packs, &t_ok);
                push_packs(ui);
            }
        },
        move |ui, err| {
            let current = with(|s| {
                if s.packs_seq != seq {
                    return false;
                }
                s.error = err_text(&err);
                s.last_target = t_err.clone();
                s.loading = false;
                true
            });
            if current {
                push_packs(ui);
            }
        },
    );
}

fn load_shaders() {
    let Some(c) = super::ctx() else { return };
    let target = target();
    let seq = with(|s| {
        s.shaders_seq += 1;
        s.sh_loading = true;
        s.sh_error.clear();
        s.shaders_seq
    });
    if target.is_empty() {
        with(|s| {
            s.shaders = None;
            s.sh_loading = false;
        });
        if let Some(ui) = win() {
            push_shaders(&ui);
        }
        return;
    }
    if let Some(ui) = win() {
        push_shaders(&ui);
    }
    c.call_with(
        move |e| async move { e.shaders_state(&target).await },
        move |ui, st| {
            let current = with(|s| {
                if s.shaders_seq != seq {
                    return false;
                }
                s.shaders = Some(st);
                s.sh_loading = false;
                true
            });
            if current {
                push_shaders(ui);
            }
        },
        move |ui, err| {
            let current = with(|s| {
                if s.shaders_seq != seq {
                    return false;
                }
                s.sh_error = err_text(&err);
                s.sh_loading = false;
                true
            });
            if current {
                push_shaders(ui);
            }
        },
    );
}

// ---------------------------------------------------------------- mutations

/// Start a pack mutation; false when one is already in flight.
fn begin_busy() -> bool {
    with(|s| !std::mem::replace(&mut s.busy, true))
}

/// Finish a mutation and refresh both panes' busy state.
fn end_busy(ui: &AppWindow) {
    with(|s| s.busy = false);
    push_packs(ui);
    push_shaders(ui);
}

/// Active resource-pack filenames, highest priority first (the engine's order).
fn active_order() -> Vec<String> {
    with(|s| s.packs.iter().filter(|p| p.enabled).map(|p| p.filename.clone()).collect())
}

/// Write the options.txt order, then re-list the packs.
fn write_order(order: Vec<String>) {
    let Some(c) = super::ctx() else { return };
    if !begin_busy() {
        return;
    }
    if let Some(ui) = win() {
        push_packs(&ui);
    }
    let target = target();
    c.call_with(
        move |e| async move { e.set_resource_packs_order(&target, &order).await },
        move |ui, ()| {
            end_busy(ui);
            load_packs();
            store::refresh_instances();
        },
        move |ui, err| {
            end_busy(ui);
            toast::err(err_text(&err));
            load_packs();
        },
    );
}

fn rp_toggle(filename: String) {
    let mut order = active_order();
    match order.iter().position(|f| *f == filename) {
        Some(pos) => {
            order.remove(pos);
        }
        // Enabling puts the pack on top, where the game gives it priority.
        None => order.insert(0, filename),
    }
    write_order(order);
}

/// Swap a pack with its neighbour; `dir` is -1 (up, higher priority) or 1 (down).
fn rp_move(filename: String, dir: i32) {
    let mut order = active_order();
    if let Some(i) = order.iter().position(|f| *f == filename) {
        let j = i as i32 + dir;
        if j >= 0 && (j as usize) < order.len() {
            order.swap(i, j as usize);
        }
    }
    write_order(order);
}

fn rp_unwrap(filename: String) {
    let Some(c) = super::ctx() else { return };
    if !begin_busy() {
        return;
    }
    if let Some(ui) = win() {
        push_packs(&ui);
    }
    let target = target();
    c.call_with(
        move |e| async move { e.unwrap_nested_resource_pack(&target, &filename).await },
        move |ui, r| {
            end_busy(ui);
            load_packs();
            toast::ok(i18n::tf("packs.unwrapped", &[("name", &r.filename)]));
        },
        move |ui, err| {
            end_busy(ui);
            toast::err(err_text(&err));
        },
    );
}

fn rp_delete(filename: String) {
    let Some(c) = super::ctx() else { return };
    if !begin_busy() {
        return;
    }
    if let Some(ui) = win() {
        ui.global::<LibraryBridge>().set_rp_delete_open(false);
        push_packs(&ui);
    }
    let target = target();
    let name = filename.clone();
    c.call_with(
        move |e| async move { e.delete_resource_pack(&target, &filename).await },
        move |ui, _| {
            end_busy(ui);
            with(|s| s.packs.retain(|p| p.filename != name));
            push_packs(ui);
            load_packs();
            toast::ok(i18n::tf("packs.deleted", &[("name", &name)]));
        },
        move |ui, err| {
            end_busy(ui);
            toast::err(err_text(&err));
        },
    );
}

fn rp_check_updates() {
    let Some(c) = super::ctx() else { return };
    let target = target();
    if target.is_empty() {
        return;
    }
    with(|s| s.checking = true);
    if let Some(ui) = win() {
        push_packs(&ui);
    }
    c.call_with(
        move |e| async move { e.check_pack_updates(&target, PackKind::ResourcePack).await },
        move |ui, updates| {
            let n = updates.len();
            with(|s| {
                s.checking = false;
                s.rp_updates = updates.into_iter().map(|u| (u.filename.clone(), u)).collect();
            });
            push_packs(ui);
            toast::info(updates_text(n));
        },
        move |ui, err| {
            with(|s| s.checking = false);
            push_packs(ui);
            toast::err(err_text(&err));
        },
    );
}

fn updates_text(n: usize) -> String {
    if n == 0 {
        i18n::t("packs.updatesNone")
    } else {
        i18n::tf("packs.updatesFound", &[("count", &n.to_string())])
    }
}

fn rp_clear_server_cache() {
    let Some(c) = super::ctx() else { return };
    if !begin_busy() {
        return;
    }
    if let Some(ui) = win() {
        push_packs(&ui);
    }
    let target = target();
    c.call_with(
        move |e| async move { e.clear_server_pack_cache(&target).await },
        move |ui, info| {
            end_busy(ui);
            with(|s| s.cache = ServerPackCacheInfo { file_count: 0, total_bytes: 0 });
            push_packs(ui);
            toast::ok(i18n::tf("packs.cacheCleared", &[("size", &util::format_bytes(info.total_bytes as f64))]));
        },
        move |ui, err| {
            end_busy(ui);
            toast::err(err_text(&err));
        },
    );
}

// ---------------------------------------------------------------- copy from instance

fn rp_copy_from() {
    let Some(ui) = win() else { return };
    let target = target();
    let rows: Vec<RpCopyInstanceRow> = store::instances()
        .iter()
        .filter(|i| i.name != target)
        .map(|i| RpCopyInstanceRow { name: ss(&i.name), tag: ss(format!("{} · {}", i.loader.as_str(), i.version)) })
        .collect();
    with(|s| {
        s.copy_source.clear();
        s.copy_rows.clear();
        s.copy_loading = false;
    });
    let b = ui.global::<LibraryBridge>();
    b.set_rp_copy_instances(model(rows));
    b.set_rp_copy_keep_order(true);
    push_copy(&ui);
    b.set_rp_copy_open(true);
}

fn rp_copy_pick(source: String) {
    let Some(c) = super::ctx() else { return };
    with(|s| {
        s.copy_source = source.clone();
        s.copy_loading = true;
        s.copy_rows.clear();
    });
    if let Some(ui) = win() {
        push_copy(&ui);
    }
    c.call_with(
        move |e| async move { e.list_resource_packs(&source).await },
        move |ui, packs| {
            with(|s| {
                s.copy_loading = false;
                s.copy_rows = packs
                    .iter()
                    .map(|p| RpCopyPackRow {
                        filename: ss(&p.filename),
                        name: ss(if p.display_name.is_empty() { &p.filename } else { &p.display_name }),
                        enabled: p.enabled,
                        checked: true,
                    })
                    .collect();
            });
            push_copy(ui);
        },
        move |ui, err| {
            with(|s| {
                s.copy_loading = false;
                s.copy_source.clear();
            });
            push_copy(ui);
            toast::err(err_text(&err));
        },
    );
}

fn rp_copy_toggle(filename: String) {
    with(|s| {
        if let Some(r) = s.copy_rows.iter_mut().find(|r| r.filename.as_str() == filename) {
            r.checked = !r.checked;
        }
    });
    if let Some(ui) = win() {
        push_copy(&ui);
    }
}

fn rp_copy_cancel() {
    let Some(ui) = win() else { return };
    if with(|s| s.copy_busy) {
        return;
    }
    with(|s| {
        s.copy_source.clear();
        s.copy_rows.clear();
        s.copy_loading = false;
    });
    ui.global::<LibraryBridge>().set_rp_copy_open(false);
    push_copy(&ui);
}

fn rp_copy_confirm() {
    let Some(c) = super::ctx() else { return };
    let Some(ui) = win() else { return };
    if with(|s| s.copy_busy) {
        return;
    }
    let (source, files) = with(|s| {
        let files: Vec<String> = s.copy_rows.iter().filter(|r| r.checked).map(|r| r.filename.to_string()).collect();
        (s.copy_source.clone(), files)
    });
    let target = target();
    if target.is_empty() || source.is_empty() || files.is_empty() {
        return;
    }
    let keep_order = ui.global::<LibraryBridge>().get_rp_copy_keep_order();
    with(|s| s.copy_busy = true);
    push_copy(&ui);
    c.call_with(
        move |e| async move { e.copy_resource_packs_from_instance(&target, &source, files, keep_order).await },
        move |ui, res| {
            with(|s| {
                s.copy_busy = false;
                s.copy_source.clear();
                s.copy_rows.clear();
            });
            ui.global::<LibraryBridge>().set_rp_copy_open(false);
            push_copy(ui);
            load_packs();
            toast::ok(i18n::tf(
                "packs.copyDone",
                &[("count", &res.copied.len().to_string()), ("skipped", &res.skipped.len().to_string())],
            ));
        },
        move |ui, err| {
            with(|s| s.copy_busy = false);
            push_copy(ui);
            toast::err(err_text(&err));
        },
    );
}

// ---------------------------------------------------------------- shaders

fn sh_select(filename: String) {
    let Some(c) = super::ctx() else { return };
    if !begin_busy() {
        return;
    }
    if let Some(ui) = win() {
        push_shaders(&ui);
    }
    let target = target();
    c.call_with(
        move |e| async move { e.set_active_shader(&target, Some(filename)).await },
        move |ui, st| {
            end_busy(ui);
            with(|s| s.shaders = Some(st));
            push_shaders(ui);
        },
        move |ui, err| {
            end_busy(ui);
            toast::err(err_text(&err));
            load_shaders();
        },
    );
}

/// Master toggle. Turning it on re-activates the last pack (or the first one);
/// turning it off writes `enableShaders=false` and keeps the pick for later.
fn sh_set_enabled(on: bool) {
    let Some(c) = super::ctx() else { return };
    let pick = with(|s| {
        s.shaders
            .as_ref()
            .and_then(|st| st.active_pack.clone().or_else(|| st.packs.first().map(|p| p.filename.clone())))
    });
    if on && pick.is_none() {
        // Nothing to switch to: resync the toggle with the engine state.
        if let Some(ui) = win() {
            push_shaders(&ui);
        }
        return;
    }
    if !begin_busy() {
        return;
    }
    if let Some(ui) = win() {
        push_shaders(&ui);
    }
    let target = target();
    let shader = if on { pick } else { None };
    c.call_with(
        move |e| async move { e.set_active_shader(&target, shader).await },
        move |ui, st| {
            end_busy(ui);
            with(|s| s.shaders = Some(st));
            push_shaders(ui);
        },
        move |ui, err| {
            end_busy(ui);
            toast::err(err_text(&err));
            load_shaders();
        },
    );
}

/// One-click Iris install: Iris pulls Sodium through the normal Modrinth
/// dependency plan (the same commit the Modrinth tab uses).
fn sh_install_iris() {
    let Some(c) = super::ctx() else { return };
    let target = target();
    if target.is_empty() || with(|s| s.installing_iris) {
        return;
    }
    with(|s| s.installing_iris = true);
    if let Some(ui) = win() {
        push_shaders(&ui);
    }
    c.call_with(
        move |e| async move {
            e.commit_modrinth_install(&target, espectral_engine::mods::IRIS_PROJECT_ID, false, false, None, Vec::new())
                .await
        },
        move |ui, _plan| {
            with(|s| s.installing_iris = false);
            push_shaders(ui);
            toast::info(i18n::t("packs.irisInstalling"));
            load_shaders();
            store::refresh_instances();
        },
        move |ui, err| {
            with(|s| s.installing_iris = false);
            push_shaders(ui);
            toast::err(err_text(&err));
        },
    );
}

fn sh_delete(filename: String) {
    let Some(c) = super::ctx() else { return };
    if !begin_busy() {
        return;
    }
    if let Some(ui) = win() {
        ui.global::<LibraryBridge>().set_sh_delete_open(false);
        push_shaders(&ui);
    }
    let target = target();
    let name = filename.clone();
    c.call_with(
        move |e| async move { e.delete_shader_pack(&target, &filename).await },
        move |ui, _| {
            end_busy(ui);
            load_shaders();
            toast::ok(i18n::tf("packs.deleted", &[("name", &name)]));
        },
        move |ui, err| {
            end_busy(ui);
            toast::err(err_text(&err));
        },
    );
}

fn sh_check_updates() {
    let Some(c) = super::ctx() else { return };
    let target = target();
    if target.is_empty() {
        return;
    }
    c.call_with(
        move |e| async move { e.check_pack_updates(&target, PackKind::Shader).await },
        move |ui, updates| {
            let n = updates.len();
            with(|s| s.sh_updates = updates.into_iter().map(|u| (u.filename.clone(), u)).collect());
            push_shaders(ui);
            toast::info(updates_text(n));
        },
        move |_ui, err| toast::err(err_text(&err)),
    );
}

// ---------------------------------------------------------------- wiring

/// Wire the pane callbacks (called once from `library_mods::init`).
pub fn init(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    b.on_rp_set_filter(|f| {
        if let Some(ui) = win() {
            ui.global::<LibraryBridge>().set_rp_filter(f);
            push_packs(&ui);
        }
    });
    b.on_rp_set_query(|_q| {
        if let Some(ui) = win() {
            push_packs(&ui);
        }
    });
    b.on_rp_reload(load_packs);
    b.on_rp_toggle(|f| rp_toggle(f.to_string()));
    b.on_rp_move_up(|f| rp_move(f.to_string(), -1));
    b.on_rp_move_down(|f| rp_move(f.to_string(), 1));
    b.on_rp_unwrap(|f| rp_unwrap(f.to_string()));
    b.on_rp_delete(|f| rp_delete(f.to_string()));
    b.on_rp_check_updates(rp_check_updates);
    b.on_rp_copy_from(rp_copy_from);
    b.on_rp_copy_pick(|name| rp_copy_pick(name.to_string()));
    b.on_rp_copy_toggle(|f| rp_copy_toggle(f.to_string()));
    b.on_rp_copy_confirm(rp_copy_confirm);
    b.on_rp_copy_cancel(rp_copy_cancel);
    b.on_rp_clear_server_cache(rp_clear_server_cache);
    b.on_sh_reload(load_shaders);
    b.on_sh_select(|f| sh_select(f.to_string()));
    b.on_sh_set_enabled(sh_set_enabled);
    b.on_sh_install_iris(sh_install_iris);
    b.on_sh_delete(|f| sh_delete(f.to_string()));
    b.on_sh_check_updates(sh_check_updates);
}

/// The ModCatalog tab changed to `tab`: load the pane's data if it is stale
/// (another instance, or never loaded). Other tabs need nothing.
pub fn on_tab(ui: &AppWindow, tab: i32) {
    match tab {
        1 => {
            let target = target();
            let stale = with(|s| s.last_target != target || (s.packs.is_empty() && !s.loading));
            if stale {
                load_packs();
            } else {
                push_packs(ui);
            }
        }
        2 => {
            let target = target();
            let stale = with(|s| s.last_target != target || (s.shaders.is_none() && !s.sh_loading));
            if stale {
                load_shaders();
            } else {
                push_shaders(ui);
            }
        }
        _ => {}
    }
}

/// The ModCatalog instance switched: drop both panes' data and reload the
/// visible one. The hidden pane reloads when its tab opens (see [`on_tab`]).
pub fn target_changed() {
    with(|s| {
        s.packs.clear();
        s.rp_updates.clear();
        s.shaders = None;
        s.sh_updates.clear();
        s.last_target.clear();
        s.copy_source.clear();
        s.copy_rows.clear();
    });
    let Some(ui) = win() else { return };
    let tab = ui.global::<LibraryBridge>().get_m_tab();
    on_tab(&ui, tab);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ranges() {
        let r = |min: (u32, u32), max: (u32, u32)| PackFormatRange { min, max };
        assert_eq!(format_range(&r((34, 0), (34, 0))), "34");
        assert_eq!(format_range(&r((34, 0), (46, 0))), "34–46");
        assert_eq!(format_range(&r((88, 0), (88, u32::MAX))), "88+");
        assert_eq!(format_range(&r((88, 1), (88, 2))), "88.1–88.2");
    }

    #[test]
    fn issue_statuses() {
        assert!(!has_issue(PackCompatStatus::Compatible));
        assert!(!has_issue(PackCompatStatus::Unknown));
        assert!(has_issue(PackCompatStatus::TooOld));
        assert!(has_issue(PackCompatStatus::NestedArchive));
    }
}
