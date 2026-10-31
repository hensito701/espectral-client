//! Wardrobe (#/armario, Wardrobe.svelte): per-account identity verification,
//! Discord link (system browser + poll), supporter / MVP flags, the
//! "share my cosmetics" consent, MVP-gated slot pickers with exclusivity,
//! local emotes toggle and the hidden players list.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use espectral_engine::download::FetchOptions;
use espectral_engine::types::{
    CosmeticsAccountState, CosmeticsPatch, CosmeticsState, IdentityAccount, IdentityState, PublicAccount, TokenKind,
};
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::{err_msg, valid_mc_name};
use crate::bridge::Ctx;
use crate::router::{self, Route};
use crate::{i18n, prefs, toast, util};
use crate::{AccountBridge, AppWindow, Page, WardrobeAccount, WardrobeItem, WardrobeSlot};

const PREF_ACCOUNT: &str = "wardrobe.account";
const LINK_POLL: Duration = Duration::from_millis(2000);
const LINK_POLL_MAX: u32 = 90;

#[derive(Default)]
struct State {
    accounts: Vec<PublicAccount>,
    avatars: HashMap<String, SharedPixelBuffer<Rgba8Pixel>>,
    identity: Option<IdentityState>,
    cosmetics: Option<CosmeticsState>,
    selected: String,
    // editable draft (per selected account)
    share: bool,
    emotes: bool,
    loadout: Vec<(String, Option<String>)>,
    hidden: Vec<String>,
    saving: bool,
    pending: Option<CosmeticsPatch>,
    link_gen: u64,
    discord_avatars: HashMap<String, SharedPixelBuffer<Rgba8Pixel>>,
    discord_loading: Vec<String>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::default());
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn ctx() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

fn ui() -> Option<AppWindow> {
    ctx().and_then(|c| c.ui())
}

fn bridge(ui: &AppWindow) -> AccountBridge<'_> {
    ui.global::<AccountBridge>()
}

fn norm(uuid: &str) -> String {
    uuid.replace('-', "").to_lowercase()
}

pub fn init(init_ctx: &Ctx, init_ui: &AppWindow) {
    CTX.with(|c| *c.borrow_mut() = Some(init_ctx.clone()));
    ST.with(|s| s.borrow_mut().selected = prefs::get_str(PREF_ACCOUNT).unwrap_or_default());
    let b = bridge(init_ui);
    b.on_w_load(load);
    b.on_w_select(|i| {
        let uuid = ST.with(|s| {
            s.borrow().identity.as_ref().and_then(|id| id.accounts.get(i.max(0) as usize)).map(|a| a.uuid.clone())
        });
        if let Some(uuid) = uuid {
            select(uuid);
        }
    });
    b.on_w_verify(verify);
    b.on_w_refresh_flags(refresh_flags);
    b.on_w_link(link_discord);
    b.on_w_cancel_link(|| {
        ST.with(|s| s.borrow_mut().link_gen += 1);
        if let Some(ui) = ui() {
            bridge(&ui).set_w_linking(false);
        }
    });
    b.on_w_unlink(unlink_discord);
    b.on_w_toggle_share(|on| {
        ST.with(|s| s.borrow_mut().share = on);
        publish();
        save(CosmeticsPatch { share: Some(on), ..Default::default() });
    });
    b.on_w_toggle_emotes(|on| {
        ST.with(|s| s.borrow_mut().emotes = on);
        publish();
        save(CosmeticsPatch { emotes: Some(on), ..Default::default() });
    });
    b.on_w_equip(|slot, item| equip(slot.to_string(), (!item.is_empty()).then(|| item.to_string())));
    b.on_w_add_hidden(|name| add_hidden(name.trim().to_string()));
    b.on_w_remove_hidden(|name| remove_hidden(name.to_string()));

    router::on_route(|ui, route: &Route| {
        if route.page == Page::Wardrobe {
            // fresh component (Svelte onMount): stop a link poll from a previous visit
            ST.with(|s| s.borrow_mut().link_gen += 1);
            let b = bridge(ui);
            b.set_w_linking(false);
            b.set_w_hidden_error("".into());
            load();
        }
    });
}

// ---------------------------------------------------------------------------
// load
// ---------------------------------------------------------------------------

fn load() {
    let Some(c) = ctx() else { return };
    if let Some(ui) = c.ui() {
        let b = bridge(&ui);
        b.set_w_loading(true);
        b.set_w_load_error("".into());
    }
    c.call_with(
        |e| async move {
            let (accounts, identity, cosmetics) =
                tokio::try_join!(e.list_accounts(), e.identity_state(), e.get_cosmetics())?;
            let mut avatars = HashMap::new();
            for a in &accounts {
                if a.has_avatar {
                    if let Some(buf) = e.account_avatar(&a.username).await.ok().and_then(|b| util::decode_png(&b)) {
                        avatars.insert(a.username.clone(), buf);
                    }
                }
            }
            Ok((accounts, avatars, identity, cosmetics))
        },
        |ui, (accounts, avatars, identity, cosmetics)| {
            ST.with(|s| {
                let mut s = s.borrow_mut();
                s.accounts = accounts;
                s.avatars = avatars;
                s.identity = Some(identity);
                s.cosmetics = Some(cosmetics);
            });
            bridge(ui).set_w_loading(false);
            fix_selection();
            hydrate();
            publish();
        },
        |ui, e| {
            let b = bridge(ui);
            b.set_w_loading(false);
            b.set_w_load_error(err_msg(&e).into());
        },
    );
}

/// `account = find(selectedUuid) ?? accounts[0]` + persist the selection.
fn fix_selection() {
    let sel = ST.with(|s| {
        let mut s = s.borrow_mut();
        let list = s.identity.as_ref().map(|i| i.accounts.clone()).unwrap_or_default();
        let found = list.iter().any(|a| a.uuid == s.selected);
        if !found {
            s.selected = list.first().map(|a| a.uuid.clone()).unwrap_or_default();
        }
        s.selected.clone()
    });
    if !sel.is_empty() {
        prefs::set(PREF_ACCOUNT, serde_json::Value::from(sel));
    }
}

fn select(uuid: String) {
    let same = ST.with(|s| s.borrow().selected == uuid);
    if same {
        return;
    }
    ST.with(|s| s.borrow_mut().selected = uuid);
    fix_selection();
    hydrate();
    publish();
}

fn selected_identity() -> Option<IdentityAccount> {
    ST.with(|s| {
        let s = s.borrow();
        s.identity.as_ref().and_then(|i| i.accounts.iter().find(|a| a.uuid == s.selected).cloned())
    })
}

/// Hydrate the editable draft from the server state of the selected account.
fn hydrate() {
    ST.with(|s| {
        let mut s = s.borrow_mut();
        let Some(cos) = s.cosmetics.as_ref() else { return };
        let Some(st) = cos.accounts.get(&s.selected).cloned() else { return };
        let slots = cos.catalog.slots.clone();
        s.share = st.share;
        s.emotes = st.emotes;
        s.hidden = st.hidden_players.clone();
        s.loadout = slots.iter().map(|slot| (slot.clone(), st.loadout.get(slot).cloned().flatten())).collect();
    });
}

// ---------------------------------------------------------------------------
// models
// ---------------------------------------------------------------------------

fn grant_exp(acc: &IdentityAccount, lang: i18n::Lang) -> String {
    let Some(exp) = acc.grant_exp else { return String::new() };
    let Some(dt) = chrono::DateTime::from_timestamp(exp, 0) else { return String::new() };
    let local = dt.with_timezone(&chrono::Local);
    match lang {
        // toLocaleString('en-US') / ('es-ES')
        i18n::Lang::En => local.format("%-m/%-d/%Y, %-I:%M:%S %p").to_string(),
        _ => local.format("%-d/%-m/%Y, %-H:%M:%S").to_string(),
    }
}

fn discord_avatar_url(d: &espectral_engine::types::IdentityDiscord) -> Option<String> {
    d.avatar.as_ref().map(|a| format!("https://cdn.discordapp.com/avatars/{}/{}.png?size=64", d.id, a))
}

/// Equipped items that would conflict with equipping `item` into `slot`.
fn conflicting(
    exclusive: &[(String, Vec<String>)],
    loadout: &[(String, Option<String>)],
    slot: &str,
    item: &str,
) -> Vec<(String, String)> {
    let get = |sl: &str| loadout.iter().find(|(s, _)| s == sl).and_then(|(_, v)| v.clone());
    let mut out = vec![];
    for (ex_slot, ids) in exclusive {
        if slot == ex_slot {
            for (other, equipped) in loadout {
                if let Some(eq) = equipped {
                    if other != slot && ids.contains(eq) {
                        out.push((other.clone(), eq.clone()));
                    }
                }
            }
        } else if ids.iter().any(|i| i == item) {
            if let Some(eq) = get(ex_slot) {
                out.push((ex_slot.clone(), eq));
            }
        }
    }
    out
}

/// Push the whole wardrobe state into AccountBridge.
fn publish() {
    let Some(ui) = ui() else { return };
    let b = bridge(&ui);
    let mut want_discord: Vec<String> = vec![];
    ST.with(|s| {
        let s = s.borrow();
        let ids = s.identity.as_ref().map(|i| i.accounts.clone()).unwrap_or_default();
        let rows: Vec<WardrobeAccount> = ids
            .iter()
            .map(|a| {
                let public = s.accounts.iter().find(|p| norm(&p.uuid) == a.uuid);
                let avatar = public.and_then(|p| s.avatars.get(&p.username)).cloned();
                let durl = a.discord.as_ref().and_then(discord_avatar_url);
                let davatar = durl.as_ref().and_then(|u| s.discord_avatars.get(u)).cloned();
                if let Some(u) = durl.filter(|u| !s.discord_avatars.contains_key(u)) {
                    want_discord.push(u);
                }
                WardrobeAccount {
                    uuid: a.uuid.as_str().into(),
                    username: a.username.as_str().into(),
                    verified: a.verified,
                    supporter: a.flags.iter().any(|f| f == "supporter"),
                    mvp: a.flags.iter().any(|f| f == "mvp"),
                    offline: public.is_some_and(|p| p.token_kind == TokenKind::Offline),
                    has_discord: a.discord.is_some(),
                    discord_username: a.discord.as_ref().map(|d| d.username.as_str()).unwrap_or("").into(),
                    has_discord_avatar: davatar.is_some(),
                    discord_avatar: davatar.map(util::to_image).unwrap_or_default(),
                    grant_exp_es: grant_exp(a, i18n::Lang::Es).into(),
                    grant_exp_en: grant_exp(a, i18n::Lang::En).into(),
                    has_avatar: avatar.is_some(),
                    avatar: avatar.map(util::to_image).unwrap_or_default(),
                }
            })
            .collect();
        let sel = ids.iter().position(|a| a.uuid == s.selected).map(|i| i as i32).unwrap_or(-1);
        b.set_w_accounts(ModelRc::new(VecModel::from(rows)));
        b.set_w_selected(sel);
        b.set_w_share(s.share);
        b.set_w_emotes(s.emotes);
        b.set_w_saving(s.saving);
        let hidden: Vec<SharedString> = s.hidden.iter().map(|h| SharedString::from(h.as_str())).collect();
        b.set_w_hidden(ModelRc::new(VecModel::from(hidden)));

        let (slots, emotes) = match s.cosmetics.as_ref() {
            Some(cos) => {
                let cat = &cos.catalog;
                let slots: Vec<WardrobeSlot> = cat
                    .slots
                    .iter()
                    .map(|slot| {
                        let equipped = s.loadout.iter().find(|(sl, _)| sl == slot).and_then(|(_, v)| v.clone());
                        let items: Vec<WardrobeItem> = cat
                            .items
                            .iter()
                            .filter(|i| &i.slot == slot)
                            .map(|i| WardrobeItem {
                                id: i.id.as_str().into(),
                                name_es: i.name.es.as_str().into(),
                                name_en: i.name.en.as_str().into(),
                                equipped: equipped.as_deref() == Some(i.id.as_str()),
                                conflict: !conflicting(&cat.exclusive, &s.loadout, slot, &i.id).is_empty(),
                            })
                            .collect();
                        let key = format!("wardrobe.slot.{slot}");
                        WardrobeSlot {
                            slot: slot.as_str().into(),
                            label_es: i18n::lookup(&key, i18n::Lang::Es).into(),
                            label_en: i18n::lookup(&key, i18n::Lang::En).into(),
                            empty: equipped.is_none(),
                            items: ModelRc::new(VecModel::from(items)),
                        }
                    })
                    .collect();
                let emotes: Vec<WardrobeItem> = cat
                    .emotes
                    .iter()
                    .map(|e| WardrobeItem {
                        id: e.id.as_str().into(),
                        name_es: e.name.es.as_str().into(),
                        name_en: e.name.en.as_str().into(),
                        equipped: false,
                        conflict: false,
                    })
                    .collect();
                (slots, emotes)
            }
            None => (vec![], vec![]),
        };
        b.set_w_slots(ModelRc::new(VecModel::from(slots)));
        b.set_w_emotes_catalog(ModelRc::new(VecModel::from(emotes)));
    });
    for url in want_discord {
        fetch_discord_avatar(url);
    }
}

fn fetch_discord_avatar(url: String) {
    let Some(c) = ctx() else { return };
    let fresh = ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.discord_loading.contains(&url) {
            false
        } else {
            s.discord_loading.push(url.clone());
            true
        }
    });
    if !fresh {
        return;
    }
    let engine = c.engine().clone();
    let c2 = c.clone();
    c.spawn(async move {
        let buf =
            engine.http().fetch_bytes(&url, &FetchOptions::default()).await.ok().and_then(|b| util::decode_png(&b));
        if let Some(buf) = buf {
            c2.on_ui(move |_ui| {
                ST.with(|s| s.borrow_mut().discord_avatars.insert(url, buf));
                publish();
            });
        }
    });
}

// ---------------------------------------------------------------------------
// identity
// ---------------------------------------------------------------------------

fn reload_identity(toast_verified: bool) {
    let Some(c) = ctx() else { return };
    c.call_with(
        |e| async move { e.identity_state().await },
        move |_ui, identity| {
            ST.with(|s| s.borrow_mut().identity = Some(identity));
            fix_selection();
            publish();
            if toast_verified {
                toast::ok(i18n::t("wardrobe.identity.verified"));
            }
        },
        |_ui, _e| { /* transient — keep the last good snapshot */ },
    );
}

fn save_error(e: &espectral_engine::EngineError) {
    toast::err(i18n::tf("wardrobe.saveError", &[("error", &err_msg(e))]));
}

fn verify() {
    let Some(ui) = ui() else { return };
    let Some(acc) = selected_identity() else { return };
    let b = bridge(&ui);
    if b.get_w_verifying() {
        return;
    }
    b.set_w_verifying(true);
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.verify_identity(&acc.uuid).await },
        |ui, _| {
            bridge(ui).set_w_verifying(false);
            reload_identity(true);
        },
        |ui, e| {
            bridge(ui).set_w_verifying(false);
            save_error(&e);
        },
    );
}

fn refresh_flags() {
    let Some(ui) = ui() else { return };
    let Some(acc) = selected_identity() else { return };
    let b = bridge(&ui);
    if b.get_w_refreshing() {
        return;
    }
    b.set_w_refreshing(true);
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.identity_refresh(&acc.uuid).await },
        |ui, _| {
            bridge(ui).set_w_refreshing(false);
            reload_identity(false);
        },
        |ui, e| {
            bridge(ui).set_w_refreshing(false);
            save_error(&e);
        },
    );
}

/// Verify first when needed, open the authorize URL in the system browser,
/// then poll the identity until the account shows a linked Discord.
fn link_discord() {
    let Some(ui) = ui() else { return };
    let Some(acc) = selected_identity() else { return };
    let b = bridge(&ui);
    if b.get_w_linking() {
        return;
    }
    b.set_w_linking(true);
    let gen = ST.with(|s| {
        let mut s = s.borrow_mut();
        s.link_gen += 1;
        s.link_gen
    });
    let Some(c) = ctx() else { return };
    let uuid = acc.uuid.clone();
    let verified = acc.verified;
    let c2 = c.clone();
    c.call_with(
        move |e| async move {
            if !verified {
                e.verify_identity(&uuid).await?;
            }
            let start = e.identity_discord_start_with_state(&uuid).await?;
            Ok((uuid, start.url))
        },
        move |ui, (uuid, url)| {
            if ST.with(|s| s.borrow().link_gen) != gen {
                return;
            }
            if !verified {
                reload_identity(false);
            }
            if open::that_detached(&url).is_err() {
                bridge(ui).set_w_linking(false);
                return;
            }
            poll_link(c2.clone(), uuid, gen);
        },
        |ui, e| {
            bridge(ui).set_w_linking(false);
            save_error(&e);
        },
    );
}

fn poll_link(c: Ctx, uuid: String, gen: u64) {
    let engine = c.engine().clone();
    let c2 = c.clone();
    c.spawn(async move {
        for _ in 0..LINK_POLL_MAX {
            tokio::time::sleep(LINK_POLL).await;
            let still = {
                let (tx, rx) = tokio::sync::oneshot::channel();
                c2.on_ui(move |_ui| {
                    let _ = tx.send(ST.with(|s| s.borrow().link_gen) == gen);
                });
                rx.await.unwrap_or(false)
            };
            if !still {
                return;
            }
            let Ok(identity) = engine.identity_state().await else { continue };
            let linked = identity.accounts.iter().find(|a| a.uuid == uuid).is_some_and(|a| a.discord.is_some());
            c2.on_ui(move |ui| {
                if ST.with(|s| s.borrow().link_gen) != gen {
                    return;
                }
                ST.with(|s| s.borrow_mut().identity = Some(identity));
                publish();
                if linked {
                    bridge(ui).set_w_linking(false);
                    toast::ok(i18n::t("wardrobe.discord.linked"));
                }
            });
            if linked {
                return;
            }
        }
        c2.on_ui(move |ui| {
            if ST.with(|s| s.borrow().link_gen) == gen {
                bridge(ui).set_w_linking(false);
            }
        });
    });
}

fn unlink_discord() {
    let Some(ui) = ui() else { return };
    let Some(acc) = selected_identity() else { return };
    let b = bridge(&ui);
    if b.get_w_unlinking() {
        return;
    }
    b.set_w_unlinking(true);
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.identity_discord_unlink(&acc.uuid).await },
        |ui, _| {
            bridge(ui).set_w_unlinking(false);
            reload_identity(false);
        },
        |ui, e| {
            bridge(ui).set_w_unlinking(false);
            save_error(&e);
        },
    );
}

// ---------------------------------------------------------------------------
// cosmetics persistence (edits during an in-flight PUT are merged and flushed)
// ---------------------------------------------------------------------------

fn merge(into: &mut CosmeticsPatch, p: CosmeticsPatch) {
    if p.share.is_some() {
        into.share = p.share;
    }
    if p.loadout.is_some() {
        into.loadout = p.loadout;
    }
    if p.hidden_players.is_some() {
        into.hidden_players = p.hidden_players;
    }
    if p.emotes.is_some() {
        into.emotes = p.emotes;
    }
}

fn save(patch: CosmeticsPatch) {
    let Some(uuid) = selected_identity().map(|a| a.uuid) else { return };
    let busy = ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.saving {
            match s.pending.as_mut() {
                Some(p) => merge(p, patch.clone()),
                None => s.pending = Some(patch.clone()),
            }
            true
        } else {
            s.saving = true;
            false
        }
    });
    if busy {
        return;
    }
    if let Some(ui) = ui() {
        bridge(&ui).set_w_saving(true);
    }
    put(uuid, patch);
}

fn put(uuid: String, patch: CosmeticsPatch) {
    let Some(c) = ctx() else { return };
    let u2 = uuid.clone();
    c.call_with(
        move |e| async move { e.put_cosmetics(&uuid, patch).await },
        move |_ui, resp: CosmeticsAccountState| {
            let next = ST.with(|s| {
                let mut s = s.borrow_mut();
                if let Some(cos) = s.cosmetics.as_mut() {
                    cos.accounts.insert(u2.clone(), resp);
                }
                s.pending.take()
            });
            match next {
                Some(p) => put(u2, p),
                None => {
                    ST.with(|s| s.borrow_mut().saving = false);
                    toast::ok(i18n::t("wardrobe.saved"));
                    hydrate_if_selected(&u2);
                    publish();
                }
            }
        },
        |_ui, e| {
            save_error(&e);
            // revert the optimistic draft to the server state, then flush anything queued
            let Some(c) = ctx() else { return };
            c.call_with(
                |e| async move { e.get_cosmetics().await },
                |_ui, cos| {
                    ST.with(|s| s.borrow_mut().cosmetics = Some(cos));
                    finish_after_error();
                },
                |_ui, _e| finish_after_error(),
            );
        },
    );
}

fn finish_after_error() {
    let queued = ST.with(|s| {
        let mut s = s.borrow_mut();
        s.saving = false;
        s.pending.take()
    });
    hydrate();
    publish();
    if let Some(p) = queued {
        save(p);
    }
}

fn hydrate_if_selected(uuid: &str) {
    if ST.with(|s| s.borrow().selected == uuid) {
        hydrate();
    }
}

fn item_name(id: &str) -> String {
    ST.with(|s| {
        let s = s.borrow();
        s.cosmetics
            .as_ref()
            .and_then(|c| c.catalog.items.iter().find(|i| i.id == id))
            .map(|i| if i18n::lang() == i18n::Lang::En { i.name.en.clone() } else { i.name.es.clone() })
            .unwrap_or_else(|| id.to_string())
    })
}

fn equip(slot: String, item: Option<String>) {
    let (next, dropped) = ST.with(|s| {
        let mut s = s.borrow_mut();
        let exclusive = s.cosmetics.as_ref().map(|c| c.catalog.exclusive.clone()).unwrap_or_default();
        let mut dropped = vec![];
        if let Some(item) = &item {
            for (cslot, citem) in conflicting(&exclusive, &s.loadout, &slot, item) {
                if let Some(e) = s.loadout.iter_mut().find(|(sl, _)| *sl == cslot) {
                    e.1 = None;
                }
                dropped.push(citem);
            }
        }
        match s.loadout.iter_mut().find(|(sl, _)| *sl == slot) {
            Some(e) => e.1 = item.clone(),
            None => s.loadout.push((slot.clone(), item.clone())),
        }
        (s.loadout.clone(), dropped)
    });
    for d in dropped {
        toast::info(i18n::tf("wardrobe.slots.unequipped", &[("name", &item_name(&d))]));
    }
    publish();
    save(CosmeticsPatch { loadout: Some(next.into_iter().collect()), ..Default::default() });
}

fn add_hidden(name: String) {
    let Some(ui) = ui() else { return };
    let b = bridge(&ui);
    if !valid_mc_name(&name) {
        b.set_w_hidden_error(i18n::t("wardrobe.hidden.invalid").into());
        return;
    }
    b.set_w_hidden_error("".into());
    b.set_w_hidden_serial(b.get_w_hidden_serial() + 1); // clears the input
    let next = ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.hidden.iter().any(|h| h.eq_ignore_ascii_case(&name)) {
            return None;
        }
        s.hidden.push(name);
        Some(s.hidden.clone())
    });
    if let Some(next) = next {
        publish();
        save(CosmeticsPatch { hidden_players: Some(next), ..Default::default() });
    }
}

fn remove_hidden(name: String) {
    let next = ST.with(|s| {
        let mut s = s.borrow_mut();
        s.hidden.retain(|h| *h != name);
        s.hidden.clone()
    });
    publish();
    save(CosmeticsPatch { hidden_players: Some(next), ..Default::default() });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusivity() {
        let ex = vec![("cuello".to_string(), vec!["capa".to_string(), "alas".to_string()])];
        let lo = vec![
            ("espalda".to_string(), Some("capa".to_string())),
            ("cuello".to_string(), None),
            ("cabeza".to_string(), Some("gorro".to_string())),
        ];
        // equipping into the exclusive slot drops the listed ids worn elsewhere
        assert_eq!(conflicting(&ex, &lo, "cuello", "bufanda"), vec![("espalda".into(), "capa".into())]);
        // equipping a listed id drops what is worn in the exclusive slot
        let lo2 = vec![("espalda".to_string(), None), ("cuello".to_string(), Some("bufanda".to_string()))];
        assert_eq!(conflicting(&ex, &lo2, "espalda", "alas"), vec![("cuello".into(), "bufanda".into())]);
        assert!(conflicting(&ex, &lo2, "cabeza", "gorro").is_empty());
    }
}
