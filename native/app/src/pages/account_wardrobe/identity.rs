//! Espectral identity of the account on the pedestal — the sections that used
//! to be the Armario page (#/armario): Minecraft verification, Discord link
//! (system browser + poll), supporter / MVP flags, the cosmetics pointer and
//! the hidden players list. They live on #/account now, follow the ACTIVE
//! account (the one the Cuenta page shows on the pedestal) and load together
//! with the vault when the route is entered.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use espectral_engine::download::FetchOptions;
use espectral_engine::types::{
    CosmeticsAccountState, CosmeticsPatch, CosmeticsState, IdentityAccount, IdentityState, TokenKind,
};
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::{err_msg, valid_mc_name};
use crate::bridge::Ctx;
use crate::router::{self, Route};
use crate::{i18n, toast, util};
use crate::{AccountBridge, AppWindow, EspectralIdentity, Page};

const LINK_POLL: Duration = Duration::from_millis(2000);
const LINK_POLL_MAX: u32 = 90;

#[derive(Default)]
struct State {
    identity: Option<IdentityState>,
    cosmetics: Option<CosmeticsState>,
    // editable draft (per active account): only hidden players remain here
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

/// Flat UUID of the account on the pedestal (level with the Cuenta roster).
fn active_flat() -> Option<String> {
    super::vault::active_account().map(|a| norm(&a.uuid))
}

/// The identity entry of the active account, if the snapshot knows it.
fn active_identity() -> Option<IdentityAccount> {
    let active = active_flat()?;
    ST.with(|s| {
        s.borrow().identity.as_ref().and_then(|i| i.accounts.iter().find(|a| a.uuid == active).cloned())
    })
}

pub fn init(init_ctx: &Ctx, init_ui: &AppWindow) {
    CTX.with(|c| *c.borrow_mut() = Some(init_ctx.clone()));
    let b = bridge(init_ui);
    b.on_identity_load(load);
    b.on_identity_verify(verify);
    b.on_identity_refresh_flags(refresh_flags);
    b.on_identity_link(link_discord);
    b.on_identity_cancel_link(|| {
        ST.with(|s| s.borrow_mut().link_gen += 1);
        if let Some(ui) = ui() {
            bridge(&ui).set_identity_linking(false);
        }
    });
    b.on_identity_unlink(unlink_discord);
    b.on_identity_add_hidden(|name| add_hidden(name.trim().to_string()));
    b.on_identity_remove_hidden(|name| remove_hidden(name.to_string()));

    router::on_route(|ui, route: &Route| {
        // any navigation invalidates a Discord poll started by the last visit
        ST.with(|s| s.borrow_mut().link_gen += 1);
        if route.page == Page::Account {
            let b = bridge(ui);
            b.set_identity_linking(false);
            b.set_identity_hidden_error("".into());
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
        b.set_identity_loading(true);
        b.set_identity_load_error("".into());
    }
    c.call_with(
        |e| async move {
            let (identity, cosmetics) = tokio::try_join!(e.identity_state(), e.get_cosmetics())?;
            Ok((identity, cosmetics))
        },
        |ui, (identity, cosmetics)| {
            ST.with(|s| {
                let mut s = s.borrow_mut();
                s.identity = Some(identity);
                s.cosmetics = Some(cosmetics);
            });
            bridge(ui).set_identity_loading(false);
            hydrate();
            publish();
        },
        |ui, e| {
            let b = bridge(ui);
            b.set_identity_loading(false);
            b.set_identity_load_error(err_msg(&e).into());
        },
    );
}

/// Called by the vault after the account list changed (switch, create, logout,
/// import): the identity sections follow the new account on the pedestal,
/// refetching the snapshot only when it does not know that account yet.
pub(super) fn on_accounts_changed() {
    // a fetch already in flight publishes the new active account when it lands
    if ui().is_some_and(|u| bridge(&u).get_identity_loading()) {
        return;
    }
    if ST.with(|s| s.borrow().identity.is_some()) && active_identity().is_none() && active_flat().is_some() {
        load();
        return;
    }
    hydrate();
    publish();
}

/// Hydrate the editable draft (the hidden players list) from the server state.
/// An account the snapshot does not know yet starts with an empty list, never
/// with the previous account's names.
fn hydrate() {
    let Some(active) = active_flat() else {
        ST.with(|s| s.borrow_mut().hidden.clear());
        return;
    };
    let next = ST.with(|s| {
        s.borrow()
            .cosmetics
            .as_ref()
            .and_then(|cos| cos.accounts.get(&active))
            .map(|st| st.hidden_players.clone())
            .unwrap_or_default()
    });
    ST.with(|s| s.borrow_mut().hidden = next);
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

/// Push the identity of the active account into AccountBridge.
fn publish() {
    let Some(ui) = ui() else { return };
    let b = bridge(&ui);
    let active = active_flat();
    let public = super::vault::active_account();
    let mut want_discord = vec![];
    let row = ST.with(|s| {
        let s = s.borrow();
        let acc = active
            .as_ref()
            .and_then(|u| s.identity.as_ref().and_then(|i| i.accounts.iter().find(|a| &a.uuid == u)))?;
        let durl = acc.discord.as_ref().and_then(discord_avatar_url);
        let davatar = durl.as_ref().and_then(|u| s.discord_avatars.get(u)).cloned();
        if let Some(u) = durl.filter(|u| !s.discord_avatars.contains_key(u)) {
            want_discord.push(u);
        }
        Some(EspectralIdentity {
            username: acc.username.as_str().into(),
            verified: acc.verified,
            supporter: acc.flags.iter().any(|f| f == "supporter"),
            mvp: acc.flags.iter().any(|f| f == "mvp"),
            offline: public.as_ref().is_some_and(|p| p.token_kind == TokenKind::Offline),
            has_discord: acc.discord.is_some(),
            discord_username: acc.discord.as_ref().map(|d| d.username.as_str()).unwrap_or("").into(),
            has_discord_avatar: davatar.is_some(),
            discord_avatar: davatar.map(util::to_image).unwrap_or_default(),
            grant_exp_es: grant_exp(acc, i18n::Lang::Es).into(),
            grant_exp_en: grant_exp(acc, i18n::Lang::En).into(),
        })
    });
    match &row {
        Some(row) => {
            b.set_identity(row.clone());
            b.set_identity_available(true);
        }
        None => b.set_identity_available(false),
    }
    let hidden: Vec<SharedString> = ST.with(|s| {
        if row.is_none() {
            return vec![];
        }
        s.borrow().hidden.iter().map(|h| SharedString::from(h.as_str())).collect()
    });
    b.set_identity_hidden(ModelRc::new(VecModel::from(hidden)));
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
    let Some(acc) = active_identity() else { return };
    let b = bridge(&ui);
    if b.get_identity_verifying() {
        return;
    }
    b.set_identity_verifying(true);
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.verify_identity(&acc.uuid).await },
        |ui, _| {
            bridge(ui).set_identity_verifying(false);
            reload_identity(true);
        },
        |ui, e| {
            bridge(ui).set_identity_verifying(false);
            save_error(&e);
        },
    );
}

fn refresh_flags() {
    let Some(ui) = ui() else { return };
    let Some(acc) = active_identity() else { return };
    let b = bridge(&ui);
    if b.get_identity_refreshing() {
        return;
    }
    b.set_identity_refreshing(true);
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.identity_refresh(&acc.uuid).await },
        |ui, _| {
            bridge(ui).set_identity_refreshing(false);
            reload_identity(false);
        },
        |ui, e| {
            bridge(ui).set_identity_refreshing(false);
            save_error(&e);
        },
    );
}

/// Verify first when needed, open the authorize URL in the system browser,
/// then poll the identity until the account shows a linked Discord.
fn link_discord() {
    let Some(ui) = ui() else { return };
    let Some(acc) = active_identity() else { return };
    let b = bridge(&ui);
    if b.get_identity_linking() {
        return;
    }
    b.set_identity_linking(true);
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
                bridge(ui).set_identity_linking(false);
                return;
            }
            poll_link(c2.clone(), uuid, gen);
        },
        |ui, e| {
            bridge(ui).set_identity_linking(false);
            if e.has_code("DISCORD_CALLBACK_UNAVAILABLE") {
                toast::err(i18n::t("gate.callbackBusy"));
            } else {
                save_error(&e);
            }
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
                    bridge(ui).set_identity_linking(false);
                    toast::ok(i18n::t("wardrobe.discord.linked"));
                }
            });
            if linked {
                return;
            }
        }
        c2.on_ui(move |ui| {
            if ST.with(|s| s.borrow().link_gen) == gen {
                bridge(ui).set_identity_linking(false);
            }
        });
    });
}

fn unlink_discord() {
    let Some(ui) = ui() else { return };
    let Some(acc) = active_identity() else { return };
    let b = bridge(&ui);
    if b.get_identity_unlinking() {
        return;
    }
    b.set_identity_unlinking(true);
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.identity_discord_unlink(&acc.uuid).await },
        |ui, _| {
            bridge(ui).set_identity_unlinking(false);
            reload_identity(false);
        },
        |ui, e| {
            bridge(ui).set_identity_unlinking(false);
            save_error(&e);
        },
    );
}

// ---------------------------------------------------------------------------
// hidden players persistence (edits during an in-flight PUT are merged and flushed)
// ---------------------------------------------------------------------------

fn merge(into: &mut CosmeticsPatch, p: CosmeticsPatch) {
    if p.hidden_players.is_some() {
        into.hidden_players = p.hidden_players;
    }
}

fn save(patch: CosmeticsPatch) {
    let Some(uuid) = active_flat() else { return };
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
                    hydrate_if_active(&u2);
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

fn hydrate_if_active(uuid: &str) {
    if active_flat().as_deref() == Some(uuid) {
        hydrate();
    }
}

fn add_hidden(name: String) {
    let Some(ui) = ui() else { return };
    let b = bridge(&ui);
    if !valid_mc_name(&name) {
        b.set_identity_hidden_error(i18n::t("wardrobe.hidden.invalid").into());
        return;
    }
    b.set_identity_hidden_error("".into());
    b.set_identity_hidden_serial(b.get_identity_hidden_serial() + 1); // clears the input
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
