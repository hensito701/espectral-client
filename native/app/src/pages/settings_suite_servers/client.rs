//! ClientSuite (#/client): in-game feature toggles, suite master switch, HUD
//! theme and the macro editor of the selected instance's client config.
//!
//! The engine's `ClientInfo` and the macro draft live here (UI thread); the
//! Slint side only gets flat rows. Text edits of the macro draft never rebuild
//! the models (that would drop the focused TextInput), only structural edits do.

use std::cell::RefCell;

use espectral_engine::events::EngineEvent;
use espectral_engine::types::{ClientInfo, ClientMacro, ClientMacroAction, ClientPatch, PersonalizationPatch};
use serde_json::{json, Map, Value};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::err_text;
use crate::bridge::{self, Ctx};
use crate::router::{self, Route};
use crate::{
    i18n, prefs, store, toast, AppWindow, Page, SettingsBridge, SuiteCategoryRow, SuiteFeatureRow, SuiteMacroAction,
    SuiteMacroPill,
};

/// Engine macro constraints (mirrored client-side to prevent 400s).
const MAX_MACROS: usize = 32;
const MAX_MACRO_NAME: usize = 40;
const MAX_KEYBIND: usize = 32;
const MAX_ACTIONS: usize = 16;
const MAX_ACTION_TEXT: usize = 256;
const PREF_INSTANCE: &str = "client.instance";

#[derive(Default)]
struct State {
    info: Option<ClientInfo>,
    macros: Vec<ClientMacro>,
    active: usize,
    dirty: bool,
    load_seq: u64,
    /// Instance the last load was started for ("" = none).
    loaded_for: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let b = ui.global::<SettingsBridge>();

    let c = ctx.clone();
    b.on_client_ensure(move || ensure_instance(&c, false));

    let c = ctx.clone();
    b.on_client_select(move |name| {
        let Some(ui) = c.ui() else { return };
        if name.is_empty() {
            return;
        }
        prefs::set(PREF_INSTANCE, Value::from(name.as_str()));
        ui.global::<SettingsBridge>().set_client_instance(name.clone());
        set_instance_meta(&ui, &name);
        load(&c, false);
    });

    let c = ctx.clone();
    b.on_client_reload(move || load(&c, false));

    let c = ctx.clone();
    b.on_client_filter_changed(move || {
        if let Some(ui) = c.ui() {
            push_info(&ui);
        }
    });

    let c = ctx.clone();
    b.on_client_toggle_feature(move |id, next| toggle_feature(&c, id.to_string(), next));

    let c = ctx.clone();
    b.on_client_toggle_suite(move |next| toggle_suite(&c, next));

    let c = ctx.clone();
    b.on_client_reset(move || reset_defaults(&c));

    let c = ctx.clone();
    b.on_client_set_hud(move |id| set_hud(&c, id.to_string()));

    // ---- macro draft ----
    let c = ctx.clone();
    b.on_macro_add(move || {
        with(|s| {
            if s.macros.len() >= MAX_MACROS {
                return;
            }
            let id = new_macro_id(&s.macros);
            s.macros.push(ClientMacro {
                id,
                name: String::new(),
                keybind: String::new(),
                actions: vec![ClientMacroAction { kind: "chat".into(), text: String::new() }],
            });
            s.active = s.macros.len() - 1;
            s.dirty = true;
        });
        clear_macro_msgs(&c);
        refresh_macros(&c, true);
    });

    let c = ctx.clone();
    b.on_macro_remove(move |i| {
        with(|s| {
            let i = i as usize;
            if i < s.macros.len() {
                s.macros.remove(i);
            }
            if s.active >= s.macros.len() {
                s.active = s.macros.len().saturating_sub(1);
            }
            s.dirty = true;
        });
        clear_macro_msgs(&c);
        refresh_macros(&c, true);
    });

    let c = ctx.clone();
    b.on_macro_select(move |i| {
        with(|s| {
            if (i as usize) < s.macros.len() {
                s.active = i as usize;
            }
        });
        refresh_macros(&c, true);
    });

    let c = ctx.clone();
    b.on_macro_set_name(move |name| {
        let name: String = name.chars().take(MAX_MACRO_NAME).collect();
        let row = with(|s| {
            let a = s.active;
            let m = s.macros.get_mut(a)?;
            m.name = name;
            s.dirty = true;
            Some((a, pill(m)))
        });
        clear_macro_msgs(&c);
        // update just the pill (the name input keeps focus)
        if let (Some((a, p)), Some(ui)) = (row, c.ui()) {
            let model = ui.global::<SettingsBridge>().get_macros();
            if a < model.row_count() {
                model.set_row_data(a, p);
            }
        }
        refresh_macros(&c, false);
    });

    let c = ctx.clone();
    b.on_macro_capture_key(move |text| {
        let Some(kb) = key_to_keybind(&text) else { return false };
        let kb: String = kb.chars().take(MAX_KEYBIND).collect();
        with(|s| {
            let a = s.active;
            if let Some(m) = s.macros.get_mut(a) {
                m.keybind = kb;
                s.dirty = true;
            }
        });
        clear_macro_msgs(&c);
        refresh_macros(&c, true);
        true
    });

    let c = ctx.clone();
    b.on_action_add(move || {
        with(|s| {
            let a = s.active;
            if let Some(m) = s.macros.get_mut(a) {
                if m.actions.len() < MAX_ACTIONS {
                    m.actions.push(ClientMacroAction { kind: "chat".into(), text: String::new() });
                    s.dirty = true;
                }
            }
        });
        clear_saved(&c);
        refresh_macros(&c, true);
    });

    let c = ctx.clone();
    b.on_action_remove(move |i| {
        with(|s| {
            let a = s.active;
            if let Some(m) = s.macros.get_mut(a) {
                if m.actions.len() > 1 && (i as usize) < m.actions.len() {
                    m.actions.remove(i as usize);
                    s.dirty = true;
                }
            }
        });
        clear_saved(&c);
        refresh_macros(&c, true);
    });

    let c = ctx.clone();
    b.on_action_set_kind(move |i, kind| {
        with(|s| {
            let a = s.active;
            if let Some(act) = s.macros.get_mut(a).and_then(|m| m.actions.get_mut(i as usize)) {
                act.kind = kind.to_string();
                s.dirty = true;
            }
        });
        clear_macro_msgs(&c);
        refresh_macros(&c, true);
    });

    let c = ctx.clone();
    b.on_action_set_text(move |i, text| {
        let text: String = text.chars().take(MAX_ACTION_TEXT).collect();
        with(|s| {
            let a = s.active;
            if let Some(act) = s.macros.get_mut(a).and_then(|m| m.actions.get_mut(i as usize)) {
                act.text = text;
                s.dirty = true;
            }
        });
        clear_macro_msgs(&c);
        refresh_macros(&c, false);
    });

    let c = ctx.clone();
    b.on_macros_save(move || save_macros(&c));

    // Svelte remounts the page per visit: re-pick + reload, drop transient flags.
    let c = ctx.clone();
    router::on_route(move |ui, route: &Route| {
        if route.page == Page::Client {
            let b = ui.global::<SettingsBridge>();
            b.set_client_search("".into());
            b.set_client_category("all".into());
            b.set_client_confirming_reset(false);
            b.set_client_patch_error("".into());
            with(|s| s.active = 0);
            ensure_instance(&c, true);
        }
    });

    // Contract A: refresh the mirror after a game exits (keep a dirty macro draft).
    let c = ctx.clone();
    bridge::on_event(move |_ui, ev| {
        if let EngineEvent::LaunchExit(_) = ev {
            if crate::router::current().page == Page::Client {
                load(&c, true);
            }
        }
    });
}

/// Keep the selection valid: prefs choice if it still exists, else the first
/// instance; (re)load when the target changed or `force` (route entered).
fn ensure_instance(ctx: &Ctx, force: bool) {
    let Some(ui) = ctx.ui() else { return };
    let list = store::instances();
    let b = ui.global::<SettingsBridge>();
    if list.is_empty() {
        b.set_client_instance("".into());
        if ui.global::<crate::AppState>().get_instances_loaded() {
            b.set_client_loading(false);
        } else if crate::router::current().page == Page::Client {
            // the shared instance list is still loading: look again shortly
            let c = ctx.clone();
            slint::Timer::single_shot(std::time::Duration::from_millis(150), move || ensure_instance(&c, force));
        }
        return;
    }
    let mut target = b.get_client_instance().to_string();
    if target.is_empty() {
        target = prefs::get_str(PREF_INSTANCE).unwrap_or_default();
    }
    if !list.iter().any(|i| i.name == target) {
        target = list[0].name.clone();
    }
    b.set_client_instance(target.as_str().into());
    prefs::set(PREF_INSTANCE, Value::from(target.as_str()));
    set_instance_meta(&ui, &target);
    let stale = with(|s| s.loaded_for != target);
    if force || stale {
        load(ctx, false);
    }
}

fn set_instance_meta(ui: &AppWindow, name: &str) {
    let b = ui.global::<SettingsBridge>();
    if let Some(i) = store::instances().into_iter().find(|i| i.name == name) {
        b.set_client_loader(i.loader.as_str().into());
        b.set_client_version(i.version.as_str().into());
    } else {
        b.set_client_loader("".into());
        b.set_client_version("".into());
    }
}

fn load(ctx: &Ctx, skip_macros_if_dirty: bool) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    let name = b.get_client_instance().to_string();
    if name.is_empty() {
        return;
    }
    let seq = with(|s| {
        s.load_seq += 1;
        s.loaded_for = name.clone();
        s.load_seq
    });
    b.set_client_loading(true);
    b.set_client_error("".into());
    b.set_client_patch_error("".into());
    b.set_macro_saved_msg("".into());
    b.set_client_confirming_reset(false);
    ctx.call_with(
        move |e| async move { e.get_client_info(&name).await },
        move |ui, info| {
            if with(|s| s.load_seq) != seq {
                return; // a newer load won the race
            }
            with(|s| {
                if !skip_macros_if_dirty || !s.dirty {
                    s.macros = info.config.macros.clone();
                    s.dirty = false;
                    if s.active >= s.macros.len() {
                        s.active = 0;
                    }
                }
                s.info = Some(info);
            });
            let b = ui.global::<SettingsBridge>();
            b.set_client_loading(false);
            push_info(ui);
            refresh_macros_ui(ui, true);
        },
        move |ui, e| {
            if with(|s| s.load_seq) != seq {
                return;
            }
            with(|s| s.info = None);
            let b = ui.global::<SettingsBridge>();
            b.set_client_loading(false);
            b.set_client_has_info(false);
            b.set_client_error(err_text(&e).into());
        },
    );
}

/// `t(key, fallback)`: the fallback when the key is in no table.
fn t_or(key: &str, fallback: &str) -> String {
    let v = i18n::t(key);
    if v == key {
        fallback.to_string()
    } else {
        v
    }
}

/// Accent-insensitive fold so "musica" matches "música".
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

fn feature_enabled(info: &ClientInfo, id: &str, default: bool) -> bool {
    info.config.features.get(id).and_then(|v| v.get("enabled")).and_then(Value::as_bool).unwrap_or(default)
}

fn suite_enabled(info: &ClientInfo) -> bool {
    info.config.suite.as_ref().and_then(|s| s.get("enabled")).and_then(Value::as_bool).unwrap_or(true)
}

/// Mirror `info` (features filtered by search/category, suite, HUD theme) into the bridge.
fn push_info(ui: &AppWindow) {
    let b = ui.global::<SettingsBridge>();
    let rows = STATE.with(|s| {
        let s = s.borrow();
        let info = s.info.as_ref()?;
        let q = fold(b.get_client_search().trim());
        let cat = b.get_client_category().to_string();
        let mut cats: Vec<String> = Vec::new();
        for f in &info.registry {
            if let Some(c) = f.category.as_deref().filter(|c| !c.is_empty()) {
                if !cats.iter().any(|x| x == c) {
                    cats.push(c.to_string());
                }
            }
        }
        let feats: Vec<SuiteFeatureRow> = info
            .registry
            .iter()
            .filter(|f| cat == "all" || f.category.as_deref().unwrap_or("") == cat)
            .filter_map(|f| {
                let name = t_or(&format!("client.feat.{}.name", f.id), &f.name);
                let desc = t_or(&format!("client.feat.{}.desc", f.id), &f.description);
                if !q.is_empty() && !fold(&name).contains(&q) && !fold(&desc).contains(&q) && !fold(&f.id).contains(&q)
                {
                    return None;
                }
                Some(SuiteFeatureRow {
                    id: f.id.as_str().into(),
                    name: name.into(),
                    desc: desc.into(),
                    managed: f.kind == "managed",
                    on: feature_enabled(info, &f.id, f.default_enabled),
                })
            })
            .collect();
        let hud = info
            .config
            .personalization
            .as_ref()
            .and_then(|p| p.get("hud_theme"))
            .and_then(Value::as_str)
            .unwrap_or("cristal")
            .to_string();
        Some((feats, cats, info.registry.len(), info.supported, suite_enabled(info), hud))
    });
    let Some((feats, cats, total, supported, suite, hud)) = rows else {
        b.set_client_has_info(false);
        return;
    };
    let mut cat_rows = vec![SuiteCategoryRow { id: "all".into(), label: i18n::t("client.categoryAll").into() }];
    cat_rows.extend(
        cats.iter().map(|c| SuiteCategoryRow {
            id: c.as_str().into(),
            label: t_or(&format!("client.category.{c}"), c).into(),
        }),
    );
    b.set_client_features(ModelRc::new(VecModel::from(feats)));
    b.set_client_categories(ModelRc::new(VecModel::from(cat_rows)));
    b.set_client_feature_total(total as i32);
    b.set_client_supported(supported);
    b.set_client_suite_enabled(suite);
    b.set_client_hud_theme(hud.into());
    b.set_client_has_info(true);
}

/// Contract B: merge PATCH replies (never revert a switch: the engine persisted intent).
fn absorb_patch(resp: ClientInfo) {
    with(|s| {
        if !resp.registry.is_empty() {
            s.info = Some(resp);
        } else if let Some(info) = s.info.as_mut() {
            info.config = resp.config;
            info.errors = resp.errors;
        }
    });
}

fn warn_or_ok(resp: &ClientInfo, ok_text: String) {
    let ids: Vec<&str> = resp.errors.iter().flatten().map(|e| e.feature.as_str()).collect();
    if ids.is_empty() {
        toast::ok(ok_text);
    } else {
        toast::info(i18n::tf("client.savedWithWarnings", &[("features", &ids.join(", "))]));
    }
}

fn patch_client(
    ctx: &Ctx,
    patch: ClientPatch,
    ok: impl FnOnce(&AppWindow, &ClientInfo) -> String + 'static + Send,
    done: impl Fn(&AppWindow) + 'static + Send + Clone,
) {
    let Some(ui) = ctx.ui() else { return };
    let name = ui.global::<SettingsBridge>().get_client_instance().to_string();
    let done2 = done.clone();
    ctx.call_with(
        move |e| async move { e.patch_client(&name, patch).await },
        move |ui, resp| {
            done(ui);
            let text = ok(ui, &resp);
            warn_or_ok(&resp, text);
            absorb_patch(resp);
            push_info(ui);
        },
        move |ui, e| {
            done2(ui);
            let msg = err_text(&e);
            ui.global::<SettingsBridge>().set_client_patch_error(msg.as_str().into());
            toast::err(msg);
        },
    );
}

fn toggle_feature(ctx: &Ctx, id: String, next: bool) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if b.get_client_instance().is_empty() || !b.get_client_toggling_feature().is_empty() {
        push_info(&ui); // re-sync the switch the user flipped
        return;
    }
    b.set_client_toggling_feature(id.as_str().into());
    b.set_client_patch_error("".into());
    let mut features = Map::new();
    features.insert(id.clone(), json!({ "enabled": next }));
    let fallback_name = with(|s| {
        s.info.as_ref().and_then(|i| i.registry.iter().find(|f| f.id == id).map(|f| f.name.clone())).unwrap_or_default()
    });
    patch_client(
        ctx,
        ClientPatch { features: Some(Value::Object(features)), ..Default::default() },
        move |_ui, _| {
            let name = t_or(&format!("client.feat.{id}.name"), &fallback_name);
            let state = if next { t_or("mods.enabled", "activado") } else { t_or("mods.disabled", "desactivado") };
            format!("{name} {state}")
        },
        |ui| {
            ui.global::<SettingsBridge>().set_client_toggling_feature("".into());
            push_info(ui);
        },
    );
}

fn toggle_suite(ctx: &Ctx, next: bool) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if b.get_client_instance().is_empty() || b.get_client_toggling_suite() {
        push_info(&ui);
        return;
    }
    b.set_client_toggling_suite(true);
    b.set_client_patch_error("".into());
    patch_client(
        ctx,
        ClientPatch { suite: Some(json!({ "enabled": next })), ..Default::default() },
        move |_ui, _| i18n::t(if next { "client.suiteEnabled" } else { "client.suiteDisabled" }),
        |ui| {
            ui.global::<SettingsBridge>().set_client_toggling_suite(false);
            push_info(ui);
        },
    );
}

/// Every feature back to its registry default in ONE PATCH (master switch untouched).
fn reset_defaults(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if b.get_client_resetting() {
        return;
    }
    let Some(features) = with(|s| {
        s.info.as_ref().map(|i| {
            let mut m = Map::new();
            for f in &i.registry {
                m.insert(f.id.clone(), json!({ "enabled": f.default_enabled }));
            }
            m
        })
    }) else {
        return;
    };
    b.set_client_resetting(true);
    b.set_client_patch_error("".into());
    patch_client(
        ctx,
        ClientPatch { features: Some(Value::Object(features)), ..Default::default() },
        |ui, _| {
            ui.global::<SettingsBridge>().set_client_confirming_reset(false);
            i18n::t("client.resetDone")
        },
        |ui| ui.global::<SettingsBridge>().set_client_resetting(false),
    );
}

fn set_hud(ctx: &Ctx, id: String) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    let name = b.get_client_instance().to_string();
    if name.is_empty() || b.get_client_saving_hud() || b.get_client_hud_theme().as_str() == id {
        return;
    }
    b.set_client_saving_hud(true);
    b.set_client_patch_error("".into());
    ctx.call_with(
        move |e| async move {
            e.patch_personalization(&name, PersonalizationPatch { hud_theme: Some(Some(id)), ..Default::default() })
                .await
        },
        |ui, resp| {
            ui.global::<SettingsBridge>().set_client_saving_hud(false);
            absorb_patch(resp);
            push_info(ui);
            toast::ok(i18n::t("perso.hudSaveOk"));
        },
        |ui, e| {
            let b = ui.global::<SettingsBridge>();
            b.set_client_saving_hud(false);
            b.set_client_patch_error(err_text(&e).into());
            toast::err(i18n::t("perso.hudSaveError"));
        },
    );
}

// ---- macros -------------------------------------------------------------------------

fn new_macro_id(macros: &[ClientMacro]) -> String {
    let mut n = macros.len() + 1;
    loop {
        let id = format!("macro{n}");
        if !macros.iter().any(|m| m.id == id) && valid_id(&id) {
            return id;
        }
        n += 1;
    }
}

/// /^[A-Za-z0-9_-]{1,32}$/
fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn macros_valid(macros: &[ClientMacro]) -> bool {
    let mut ids: Vec<&str> = macros.iter().map(|m| m.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    macros.len() <= MAX_MACROS
        && ids.len() == macros.len()
        && macros.iter().all(|m| {
            valid_id(&m.id)
                && !m.name.trim().is_empty()
                && m.name.chars().count() <= MAX_MACRO_NAME
                && !m.keybind.trim().is_empty()
                && m.keybind.chars().count() <= MAX_KEYBIND
                && (1..=MAX_ACTIONS).contains(&m.actions.len())
                && m.actions.iter().all(|a| !a.text.trim().is_empty() && a.text.chars().count() <= MAX_ACTION_TEXT)
        })
}

fn pill(m: &ClientMacro) -> SuiteMacroPill {
    SuiteMacroPill {
        id: m.id.as_str().into(),
        label: if m.name.is_empty() { m.id.as_str().into() } else { m.name.as_str().into() },
        keybind: m.keybind.as_str().into(),
    }
}

fn clear_saved(ctx: &Ctx) {
    if let Some(ui) = ctx.ui() {
        ui.global::<SettingsBridge>().set_macro_saved_msg("".into());
    }
}

fn clear_macro_msgs(ctx: &Ctx) {
    if let Some(ui) = ctx.ui() {
        let b = ui.global::<SettingsBridge>();
        b.set_macro_saved_msg("".into());
        b.set_macro_error("".into());
    }
}

fn refresh_macros(ctx: &Ctx, structural: bool) {
    if let Some(ui) = ctx.ui() {
        refresh_macros_ui(&ui, structural);
    }
}

/// Flags always; models (pills, active macro fields, actions) only when `structural`.
fn refresh_macros_ui(ui: &AppWindow, structural: bool) {
    let b = ui.global::<SettingsBridge>();
    STATE.with(|s| {
        let s = s.borrow();
        b.set_macros_dirty(s.dirty);
        b.set_macros_valid(macros_valid(&s.macros));
        b.set_macro_count(s.macros.len() as i32);
        if !structural {
            return;
        }
        b.set_macros(ModelRc::new(VecModel::from(s.macros.iter().map(pill).collect::<Vec<_>>())));
        b.set_macro_active(s.active as i32);
        let active = s.macros.get(s.active);
        b.set_macro_name(active.map(|m| SharedString::from(m.name.as_str())).unwrap_or_default());
        b.set_macro_keybind(active.map(|m| SharedString::from(m.keybind.as_str())).unwrap_or_default());
        let actions: Vec<SuiteMacroAction> = active
            .map(|m| {
                m.actions
                    .iter()
                    .map(|a| SuiteMacroAction { kind: a.kind.as_str().into(), text: a.text.as_str().into() })
                    .collect()
            })
            .unwrap_or_default();
        b.set_macro_actions(ModelRc::new(VecModel::from(actions)));
        b.set_macro_serial(b.get_macro_serial().wrapping_add(1));
    });
}

fn save_macros(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    let macros = with(|s| s.macros.clone());
    if b.get_client_instance().is_empty() || !macros_valid(&macros) || b.get_saving_macros() {
        return;
    }
    b.set_saving_macros(true);
    b.set_macro_error("".into());
    b.set_macro_saved_msg("".into());
    let name = b.get_client_instance().to_string();
    let payload = serde_json::to_value(&macros).unwrap_or(Value::Array(vec![]));
    ctx.call_with(
        move |e| async move { e.patch_client(&name, ClientPatch { macros: Some(payload), ..Default::default() }).await },
        |ui, resp| {
            let b = ui.global::<SettingsBridge>();
            b.set_saving_macros(false);
            let saved = i18n::t("client.saved");
            warn_or_ok(&resp, saved.clone());
            absorb_patch(resp);
            with(|s| {
                if let Some(info) = &s.info {
                    s.macros = info.config.macros.clone();
                }
                s.dirty = false;
                if s.active >= s.macros.len() {
                    s.active = s.macros.len().saturating_sub(1);
                }
            });
            b.set_macro_saved_msg(saved.into());
            push_info(ui);
            refresh_macros_ui(ui, true);
        },
        |ui, e| {
            let b = ui.global::<SettingsBridge>();
            b.set_saving_macros(false);
            let msg = err_text(&e);
            b.set_macro_error(msg.as_str().into());
            toast::err(msg);
        },
    );
}

/// Keybind capture: Slint `KeyEvent.text` -> a MacroEngine-valid key name
/// (ClientSuite.svelte codeToKeybind). `None` = unknown key, stay armed.
fn key_to_keybind(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let named = match c {
        'a'..='z' => return Some(c.to_string()),
        'A'..='Z' => return Some(c.to_ascii_lowercase().to_string()),
        '0'..='9' => return Some(c.to_string()),
        '\u{F704}'..='\u{F712}' => return Some(format!("f{}", c as u32 - 0xF704 + 1)),
        ' ' => "space",
        '\n' | '\r' => "enter",
        '\t' => "tab",
        '\u{0008}' => "backspace",
        '\u{0010}' => "shift",
        '\u{0015}' => "right.shift",
        '\u{0011}' => "control",
        '\u{0016}' => "right.control",
        '\u{0012}' => "alt",
        '\u{0013}' => "right.alt",
        '-' | '_' => "minus",
        '=' | '+' => "equals",
        '[' | '{' => "lbracket",
        ']' | '}' => "rbracket",
        ';' | ':' => "semicolon",
        '\'' | '"' => "apostrophe",
        ',' | '<' => "comma",
        '.' | '>' => "period",
        '/' | '?' => "slash",
        '\\' | '|' => "backslash",
        '`' | '~' => "grave",
        _ => return None,
    };
    Some(named.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keybinds_map_like_the_svelte_capture() {
        assert_eq!(key_to_keybind("G").as_deref(), Some("g"));
        assert_eq!(key_to_keybind("7").as_deref(), Some("7"));
        assert_eq!(key_to_keybind("\u{F704}").as_deref(), Some("f1"));
        assert_eq!(key_to_keybind("\u{F712}").as_deref(), Some("f15"));
        assert_eq!(key_to_keybind(" ").as_deref(), Some("space"));
        assert_eq!(key_to_keybind("\u{0015}").as_deref(), Some("right.shift"));
        assert_eq!(key_to_keybind("\u{F700}"), None);
    }

    #[test]
    fn macro_validation() {
        let mut m = ClientMacro {
            id: "macro1".into(),
            name: "Hola".into(),
            keybind: "g".into(),
            actions: vec![ClientMacroAction { kind: "chat".into(), text: "hi".into() }],
        };
        assert!(macros_valid(std::slice::from_ref(&m)));
        m.name = "  ".into();
        assert!(!macros_valid(&[m]));
        assert_eq!(new_macro_id(&[]), "macro1");
        assert_eq!(fold("Música"), "musica");
    }
}
